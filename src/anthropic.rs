//! Minimal Anthropic Messages API client over a plain HTTPS POST. Rust has no
//! official SDK, so we hand-roll the request — build the JSON with serde_json,
//! send the raw string via ureq (no extra ureq features needed), parse the
//! `content[].text` out of the response.
//!
//! Used to generate short "what is this session doing / where is it headed"
//! summaries with the cheapest fast model (Haiku).

use serde_json::{json, Value};

pub const SUMMARY_MODEL: &str = "claude-haiku-4-5";

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

const SYSTEM_PROMPT: &str = "\
You are observing one Claude Code coding session through a snapshot of its \
transcript. Write a tight briefing for someone supervising several sessions at \
once. Use exactly these three short sections, plain text, no markdown headings:\n\
DOING: what the session is working on (1 line).\n\
DONE: the most relevant things accomplished so far (1-3 short lines).\n\
NEXT: where it is headed — the likely next step, or what it is blocked/waiting \
on (1-2 lines).\n\
Be concrete and terse. Infer from tool calls and results. Do not invent facts.";

const ASSESS_PROMPT: &str = "\
You are a security-aware reviewer helping someone decide whether to approve a \
tool call an autonomous coding agent wants to run. Be terse and concrete. Do \
not invent facts beyond the given input.";

/// Summarize a session digest.
pub fn summarize(api_key: &str, model: &str, digest: &str) -> Result<String, String> {
    message(api_key, model, SYSTEM_PROMPT, digest, 512)
}

/// Risk-assess a pending tool call.
pub fn assess(api_key: &str, model: &str, prompt: &str) -> Result<String, String> {
    message(api_key, model, ASSESS_PROMPT, prompt, 256)
}

/// One-shot Messages API call. Errors are returned as human-readable strings.
fn message(
    api_key: &str,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let payload = json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
    })
    .to_string();

    let resp = ureq::post(ENDPOINT)
        .set("x-api-key", api_key)
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("content-type", "application/json")
        .send_string(&payload);

    match resp {
        Ok(r) => {
            let body = r.into_string().map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            if v.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
                return Err("model declined to summarize this session".into());
            }
            extract_text(&v).ok_or_else(|| "no text in API response".to_string())
        }
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {}", api_error_message(&body)))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Concatenate every `text` block in the response content array.
fn extract_text(v: &Value) -> Option<String> {
    let blocks = v.get("content")?.as_array()?;
    let mut out = String::new();
    for b in blocks {
        if b.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(t) = b.get("text").and_then(Value::as_str) {
                out.push_str(t);
            }
        }
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out.trim().to_string())
    }
}

/// Pull the `error.message` out of an API error body, falling back to the raw body.
fn api_error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(160).collect())
}
