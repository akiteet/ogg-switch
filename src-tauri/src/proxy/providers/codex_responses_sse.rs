//! Shared builders for the OpenAI Responses SSE envelope.
//!
//! The two Codex streaming converters — `streaming_codex_chat` (Chat Completions SSE →
//! Responses SSE) and `streaming_codex_anthropic` (Anthropic Messages SSE → Responses
//! SSE) — have completely different *input* state machines but must emit the identical
//! Responses event stream the Codex client understands. This module owns that output
//! envelope so the two converters cannot drift when an event's shape changes: a wire fix
//! lands here once instead of being mirrored in both files.
//!
//! Each function is pure — it takes primitives or a caller-built `item` `Value` and
//! returns the exact bytes the converters previously constructed inline. Item shapes that
//! vary per converter (including function, namespace, custom, and tool-search calls)
//! are supplied by the caller via the generic
//! `output_item_added` / `output_item_done` helpers.

use bytes::Bytes;
use futures::Stream;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter for Responses SSE events. The grok CLI (>= 1.0.30) hard-requires
/// a `sequence_number` on every Responses event and fails deserialization without it.
/// A process-wide counter keeps every stream strictly increasing; upstream OpenAI
/// semantics only require the number to increase within a stream.
static RESPONSE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Serialize one Responses SSE event with the standard `event:`/`data:` framing.
pub(crate) fn sse_event(event: &str, mut data: Value) -> Bytes {
    let sequence = RESPONSE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    if let Some(obj) = data.as_object_mut() {
        obj.insert("sequence_number".to_string(), Value::from(sequence));
    }
    Bytes::from(format!(
        "event: {event}\ndata: {}\n\n",
        serde_json::to_string(&data).unwrap_or_default()
    ))
}

/// End index (exclusive) of the first complete SSE frame delimiter.
fn find_event_end(buffer: &str) -> Option<usize> {
    let lf = buffer.find("\n\n").map(|index| index + 2);
    let crlf = buffer.find("\r\n\r\n").map(|index| index + 4);
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Inject `sequence_number` into every JSON `data:` line of one SSE frame that
/// lacks it. Frames whose payload is not a JSON object (comments, `[DONE]`,
/// pretty-printed bodies) pass through untouched.
fn normalize_frame(frame: String, counter: &mut u64) -> String {
    if !frame.contains("data:") {
        return frame;
    }
    let mut out = String::with_capacity(frame.len() + 32);
    let mut modified = false;
    for line in frame.split_inclusive('\n') {
        let rebuilt = line.strip_prefix("data:").and_then(|payload| {
            let trimmed = payload.trim();
            if trimmed == "[DONE]" {
                return None;
            }
            let mut value = trimmed.parse::<Value>().ok()?;
            if !value.is_object() || value.get("sequence_number").is_some() {
                return None;
            }
            let obj = value.as_object_mut()?;
            obj.insert(
                "sequence_number".to_string(),
                Value::from(*counter),
            );
            *counter += 1;
            Some(format!(
                "data: {}\n",
                serde_json::to_string(&value).unwrap_or_else(|_| trimmed.to_string())
            ))
        });
        match rebuilt {
            Some(new_line) => {
                out.push_str(&new_line);
                modified = true;
            }
            None => out.push_str(line),
        }
    }
    if modified { out } else { frame }
}

/// Wrap a native-Responses passthrough byte stream so every event carries a
/// `sequence_number` (grok CLI >= 1.0.30 fails to deserialize events without
/// it, and some third-party Responses relays omit the field). Events that
/// already carry one are forwarded unchanged; injected values use a per-stream
/// monotonic counter.
///
/// Multi-line (pretty-printed) JSON payloads and unknown binary tails are
/// passed through unchanged; frame splitting understands `\n\n` and `\r\n\r\n`.
pub(crate) fn sequence_number_stream(
    stream: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    use futures::StreamExt;
    let mut input = Box::pin(stream);
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut counter: u64 = 0;
        'outer: loop {
            while let Some(end) = find_event_end(&buffer) {
                let frame = buffer.drain(..end).collect::<String>();
                yield Ok(Bytes::from(normalize_frame(frame, &mut counter)));
            }
            if buffer.len() > 1_048_576 {
                // Safety valve: a pathological stream without frame delimiters
                // must not buffer forever. Flush lossily in large chunks.
                let frame = std::mem::take(&mut buffer);
                yield Ok(Bytes::from(normalize_frame(frame, &mut counter)));
            }
            match input.next().await {
                Some(Ok(bytes)) => {
                    utf8_remainder.extend_from_slice(&bytes);
                    match std::str::from_utf8(&utf8_remainder) {
                        Ok(text) => {
                            buffer.push_str(text);
                            utf8_remainder.clear();
                        }
                        Err(error) => {
                            let valid = error.valid_up_to();
                            if valid > 0 {
                                buffer.push_str(&String::from_utf8_lossy(&utf8_remainder[..valid]));
                                utf8_remainder.drain(..valid);
                            } else if utf8_remainder.len() > 4 {
                                buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
                                utf8_remainder.clear();
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    yield Err(e);
                    break 'outer;
                }
                None => {
                    if !utf8_remainder.is_empty() {
                        buffer.push_str(&String::from_utf8_lossy(&utf8_remainder));
                        utf8_remainder.clear();
                    }
                    if !buffer.is_empty() {
                        let frame = std::mem::take(&mut buffer);
                        yield Ok(Bytes::from(normalize_frame(frame, &mut counter)));
                    }
                    break 'outer;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Response lifecycle (created / in_progress / completed / failed)
// ---------------------------------------------------------------------------

/// `response.created`, wrapping a caller-built `response` object (usage/created_at differ
/// per converter, so the caller supplies the whole object).
pub(crate) fn response_created(response: &Value) -> Bytes {
    sse_event(
        "response.created",
        json!({ "type": "response.created", "response": response }),
    )
}

/// `response.in_progress`.
pub(crate) fn response_in_progress(response: &Value) -> Bytes {
    sse_event(
        "response.in_progress",
        json!({ "type": "response.in_progress", "response": response }),
    )
}

/// `response.completed`.
pub(crate) fn response_completed(response: &Value) -> Bytes {
    sse_event(
        "response.completed",
        json!({ "type": "response.completed", "response": response }),
    )
}

/// `response.failed`.
pub(crate) fn response_failed(response: &Value) -> Bytes {
    sse_event(
        "response.failed",
        json!({ "type": "response.failed", "response": response }),
    )
}

// ---------------------------------------------------------------------------
// Generic output-item add/done (item value supplied by the caller)
// ---------------------------------------------------------------------------

/// `response.output_item.added` with a caller-built item (message / reasoning /
/// function_call / custom_tool_call).
pub(crate) fn output_item_added(output_index: u32, item: &Value) -> Bytes {
    sse_event(
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": item
        }),
    )
}

/// `response.output_item.done` with a caller-built item.
pub(crate) fn output_item_done(output_index: u32, item: &Value) -> Bytes {
    sse_event(
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "output_index": output_index,
            "item": item
        }),
    )
}

// ---------------------------------------------------------------------------
// Assistant message (text) lifecycle
// ---------------------------------------------------------------------------

/// `response.output_item.added` for an in-progress assistant message.
pub(crate) fn message_item_added(output_index: u32, item_id: &str) -> Bytes {
    output_item_added(
        output_index,
        &json!({
            "id": item_id,
            "type": "message",
            "status": "in_progress",
            "role": "assistant",
            "content": []
        }),
    )
}

/// `response.content_part.added` for the (empty) output_text part of a message.
pub(crate) fn message_content_part_added(output_index: u32, item_id: &str) -> Bytes {
    sse_event(
        "response.content_part.added",
        json!({
            "type": "response.content_part.added",
            "item_id": item_id,
            "output_index": output_index,
            "content_index": 0,
            "part": { "type": "output_text", "text": "", "annotations": [] }
        }),
    )
}

/// `response.output_text.delta`.
pub(crate) fn output_text_delta(output_index: u32, item_id: &str, delta: &str) -> Bytes {
    sse_event(
        "response.output_text.delta",
        json!({
            "type": "response.output_text.delta",
            "item_id": item_id,
            "output_index": output_index,
            "content_index": 0,
            "delta": delta
        }),
    )
}

/// The completed assistant-message item value.
pub(crate) fn message_item(item_id: &str, text: &str) -> Value {
    json!({
        "id": item_id,
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text, "annotations": [] }]
    })
}

/// Close an assistant message: emits `output_text.done` → `content_part.done` →
/// `output_item.done`, and returns the completed item so the caller can record it.
pub(crate) fn message_close(output_index: u32, item_id: &str, text: &str) -> (Vec<Bytes>, Value) {
    let item = message_item(item_id, text);
    let events = vec![
        sse_event(
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "text": text
            }),
        ),
        sse_event(
            "response.content_part.done",
            json!({
                "type": "response.content_part.done",
                "item_id": item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": text, "annotations": [] }
            }),
        ),
        output_item_done(output_index, &item),
    ];
    (events, item)
}

// ---------------------------------------------------------------------------
// Reasoning (summary) lifecycle
// ---------------------------------------------------------------------------

/// `response.output_item.added` for an in-progress reasoning item.
pub(crate) fn reasoning_item_added(output_index: u32, item_id: &str) -> Bytes {
    output_item_added(
        output_index,
        &json!({
            "id": item_id,
            "type": "reasoning",
            "status": "in_progress",
            "summary": []
        }),
    )
}

/// `response.reasoning_summary_part.added` for the (empty) summary part.
pub(crate) fn reasoning_summary_part_added(output_index: u32, item_id: &str) -> Bytes {
    sse_event(
        "response.reasoning_summary_part.added",
        json!({
            "type": "response.reasoning_summary_part.added",
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "part": { "type": "summary_text", "text": "" }
        }),
    )
}

/// `response.reasoning_summary_text.delta`.
pub(crate) fn reasoning_summary_text_delta(output_index: u32, item_id: &str, delta: &str) -> Bytes {
    sse_event(
        "response.reasoning_summary_text.delta",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "delta": delta
        }),
    )
}

/// The completed reasoning item value (note: no `status` field, matching both converters).
pub(crate) fn reasoning_item(item_id: &str, text: &str) -> Value {
    json!({
        "id": item_id,
        "type": "reasoning",
        "summary": [{ "type": "summary_text", "text": text }]
    })
}

/// Close a reasoning item: emits `reasoning_summary_text.done` →
/// `reasoning_summary_part.done` → `output_item.done`, and returns the completed item.
pub(crate) fn reasoning_close(output_index: u32, item_id: &str, text: &str) -> (Vec<Bytes>, Value) {
    let item = reasoning_item(item_id, text);
    let events = reasoning_close_with_item(output_index, item_id, text, &item, true);
    (events, item)
}

/// Close a reasoning item whose completed shape is supplied by the converter.
/// Anthropic uses this to attach opaque signed/redacted thinking in
/// `encrypted_content` while keeping the standard Responses event lifecycle.
pub(crate) fn reasoning_close_with_item(
    output_index: u32,
    item_id: &str,
    text: &str,
    item: &Value,
    has_visible_summary: bool,
) -> Vec<Bytes> {
    let mut events = Vec::new();
    if has_visible_summary {
        events.extend([
            sse_event(
                "response.reasoning_summary_text.done",
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "text": text
                }),
            ),
            sse_event(
                "response.reasoning_summary_part.done",
                json!({
                    "type": "response.reasoning_summary_part.done",
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": text }
                }),
            ),
        ]);
    }
    events.push(output_item_done(output_index, item));
    events
}

// ---------------------------------------------------------------------------
// Tool-call argument streaming (item value supplied by the caller)
// ---------------------------------------------------------------------------

/// `response.function_call_arguments.delta`.
pub(crate) fn function_call_arguments_delta(
    output_index: u32,
    item_id: &str,
    delta: &str,
) -> Bytes {
    sse_event(
        "response.function_call_arguments.delta",
        json!({
            "type": "response.function_call_arguments.delta",
            "item_id": item_id,
            "output_index": output_index,
            "delta": delta
        }),
    )
}

/// `response.function_call_arguments.done`.
pub(crate) fn function_call_arguments_done(
    output_index: u32,
    item_id: &str,
    arguments: &str,
) -> Bytes {
    sse_event(
        "response.function_call_arguments.done",
        json!({
            "type": "response.function_call_arguments.done",
            "item_id": item_id,
            "output_index": output_index,
            "arguments": arguments
        }),
    )
}

/// `response.custom_tool_call_input.delta` (Chat freeform tools only).
pub(crate) fn custom_tool_call_input_delta(output_index: u32, item_id: &str, delta: &str) -> Bytes {
    sse_event(
        "response.custom_tool_call_input.delta",
        json!({
            "type": "response.custom_tool_call_input.delta",
            "item_id": item_id,
            "output_index": output_index,
            "delta": delta
        }),
    )
}

/// `response.custom_tool_call_input.done` (Chat freeform tools only).
pub(crate) fn custom_tool_call_input_done(output_index: u32, item_id: &str, input: &str) -> Bytes {
    sse_event(
        "response.custom_tool_call_input.done",
        json!({
            "type": "response.custom_tool_call_input.done",
            "item_id": item_id,
            "output_index": output_index,
            "input": input
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(bytes: &Bytes) -> String {
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn sse_event_framing() {
        let ev = sse_event("response.created", json!({ "a": 1 }));
        assert!(body(&ev).starts_with("event: response.created\ndata: "));
        assert!(body(&ev).contains("\"a\":1"));
        assert!(body(&ev).contains("\"sequence_number\":"));
    }

    #[test]
    fn message_close_shapes_match_legacy() {
        let (events, item) = message_close(2, "resp_1_msg", "hi");
        assert_eq!(events.len(), 3);
        assert!(body(&events[0]).contains("\"type\":\"response.output_text.done\""));
        assert!(body(&events[0]).contains("\"text\":\"hi\""));
        assert!(body(&events[1]).contains("\"type\":\"response.content_part.done\""));
        assert!(body(&events[2]).contains("\"type\":\"response.output_item.done\""));
        assert_eq!(item["type"], "message");
        assert_eq!(item["status"], "completed");
        assert_eq!(item["content"][0]["text"], "hi");
    }

    #[test]
    fn reasoning_close_item_has_no_status() {
        let (events, item) = reasoning_close(0, "rs_1", "because");
        assert_eq!(events.len(), 3);
        assert!(body(&events[0]).contains("\"type\":\"response.reasoning_summary_text.done\""));
        assert!(body(&events[1]).contains("\"type\":\"response.reasoning_summary_part.done\""));
        // The completed reasoning item intentionally carries no `status` field.
        assert!(item.get("status").is_none());
        assert_eq!(item["summary"][0]["text"], "because");
    }

    #[test]
    fn message_item_added_is_in_progress() {
        let ev = message_item_added(0, "m1");
        let s = body(&ev);
        assert!(s.contains("\"type\":\"response.output_item.added\""));
        assert!(s.contains("\"status\":\"in_progress\""));
        assert!(s.contains("\"role\":\"assistant\""));
    }

    #[test]
    fn function_call_argument_events() {
        assert!(body(&function_call_arguments_delta(1, "fc_x", "{\"a\":"))
            .contains("\"type\":\"response.function_call_arguments.delta\""));
        assert!(body(&function_call_arguments_done(1, "fc_x", "{\"a\":1}"))
            .contains("\"arguments\":\"{\\\"a\\\":1}\""));
    }
}
