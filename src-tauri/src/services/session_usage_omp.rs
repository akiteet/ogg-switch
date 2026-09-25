//! Oh My Pi (OMP) 会话用量追踪
//!
//! OMP 不走本地代理（`PROXY_APP_IDS` 不含 omp），统计只能来自 OMP 自己的记账。
//! 两个数据源：
//!
//! 1. `~/.omp/agent/sessions/**/*.jsonl` 的逐请求真值（**主源**）：每条
//!    assistant 消息带 `message.usage`（input/output/cacheRead/cacheWrite +
//!    cost）与 `message.provider`（SenseNova、Rigel、SUPER-NB、优云智算等
//!    第三方供应商 key）——按供应商写入 `provider_id`，来源维度即可用。
//!    实测 provider 的 input 不含 cacheRead（input+output+cacheRead =
//!    totalTokens），按 FRESH 语义入账。
//! 2. `~/.omp/agent/agent.db` 的 `client_usage` 表（备用）：本机实测 0 行，
//!    OMP 当前版本并不写入；保留导入路径以防上游版本开始写入。
//!
//! ## 数据流
//! ```text
//! ~/.omp/agent/sessions/**/*.jsonl（只读） → 字节游标 + 去重账本 → proxy_request_logs（app_type="omp"）
//! ~/.omp/agent/agent.db:client_usage（只读） → id 高水位游标 → proxy_request_logs（app_type="omp"）
//! ```
//!
//! ## 游标（session_log_sync）
//! - agent.db：`file_path` = `omp:agent.db`（虚拟键），`last_line_offset` =
//!   client_usage.id 高水位（id 单调递增）。
//! - 会话文件：`file_path` = 真实路径，`last_byte_offset` = 已提交行的字节
//!   偏移，`last_tail_fingerprint` = 游标前 4KB 尾部指纹（识别重写/截断）。
//!
//! ## 去重
//! 会话文件重写/截断时整文件重读，由持久账本
//! （`session_usage_dedup`，data_source="omp_session"，request_id =
//! `omp_session:{session_id}:{entry_id}`）保证幂等；明细被 rollup 剪除后
//! 重导也不会双算。
//!
//! ## 口径
//! - 两个源都**全程 SQLITE_OPEN_READ_ONLY，绝不写回**（jsonl 只读打开）。
//! - `cost` 优先用 OMP 自报值；自报为 0 时用本地定价表兜底计算。
//! - OMP 无本地代理去重问题（不走 proxy），无需沉降窗/接管守卫。

use crate::config::get_home_dir;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::usage::calculator::CostCalculator;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::{
    metadata_modified_nanos, update_sync_state, SessionSyncResult, SyncCursor,
};
use crate::services::sql_helpers::INPUT_TOKEN_SEMANTICS_FRESH;
use crate::services::usage_stats::find_model_pricing;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

const CURSOR_KEY: &str = "omp:agent.db";
const DATA_SOURCE: &str = "omp_session";
const PROVIDER_ID: &str = "_omp_session";
const APP_TYPE: &str = "omp";
const UNKNOWN_MODEL: &str = "unknown";
/// 与 session_usage.rs 的尾部指纹窗口同值。
const TAIL_FINGERPRINT_BYTES: u64 = 4096;
/// 单文件安全上限（实测最大 1.2MB；防御异常大文件拖垮同步）。
const OMP_MAX_SESSION_BYTES: u64 = 256 * 1024 * 1024;

/// 只读打开 SQLite：优先 file: URI，让 WAL 对只读连接可见；URI 失败再回退普通路径。
fn open_readonly_sqlite(path: &std::path::Path) -> Result<rusqlite::Connection, rusqlite::Error> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let unix = path.to_string_lossy().replace('\\', "/");
    let uri = if unix.starts_with('/') {
        format!("file:{unix}?mode=ro")
    } else {
        format!("file:///{unix}?mode=ro")
    };
    rusqlite::Connection::open_with_flags(&uri, flags).or_else(|_| {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    })
}

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
    let conn = open_readonly_sqlite(agent_db)
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
///
/// 源优先级：会话 JSONL（逐请求真值，`sessions/**/*.jsonl`）存在时独占导入；
/// 仅当会话存储整体缺席（老版本 OMP / 目录被清）才回退 agent.db:client_usage。
/// 两个源都是请求粒度、时间线重叠，同时导入必然双算，因此不做并行合并。
pub fn sync_omp_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let files = match crate::session_manager::providers::omp::session_files() {
        Ok(files) => files,
        Err(error) => {
            // 发现失败时本轮不导入任何源：宁可空转，也不回退到 client_usage 双算。
            let mut result = SessionSyncResult::default();
            result.errors.push(format!("OMP 会话发现失败: {error}"));
            return Ok(result);
        }
    };
    let mut result = if files.is_empty() {
        match sync_omp_usage_from(db, &omp_agent_db()) {
            Ok(result) => result,
            Err(e) => {
                let mut result = SessionSyncResult::default();
                result
                    .errors
                    .push(format!("OMP client_usage 同步失败: {e}"));
                result
            }
        }
    } else {
        SessionSyncResult::default()
    };
    result.merge(sync_omp_session_files(db, &files));
    Ok(result)
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
        &fs::metadata(agent_db)
            .map_err(|e| AppError::Message(format!("读取 agent.db 元数据失败: {e}")))?,
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

// ---------------------------------------------------------------------------
// 会话 JSONL 导入（主源：~/.omp/agent/sessions/**/*.jsonl）
// ---------------------------------------------------------------------------

/// OMP 自报成本（USD）。与 Pi 相同口径：分项 + 总计，自报为 0 时才用本地定价兜底。
#[derive(Debug, Clone, Copy, Default)]
struct OmpCosts {
    input: Decimal,
    output: Decimal,
    cache_read: Decimal,
    cache_write: Decimal,
    total: Decimal,
}

impl OmpCosts {
    fn reported(self) -> Option<(Decimal, Decimal, Decimal, Decimal, Decimal)> {
        let component_total = self.input + self.output + self.cache_read + self.cache_write;
        let total = if self.total > Decimal::ZERO {
            self.total
        } else {
            component_total
        };
        (total > Decimal::ZERO).then_some((
            self.input,
            self.output,
            self.cache_read,
            self.cache_write,
            total,
        ))
    }
}

#[derive(Debug)]
struct OmpUsageRecord {
    request_id: String,
    provider_id: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    costs: OmpCosts,
    status_code: i64,
    error_message: Option<String>,
    created_at: i64,
    session_id: String,
    latency_ms: i64,
    first_token_ms: Option<i64>,
}

/// 同步指定的 OMP 会话文件（文件列表注入，测试用 tempfile，绝不碰真实 ~/.omp）。
fn sync_omp_session_files(db: &Database, files: &[PathBuf]) -> SessionSyncResult {
    let mut result = SessionSyncResult::default();
    result.files_scanned = files.len().min(u32::MAX as usize) as u32;

    // 游标预取失败必须中止本轮（不能当空表全量重导）。
    let cursors = match crate::services::session_usage::load_sync_cursors(db) {
        Ok(cursors) => cursors,
        Err(error) => {
            result.errors.push(format!("OMP 会话游标预取失败: {error}"));
            return result;
        }
    };

    for file in files {
        let key = file.to_string_lossy().to_string();
        match sync_single_omp_session(db, file, cursors.get(&key)) {
            Ok(file_result) => result.merge(file_result),
            Err(error) => {
                let message = format!("{}: {error}", file.display());
                log::warn!("[OMP-SESSION] 会话文件处理失败: {message}");
                result.errors.push(message);
            }
        }
    }
    result
}

/// 同步单个会话 JSONL。
///
/// 增量语义与 Claude 路径一致：游标为字节偏移 + 边界前 4KB 尾部指纹。
/// 非追加变化（截断/重写）时从头重读——幂等由持久去重账本
/// （`session_usage_dedup`，request_id = `omp_session:{会话}:{entry_id}`）
/// 保证：重复读到的 entry 直接跳过，被 rollup 剪除的旧行为也不会双算。
fn sync_single_omp_session(
    db: &Database,
    file_path: &Path,
    cursor: Option<&SyncCursor>,
) -> Result<SessionSyncResult, AppError> {
    let mut result = SessionSyncResult::default();
    let file_path_str = file_path.to_string_lossy().to_string();

    let metadata = fs::symlink_metadata(file_path)
        .map_err(|e| AppError::Message(format!("读取 OMP 会话元数据失败: {e}")))?;
    if !metadata.file_type().is_file() {
        return Ok(result);
    }
    let file_size = metadata.len();
    if file_size > OMP_MAX_SESSION_BYTES {
        return Err(AppError::Message(format!(
            "OMP 会话文件超过 {OMP_MAX_SESSION_BYTES} 字节安全上限"
        )));
    }
    let file_modified = metadata_modified_nanos(&metadata);
    let last_modified = cursor.map_or(0, |c| c.last_modified);
    let last_byte_offset = cursor.and_then(|c| c.last_byte_offset);
    // 跳过条件除 mtime 外还须"文件没有增长"：Windows 文件时间戳粒度约
    // 15.6ms，同一 tick 内的追加会让 mtime 相等；只看 mtime 会把这段
    // 尾部新增永久漏掉（文件此后再无修改时）。
    let no_new_bytes = match last_byte_offset {
        Some(offset) => (file_size as i64) <= offset,
        None => true,
    };
    if file_modified <= last_modified && no_new_bytes {
        return Ok(result);
    }

    // 文件名 `<ISO 时间戳>_<sessionId>.jsonl`：增量读取可能从文件中段
    // 开始，拿不到 header 行，因此会话 ID 始终用文件名尾段（实测与
    // header id 一致），保证 request_id 跨轮稳定。
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let session_id = stem.rsplit('_').next().unwrap_or(stem).to_string();

    let mut file = fs::File::open(file_path)
        .map_err(|e| AppError::Message(format!("打开 OMP 会话失败: {e}")))?;

    // 追加探测：游标有效且边界前尾部指纹吻合 → 从游标处继续；否则从头。
    let last_fingerprint = cursor.and_then(|c| c.last_tail_fingerprint);
    let (start_byte, mut tail_buf) = match last_byte_offset {
        Some(offset) if (0..=file_size as i64).contains(&offset) => {
            let seed = read_tail_before(&mut file, offset as u64)?;
            let rewritten = last_fingerprint
                .map(|expected| omp_tail_fingerprint(&seed) != expected)
                .unwrap_or(false);
            if rewritten {
                log::warn!(
                    "[OMP-SESSION] 会话文件被外部重写，从头重读（去重账本保证幂等）: {}",
                    file_path.display()
                );
                file.seek(SeekFrom::Start(0))
                    .map_err(|e| AppError::Message(format!("定位 OMP 会话失败: {e}")))?;
                (0u64, Vec::new())
            } else {
                // read_tail_before 已把文件位置停在 offset，无需再 seek
                (offset as u64, seed)
            }
        }
        _ => {
            file.seek(SeekFrom::Start(0))
                .map_err(|e| AppError::Message(format!("定位 OMP 会话失败: {e}")))?;
            (0u64, Vec::new())
        }
    };

    let mut reader = BufReader::new(file);
    let mut committed_offset = start_byte;
    let mut incomplete_tail = false;
    let mut line_buf: Vec<u8> = Vec::new();
    let mut records: Vec<OmpUsageRecord> = Vec::new();

    loop {
        let remaining = file_size.saturating_sub(committed_offset);
        if remaining == 0 {
            break;
        }
        line_buf.clear();
        let read = Read::by_ref(&mut reader)
            .take(remaining)
            .read_until(b'\n', &mut line_buf)
            .map_err(|e| AppError::Message(format!("读取 OMP 会话失败: {e}")))?;
        if read == 0 {
            break;
        }
        let has_newline = line_buf.ends_with(b"\n");
        let trimmed = trim_bytes(&line_buf);
        if !has_newline {
            // 半行（写入中途）：不提交、不解析，等追加完成后重试。
            incomplete_tail = true;
            break;
        }
        if !trimmed.is_empty() {
            if let Ok(value) = serde_json::from_slice::<Value>(trimmed) {
                if let Some(record) = parse_omp_usage_record(&value, &session_id) {
                    records.push(record);
                }
            }
            // 无法解析的完整行按"跳过但提交"处理，避免坏行导致每轮卡死。
        }
        committed_offset = committed_offset.saturating_add(read as u64);
        push_committed_tail(&mut tail_buf, &line_buf);
    }

    let conn = lock_conn!(db.conn);
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| AppError::Database(format!("启动 OMP 会话导入事务失败: {e}")))?;
    for record in &records {
        if insert_omp_session_record(&tx, record)? {
            result.imported = result.imported.saturating_add(1);
        } else {
            result.skipped = result.skipped.saturating_add(1);
        }
    }
    let fingerprint = omp_tail_fingerprint(&tail_buf);
    update_omp_session_sync_state_on_conn(
        &tx,
        &file_path_str,
        file_modified,
        committed_offset as i64,
        Some(fingerprint),
    )?;
    tx.commit()
        .map_err(|e| AppError::Database(format!("提交 OMP 会话导入事务失败: {e}")))?;
    if incomplete_tail {
        result.deferred_files = 1;
    }
    Ok(result)
}

/// 裁剪行尾的 `\r`/`\n`/空白。
fn trim_bytes(mut bytes: &[u8]) -> &[u8] {
    while let Some((last, rest)) = bytes.split_last() {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

/// 读取 `end` 之前最多 [`TAIL_FINGERPRINT_BYTES`] 字节；返回后文件位置停在 `end`。
fn read_tail_before(file: &mut fs::File, end: u64) -> Result<Vec<u8>, AppError> {
    let len = end.min(TAIL_FINGERPRINT_BYTES);
    let mut tail = vec![0u8; len as usize];
    file.seek(SeekFrom::Start(end - len))
        .map_err(|e| AppError::Message(format!("定位 OMP 会话偏移失败: {e}")))?;
    if len > 0 {
        file.read_exact(&mut tail)
            .map_err(|e| AppError::Message(format!("读取 OMP 会话边界尾部失败: {e}")))?;
    }
    Ok(tail)
}

/// 滚动尾部缓冲：只保留游标前的最后 [`TAIL_FINGERPRINT_BYTES`] 字节。
fn push_committed_tail(tail_buf: &mut Vec<u8>, bytes: &[u8]) {
    tail_buf.extend_from_slice(bytes);
    let max = TAIL_FINGERPRINT_BYTES as usize;
    if tail_buf.len() > max {
        tail_buf.drain(..tail_buf.len() - max);
    }
}

fn omp_tail_fingerprint(tail: &[u8]) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(b"omp-session-tail-v1");
    hasher.update(tail);
    let digest = hasher.finalize();
    i64::from(u32::from_be_bytes(
        digest[..4].try_into().unwrap_or_default(),
    ))
}

/// 写入 OMP 会话路径的字节游标（`last_line_offset` 置 0：纯字节语义）。
fn update_omp_session_sync_state_on_conn(
    conn: &rusqlite::Connection,
    file_path: &str,
    last_modified: i64,
    byte_offset: i64,
    tail_fingerprint: Option<i64>,
) -> Result<(), AppError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.prepare_cached(
        "INSERT OR REPLACE INTO session_log_sync
             (file_path, last_modified, last_line_offset, last_synced_at, last_byte_offset,
              last_tail_fingerprint)
         VALUES (?1, ?2, 0, ?3, ?4, ?5)",
    )
    .and_then(|mut stmt| {
        stmt.execute(rusqlite::params![
            file_path,
            last_modified,
            now,
            byte_offset,
            tail_fingerprint
        ])
    })
    .map_err(|e| AppError::Database(format!("更新 OMP 会话同步状态失败: {e}")))?;
    Ok(())
}

/// 解析单条会话记录为用量（非 assistant/无 usage 返回 None）。
///
/// 口径（对齐 OMP 自家 model_perf，供对账）：
/// - `output_tokens` 只计 `usage.output`，`reasoningTokens` 不计入（OMP 的
///   成本也按 output 计费，实测 kimi-k3 输出恰好 $15/M × output）；
/// - 免费模型（如 sensenova-6.8-flash-lite）自报成本全 0 → 金额保持 0，
///   不编造；
/// - error/aborted 行（token 全 0）仍入库，记 500/499，与 Pi 路径一致。
fn parse_omp_usage_record(value: &Value, session_id: &str) -> Option<OmpUsageRecord> {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = value.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let usage_value = message.get("usage")?;
    let input_tokens = token_count(usage_value, "input");
    let output_tokens = token_count(usage_value, "output");
    let cache_read_tokens = token_count(usage_value, "cacheRead");
    let cache_write_tokens = token_count(usage_value, "cacheWrite");
    let costs = parse_omp_costs(usage_value.get("cost"));
    let stop_reason = message.get("stopReason").and_then(Value::as_str);
    let failed = matches!(stop_reason, Some("error") | Some("aborted"));
    if input_tokens == 0
        && output_tokens == 0
        && cache_read_tokens == 0
        && cache_write_tokens == 0
        && costs.reported().is_none()
        && !failed
    {
        return None;
    }

    let provider_id = bounded_label(message.get("provider"), PROVIDER_ID);
    let model = bounded_label(message.get("model"), UNKNOWN_MODEL);
    let (status_code, error_message) = match stop_reason {
        Some("aborted") => (
            499i64,
            Some(
                nonempty_string(message.get("errorMessage"))
                    .unwrap_or("OMP request aborted")
                    .chars()
                    .take(4096)
                    .collect(),
            ),
        ),
        Some("error") => (
            500i64,
            Some(
                nonempty_string(message.get("errorMessage"))
                    .unwrap_or("OMP request failed")
                    .chars()
                    .take(4096)
                    .collect(),
            ),
        ),
        _ => (200i64, None),
    };

    let created_at = message
        .get("timestamp")
        .and_then(timestamp_secs)
        .or_else(|| value.get("timestamp").and_then(timestamp_secs))
        .filter(|secs| (1_000_000_000..4_102_444_800).contains(secs))
        .unwrap_or_else(now_secs);

    let latency_ms = message
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| d.round() as i64)
        .unwrap_or(0);
    let first_token_ms = message
        .get("ttft")
        .and_then(Value::as_f64)
        .filter(|t| t.is_finite() && *t >= 0.0)
        .map(|t| t.round() as i64);

    let request_id = match nonempty_string(value.get("id")) {
        Some(entry_id) => format!(
            "omp_session:{session_id}:{}",
            truncate_usage_label(entry_id)
        ),
        None => {
            // 罕见兜底（entry 无 id）：以内容指纹作稳定键。
            let mut hasher = Sha256::new();
            hasher.update(b"omp-session-entry-v1");
            hasher.update(session_id.as_bytes());
            hasher.update(value.to_string().as_bytes());
            format!("omp_session:{session_id}:{:x}", hasher.finalize())
        }
    };

    Some(OmpUsageRecord {
        request_id,
        provider_id,
        model: model.clone(),
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        costs,
        status_code,
        error_message,
        created_at,
        session_id: session_id.to_string(),
        latency_ms,
        first_token_ms,
    })
}

/// 插入一条会话用量（先查持久账本，幂等）。
fn insert_omp_session_record(
    conn: &rusqlite::Connection,
    record: &OmpUsageRecord,
) -> Result<bool, AppError> {
    let already_seen: bool = conn
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM session_usage_dedup
                 WHERE data_source = ?1 AND request_id = ?2
             )",
            rusqlite::params![DATA_SOURCE, record.request_id],
            |row| row.get(0),
        )
        .map_err(|e| AppError::Database(format!("查询 OMP 会话去重账本失败: {e}")))?;
    if already_seen {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR IGNORE INTO session_usage_dedup
         (data_source, request_id, semantic_id, has_entry_id)
         VALUES (?1, ?2, ?3, 1)",
        rusqlite::params![DATA_SOURCE, record.request_id, record.request_id],
    )
    .map_err(|e| AppError::Database(format!("写入 OMP 会话去重账本失败: {e}")))?;

    let usage = TokenUsage {
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_read_tokens: record.cache_read_tokens,
        cache_creation_tokens: record.cache_write_tokens,
        model: Some(record.model.clone()),
        message_id: None,
    };
    let costs = record.costs.reported().or_else(|| {
        find_model_pricing(conn, &record.model).map(|pricing| {
            let calculated =
                CostCalculator::calculate_for_app(APP_TYPE, &usage, &pricing, Decimal::ONE);
            (
                calculated.input_cost,
                calculated.output_cost,
                calculated.cache_read_cost,
                calculated.cache_creation_cost,
                calculated.total_cost,
            )
        })
    });
    let (input_cost, output_cost, cache_read_cost, cache_write_cost, total_cost) =
        costs.unwrap_or((
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        ));

    let clamp = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    conn.execute(
        "INSERT OR IGNORE INTO proxy_request_logs (
            request_id, provider_id, app_type, model, request_model, pricing_model,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            input_token_semantics,
            input_cost_usd, output_cost_usd, cache_read_cost_usd,
            cache_creation_cost_usd, total_cost_usd,
            latency_ms, first_token_ms, status_code, error_message, session_id,
            provider_type, is_streaming, cost_multiplier, created_at, data_source
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
            ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
        )",
        rusqlite::params![
            record.request_id,
            record.provider_id,
            APP_TYPE,
            record.model,
            record.model,
            record.model,
            clamp(record.input_tokens as i64),
            clamp(record.output_tokens as i64),
            clamp(record.cache_read_tokens as i64),
            clamp(record.cache_write_tokens as i64),
            INPUT_TOKEN_SEMANTICS_FRESH,
            input_cost.to_string(),
            output_cost.to_string(),
            cache_read_cost.to_string(),
            cache_write_cost.to_string(),
            total_cost.to_string(),
            record.latency_ms,
            record.first_token_ms,
            record.status_code,
            record.error_message,
            record.session_id,
            Some(DATA_SOURCE),
            1i64,
            "1.0",
            record.created_at,
            DATA_SOURCE,
        ],
    )
    .map(|changed| changed > 0)
    .map_err(|e| AppError::Database(format!("插入 OMP 会话用量失败: {e}")))
}

/// 解析 `usage.cost`（input/output/cacheRead/cacheWrite/total）。
fn parse_omp_costs(value: Option<&Value>) -> OmpCosts {
    let decimal = |key| {
        value
            .and_then(|cost| cost.get(key))
            .and_then(parse_decimal)
            .unwrap_or(Decimal::ZERO)
            .max(Decimal::ZERO)
    };
    OmpCosts {
        input: decimal("input"),
        output: decimal("output"),
        cache_read: decimal("cacheRead"),
        cache_write: decimal("cacheWrite"),
        total: decimal("total"),
    }
}

fn parse_decimal(value: &Value) -> Option<Decimal> {
    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(value) => value.clone(),
        _ => return None,
    };
    Decimal::from_str(&raw)
        .or_else(|_| Decimal::from_scientific(&raw))
        .ok()
}

fn token_count(usage: &Value, key: &str) -> u32 {
    usage
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32
}

/// 毫秒或秒级 epoch / RFC3339 字符串统一转秒。
fn timestamp_secs(value: &Value) -> Option<i64> {
    if let Some(raw) = value.as_i64() {
        let millis = if raw > 100_000_000_000 {
            raw
        } else {
            raw * 1000
        };
        return Some(millis / 1000);
    }
    if let Some(raw) = value.as_f64() {
        return Some((raw / 1000.0) as i64);
    }
    value
        .as_str()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.timestamp())
}

fn nonempty_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn bounded_label(value: Option<&Value>, fallback: &str) -> String {
    truncate_usage_label(nonempty_string(value).unwrap_or(fallback)).to_string()
}

/// 标签最长 512 字节（与 Pi 路径一致），且不切在 UTF-8 中间。
fn truncate_usage_label(value: &str) -> &str {
    const MAX_USAGE_LABEL_BYTES: usize = 512;
    if value.len() <= MAX_USAGE_LABEL_BYTES {
        return value;
    }
    let mut end = MAX_USAGE_LABEL_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpQuotaWindow {
    pub provider: String,
    pub used_fraction: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
    /// 账号标识（`oauth|account:<uuid>|email:…`）。卡片按 provider 聚合时用它去重，
    /// 多账号同一窗口才能各自显示（v1.1.3 接到卡片时补的字段，此前只在 GROUP BY 用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_key: Option<String>,
    /// 窗口的完整 id（如 `xai-oauth:credits:1w`），比 `label` 更适合做 tier 身份。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_id: Option<String>,
}

/// 读取 OMP 配额窗口（usage_history），不是 token。client_usage 为空时给看板展示。
pub fn list_omp_quota_windows() -> Result<Vec<OmpQuotaWindow>, AppError> {
    list_omp_quota_windows_from(&omp_agent_db())
}

fn list_omp_quota_windows_from(path: &std::path::Path) -> Result<Vec<OmpQuotaWindow>, AppError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = open_readonly_sqlite(path)
        .map_err(|e| AppError::Database(format!("只读打开 agent.db 失败: {e}")))?;
    let columns: Vec<String> = {
        let mut stmt = match conn.prepare("PRAGMA table_info(usage_history)") {
            Ok(stmt) => stmt,
            Err(_) => return Ok(Vec::new()),
        };
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| AppError::Database(e.to_string()))?;
        rows.filter_map(Result::ok).collect()
    };
    if columns.is_empty() {
        return Ok(Vec::new());
    }
    let provider_col = ["provider", "app", "source"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)))
        .unwrap_or("provider");
    let fraction_col = ["used_fraction", "used_percent", "fraction"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)))
        .unwrap_or("used_fraction");
    let resets_col = ["resets_at", "reset_at", "expires_at"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)));
    let label_col = ["window_label", "label"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)));
    let account_col = ["account_key", "account"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)));
    let limit_col = ["limit_id", "limit"]
        .into_iter()
        .find(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)));
    let resets_expr = resets_col.unwrap_or("NULL");
    let label_expr = label_col.unwrap_or("NULL");
    let account_expr = account_col.unwrap_or("NULL");
    let limit_expr = limit_col.unwrap_or("NULL");
    let group_cols = ["account_key", "limit_id", "provider"]
        .into_iter()
        .filter(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)))
        .collect::<Vec<_>>();
    let group_expr = if group_cols.is_empty() {
        provider_col.to_string()
    } else {
        group_cols.join(", ")
    };
    let sql = format!(
        "SELECT {provider_col}, {fraction_col}, {label_expr}, {resets_expr}, {account_expr}, {limit_expr}
         FROM usage_history
         WHERE rowid IN (
           SELECT MAX(rowid) FROM usage_history GROUP BY {group_expr}
         )
         ORDER BY rowid DESC
         LIMIT 20"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AppError::Database(format!("查询 usage_history 失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OmpQuotaWindow {
                provider: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                used_fraction: row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                label: row.get::<_, Option<String>>(2).ok().flatten(),
                resets_at: row.get::<_, Option<i64>>(3).ok().flatten(),
                account_key: row.get::<_, Option<String>>(4).ok().flatten(),
                limit_id: row.get::<_, Option<String>>(5).ok().flatten(),
            })
        })
        .map_err(|e| AppError::Database(format!("读取 usage_history 失败: {e}")))?;
    Ok(rows.filter_map(Result::ok).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_agent_db_is_noop() -> Result<(), AppError> {
        let db = Database::memory()?;
        let result =
            sync_omp_usage_from(&db, std::path::Path::new("Z:/definitely/missing/agent.db"))?;
        assert_eq!(result.imported, 0);
        Ok(())
    }

    #[test]
    fn quota_windows_keep_latest_snapshot_per_limit() -> Result<(), AppError> {
        let tmp = tempfile::tempdir().unwrap();
        let agent_db = tmp.path().join("agent.db");
        let conn = rusqlite::Connection::open(&agent_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_history (
                id INTEGER PRIMARY KEY,
                recorded_at INTEGER NOT NULL,
                provider TEXT NOT NULL,
                account_key TEXT NOT NULL,
                limit_id TEXT NOT NULL,
                window_label TEXT,
                used_fraction REAL,
                resets_at INTEGER
             );",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO usage_history (
                recorded_at, provider, account_key, limit_id, window_label, used_fraction, resets_at
             ) VALUES
               (1, 'openai-codex', 'a', 'openai-codex:primary', '30 days', 0.99, 10),
               (2, 'xai-oauth', 'a', 'xai-oauth:credits:1w', 'Weekly', 0.0, 20),
               (3, 'openai-codex', 'a', 'openai-codex:primary', '30 days', 0.0, 30),
               (4, 'xai-oauth', 'a', 'xai-oauth:credits:1w', 'Weekly', 0.0, 40);",
        )
        .unwrap();
        drop(conn);

        let windows = list_omp_quota_windows_from(&agent_db)?;
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].provider, "xai-oauth");
        assert_eq!(windows[0].label.as_deref(), Some("Weekly"));
        assert_eq!(windows[0].used_fraction, 0.0);
        assert_eq!(windows[1].provider, "openai-codex");
        assert_eq!(windows[1].label.as_deref(), Some("30 days"));
        assert_eq!(windows[1].used_fraction, 0.0);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // 会话 JSONL 导入
    // -----------------------------------------------------------------------

    fn assistant_line(
        entry_id: &str,
        provider: &str,
        model: &str,
        ts_ms: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        cost_total: f64,
        stop_reason: &str,
    ) -> String {
        format!(
            r#"{{"type":"message","id":"{entry_id}","parentId":"p","timestamp":"2026-09-07T09:54:17.038Z","message":{{"role":"assistant","content":[],"api":"openai-completions","provider":"{provider}","model":"{model}","usage":{{"input":{input},"output":{output},"cacheRead":{cache_read},"cacheWrite":0,"totalTokens":{total},"reasoningTokens":7,"cost":{{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":{cost_total}}}}},"stopReason":"{stop_reason}","timestamp":{ts_ms},"responseId":"r1","duration":3185.7,"ttft":2195.3}}}}"#,
            total = input + output + cache_read,
        )
    }

    fn write_session(path: &std::path::Path, lines: &[String]) {
        let mut body = String::new();
        body.push_str(
            r#"{"type":"title","v":1,"title":"T","source":"auto","updatedAt":"2026-09-07T09:54:13.790Z"}"#,
        );
        body.push('\n');
        body.push_str(
            r#"{"type":"session","version":3,"id":"01a07b4a-2c7b-7699-8ad4-485ea25e2089","timestamp":"2026-09-07T09:54:13.755Z","cwd":"E:\\Omp"}"#,
        );
        body.push('\n');
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn imports_omp_session_usage_with_provider_dimension() -> Result<(), AppError> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join("2026-09-07T09-54-13-755Z_01a07b4a-2c7b-7699-8ad4-485ea25e2089.jsonl");
        write_session(
            &path,
            &[
                assistant_line(
                    "e1",
                    "SenseNova",
                    "kimi-k3",
                    1_789_774_857_848,
                    100,
                    20,
                    300,
                    0.0,
                    "toolUse",
                ),
                assistant_line(
                    "e2",
                    "Rigel",
                    "grok-4.6",
                    1_789_774_860_000,
                    50,
                    10,
                    400,
                    0.076,
                    "stop",
                ),
                // error 行：token 与成本全 0，仍入库记 500
                assistant_line(
                    "e3",
                    "SenseNova",
                    "kimi-k3",
                    1_789_774_870_000,
                    0,
                    0,
                    0,
                    0.0,
                    "error",
                ),
            ],
        );

        let db = Database::memory()?;
        let result = sync_omp_session_files(&db, std::slice::from_ref(&path));
        assert_eq!(result.imported, 3);
        assert!(result.errors.is_empty());

        let conn = lock_conn!(db.conn);
        let rows: Vec<(String, String, String, i64, i64, i64, i64, i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT provider_id, model, request_model, input_tokens, output_tokens,
                        cache_read_tokens, status_code, created_at, total_cost_usd
                 FROM proxy_request_logs WHERE app_type = 'omp' ORDER BY request_id",
            )?;
            let collected = stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            collected
        };
        assert_eq!(rows.len(), 3);
        // 供应商维度（按 entry id 排序：e1, e2, e3）
        assert_eq!(rows[0].0, "SenseNova");
        assert_eq!(rows[0].1, "kimi-k3");
        assert_eq!(rows[0].2, "kimi-k3");
        assert_eq!(rows[0].3, 100);
        assert_eq!(rows[0].4, 20);
        assert_eq!(rows[0].5, 300);
        assert_eq!(rows[0].6, 200);
        assert_eq!(rows[0].7, 1_789_774_857);
        // kimi-k3 有本地定价（3/15/0.3），自报 0 → 兜底计算非零
        assert_ne!(rows[0].8, "0");
        // Rigel/grok-4.6 自报 0.076 直接入账
        assert_eq!(rows[1].0, "Rigel");
        assert_eq!(rows[1].8, "0.076");
        // error 行记录状态，token 全 0
        assert_eq!(rows[2].0, "SenseNova");
        assert_eq!(rows[2].6, 500);
        assert_eq!(rows[2].3, 0);
        Ok(())
    }

    #[test]
    fn omp_session_incremental_and_dedup_on_rewrite() -> Result<(), AppError> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join("2026-09-07T09-54-13-755Z_01a07b4a-2c7b-7699-8ad4-485ea25e2089.jsonl");
        write_session(
            &path,
            &[assistant_line(
                "e1",
                "Rigel",
                "grok-4.6",
                1_789_774_857_848,
                100,
                20,
                300,
                0.076,
                "stop",
            )],
        );

        let db = Database::memory()?;
        assert_eq!(
            sync_omp_session_files(&db, std::slice::from_ref(&path)).imported,
            1
        );
        // 未变化 → 不重读
        assert_eq!(
            sync_omp_session_files(&db, std::slice::from_ref(&path)).imported,
            0
        );

        // 追加一行 → 只导入新增
        {
            let mut body = fs::read_to_string(&path).unwrap();
            body.push_str(&assistant_line(
                "e2",
                "Rigel",
                "grok-4.6",
                1_789_774_860_000,
                50,
                10,
                400,
                0.02,
                "stop",
            ));
            body.push('\n');
            fs::write(&path, body).unwrap();
        }
        assert_eq!(
            sync_omp_session_files(&db, std::slice::from_ref(&path)).imported,
            1
        );

        // 重写整个文件（内容不同的同尺寸替换近似）→ 从头重读但账本去重
        {
            let mut body = String::new();
            body.push_str(
                r#"{"type":"title","v":1,"title":"T2","source":"auto","updatedAt":"2026-09-07T09:54:13.790Z"}"#,
            );
            body.push('\n');
            body.push_str(
                r#"{"type":"session","version":3,"id":"01a07b4a-2c7b-7699-8ad4-485ea25e2089","timestamp":"2026-09-07T09:54:13.755Z","cwd":"E:\\Omp"}"#,
            );
            body.push('\n');
            body.push_str(&assistant_line(
                "e1",
                "Rigel",
                "grok-4.6",
                1_789_774_857_848,
                100,
                20,
                300,
                0.076,
                "stop",
            ));
            body.push('\n');
            body.push_str(&assistant_line(
                "e2",
                "Rigel",
                "grok-4.6",
                1_789_774_860_000,
                50,
                10,
                400,
                0.02,
                "stop",
            ));
            body.push('\n');
            body.push_str(&assistant_line(
                "e3",
                "Rigel",
                "grok-4.6",
                1_789_774_870_000,
                10,
                5,
                0,
                0.002,
                "stop",
            ));
            body.push('\n');
            fs::write(&path, body).unwrap();
        }
        let result = sync_omp_session_files(&db, std::slice::from_ref(&path));
        assert_eq!(result.imported, 1, "仅新 entry 入账");
        assert_eq!(result.skipped, 2, "旧 entry 由账本去重");

        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE app_type = 'omp'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 3);
        let ledger: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_usage_dedup WHERE data_source = 'omp_session'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(ledger, 3);
        Ok(())
    }

    #[test]
    fn omp_session_skips_incomplete_tail_line() -> Result<(), AppError> {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp
            .path()
            .join("2026-09-07T09-54-13-755Z_01a07b4a-2c7b-7699-8ad4-485ea25e2089.jsonl");
        write_session(
            &path,
            &[assistant_line(
                "e1",
                "Rigel",
                "grok-4.6",
                1_789_774_857_848,
                100,
                20,
                300,
                0.076,
                "stop",
            )],
        );
        let full_e2 = assistant_line(
            "e2",
            "Rigel",
            "grok-4.6",
            1_789_774_860_000,
            50,
            10,
            400,
            0.02,
            "stop",
        );
        // 追加 e2 的前半段（无换行，模拟写入中途）
        let split = full_e2.len() / 2;
        {
            let mut body = fs::read_to_string(&path).unwrap();
            body.push_str(&full_e2[..split]);
            fs::write(&path, body).unwrap();
        }
        let db = Database::memory()?;
        let result = sync_omp_session_files(&db, std::slice::from_ref(&path));
        assert_eq!(result.imported, 1, "完整行照常导入");
        assert_eq!(result.deferred_files, 1, "半行延后");

        // 补全后半段：从半行起点重读，整行解析成功
        {
            let mut body = fs::read_to_string(&path).unwrap();
            body.push_str(&full_e2[split..]);
            body.push('\n');
            fs::write(&path, body).unwrap();
        }
        let result = sync_omp_session_files(&db, std::slice::from_ref(&path));
        assert_eq!(result.imported, 1);
        assert_eq!(result.deferred_files, 0);
        let conn = lock_conn!(db.conn);
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM proxy_request_logs WHERE app_type = 'omp'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);
        Ok(())
    }

    #[test]
    fn omp_session_accepts_unknown_provider_with_placeholder() -> Result<(), AppError> {
        let value: Value = serde_json::from_str(
            r#"{"type":"message","id":"e9","message":{"role":"assistant","api":"openai-completions","model":"gpt-5.6-luna","usage":{"input":10,"output":2,"cacheRead":0,"cacheWrite":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1789774515000}}"#,
        )
        .unwrap();
        let record = parse_omp_usage_record(&value, "sess").expect("record");
        assert_eq!(record.provider_id, PROVIDER_ID);
        assert_eq!(record.request_id, "omp_session:sess:e9");
        assert_eq!(record.created_at, 1_789_774_515);
        // 无 provider 的字面量不得进入供应商维度
        Ok(())
    }
}
