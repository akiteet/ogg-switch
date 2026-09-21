//! Antigravity CLI（agy）会话扫描器。
//!
//! 会话身份是 UUID。本地索引来自（并集）：
//! - `conversations/<uuid>.db`
//! - `brain/<uuid>/`
//! - `conversation_summaries.db`
//! - `cache/conversation_metadata.json`
//! - `cache/last_conversations.json`（每工作区只留最新 UUID）
//!
//! 正文：`brain/<uuid>/.system_generated/logs/transcript.jsonl`
//! （fallback `transcript_full.jsonl`）。真实 schema 是
//! `USER_INPUT` / `PLANNER_RESPONSE` + `created_at`，不是 `role=user`。
//! 恢复命令：`agy --conversation <uuid>`。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use crate::antigravity_config::get_antigravity_cli_dir;
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{parse_timestamp_to_ms, truncate_summary, TITLE_MAX_CHARS};

const PROVIDER_ID: &str = "antigravity";

pub fn session_roots() -> Vec<PathBuf> {
    vec![get_antigravity_cli_dir()]
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    scan_sessions_from(&get_antigravity_cli_dir())
}

pub(crate) fn scan_sessions_from(cli_dir: &Path) -> Vec<SessionMeta> {
    if !cli_dir.exists() {
        return Vec::new();
    }

    let mut by_id: HashMap<String, Draft> = HashMap::new();

    collect_conversation_dbs(cli_dir, &mut by_id);
    collect_brain_dirs(cli_dir, &mut by_id);
    merge_summaries_db(cli_dir, &mut by_id);
    merge_metadata_json(cli_dir, &mut by_id);
    merge_last_conversations(cli_dir, &mut by_id);

    let mut sessions: Vec<SessionMeta> = by_id
        .into_iter()
        .filter_map(|(id, draft)| draft.into_meta(&id, cli_dir))
        .collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
    sessions
}

#[derive(Default)]
struct Draft {
    title: Option<String>,
    summary: Option<String>,
    project_dir: Option<String>,
    created_at: Option<i64>,
    last_active_at: Option<i64>,
    source_path: Option<PathBuf>,
}

impl Draft {
    fn into_meta(self, session_id: &str, cli_dir: &Path) -> Option<SessionMeta> {
        if !is_conversation_uuid(session_id) {
            return None;
        }
        let source_path = self
            .source_path
            .or_else(|| preferred_source_path(cli_dir, session_id));
        Some(SessionMeta {
            provider_id: PROVIDER_ID.to_string(),
            session_id: session_id.to_string(),
            title: self.title.filter(|s| !s.is_empty()),
            summary: self.summary.filter(|s| !s.is_empty()),
            project_dir: self.project_dir.filter(|s| !s.is_empty()),
            created_at: self.created_at,
            last_active_at: self.last_active_at.or(self.created_at),
            source_path: source_path.map(|p| p.to_string_lossy().to_string()),
            resume_command: Some(format!("agy --conversation {session_id}")),
        })
    }
}

fn preferred_source_path(cli_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let transcript = transcript_path(cli_dir, session_id);
    if transcript.exists() {
        return Some(transcript);
    }
    let full = transcript_full_path(cli_dir, session_id);
    if full.exists() {
        return Some(full);
    }
    let db = conversation_db_path(cli_dir, session_id);
    if db.exists() {
        return Some(db);
    }
    let brain = brain_dir(cli_dir, session_id);
    if brain.exists() {
        return Some(brain);
    }
    None
}

fn transcript_path(cli_dir: &Path, session_id: &str) -> PathBuf {
    brain_dir(cli_dir, session_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl")
}

fn transcript_full_path(cli_dir: &Path, session_id: &str) -> PathBuf {
    brain_dir(cli_dir, session_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript_full.jsonl")
}

fn conversation_db_path(cli_dir: &Path, session_id: &str) -> PathBuf {
    cli_dir
        .join("conversations")
        .join(format!("{session_id}.db"))
}

fn brain_dir(cli_dir: &Path, session_id: &str) -> PathBuf {
    cli_dir.join("brain").join(session_id)
}

fn collect_conversation_dbs(cli_dir: &Path, by_id: &mut HashMap<String, Draft>) {
    let dir = cli_dir.join("conversations");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("db") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_conversation_uuid(id) {
            continue;
        }
        let draft = by_id.entry(id.to_string()).or_default();
        if draft.last_active_at.is_none() {
            draft.last_active_at = file_modified_ms(&path);
        }
        if draft.source_path.is_none() {
            draft.source_path = Some(path);
        }
    }
}

fn collect_brain_dirs(cli_dir: &Path, by_id: &mut HashMap<String, Draft>) {
    let dir = cli_dir.join("brain");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_conversation_uuid(id) {
            continue;
        }
        let draft = by_id.entry(id.to_string()).or_default();
        if let Some(transcript) = preferred_source_path(cli_dir, id) {
            if transcript.extension().and_then(|e| e.to_str()) == Some("jsonl")
                || draft.source_path.is_none()
            {
                draft.source_path = Some(transcript);
            }
        }
        if draft.title.is_none() {
            if let Some(messages) = load_transcript_messages(cli_dir, id)
                .ok()
                .filter(|m| !m.is_empty())
            {
                if let Some(user) = messages.iter().find(|m| m.role == "user") {
                    let extracted = extract_user_request(&user.content);
                    let title_src = if extracted.is_empty() {
                        user.content.as_str()
                    } else {
                        extracted.as_str()
                    };
                    draft.title = Some(truncate_summary(title_src, TITLE_MAX_CHARS));
                }
                if draft.last_active_at.is_none() {
                    draft.last_active_at = messages.iter().rev().find_map(|m| m.ts);
                }
                if draft.created_at.is_none() {
                    draft.created_at = messages.iter().find_map(|m| m.ts);
                }
            }
        }
        if draft.last_active_at.is_none() {
            draft.last_active_at = file_modified_ms(&path);
        }
    }
}

fn merge_summaries_db(cli_dir: &Path, by_id: &mut HashMap<String, Draft>) {
    let path = cli_dir.join("conversation_summaries.db");
    if !path.exists() {
        return;
    }
    let Ok(conn) = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT conversation_id, title, preview, last_modified_time, workspace_uris, last_user_input_time
         FROM conversation_summaries",
    ) else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok(SummaryRow {
            id: row.get::<_, String>(0)?,
            title: row.get::<_, Option<String>>(1)?,
            preview: row.get::<_, Option<String>>(2)?,
            last_modified: row.get::<_, Option<String>>(3)?,
            workspace_uris: row.get::<_, Option<String>>(4)?,
            last_user_input: row.get::<_, Option<String>>(5)?,
        })
    }) else {
        return;
    };
    for row in rows.flatten() {
        apply_summary_row(by_id, row);
    }
}

struct SummaryRow {
    id: String,
    title: Option<String>,
    preview: Option<String>,
    last_modified: Option<String>,
    workspace_uris: Option<String>,
    last_user_input: Option<String>,
}

fn apply_summary_row(by_id: &mut HashMap<String, Draft>, row: SummaryRow) {
    if !is_conversation_uuid(&row.id) {
        return;
    }
    let draft = by_id.entry(row.id).or_default();
    let preview = row.preview.filter(|s| !s.trim().is_empty());
    if let Some(title) = row.title.filter(|s| !s.trim().is_empty()) {
        draft.title = Some(truncate_summary(&title, TITLE_MAX_CHARS));
    } else if draft.title.is_none() {
        if let Some(preview) = preview.as_deref() {
            draft.title = Some(truncate_summary(preview, TITLE_MAX_CHARS));
        }
    }
    if draft.summary.is_none() {
        if let Some(preview) = preview.as_deref() {
            draft.summary = Some(truncate_summary(preview, TITLE_MAX_CHARS));
        }
    }
    if draft.project_dir.is_none() {
        if let Some(uris) = row.workspace_uris.as_deref() {
            draft.project_dir = first_workspace_path(uris);
        }
    }
    let ts = row
        .last_modified
        .as_deref()
        .and_then(parse_flexible_time)
        .or_else(|| row.last_user_input.as_deref().and_then(parse_flexible_time));
    if let Some(ts) = ts {
        draft.last_active_at = Some(draft.last_active_at.map_or(ts, |old| old.max(ts)));
    }
}

fn merge_metadata_json(cli_dir: &Path, by_id: &mut HashMap<String, Draft>) {
    let path = cli_dir.join("cache").join("conversation_metadata.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(conversations) = value.get("conversations").and_then(Value::as_object) else {
        return;
    };
    for (id, entry) in conversations {
        if !is_conversation_uuid(id) {
            continue;
        }
        let draft = by_id.entry(id.clone()).or_default();
        let summary = entry.get("summary");
        let title = summary
            .and_then(|s| s.get("Title"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());
        let preview = summary
            .and_then(|s| s.get("Preview"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());
        if let Some(title) = title {
            draft.title = Some(truncate_summary(title, TITLE_MAX_CHARS));
        } else if draft.title.is_none() {
            if let Some(preview) = preview {
                draft.title = Some(truncate_summary(preview, TITLE_MAX_CHARS));
            }
        }
        if draft.summary.is_none() {
            if let Some(preview) = preview {
                draft.summary = Some(truncate_summary(preview, TITLE_MAX_CHARS));
            }
        }
        if draft.project_dir.is_none() {
            if let Some(uris) = summary.and_then(|s| s.get("WorkspaceURIs")) {
                draft.project_dir = workspace_from_json(uris);
            }
        }
        let ts = entry
            .get("last_modified_time")
            .and_then(Value::as_str)
            .and_then(parse_flexible_time)
            .or_else(|| {
                summary
                    .and_then(|s| s.get("UpdatedAt"))
                    .and_then(Value::as_str)
                    .and_then(parse_flexible_time)
            });
        if let Some(ts) = ts {
            draft.last_active_at = Some(draft.last_active_at.map_or(ts, |old| old.max(ts)));
        }
    }
}

fn merge_last_conversations(cli_dir: &Path, by_id: &mut HashMap<String, Draft>) {
    let path = cli_dir.join("cache").join("last_conversations.json");
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(map) = value.as_object() else {
        return;
    };
    for (workspace, id) in map {
        let Some(session_id) = id.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !is_conversation_uuid(session_id) {
            continue;
        }
        let draft = by_id.entry(session_id.to_string()).or_default();
        if draft.project_dir.is_none() {
            draft.project_dir = Some(workspace.clone());
        }
        if draft.title.is_none() {
            let workspace_name = Path::new(workspace)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| workspace.clone());
            draft.title = Some(truncate_summary(&workspace_name, TITLE_MAX_CHARS));
        }
    }
}

fn load_transcript_messages(
    cli_dir: &Path,
    session_id: &str,
) -> Result<Vec<SessionMessage>, String> {
    let primary = transcript_path(cli_dir, session_id);
    if primary.exists() {
        return load_messages(&primary);
    }
    let fallback = transcript_full_path(cli_dir, session_id);
    if fallback.exists() {
        return load_messages(&fallback);
    }
    Ok(Vec::new())
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if file_name == "last_conversations.json" || file_name == "conversation_metadata.json" {
        return Err(
            "该会话只有索引记录，正文未在本地缓存；在对应工作区运行 `agy --conversation <id>` 可继续该会话"
                .to_string(),
        );
    }
    if path.extension().and_then(|e| e.to_str()) == Some("db") {
        return Ok(Vec::new());
    }
    if path.is_dir() {
        let transcript = path
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");
        if transcript.exists() {
            return load_messages(&transcript);
        }
        let full = path
            .join(".system_generated")
            .join("logs")
            .join("transcript_full.jsonl");
        if full.exists() {
            return load_messages(&full);
        }
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(path).map_err(|e| format!("Failed to read session: {e}"))?;
    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
        return parse_jsonl_messages(&text);
    }
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(messages) = value.get("messages").and_then(Value::as_array) {
            return Ok(messages
                .iter()
                .filter_map(parse_structured_message)
                .collect());
        }
    }
    parse_jsonl_messages(&text)
}

fn parse_jsonl_messages(text: &str) -> Result<Vec<SessionMessage>, String> {
    let mut result = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("message") {
            if let Some(message) = value.get("message") {
                if let Some(msg) = parse_structured_message(message) {
                    result.push(msg);
                }
                continue;
            }
        }
        if let Some(msg) = parse_structured_message(&value) {
            result.push(msg);
        }
    }
    Ok(result)
}

fn parse_structured_message(value: &Value) -> Option<SessionMessage> {
    let type_or_role = value
        .get("type")
        .or_else(|| value.get("role"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let source = value.get("source").and_then(Value::as_str).unwrap_or("");
    let role = match type_or_role {
        "USER_INPUT" | "user" => "user",
        "PLANNER_RESPONSE" | "assistant" | "gemini" | "model" => "assistant",
        _ if source.starts_with("USER") => "user",
        _ if source.eq_ignore_ascii_case("MODEL") => "assistant",
        _ => return None,
    };
    let raw = value.get("content").and_then(Value::as_str).unwrap_or("");
    let content = if role == "user" {
        let extracted = extract_user_request(raw);
        if extracted.is_empty() {
            raw.trim().to_string()
        } else {
            extracted
        }
    } else {
        raw.trim().to_string()
    };
    if content.is_empty() {
        return None;
    }
    let ts = value
        .get("created_at")
        .or_else(|| value.get("timestamp"))
        .or_else(|| value.get("ts"))
        .and_then(|v| {
            v.as_str()
                .and_then(parse_flexible_time)
                .or_else(|| parse_timestamp_to_ms(v))
        });
    Some(SessionMessage {
        role: role.to_string(),
        content,
        ts,
    })
}

fn extract_user_request(content: &str) -> String {
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    let Some(start) = content.find(OPEN) else {
        return String::new();
    };
    let rest = &content[start + OPEN.len()..];
    let inner = rest.split(CLOSE).next().unwrap_or(rest);
    inner.trim().to_string()
}

fn first_workspace_path(uris: &str) -> Option<String> {
    let trimmed = uris.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return workspace_from_json(&value);
    }
    Some(strip_file_uri(trimmed))
}

fn workspace_from_json(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(strip_file_uri)
            .find(|s| !s.is_empty()),
        Value::String(s) => Some(strip_file_uri(s)),
        _ => None,
    }
}

fn strip_file_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("file:///")
        .or_else(|| trimmed.strip_prefix("file://"))
        .unwrap_or(trimmed);
    let decoded = percent_decode(without_scheme);
    if cfg!(windows) {
        decoded.replace('/', "\\")
    } else {
        if decoded.starts_with('/') || Path::new(&decoded).is_absolute() {
            decoded
        } else {
            format!("/{decoded}")
        }
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &input[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_flexible_time(raw: &str) -> Option<i64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with("0001-01-01") {
        return None;
    }
    if let Some(ms) = parse_timestamp_to_ms(&Value::String(trimmed.to_string())) {
        return Some(ms);
    }
    // RFC3339 小数秒超过 6 位时 chrono 会失败，截到毫秒再试。
    if let Some(dot) = trimmed.find('.') {
        let (head, rest) = trimmed.split_at(dot + 1);
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() > 3 {
            let suffix: String = rest.chars().skip_while(|c| c.is_ascii_digit()).collect();
            let normalized = format!("{head}{}{suffix}", &digits[..3]);
            return parse_timestamp_to_ms(&Value::String(normalized));
        }
    }
    None
}

fn file_modified_ms(path: &Path) -> Option<i64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

fn is_conversation_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let is_hex = |b: u8| b.is_ascii_hexdigit();
    for (i, b) in bytes.iter().copied().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            _ if !is_hex(b) => return false,
            _ => {}
        }
    }
    true
}

pub fn delete_session(root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|e| format!("Failed to resolve antigravity root: {e}"))?;
    if path.exists() {
        let canonical_path =
            fs::canonicalize(path).map_err(|e| format!("Failed to resolve session path: {e}"))?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "Session path is outside antigravity roots: {}",
                path.display()
            ));
        }
    }
    if !is_conversation_uuid(session_id) {
        return Err(format!("Invalid Antigravity session id: {session_id}"));
    }

    let db_path = conversation_db_path(&canonical_root, session_id);
    let brain = brain_dir(&canonical_root, session_id);
    let mut deleted = false;
    if db_path.exists() {
        fs::remove_file(&db_path).map_err(|e| format!("Failed to delete conversation db: {e}"))?;
        deleted = true;
    }
    if brain.exists() {
        fs::remove_dir_all(&brain).map_err(|e| format!("Failed to delete brain directory: {e}"))?;
        deleted = true;
    }
    if !deleted && path.exists() && path.is_file() {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "last_conversations.json" || file_name == "conversation_metadata.json" {
            return Err("会话索引文件不支持删除".to_string());
        }
        fs::remove_file(path).map_err(|e| format!("Failed to delete session: {e}"))?;
        deleted = true;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, content).expect("write");
    }

    fn fixture_layout(root: &Path) {
        let latest = "2bc468d4-b833-4e8f-ac2b-1871f6726fc2";
        let empty = "24634430-ecf2-41f8-b2c0-89907db59850";
        let hello = "c412aa48-7f19-4a3b-9dd6-9a4811afb3db";
        let titled = "ef9d0e9e-7961-4b54-b159-d3087a0ec72f";

        write(
            &root.join("cache").join("last_conversations.json"),
            r#"{"E:\\Antigravity":"2bc468d4-b833-4e8f-ac2b-1871f6726fc2"}"#,
        );
        write(
            &root.join("cache").join("conversation_metadata.json"),
            r#"{
              "conversations": {
                "24634430-ecf2-41f8-b2c0-89907db59850": {
                  "summary": {
                    "ID": "24634430-ecf2-41f8-b2c0-89907db59850",
                    "Title": "",
                    "Preview": "",
                    "WorkspaceURIs": ["file:///E:/Antigravity"]
                  },
                  "last_modified_time": "2026-09-19T15:23:15.5592474+08:00"
                },
                "c412aa48-7f19-4a3b-9dd6-9a4811afb3db": {
                  "summary": {
                    "ID": "c412aa48-7f19-4a3b-9dd6-9a4811afb3db",
                    "Title": "",
                    "Preview": "你好",
                    "WorkspaceURIs": ["file:///E:/Antigravity"]
                  },
                  "last_modified_time": "2026-09-19T18:57:34.9619823+08:00"
                },
                "ef9d0e9e-7961-4b54-b159-d3087a0ec72f": {
                  "summary": {
                    "ID": "ef9d0e9e-7961-4b54-b159-d3087a0ec72f",
                    "Title": "Friendly Initial Greeting",
                    "Preview": "你好啊",
                    "WorkspaceURIs": ["file:///E:/Antigravity"]
                  },
                  "last_modified_time": "2026-09-19T15:18:38.9548692+08:00"
                }
              }
            }"#,
        );

        for id in [latest, empty, hello, titled] {
            write(&root.join("conversations").join(format!("{id}.db")), "");
            fs::create_dir_all(root.join("brain").join(id)).expect("brain");
        }

        write(
            &transcript_path(root, hello),
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-09-19T10:57:00Z","content":"<USER_REQUEST>\n你好\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nnow\n</ADDITIONAL_METADATA>"}"#,
                "\n",
                r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-09-19T10:57:01Z","content":"你好！我是 Antigravity"}"#,
                "\n",
            ),
        );
        write(
            &transcript_path(root, titled),
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","created_at":"2026-09-19T07:18:00Z","content":"<USER_REQUEST>\n你好啊\n</USER_REQUEST>"}"#,
                "\n",
                r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-09-19T07:18:01Z","content":"Hi"}"#,
                "\n",
            ),
        );
        write(
            &transcript_path(root, latest),
            concat!(
                r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","created_at":"2026-09-19T20:00:00Z","content":"<USER_REQUEST>\n最新会话\n</USER_REQUEST>"}"#,
                "\n",
                r#"{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-09-19T20:00:01Z","content":"收到"}"#,
                "\n",
            ),
        );
    }

    #[test]
    fn scans_uuid_index_and_real_transcript_schema() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        fixture_layout(root);

        let sessions = scan_sessions_from(root);
        assert_eq!(sessions.len(), 4);

        let by_id: HashMap<_, _> = sessions
            .into_iter()
            .map(|s| (s.session_id.clone(), s))
            .collect();

        let hello = &by_id["c412aa48-7f19-4a3b-9dd6-9a4811afb3db"];
        assert_eq!(hello.title.as_deref(), Some("你好"));
        assert_eq!(
            hello.resume_command.as_deref(),
            Some("agy --conversation c412aa48-7f19-4a3b-9dd6-9a4811afb3db")
        );
        assert!(hello
            .source_path
            .as_deref()
            .unwrap_or("")
            .ends_with("transcript.jsonl"));

        let titled = &by_id["ef9d0e9e-7961-4b54-b159-d3087a0ec72f"];
        assert_eq!(titled.title.as_deref(), Some("Friendly Initial Greeting"));

        let latest = &by_id["2bc468d4-b833-4e8f-ac2b-1871f6726fc2"];
        assert_eq!(latest.title.as_deref(), Some("最新会话"));
        assert!(latest.project_dir.as_deref().is_some());

        let empty = &by_id["24634430-ecf2-41f8-b2c0-89907db59850"];
        assert!(empty.source_path.is_some());
        assert_eq!(
            empty.resume_command.as_deref(),
            Some("agy --conversation 24634430-ecf2-41f8-b2c0-89907db59850")
        );
    }

    #[test]
    fn loads_user_input_planner_response_messages() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("transcript.jsonl");
        write(
            &path,
            concat!(
                r#"{"type":"USER_INPUT","source":"USER_EXPLICIT","created_at":"2026-09-19T10:57:00Z","content":"<USER_REQUEST>\n你好\n</USER_REQUEST>"}"#,
                "\n",
                r#"{"type":"PLANNER_RESPONSE","source":"MODEL","created_at":"2026-09-19T10:57:01Z","content":"你好！"}"#,
                "\n",
            ),
        );
        let messages = load_messages(&path).expect("load");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "你好");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "你好！");
        assert!(messages[0].ts.is_some());
    }

    #[test]
    fn delete_session_removes_db_and_brain() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let id = "c412aa48-7f19-4a3b-9dd6-9a4811afb3db";
        write(&conversation_db_path(root, id), "");
        write(&transcript_path(root, id), "{}\n");
        let sibling = "ef9d0e9e-7961-4b54-b159-d3087a0ec72f";
        write(&conversation_db_path(root, sibling), "keep");
        fs::create_dir_all(brain_dir(root, sibling)).unwrap();

        let deleted = delete_session(root, &transcript_path(root, id), id).expect("delete");
        assert!(deleted);
        assert!(!conversation_db_path(root, id).exists());
        assert!(!brain_dir(root, id).exists());
        assert!(conversation_db_path(root, sibling).exists());
    }

    #[test]
    fn uuid_check_rejects_transcript_stem() {
        assert!(!is_conversation_uuid("transcript"));
        assert!(!is_conversation_uuid("00000000"));
        assert!(is_conversation_uuid("c412aa48-7f19-4a3b-9dd6-9a4811afb3db"));
    }
}
