//! Antigravity（agy）会话用量追踪
//!
//! agy 本地转录（`brain/<uuid>/.../transcript.jsonl`）通常不含 token 字段；
//! 真实用量在 `conversations/<uuid>.db` 的 steps.metadata（step_type=15 的
//! 嵌套 field 9）。本导入器只在真实出现 usage 时入账，**禁止编造费用**。
//!
//! ## 数据流
//! ```text
//! transcript.jsonl / conversation_summaries.db 可读列 / conversations/*.db
//!   → 有 token 才写入 proxy_request_logs（app_type="antigravity"）
//! ```
//!
//! ## 模型名与时间（实测结构，2026-09 版 agy）
//! - 模型名在 gen_metadata.data（protobuf）：顶层 field 1 → 子字段 19 =
//!   `gemini-3.8-flash`（实际模型）、顶层 field 3 → 子字段 28 =
//!   `gemini-3.8-flash-high`（请求模型）。**只认这两条路径**——任意字段
//!   兜底会被实验开关集合（enable-*/jetski-*）污染。
//! - 事件时间在 steps.metadata 顶层 field 1 → 子字段 1（epoch 秒）。
//! - 已入库的历史行若 model 仍是占位值或开关名，重扫时回填修复。

use crate::antigravity_config::get_antigravity_cli_dir;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::services::session_usage::{
    metadata_modified_nanos, update_sync_state, SessionSyncResult,
};
use crate::services::sql_helpers::INPUT_TOKEN_SEMANTICS_FRESH;
use rusqlite::Connection;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const DATA_SOURCE: &str = "antigravity_session";
const PROVIDER_ID: &str = "_antigravity_session";

pub fn sync_antigravity_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    sync_antigravity_usage_from(db, &get_antigravity_cli_dir())
}

fn sync_antigravity_usage_from(
    db: &Database,
    cli_dir: &Path,
) -> Result<SessionSyncResult, AppError> {
    let mut result = SessionSyncResult::default();
    if !cli_dir.exists() {
        return Ok(result);
    }

    // 懒观测当前供应商：不经过 OGG Switch 的切换也能进时间线，用量归属
    // 覆盖"外部改配置"的情形（观测时刻即写入时刻，精度为本轮同步）。
    if let Err(e) = crate::services::provider_timeline::observe_current(db, "antigravity") {
        result.errors.push(format!("供应商时间线观测失败: {e}"));
    }

    let files = collect_transcript_files(cli_dir);
    result.files_scanned = files.len() as u32;
    let cursors = crate::services::session_usage::load_sync_cursors(db)?;

    for path in files {
        let cursor_key = path.to_string_lossy().to_string();
        let last_modified = cursors.get(&cursor_key).map_or(0, |c| c.last_modified);
        match sync_transcript(db, &path, last_modified) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                result.errors.push(format!("{}: {e}", path.display()));
            }
        }
    }

    if let Err(e) = sync_summaries_db(db, cli_dir, &mut result) {
        result
            .errors
            .push(format!("conversation_summaries.db: {e}"));
    }
    if let Err(e) = sync_conversation_dbs(db, cli_dir, &mut result) {
        result.errors.push(format!("conversations/*.db: {e}"));
    }

    Ok(result)
}

fn collect_transcript_files(cli_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let brain = cli_dir.join("brain");
    let Ok(entries) = fs::read_dir(&brain) else {
        return files;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let logs = dir.join(".system_generated").join("logs");
        let primary = logs.join("transcript.jsonl");
        if primary.exists() {
            files.push(primary);
            continue;
        }
        let full = logs.join("transcript_full.jsonl");
        if full.exists() {
            files.push(full);
        }
    }
    files
}

fn sync_transcript(db: &Database, path: &Path, last_modified: i64) -> Result<(u32, u32), AppError> {
    let metadata =
        fs::metadata(path).map_err(|e| AppError::Message(format!("读取转录元数据失败: {e}")))?;
    let file_modified = metadata_modified_nanos(&metadata);
    if file_modified <= last_modified {
        return Ok((0, 0));
    }

    let file = fs::File::open(path).map_err(|e| AppError::Message(format!("打开转录失败: {e}")))?;
    let reader = BufReader::new(file);
    let session_id = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut line_no = 0i64;
    for line in reader.lines() {
        line_no += 1;
        let Ok(line) = line else {
            skipped += 1;
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            skipped += 1;
            continue;
        };
        if let Some(usage) = extract_usage(&value) {
            if insert_usage_row(db, &session_id, line_no, &usage)? {
                imported += 1;
            } else {
                skipped += 1;
            }
        } else {
            skipped += 1;
        }
    }

    update_sync_state(db, &path.to_string_lossy(), file_modified, line_no)?;
    Ok((imported, skipped))
}

struct ConversationModels {
    model: String,
    request_model: String,
}

impl Default for ConversationModels {
    fn default() -> Self {
        Self {
            model: "unknown".to_string(),
            request_model: "unknown".to_string(),
        }
    }
}

struct UsageHit {
    model: String,
    request_model: String,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    cost_usd: f64,
    created_at: i64,
}

fn extract_usage(value: &Value) -> Option<UsageHit> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("token_usage"))
        .or_else(|| value.get("tokens"));
    let input = int_field(
        usage,
        &[
            "input_tokens",
            "prompt_tokens",
            "inputTokens",
            "promptTokenCount",
        ],
    )
    .or_else(|| int_field(Some(value), &["input_tokens", "prompt_tokens"]));
    let output = int_field(
        usage,
        &[
            "output_tokens",
            "completion_tokens",
            "outputTokens",
            "candidatesTokenCount",
        ],
    )
    .or_else(|| int_field(Some(value), &["output_tokens", "completion_tokens"]));
    if input.unwrap_or(0) == 0 && output.unwrap_or(0) == 0 {
        return None;
    }
    let created_at = value
        .get("created_at")
        .or_else(|| value.get("timestamp"))
        .and_then(json_time_secs)
        .unwrap_or_else(now_secs);
    let model = value
        .get("model")
        .or_else(|| value.get("model_name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    Some(UsageHit {
        model: model.clone(),
        request_model: model,
        input: input.unwrap_or(0),
        output: output.unwrap_or(0),
        cache_read: int_field(
            usage,
            &[
                "cache_read_tokens",
                "cached_tokens",
                "cachedContentTokenCount",
            ],
        )
        .unwrap_or(0),
        cache_write: int_field(usage, &["cache_write_tokens", "cache_creation_tokens"])
            .unwrap_or(0),
        cost_usd: usage
            .and_then(|u| u.get("cost_usd").or_else(|| u.get("cost")))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        created_at,
    })
}

fn int_field(obj: Option<&Value>, keys: &[&str]) -> Option<i64> {
    let obj = obj?;
    for key in keys {
        if let Some(n) = obj.get(*key).and_then(Value::as_i64) {
            return Some(n);
        }
        if let Some(n) = obj.get(*key).and_then(Value::as_f64) {
            return Some(n as i64);
        }
    }
    None
}

fn json_time_secs(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(if n > 1_000_000_000_000 { n / 1000 } else { n });
    }
    let raw = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn insert_usage_row(
    db: &Database,
    session_id: &str,
    line_no: i64,
    usage: &UsageHit,
) -> Result<bool, AppError> {
    // 归属：按事件真实时间查供应商时间线；查不到（早于首次观测的历史
    // 会话）保持占位来源，不编造。必须在取连接锁之前完成查询（provider_at
    // 自持锁）。
    let provider_id =
        crate::services::provider_timeline::provider_at(db, "antigravity", usage.created_at)?
            .unwrap_or_else(|| PROVIDER_ID.to_string());

    let conn = lock_conn!(db.conn);
    let clamp = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    let cost = Decimal::from_f64(usage.cost_usd).unwrap_or(Decimal::ZERO);
    let request_id = format!("antigravity_{session_id}_{line_no}");
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
            request_id,
            provider_id,
            "antigravity",
            usage.model,
            usage.request_model,
            clamp(usage.input),
            clamp(usage.output),
            clamp(usage.cache_read),
            clamp(usage.cache_write),
            "0",
            "0",
            "0",
            "0",
            cost.to_string(),
            0i64,
            Option::<i64>::None,
            200i64,
            Option::<String>::None,
            Some(session_id.to_string()),
            Some(DATA_SOURCE),
            1i64,
            "1.0",
            usage.created_at,
            DATA_SOURCE,
            INPUT_TOKEN_SEMANTICS_FRESH,
        ],
    )
    .map_err(|e| AppError::Database(format!("插入 Antigravity 用量失败: {e}")))?;
    if conn.changes() > 0 {
        return Ok(true);
    }
    if is_placeholder_model(&usage.model) {
        return Ok(false);
    }
    // 回填两类历史脏值：① 占位模型（unknown/null/none/空）；② 早期兜底 bug
    // 写进 model 的实验开关名（enable-/disable-/use-/jetski-）。同时把
    // created_at 修正为解析出的真实事件时间——旧版本写的是导入时刻，修正
    // 只在"需要修复的行"上发生，避免无谓覆盖。provider 归属不回填（历史无证据）。
    conn.execute(
        "UPDATE proxy_request_logs
         SET model = CASE
                WHEN model IN ('unknown', 'null', 'none', '')
                  OR LOWER(model) LIKE 'enable-%'
                  OR LOWER(model) LIKE 'disable-%'
                  OR LOWER(model) LIKE 'use-%'
                  OR LOWER(model) LIKE 'jetski-%'
                THEN ?1
                ELSE model
              END,
             request_model = CASE
                WHEN COALESCE(request_model, '') IN ('unknown', 'null', 'none', '') THEN ?2
                ELSE request_model
              END,
             created_at = ?3
         WHERE request_id = ?4
           AND app_type = 'antigravity'
           AND (
             model IN ('unknown', 'null', 'none', '')
             OR COALESCE(request_model, '') IN ('unknown', 'null', 'none', '')
             OR LOWER(model) LIKE 'enable-%'
             OR LOWER(model) LIKE 'disable-%'
             OR LOWER(model) LIKE 'use-%'
             OR LOWER(model) LIKE 'jetski-%'
           )",
        rusqlite::params![
            usage.model,
            usage.request_model,
            usage.created_at,
            request_id,
        ],
    )
    .map_err(|e| AppError::Database(format!("回填 Antigravity 模型名失败: {e}")))?;
    Ok(false)
}

fn sync_summaries_db(
    db: &Database,
    cli_dir: &Path,
    result: &mut SessionSyncResult,
) -> Result<(), AppError> {
    let path = cli_dir.join("conversation_summaries.db");
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::metadata(&path)
        .map_err(|e| AppError::Message(format!("读取 summaries 元数据失败: {e}")))?;
    let file_modified = metadata_modified_nanos(&metadata);
    let cursor_key = "antigravity:conversation_summaries.db";
    let cursors = crate::services::session_usage::load_sync_cursors(db)?;
    if cursors.get(cursor_key).map_or(0, |c| c.last_modified) == file_modified {
        return Ok(());
    }

    let conn = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| AppError::Database(format!("只读打开 conversation_summaries.db 失败: {e}")))?;

    let columns = pragma_columns(&conn, "conversation_summaries");
    let has_token_col = [
        "input_tokens",
        "prompt_tokens",
        "total_tokens",
        "token_count",
    ]
    .into_iter()
    .any(|c| columns.iter().any(|col| col.eq_ignore_ascii_case(c)));
    result.files_scanned = result.files_scanned.saturating_add(1);
    // summaries 表当前没有可用 token 列时不编造费用，只推进游标。
    if !has_token_col {
        log::debug!("conversation_summaries.db 无 token 列，跳过用量导入");
    }
    update_sync_state(db, cursor_key, file_modified, 0)?;
    Ok(())
}

fn sync_conversation_dbs(
    db: &Database,
    cli_dir: &Path,
    result: &mut SessionSyncResult,
) -> Result<(), AppError> {
    let dir = cli_dir.join("conversations");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };
    let cursors = crate::services::session_usage::load_sync_cursors(db)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("db") {
            continue;
        }
        let Some(session_id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let metadata = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let file_modified = metadata_modified_nanos(&metadata);
        let cursor_key = format!("antigravity:conversation:{session_id}");
        if cursors.get(&cursor_key).map_or(0, |c| c.last_modified) == file_modified
            && !session_needs_model_repair(db, session_id)
        {
            continue;
        }
        result.files_scanned = result.files_scanned.saturating_add(1);
        match import_conversation_db(db, &path, session_id) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
                update_sync_state(db, &cursor_key, file_modified, 0)?;
            }
            Err(e) => result.errors.push(format!("{}: {e}", path.display())),
        }
    }
    Ok(())
}

fn import_conversation_db(
    db: &Database,
    path: &Path,
    session_id: &str,
) -> Result<(u32, u32), AppError> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| AppError::Database(format!("只读打开 conversation db 失败: {e}")))?;
    let models = load_conversation_models(&conn);
    let mut stmt = match conn.prepare("SELECT idx, step_type, metadata FROM steps ORDER BY idx ASC")
    {
        Ok(stmt) => stmt,
        Err(_) => return Ok((0, 0)),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })
        .map_err(|e| AppError::Database(format!("读取 steps 失败: {e}")))?;
    let mut imported = 0u32;
    let mut skipped = 0u32;
    for row in rows.flatten() {
        let (idx, step_type, metadata) = row;
        if step_type != 15 {
            skipped += 1;
            continue;
        }
        let Some(blob) = metadata else {
            skipped += 1;
            continue;
        };
        let Some(usage) = extract_usage_from_metadata(&blob, &models) else {
            skipped += 1;
            continue;
        };
        if insert_usage_row(db, session_id, idx, &usage)? {
            imported += 1;
        } else {
            skipped += 1;
        }
    }
    Ok((imported, skipped))
}

/// 该会话是否存在需要修复的模型字段：占位值（unknown/null/none/空）或
/// 历史兜底 bug 写进 `model` 的实验开关名。命中即强制重扫该 conversation，
/// 让修复版解析器把正确模型名回填进去。
fn session_needs_model_repair(db: &Database, session_id: &str) -> bool {
    let Ok(conn) = db.conn.lock() else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM proxy_request_logs
         WHERE session_id = ?1
           AND app_type = 'antigravity'
           AND (
             model IN ('unknown', 'null', 'none', '')
             OR COALESCE(request_model, '') IN ('unknown', 'null', 'none', '')
             OR LOWER(model) LIKE 'enable-%'
             OR LOWER(model) LIKE 'disable-%'
             OR LOWER(model) LIKE 'use-%'
             OR LOWER(model) LIKE 'jetski-%'
           )
         LIMIT 1",
        rusqlite::params![session_id],
        |_| Ok(()),
    )
    .is_ok()
}

fn is_placeholder_model(model: &str) -> bool {
    matches!(
        model.trim().to_ascii_lowercase().as_str(),
        "" | "unknown" | "null" | "none"
    )
}

fn looks_like_model_id(value: &str) -> bool {
    let value = value.trim();
    !is_placeholder_model(value)
        && !looks_like_feature_flag(value)
        && value.len() < 80
        && value.contains('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '/'))
}

/// Antigravity 的 metadata 里混着实验/功能开关（实测：enable-owl-slash-command、
/// disable-teamwork-forced-flash-model、use-component-rewrite、
/// jetski-unified-customizations-panel-enabled 等）。它们同样"像模型 id"（短、
/// 含 `-`、纯 ASCII），早期兜底逻辑会把它们写进统计。任何走模型名判定的
/// 路径都必须先排除这些前缀。
fn looks_like_feature_flag(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    ["enable-", "disable-", "use-", "jetski-"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn load_conversation_models(conn: &Connection) -> ConversationModels {
    let mut model = None;
    let mut request_model = None;
    for table in ["gen_metadata", "executor_metadata"] {
        for blob in load_table_blobs(conn, table) {
            collect_model_strings(&blob, &mut model, &mut request_model);
        }
    }
    match (model, request_model) {
        (Some(model), Some(request_model)) => ConversationModels {
            model,
            request_model,
        },
        (Some(model), None) => ConversationModels {
            request_model: model.clone(),
            model,
        },
        (None, Some(request_model)) => ConversationModels {
            model: request_model.clone(),
            request_model,
        },
        (None, None) => ConversationModels::default(),
    }
}

fn load_table_blobs(conn: &Connection, table: &str) -> Vec<Vec<u8>> {
    let columns = pragma_columns(conn, table);
    let col = ["data", "metadata", "blob"]
        .into_iter()
        .find(|name| columns.iter().any(|col| col.eq_ignore_ascii_case(name)));
    let Some(col) = col else {
        return Vec::new();
    };
    let sql = format!("SELECT {col} FROM {table}");
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, Option<Vec<u8>>>(0)) else {
        return Vec::new();
    };
    rows.flatten().flatten().collect()
}

/// 遍历 protobuf 顶层的 length-delimited 字段，依次交给 `visit` 处理。
fn for_each_length_delimited(data: &[u8], mut visit: impl FnMut(u32, &[u8])) {
    let mut i = 0;
    while i < data.len() {
        let Some((key, next)) = read_varint(data, i) else {
            return;
        };
        i = next;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        if wire == 2 {
            let Some((len, next)) = read_varint(data, i) else {
                return;
            };
            i = next;
            let end = i.saturating_add(len as usize);
            if end > data.len() {
                return;
            }
            visit(field, &data[i..end]);
            i = end;
        } else {
            let Some(next) = skip_field(data, i, wire) else {
                return;
            };
            i = next;
        }
    }
}

/// 把 length-delimited 载荷解成候选模型名（必须通过 `looks_like_model_id`）。
fn candidate_model_text(value: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(value).ok()?;
    looks_like_model_id(text).then_some(text)
}

/// 从 gen_metadata / executor_metadata 的 protobuf blob 提取会话模型名。
///
/// 只认实测的真实路径，**绝不**做"任意字段里的模型名探测"：
/// - 顶层 field 1（消息集）→ 直接子字段 19 = 实际模型（如 gemini-3.8-flash）
/// - 顶层 field 3（运行变体）→ 直接子字段 28 = 请求模型（如 gemini-3.8-flash-high）
///
/// 早期实现把"任何含 `-` 的短 ASCII 串"兜底当作模型名，结果 fallback 先于
/// field 19 命中实验开关集合（enable-owl-slash-command、jetski-* 等），真实
/// 模型名反被 `get_or_insert` 弃掉——统计里因此出现开关名"模型"。回归测试
/// `collect_model_strings_ignores_experiment_flags` 锁定这一点。
fn collect_model_strings(
    data: &[u8],
    model: &mut Option<String>,
    request_model: &mut Option<String>,
) {
    for_each_length_delimited(data, |field, payload| match field {
        1 => {
            for_each_length_delimited(payload, |inner, value| {
                if inner == 19 {
                    if let Some(text) = candidate_model_text(value) {
                        model.get_or_insert_with(|| text.to_string());
                    }
                }
            });
        }
        3 => {
            for_each_length_delimited(payload, |inner, value| {
                if inner == 28 {
                    if let Some(text) = candidate_model_text(value) {
                        request_model.get_or_insert_with(|| text.to_string());
                    }
                }
            });
        }
        _ => {}
    });
}

fn extract_usage_from_metadata(blob: &[u8], models: &ConversationModels) -> Option<UsageHit> {
    let nested = find_length_delimited(blob, 9)?;
    let mut input = 0i64;
    let mut output = 0i64;
    let mut thoughts = 0i64;
    let mut cached = 0i64;
    let mut i = 0;
    while i < nested.len() {
        let (key, next) = read_varint(nested, i)?;
        i = next;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        match (field, wire) {
            (2, 0) => {
                let (v, next) = read_varint(nested, i)?;
                input = v as i64;
                i = next;
            }
            (3, 0) => {
                let (v, next) = read_varint(nested, i)?;
                output = v as i64;
                i = next;
            }
            (9, 0) => {
                let (v, next) = read_varint(nested, i)?;
                thoughts = v as i64;
                i = next;
            }
            (10, 0) => {
                let (v, next) = read_varint(nested, i)?;
                cached = v as i64;
                i = next;
            }
            _ => i = skip_field(nested, i, wire)?,
        }
    }
    if input == 0 && output == 0 {
        return None;
    }
    Some(UsageHit {
        model: models.model.clone(),
        request_model: models.request_model.clone(),
        input,
        output: output + thoughts,
        cache_read: cached,
        cache_write: 0,
        cost_usd: 0.0,
        created_at: extract_event_secs(blob).unwrap_or_else(now_secs),
    })
}

/// 从 steps.metadata 提取事件真实时间（秒级 epoch）。
///
/// 实测结构：顶层 field 1 = {field 1: epoch 秒, field 2: 纳秒}，三个库与
/// conversation_summaries.db 的 last_user_input_time、transcript 的
/// created_at 秒级吻合。早期版本用导入时刻 `now_secs()` 导致同批行挤在
/// 扫描当天，无法按时间归位；解析失败仍回退现状语义。
fn extract_event_secs(blob: &[u8]) -> Option<i64> {
    let mut secs = None;
    let mut i = 0;
    while i < blob.len() {
        let (key, next) = read_varint(blob, i)?;
        i = next;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        if wire == 2 && field == 1 {
            let (len, next) = read_varint(blob, i)?;
            i = next;
            let end = i.saturating_add(len as usize);
            if end > blob.len() {
                return None;
            }
            if let Some(value) = read_top_varint(&blob[i..end], 1) {
                // 合法秒级 epoch：2001..2100；防把纳秒/相对值当秒写库
                if (1_000_000_000..4_102_444_800).contains(&value) {
                    secs = Some(value as i64);
                }
            }
            i = end;
            continue;
        }
        i = skip_field(blob, i, wire)?;
    }
    secs
}

/// 读 protobuf 顶层某个 varint 字段（wire type 0；不存在返回 None）。
fn read_top_varint(data: &[u8], want: u32) -> Option<u64> {
    let mut result = None;
    let mut i = 0;
    while i < data.len() {
        let (key, next) = read_varint(data, i)?;
        i = next;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        if wire == 0 {
            let (value, next) = read_varint(data, i)?;
            i = next;
            if field == want {
                result = Some(value);
            }
        } else {
            i = skip_field(data, i, wire)?;
        }
    }
    result
}

fn find_length_delimited(data: &[u8], field: u32) -> Option<&[u8]> {
    let mut i = 0;
    while i < data.len() {
        let (key, next) = read_varint(data, i)?;
        i = next;
        let found = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        if found == field && wire == 2 {
            let (len, next) = read_varint(data, i)?;
            i = next;
            let end = i.saturating_add(len as usize);
            if end > data.len() {
                return None;
            }
            return Some(&data[i..end]);
        }
        i = skip_field(data, i, wire)?;
    }
    None
}

fn read_varint(data: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0;
    while i < data.len() {
        let byte = data[i];
        i += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn skip_field(data: &[u8], i: usize, wire: u32) -> Option<usize> {
    match wire {
        0 => read_varint(data, i).map(|(_, next)| next),
        1 => (i + 8 <= data.len()).then_some(i + 8),
        2 => {
            let (len, next) = read_varint(data, i)?;
            let end = next.saturating_add(len as usize);
            (end <= data.len()).then_some(end)
        }
        5 => (i + 4 <= data.len()).then_some(i + 4),
        _ => None,
    }
}

fn pragma_columns(conn: &Connection, table: &str) -> Vec<String> {
    let query = format!("PRAGMA table_info({table})");
    let Ok(mut stmt) = conn.prepare(&query) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn skips_transcripts_without_token_fields() -> Result<(), AppError> {
        let tmp = tempdir().unwrap();
        let cli = tmp.path();
        let logs = cli
            .join("brain")
            .join("c412aa48-7f19-4a3b-9dd6-9a4811afb3db")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("transcript.jsonl"),
            r#"{"type":"USER_INPUT","content":"hi","created_at":"2026-09-19T10:57:00Z"}
{"type":"PLANNER_RESPONSE","content":"hello","created_at":"2026-09-19T10:57:01Z"}
"#,
        )
        .unwrap();
        let db = Database::memory()?;
        let result = sync_antigravity_usage_from(&db, cli)?;
        assert_eq!(result.imported, 0);
        assert!(result.files_scanned >= 1);
        Ok(())
    }

    #[test]
    fn parse_nested_protobuf_usage_fields() {
        let usage =
            extract_usage_from_metadata(&usage_metadata_blob(), &ConversationModels::default())
                .expect("usage");
        assert_eq!(usage.input, 7366);
        assert_eq!(usage.output, 166 + 135);
        assert_eq!(usage.cache_read, 31);
        assert_eq!(usage.model, "unknown");
    }

    /// 回归：实验开关集合（enable-/disable-/jetski-）绝不能被当成模型名。
    /// 旧实现用"任意字段兜底"，开关名排在 field 19 之前命中，把真实模型挤掉。
    #[test]
    fn collect_model_strings_ignores_experiment_flags() {
        let mut model = None;
        let mut request_model = None;
        collect_model_strings(&gen_metadata_blob(), &mut model, &mut request_model);
        assert_eq!(model.as_deref(), Some("gemini-3.8-flash"));
        assert_eq!(request_model.as_deref(), Some("gemini-3.8-flash-high"));

        // 只有开关的 blob：不得产出任何"模型名"。
        let mut flags_only = Vec::new();
        let mut variant = Vec::new();
        for flag in [
            "enable-owl-slash-command",
            "disable-teamwork-forced-flash-model",
            "jetski-autonomous-mode",
            "use-component-rewrite",
        ] {
            let mut item = Vec::new();
            put_bytes_field(&mut item, 1, flag.as_bytes());
            put_bytes_field(&mut variant, 43, &item);
        }
        put_bytes_field(&mut flags_only, 3, &variant);
        let mut model = None;
        let mut request_model = None;
        collect_model_strings(&flags_only, &mut model, &mut request_model);
        assert_eq!(model, None);
        assert_eq!(request_model, None);
    }

    /// created_at 用 steps.metadata 内嵌的真实事件时间，而非导入时刻。
    #[test]
    fn uses_embedded_event_timestamp() {
        let blob = usage_metadata_blob_at(1_789_865_420);
        let usage =
            extract_usage_from_metadata(&blob, &ConversationModels::default()).expect("usage");
        assert_eq!(usage.created_at, 1_789_865_420);

        // 非法值（纳秒量级）不采纳，回退导入时刻。
        let blob = usage_metadata_blob_at(1_789_865_420_000_000_000);
        let usage =
            extract_usage_from_metadata(&blob, &ConversationModels::default()).expect("usage");
        assert_ne!(
            usage.created_at,
            1_789_865_420_000_000_000_i64 / 1_000_000_000
        );
        assert!(usage.created_at > 1_700_000_000);
    }

    fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn put_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
        encode_varint(u64::from((field << 3) | 0), out);
        encode_varint(value, out);
    }

    fn put_bytes_field(out: &mut Vec<u8>, field: u32, value: &[u8]) {
        encode_varint(u64::from((field << 3) | 2), out);
        encode_varint(value.len() as u64, out);
        out.extend_from_slice(value);
    }

    fn usage_metadata_blob() -> Vec<u8> {
        let mut nested = Vec::new();
        put_varint_field(&mut nested, 2, 7366);
        put_varint_field(&mut nested, 3, 166);
        put_varint_field(&mut nested, 9, 135);
        put_varint_field(&mut nested, 10, 31);
        let mut blob = Vec::new();
        put_bytes_field(&mut blob, 9, &nested);
        blob
    }

    /// steps.metadata 的实测形状：顶层 field 1 = {1: epoch 秒, 2: 纳秒}，
    /// 外加 field 9 的 usage。
    fn usage_metadata_blob_at(epoch_secs: u64) -> Vec<u8> {
        let mut ts = Vec::new();
        put_varint_field(&mut ts, 1, epoch_secs);
        put_varint_field(&mut ts, 2, 866_288_800);
        let mut blob = Vec::new();
        put_bytes_field(&mut blob, 1, &ts);
        let mut nested = Vec::new();
        put_varint_field(&mut nested, 2, 7366);
        put_varint_field(&mut nested, 3, 166);
        put_bytes_field(&mut blob, 9, &nested);
        blob
    }

    /// gen_metadata 的实测形状（按真实序列化顺序：2、3、4、1）：
    /// - 顶层 field 3（运行变体）含直接子字段 28 = 请求模型，以及 field 43
    ///   （实验开关集合，每个 {1: 开关名}）——**排在 field 28 之后、顶层
    ///   field 1 之前**，正是旧兜底逻辑抢先污染 model 的顺序；
    /// - 顶层 field 1（消息集）含直接子字段 19 = 实际模型。
    fn gen_metadata_blob() -> Vec<u8> {
        let mut variant = Vec::new();
        put_bytes_field(&mut variant, 28, b"gemini-3.8-flash-high");
        for flag in [
            "enable-owl-slash-command",
            "enable-generative-hooks",
            "jetski-unified-customizations-panel-enabled",
            "disable-teamwork-forced-flash-model",
        ] {
            let mut item = Vec::new();
            put_bytes_field(&mut item, 1, flag.as_bytes());
            put_bytes_field(&mut variant, 43, &item);
        }
        let mut inner = Vec::new();
        put_bytes_field(&mut inner, 19, b"gemini-3.8-flash");
        let mut blob = Vec::new();
        put_bytes_field(&mut blob, 2, &[1]);
        put_bytes_field(&mut blob, 3, &variant);
        put_bytes_field(&mut blob, 4, b"34ea529c-996c-4a94-98c4-4b8e31ffee81");
        put_bytes_field(&mut blob, 1, &inner);
        blob
    }

    fn write_conversation_db(path: &Path, metadata: &[u8], gen: &[u8]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE steps (
                idx INTEGER,
                step_type INTEGER,
                metadata BLOB
             );
             CREATE TABLE gen_metadata (data BLOB);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO steps (idx, step_type, metadata) VALUES (0, 14, NULL), (1, 15, ?1)",
            rusqlite::params![metadata],
        )
        .unwrap();
        if !gen.is_empty() {
            conn.execute(
                "INSERT INTO gen_metadata (data) VALUES (?1)",
                rusqlite::params![gen],
            )
            .unwrap();
        }
    }

    #[test]
    fn imports_conversation_db_model_from_gen_metadata() -> Result<(), AppError> {
        let tmp = tempdir().unwrap();
        let conversations = tmp.path().join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let session_id = "c412aa48-7f19-4a3b-9dd6-9a4811afb3db";
        write_conversation_db(
            &conversations.join(format!("{session_id}.db")),
            &usage_metadata_blob(),
            &gen_metadata_blob(),
        );

        let db = Database::memory()?;
        let result = sync_antigravity_usage_from(&db, tmp.path())?;
        assert_eq!(result.imported, 1);

        let conn = lock_conn!(db.conn);
        let (model, request_model, input, output, cache_read): (String, String, i64, i64, i64) =
            conn.query_row(
                "SELECT model, request_model, input_tokens, output_tokens, cache_read_tokens
                 FROM proxy_request_logs WHERE request_id = ?1",
                rusqlite::params![format!("antigravity_{session_id}_1")],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
        assert_eq!(model, "gemini-3.8-flash");
        assert_eq!(request_model, "gemini-3.8-flash-high");
        assert_eq!(input, 7366);
        assert_eq!(output, 301);
        assert_eq!(cache_read, 31);
        Ok(())
    }

    #[test]
    fn backfills_unknown_model_on_resync() -> Result<(), AppError> {
        let tmp = tempdir().unwrap();
        let conversations = tmp.path().join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let session_id = "c412aa48-7f19-4a3b-9dd6-9a4811afb3db";
        let db_path = conversations.join(format!("{session_id}.db"));
        write_conversation_db(&db_path, &usage_metadata_blob(), &[]);

        let db = Database::memory()?;
        assert_eq!(sync_antigravity_usage_from(&db, tmp.path())?.imported, 1);
        {
            let conn = lock_conn!(db.conn);
            let model: String = conn.query_row(
                "SELECT model FROM proxy_request_logs WHERE request_id = ?1",
                rusqlite::params![format!("antigravity_{session_id}_1")],
                |row| row.get(0),
            )?;
            assert_eq!(model, "unknown");
        }

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO gen_metadata (data) VALUES (?1)",
            rusqlite::params![gen_metadata_blob()],
        )
        .unwrap();
        drop(conn);

        assert_eq!(sync_antigravity_usage_from(&db, tmp.path())?.imported, 0);
        let conn = lock_conn!(db.conn);
        let (model, request_model): (String, String) = conn.query_row(
            "SELECT model, request_model FROM proxy_request_logs WHERE request_id = ?1",
            rusqlite::params![format!("antigravity_{session_id}_1")],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(model, "gemini-3.8-flash");
        assert_eq!(request_model, "gemini-3.8-flash-high");
        Ok(())
    }

    /// 旧兜底 bug 已入库的"开关名模型"必须在重扫时自愈：即使文件没变，
    /// 修复判定命中也要重扫，并把 model 与 created_at 一起修正。
    #[test]
    fn repairs_polluted_model_and_timestamp_on_resync() -> Result<(), AppError> {
        let tmp = tempdir().unwrap();
        let conversations = tmp.path().join("conversations");
        fs::create_dir_all(&conversations).unwrap();
        let session_id = "c412aa48-7f19-4a3b-9dd6-9a4811afb3db";
        let epoch = 1_789_865_420_i64;
        write_conversation_db(
            &conversations.join(format!("{session_id}.db")),
            &usage_metadata_blob_at(epoch as u64),
            &gen_metadata_blob(),
        );

        let db = Database::memory()?;
        // 预置旧版本写入的污染行（model=开关名、created_at=扫描时刻），
        // 模拟"同文件早已同步过"的存量库。
        {
            let conn = lock_conn!(db.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd,
                    cache_creation_cost_usd, total_cost_usd,
                    latency_ms, status_code, session_id, provider_type, is_streaming,
                    cost_multiplier, created_at, data_source, input_token_semantics
                 ) VALUES (?1, ?2, 'antigravity', 'enable-owl-slash-command', 'gemini-3.8-flash-high',
                    7366, 301, 31, 0, '0', '0', '0', '0', '0',
                    0, 200, ?3, ?4, 1, '1.0', 1789889087, ?4, ?5)",
                rusqlite::params![
                    format!("antigravity_{session_id}_1"),
                    PROVIDER_ID,
                    session_id,
                    DATA_SOURCE,
                    INPUT_TOKEN_SEMANTICS_FRESH,
                ],
            )?;
        }

        assert_eq!(sync_antigravity_usage_from(&db, tmp.path())?.imported, 0);
        let conn = lock_conn!(db.conn);
        let (model, request_model, created_at): (String, String, i64) = conn.query_row(
            "SELECT model, request_model, created_at FROM proxy_request_logs WHERE request_id = ?1",
            rusqlite::params![format!("antigravity_{session_id}_1")],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(model, "gemini-3.8-flash");
        assert_eq!(request_model, "gemini-3.8-flash-high");
        assert_eq!(created_at, epoch);
        Ok(())
    }

    #[test]
    fn imports_transcript_line_with_usage() -> Result<(), AppError> {
        let tmp = tempdir().unwrap();
        let cli = tmp.path();
        let logs = cli
            .join("brain")
            .join("c412aa48-7f19-4a3b-9dd6-9a4811afb3db")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("transcript.jsonl"),
            r#"{"type":"PLANNER_RESPONSE","model":"gemini-3-pro","usage":{"input_tokens":10,"output_tokens":4},"created_at":"2026-09-19T10:57:01Z"}
"#,
        )
        .unwrap();
        let db = Database::memory()?;
        let result = sync_antigravity_usage_from(&db, cli)?;
        assert_eq!(result.imported, 1);
        Ok(())
    }
}
