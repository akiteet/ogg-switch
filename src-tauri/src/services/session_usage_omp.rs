//! Oh My Pi (OMP) 会话用量追踪
//!
//! OMP 不走本地代理（`PROXY_APP_IDS` 不含 omp），统计只能来自 OMP 自己的记账。
//! `~/.omp/agent/agent.db` 的 `client_usage` 表是逐请求粒度：
//! `recorded_at / install_id / app / provider / model / requests /
//! input_tokens / output_tokens / cache_read_tokens / cache_write_tokens / cost_usd`。
//!
//! ## 数据流
//! ```text
//! ~/.omp/agent/agent.db:client_usage（只读） → id 高水位游标 → proxy_request_logs（app_type="omp"）
//! ```
//!
//! ## 游标（session_log_sync）
//! - `file_path`      = `omp:agent.db`（虚拟键，非真实路径）
//! - `last_modified`  = agent.db 文件 mtime（纳秒）；未变则整轮跳过
//! - `last_line_offset` = 已导入的 client_usage.id 高水位（id 单调递增）
//!
//! ## 口径
//! - agent.db 是 OMP 的真源数据：**全程 SQLITE_OPEN_READ_ONLY，绝不写回**。
//! - `cost_usd` 是 OMP 自报成本，直接作为 total_cost 入账（分项记 0）；
//!   本地定价表不匹配 OMP 的 `provider/model` 复合命名，不做本地复算。
//! - client_usage 的 input_tokens 与 cache_* 分列，input 语义按 FRESH
//!   （不含 cache）入账。
//! - OMP 无本地代理去重问题（不走 proxy），无需沉降窗/接管守卫。

use crate::config::get_home_dir;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::session_usage::{
    metadata_modified_nanos, update_sync_state, SessionSyncResult,
};
use crate::services::sql_helpers::INPUT_TOKEN_SEMANTICS_FRESH;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use std::fs;

const CURSOR_KEY: &str = "omp:agent.db";
const DATA_SOURCE: &str = "omp_session";
const PROVIDER_ID: &str = "_omp_session";

/// agent.db 路径（~/.omp/agent/agent.db）。
fn omp_agent_db() -> std::path::PathBuf {
    get_home_dir().join(".omp").join("agent").join("agent.db")
}

/// 读取游标（mtime 纳秒 + client_usage.id 高水位）。
fn load_cursor(db: &Database) -> Result<(i64, i64), AppError> {
    let conn = lock_conn!(db.conn);
    let result = conn.query_row(
        "SELECT last_modified, last_line_offset FROM session_log_sync WHERE file_path = ?1",
        rusqlite::params![CURSOR_KEY],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    );
    Ok(result.unwrap_or((0, 0)))
}

/// agent.db:client_usage 的一行（已裁剪成 OGG 需要的字段）。
struct OmpUsageRow {
    id: i64,
    recorded_at: i64,
    model: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    cost_usd: f64,
}

/// 从只读 agent.db 拉取 id > after_id 的用量行。
fn fetch_rows(agent_db: &std::path::Path, after_id: i64) -> Result<Vec<OmpUsageRow>, AppError> {
    let conn = rusqlite::Connection::open_with_flags(
        agent_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| AppError::Database(format!("只读打开 agent.db 失败: {e}")))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, recorded_at, model, input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens, cost_usd
             FROM client_usage WHERE id > ?1 ORDER BY id ASC",
        )
        .map_err(|e| AppError::Database(format!("查询 client_usage 失败: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![after_id], |row| {
            Ok(OmpUsageRow {
                id: row.get(0)?,
                recorded_at: row.get(1)?,
                model: row.get(2)?,
                input_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
                output_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                cache_read_tokens: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
                cache_write_tokens: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                cost_usd: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
            })
        })
        .map_err(|e| AppError::Database(format!("读取 client_usage 失败: {e}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(format!("遍历 client_usage 失败: {e}")))
}

/// 插入单行到 proxy_request_logs（幂等：request_id 带 agent.db 行 id）。
fn insert_omp_usage_entry(db: &Database, row: &OmpUsageRow) -> Result<bool, AppError> {
    let conn = lock_conn!(db.conn);

    let clamp = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    let model = row.model.clone().unwrap_or_else(|| "unknown".to_string());
    let cost = Decimal::from_f64(row.cost_usd).unwrap_or(Decimal::ZERO);

    // OMP 自报成本即 total；分项成本本地无法可靠拆分，记 0（token 照常入账）。
    conn.execute(
        "INSERT OR IGNORE INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source,
            input_token_semantics
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
        rusqlite::params![
            format!("omp_client_usage_{}", row.id),
            PROVIDER_ID,
            "omp",
            model,
            model, // request_model = model
            clamp(row.input_tokens),
            clamp(row.output_tokens),
            clamp(row.cache_read_tokens),
            clamp(row.cache_write_tokens), // OMP 的 cache write 对应 cache_creation
            "0",                           // input_cost_usd
            "0",                           // output_cost_usd
            "0",                           // cache_read_cost_usd
            "0",                           // cache_creation_cost_usd
            cost.to_string(),              // total = OMP 自报
            0i64,                          // latency_ms（client_usage 无该字段）
            Option::<i64>::None,           // first_token_ms
            200i64,                        // status_code
            Option::<String>::None,        // error_message
            Option::<String>::None,        // session_id（client_usage 无会话概念）
            Some(DATA_SOURCE),             // provider_type
            1i64,                          // is_streaming
            "1.0",                         // cost_multiplier
            row.recorded_at,               // created_at（秒级 epoch）
            DATA_SOURCE,
            INPUT_TOKEN_SEMANTICS_FRESH,
        ],
    )
    .map_err(|e| AppError::Database(format!("插入 OMP 用量日志失败: {e}")))?;

    Ok(conn.changes() > 0)
}

/// 同步 OMP 用量（调用方必须持有 [`session_sync_mutex`]）。
pub fn sync_omp_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_omp_usage_from(db, &omp_agent_db())
}

/// [`sync_omp_usage`] 的路径注入版本（测试用 tempfile，绝不碰真实 ~/.omp）。
fn sync_omp_usage_from(
    db: &Database,
    agent_db: &std::path::Path,
) -> Result<SessionSyncResult, AppError> {
    let mut result = SessionSyncResult::default();

    if !agent_db.exists() {
        return Ok(result);
    }

    let file_modified = metadata_modified_nanos(
        &fs::metadata(agent_db).map_err(|e| AppError::Message(format!("读取 agent.db 元数据失败: {e}")))?,
    );
    let (cursor_mtime, cursor_id) = load_cursor(db)?;
    if cursor_mtime == file_modified && cursor_id > 0 {
        return Ok(result);
    }

    let rows = fetch_rows(agent_db, cursor_id)?;
    let mut max_id = cursor_id;
    for row in &rows {
        max_id = max_id.max(row.id);
        // recorded_at 非法的行跳过（游标仍推进，避免每轮重复扫到）
        if row.recorded_at <= 0 {
            result.skipped += 1;
            continue;
        }
        if insert_omp_usage_entry(db, row)? {
            result.imported += 1;
        } else {
            result.skipped += 1;
        }
    }

    update_sync_state(db, CURSOR_KEY, file_modified, max_id)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_agent_db_is_noop() -> Result<(), AppError> {
        let db = Database::memory()?;
        let result = sync_omp_usage_from(&db, std::path::Path::new("Z:/definitely/missing/agent.db"))?;
        assert_eq!(result.imported, 0);
        Ok(())
    }
}
