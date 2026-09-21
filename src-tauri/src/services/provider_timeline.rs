//! 供应商切换时间线
//!
//! 用量归属用：会话导入时按「事件真实时间」查时间线，归到当时启用的供应商。
//!
//! ## 为什么需要
//! agy / OMP 这类本地会话记录里没有供应商字段（实测：agy conversation DB
//! 与 gen_metadata 无任何 baseUrl/provider 痕迹），而 OGG Switch 的切换
//! 只翻转 `is_current`、不留时间戳。没有时间线时，历史用量只能挂在
//! `_antigravity_session` 之类的占位来源，Dashboard 的"来源"维度不可用。
//!
//! ## 写入点
//! 1. `Database::set_current_provider`（真实切换动作，事务内）；
//! 2. 各同步器每轮"懒观测"：当前供应商与最新记录不同则补一条（覆盖
//!    绕过 OGG 的外部改动；时间精度为观测时刻，切换与观测之间的会话
//!    归到旧供应商——这是本地可得的极限精度）。
//!
//! ## 语义
//! - 只记录"某时刻起生效的供应商 id"，不记录名称（展示名由
//!   `provider_name_coalesce` JOIN providers 表解析）。
//! - 连续重复的 id 不重复记录。
//! - 设备本地数据：WebDAV 同步跳过（时间线是壁钟时序，跨设备合并会错乱）。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use rusqlite::{Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 在既有连接/事务上记录一次切换（幂等：与最新一条相同则跳过）。
pub(crate) fn record_switch_on_conn(
    conn: &Connection,
    app_type: &str,
    provider_id: &str,
) -> Result<(), AppError> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return Ok(());
    }
    let latest: Option<String> = conn
        .query_row(
            "SELECT provider_id FROM provider_switch_timeline
             WHERE app_type = ?1
             ORDER BY observed_at DESC, rowid DESC
             LIMIT 1",
            rusqlite::params![app_type],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| AppError::Database(format!("查询供应商时间线失败: {e}")))?;
    if latest.as_deref() == Some(provider_id) {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO provider_switch_timeline (app_type, provider_id, observed_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![app_type, provider_id, now_secs()],
    )
    .map_err(|e| AppError::Database(format!("写入供应商时间线失败: {e}")))?;
    Ok(())
}

/// [`record_switch_on_conn`] 的自持锁版本。
pub(crate) fn record_switch(
    db: &Database,
    app_type: &str,
    provider_id: &str,
) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    record_switch_on_conn(&conn, app_type, provider_id)
}

/// 返回 `at_secs` 时刻生效的供应商 id（`observed_at <= at_secs` 的最近一条）。
pub(crate) fn provider_at(
    db: &Database,
    app_type: &str,
    at_secs: i64,
) -> Result<Option<String>, AppError> {
    let conn = lock_conn!(db.conn);
    conn.query_row(
        "SELECT provider_id FROM provider_switch_timeline
         WHERE app_type = ?1 AND observed_at <= ?2
         ORDER BY observed_at DESC, rowid DESC
         LIMIT 1",
        rusqlite::params![app_type, at_secs],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| AppError::Database(format!("查询供应商时间线失败: {e}")))
}

/// 最新一条观测（provider_id, observed_at）。仅供测试断言使用。
#[cfg(test)]
pub(crate) fn latest_observation(
    db: &Database,
    app_type: &str,
) -> Result<Option<(String, i64)>, AppError> {
    let conn = lock_conn!(db.conn);
    conn.query_row(
        "SELECT provider_id, observed_at FROM provider_switch_timeline
         WHERE app_type = ?1
         ORDER BY observed_at DESC, rowid DESC
         LIMIT 1",
        rusqlite::params![app_type],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| AppError::Database(format!("查询供应商时间线失败: {e}")))
}

/// 懒观测：当前供应商与最新记录不同则记一条（观测时刻）。
pub(crate) fn observe_current(db: &Database, app_type: &str) -> Result<(), AppError> {
    let current = current_provider_id(db, app_type)?;
    observe_provider(db, app_type, current.as_deref())
}

/// 观测并记录一个已知的当前供应商（测试与调用方解耦 settings 用）。
pub(crate) fn observe_provider(
    db: &Database,
    app_type: &str,
    provider_id: Option<&str>,
) -> Result<(), AppError> {
    match provider_id {
        Some(id) if !id.trim().is_empty() => record_switch(db, app_type, id),
        _ => Ok(()),
    }
}

/// 当前供应商 id：设备级 settings 优先，DB `is_current` 兜底。
fn current_provider_id(db: &Database, app_type: &str) -> Result<Option<String>, AppError> {
    if let Ok(parsed) = app_type.parse::<crate::app_config::AppType>() {
        if let Some(id) = crate::settings::get_current_provider(&parsed) {
            let id = id.trim();
            if !id.is_empty() {
                return Ok(Some(id.to_string()));
            }
        }
    }
    db.get_current_provider(app_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_resolves_by_time() -> Result<(), AppError> {
        let db = Database::memory()?;
        record_switch(&db, "antigravity", "antigravity-official")?;
        // 相同 id 幂等
        record_switch(&db, "antigravity", "antigravity-official")?;
        let first = latest_observation(&db, "antigravity")?.expect("observation");
        assert_eq!(first.0, "antigravity-official");

        // 手动插入一条更晚的切换（模拟真实切换在 now 之前/之后）
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO provider_switch_timeline (app_type, provider_id, observed_at)
                 VALUES ('antigravity', 'packy', ?1)",
                rusqlite::params![first.1 + 100],
            )?;
            conn.execute(
                "INSERT INTO provider_switch_timeline (app_type, provider_id, observed_at)
                 VALUES ('antigravity', 'rigel', ?1)",
                rusqlite::params![first.1 + 200],
            )?;
        }

        // 切换前 → 旧供应商；切换后 → 新供应商；更晚 → 最新
        assert_eq!(
            provider_at(&db, "antigravity", first.1 + 50)?.as_deref(),
            Some("antigravity-official")
        );
        assert_eq!(
            provider_at(&db, "antigravity", first.1 + 150)?.as_deref(),
            Some("packy")
        );
        assert_eq!(
            provider_at(&db, "antigravity", first.1 + 300)?.as_deref(),
            Some("rigel")
        );
        // 早于任何观测 → None（历史无法归属）
        assert_eq!(provider_at(&db, "antigravity", first.1 - 1)?, None);
        Ok(())
    }

    #[test]
    fn observe_provider_records_once_and_ignores_none() -> Result<(), AppError> {
        let db = Database::memory()?;
        // 无当前供应商时不写时间线
        observe_provider(&db, "antigravity", None)?;
        observe_provider(&db, "antigravity", Some("  "))?;
        assert_eq!(latest_observation(&db, "antigravity")?, None);

        observe_provider(&db, "antigravity", Some("antigravity-official"))?;
        observe_provider(&db, "antigravity", Some("antigravity-official"))?;
        let (id, _) = latest_observation(&db, "antigravity")?.expect("observation");
        assert_eq!(id, "antigravity-official");
        let count: i64 = {
            let conn = lock_conn!(db.conn);
            conn.query_row(
                "SELECT COUNT(*) FROM provider_switch_timeline WHERE app_type = 'antigravity'",
                [],
                |row| row.get(0),
            )?
        };
        assert_eq!(count, 1, "重复观测不得重复写入");
        Ok(())
    }
}
