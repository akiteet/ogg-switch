//! 会话用量同步公共内核
//!
//! Grok Build / Oh My Pi / Antigravity 三个解析器共用：同步游标批量预取、
//! 游标推进（含免锁的事务内版本）、同步结果聚合与通知、数据来源统计。
//!
//! Claude Code 的 JSONL 逐行导入器已随该 agent 下线移除；`read_tail_before` /
//! `push_committed_tail` 的等价实现仍在 `session_usage_omp.rs` 内。

use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::usage_stats::effective_usage_log_filter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;
use std::time::SystemTime;

/// 同步结果
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub suspected_duplicates: u32,
    pub deferred_files: u32,
    pub errors: Vec<String>,
}

impl SessionSyncResult {
    pub fn merge(&mut self, other: SessionSyncResult) {
        self.imported = self.imported.saturating_add(other.imported);
        self.skipped = self.skipped.saturating_add(other.skipped);
        self.files_scanned = self.files_scanned.saturating_add(other.files_scanned);
        self.suspected_duplicates = self
            .suspected_duplicates
            .saturating_add(other.suspected_duplicates);
        self.deferred_files = self.deferred_files.saturating_add(other.deferred_files);
        self.errors.extend(other.errors);
    }
}

pub fn session_sync_mutex() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// session_log_sync 表一行的内存快照。
///
/// 各解析器在一轮扫描开头用 [`load_sync_cursors`] 一次性预取全表，替代
/// 逐文件的单行查询（文件数随历史只增不减，逐文件查询意味着每轮上千次
/// 取锁）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SyncCursor {
    pub last_modified: i64,
    pub last_byte_offset: Option<i64>,
    /// 游标边界前尾部字节的指纹（仅 Claude 路径写入），用于识别文件被
    /// 外部重写；NULL 表示无指纹可校验。
    pub last_tail_fingerprint: Option<i64>,
}

/// 一次性预取 session_log_sync 全表游标。
///
/// 失败必须中止本轮同步，不能回退空表当新库处理：`rollup_and_prune` 会把
/// 30 天前的明细汇总后删除，request_id 去重只查明细表，对已剪条目失明——
/// 空表回退意味着全量重导，被剪的旧条目会在下次 rollup 时再次累加进汇总，
/// 永久放大统计。
pub(crate) fn load_sync_cursors(db: &Database) -> Result<HashMap<String, SyncCursor>, AppError> {
    let conn = lock_conn!(db.conn);
    let mut stmt = conn
        .prepare(
            "SELECT file_path, last_modified, last_byte_offset, last_tail_fingerprint
             FROM session_log_sync",
        )
        .map_err(|e| AppError::Database(format!("预取同步游标失败: {e}")))?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            SyncCursor {
                last_modified: row.get(1)?,
                last_byte_offset: row.get(2)?,
                last_tail_fingerprint: row.get(3)?,
            },
        ))
    });
    rows.and_then(|rows| rows.collect::<Result<HashMap<_, _>, _>>())
        .map_err(|e| AppError::Database(format!("预取同步游标失败: {e}")))
}

fn merge_sync_step(
    aggregate: &mut SessionSyncResult,
    name: &str,
    step: Result<SessionSyncResult, AppError>,
) {
    match step {
        Ok(result) => aggregate.merge(result),
        Err(error) => aggregate.errors.push(format!("{name} 同步失败: {error}")),
    }
}

/// 调用方必须持有 [`session_sync_mutex`]。此函数是同步内核，供后台任务、
/// 手动同步和 Codex 重建共享，避免 tokio Mutex 重入。
pub fn sync_all_unlocked(db: &Database) -> SessionSyncResult {
    // grok-switch 同步 Grok Build、Oh My Pi、Antigravity 的会话用量。
    let mut result = SessionSyncResult::default();
    merge_sync_step(
        &mut result,
        "Grok Build",
        crate::services::session_usage_grokbuild::sync_grokbuild_usage(db),
    );
    merge_sync_step(
        &mut result,
        "Oh My Pi",
        crate::services::session_usage_omp::sync_omp_usage(db),
    );
    merge_sync_step(
        &mut result,
        "Antigravity",
        crate::services::session_usage_antigravity::sync_antigravity_usage(db),
    );
    notify_sync_result(&result);
    result
}

pub(crate) fn notify_sync_result(result: &SessionSyncResult) {
    if result.imported > 0 {
        crate::usage_events::notify_log_recorded();
    }
}

/// 数据来源分布
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceSummary {
    pub data_source: String,
    pub request_count: u32,
    pub total_cost_usd: String,
}

/// 获取 session_log_sync 表中某条目的同步进度。
///
/// 生产路径已改为 [`load_sync_cursors`] 批量预取；保留此单行查询给测试
/// 断言游标状态用。
#[cfg(test)]
pub(crate) fn get_sync_state(db: &Database, file_path: &str) -> Result<(i64, i64), AppError> {
    let conn = lock_conn!(db.conn);
    let result = conn.query_row(
        "SELECT last_modified, last_line_offset FROM session_log_sync WHERE file_path = ?1",
        rusqlite::params![file_path],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    );
    Ok(result.unwrap_or((0, 0)))
}

/// 返回文件 mtime 的纳秒时间戳。
///
/// `session_log_sync.last_modified` 旧数据是秒级时间戳；新写入纳秒值不需要
/// schema 迁移，旧值会自然触发一次增量重扫，并继续依赖行 offset 避免重复导入。
pub(crate) fn metadata_modified_nanos(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// 更新 session_log_sync 表中某条目的同步进度。
///
/// Shared by all session_usage_* parsers.
pub(crate) fn update_sync_state(
    db: &Database,
    file_path: &str,
    last_modified: i64,
    last_offset: i64,
) -> Result<(), AppError> {
    let conn = lock_conn!(db.conn);
    update_sync_state_on_conn(&conn, file_path, last_modified, last_offset)
}

/// [`update_sync_state`] 的免锁版本，供调用方在已持锁的事务内把游标推进
/// 与数据插入绑成原子提交。
pub(crate) fn update_sync_state_on_conn(
    conn: &rusqlite::Connection,
    file_path: &str,
    last_modified: i64,
    last_offset: i64,
) -> Result<(), AppError> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    conn.prepare_cached(
        "INSERT OR REPLACE INTO session_log_sync (file_path, last_modified, last_line_offset, last_synced_at)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .and_then(|mut stmt| stmt.execute(rusqlite::params![file_path, last_modified, last_offset, now]))
    .map_err(|e| AppError::Database(format!("更新同步状态失败: {e}")))?;
    Ok(())
}

/// 查询数据来源分布统计
pub fn get_data_source_breakdown(db: &Database) -> Result<Vec<DataSourceSummary>, AppError> {
    let conn = lock_conn!(db.conn);

    let effective_filter = effective_usage_log_filter("l");
    let sql = format!(
        "SELECT COALESCE(l.data_source, 'proxy') as ds, COUNT(*) as cnt,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as cost
         FROM proxy_request_logs l
         WHERE {effective_filter}
         GROUP BY ds
         ORDER BY cnt DESC"
    );

    let mut stmt = conn.prepare(&sql)?;

    let rows = stmt.query_map([], |row| {
        Ok(DataSourceSummary {
            data_source: row.get(0)?,
            request_count: row.get::<_, i64>(1)? as u32,
            total_cost_usd: format!("{:.6}", row.get::<_, f64>(2)?),
        })
    })?;

    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| AppError::Database(e.to_string()))?);
    }

    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_result_notification_is_coalesced_to_one_call() {
        crate::usage_events::take_test_notify_count();
        notify_sync_result(&SessionSyncResult::default());
        let result = SessionSyncResult {
            imported: 25,
            ..SessionSyncResult::default()
        };
        notify_sync_result(&result);
        assert_eq!(crate::usage_events::take_test_notify_count(), 1);
    }

    #[tokio::test]
    async fn session_sync_mutex_serializes_callers() {
        let first = session_sync_mutex().lock().await;
        assert!(session_sync_mutex().try_lock().is_err());
        drop(first);
        assert!(session_sync_mutex().try_lock().is_ok());
    }
}
