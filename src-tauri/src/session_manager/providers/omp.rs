//! Oh My Pi (OMP) 会话扫描器。
//!
//! 真实布局（实测 omp v18.x）：
//!   ~/.omp/agent/sessions/<encoded-cwd>/<ISO-ts>_<sessionId>.jsonl
//! 每个 .jsonl 是 append-only 记录流，首两行为元数据：
//!   {"type":"title","title":"...","updatedAt":"..."}
//!   {"type":"session","id":"...","timestamp":"...","cwd":"E:\\Omp"}
//! 之后是 {"type":"message","timestamp":"...","message":{"role","content":[...]}}。

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::get_home_dir;
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{extract_text, parse_timestamp_to_ms, truncate_summary, TITLE_MAX_CHARS};

pub fn session_roots() -> Vec<PathBuf> {
    vec![get_home_dir().join(".omp").join("agent").join("sessions")]
}

/// 递归列出全部 OMP 会话 JSONL 文件。
///
/// 用量导入器与测试共用；排序保证同步顺序稳定。
pub(crate) fn session_files() -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for root in session_roots() {
        collect_session_files(&root, &mut files);
    }
    files.sort();
    Ok(files)
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let mut files = Vec::new();
    for root in session_roots() {
        collect_session_files(&root, &mut files);
    }
    let mut sessions: Vec<SessionMeta> = files.iter().filter_map(|p| parse_session(p)).collect();
    // 最近活跃优先
    sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open OMP session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if role != "user" && role != "assistant" {
            continue;
        }
        let content = message.get("content").map(extract_text).unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        messages.push(SessionMessage { role, content, ts });
    }

    Ok(messages)
}

pub fn delete_session(root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    if !path.starts_with(root) {
        return Err(format!(
            "OMP session source is outside the session root: {}",
            path.display()
        ));
    }
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return Err(format!("Unexpected OMP session source: {}", path.display()));
    }
    // 文件名形如 <ts>_<sessionId>.jsonl——校验末尾段与 session_id 一致
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("Invalid OMP session filename: {}", path.display()))?;
    let file_session_id = stem.rsplit('_').next().unwrap_or(stem);
    if file_session_id != session_id && stem != session_id {
        return Err(format!(
            "OMP session ID mismatch: expected {session_id}, found {file_session_id}"
        ));
    }
    std::fs::remove_file(path)
        .map_err(|e| format!("Failed to delete OMP session file {}: {e}", path.display()))?;
    // 附带删除同名 blobs 目录（若存在）
    let blobs = path.with_extension("");
    let _ = std::fs::remove_dir_all(blobs);
    Ok(true)
}

fn collect_session_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut session_id = stem.rsplit('_').next().unwrap_or(stem).to_string();
    let mut title: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut last_active_at: Option<i64> = None;

    // 只需要前几条元数据行
    for line in reader.lines().take(8).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("title") => {
                let t = value.get("title").and_then(Value::as_str).unwrap_or("");
                if !t.trim().is_empty() {
                    title = Some(truncate_summary(t, TITLE_MAX_CHARS));
                }
            }
            Some("session") => {
                if let Some(id) = value.get("id").and_then(Value::as_str) {
                    if !id.trim().is_empty() {
                        session_id = id.to_string();
                    }
                }
                project_dir = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
            }
            _ => {}
        }
    }

    // 用文件修改时间做兜底活跃时间
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                let ms = dur.as_millis() as i64;
                last_active_at = Some(ms);
                if created_at.is_none() {
                    created_at = Some(ms);
                }
            }
        }
    }

    Some(SessionMeta {
        provider_id: "omp".to_string(),
        session_id: session_id.clone(),
        title,
        summary: None,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: Some(format!("omp --resume {session_id}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scans_omp_session_layout() {
        let temp = tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions").join("--E--Omp--");
        std::fs::create_dir_all(&sessions).expect("create dir");
        let session_id = "01a07ae5-3c1c-768c-a0aa-ef7977e29109";
        let file = sessions.join(format!("2026-09-07T08-03-58-620Z_{session_id}.jsonl"));
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"title","title":"Fix the build"}"#,
                "\n",
                r#"{"type":"session","id":"01a07ae5-3c1c-768c-a0aa-ef7977e29109","timestamp":"2026-09-07T08:03:58.620Z","cwd":"E:\\Omp"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello"}]}}"#,
                "\n"
            ),
        )
        .expect("write session");

        let mut files = Vec::new();
        collect_session_files(temp.path(), &mut files);
        let metas: Vec<_> = files.iter().filter_map(|p| parse_session(p)).collect();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].provider_id, "omp");
        assert_eq!(metas[0].session_id, session_id);
        assert_eq!(metas[0].title.as_deref(), Some("Fix the build"));
        assert_eq!(metas[0].project_dir.as_deref(), Some("E:\\Omp"));
        assert_eq!(
            metas[0].resume_command.as_deref(),
            Some(format!("omp --resume {session_id}").as_str())
        );
    }

    #[test]
    fn loads_omp_messages() {
        let temp = tempdir().expect("tempdir");
        let file = temp.path().join("s.jsonl");
        std::fs::write(
            &file,
            concat!(
                r#"{"type":"title","title":"t"}"#,
                "\n",
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#,
                "\n",
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"ho"}]}}"#,
                "\n"
            ),
        )
        .expect("write");
        let msgs = load_messages(&file).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hi");
        assert_eq!(msgs[1].content, "ho");
    }
}
