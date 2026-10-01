use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};

const ENDPOINT: &str = "https://api.openai.com/v1/responses";
const MAX_EVENT: usize = 2_000_000;
const MAX_STREAM: usize = 16_000_000;
const MAX_REPLY: usize = 1_000_000;

pub(crate) fn request(
    client: &Client,
    token: &str,
    model: &str,
    input: Value,
    instructions: &str,
) -> Result<String, String> {
    // No local, hosted or dynamically inherited tools. The OAuth token grants
    // account-specific plan usage; an API key is never read as a fallback.
    let response = client
        .post(ENDPOINT)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .json(&json!({
            "model":model,"input":input,"instructions":instructions,"store":false,"stream":true
        }))
        .send()
        .map_err(|_| {
            "Could not connect to ChatGPT. Your message was not confirmed as completed."
        })?;
    if !response.status().is_success() {
        return Err(http_error(response.status().as_u16()));
    }
    let status = response.status().as_u16();
    let format = content_type(response.headers());
    // SIWC may omit Content-Type on a valid event stream. Validation remains
    // bounded and requires the completed event; explicit other types fail.
    if !matches!(format, "text/event-stream" | "none") {
        return Err(format!(
            "ChatGPT returned an unexpected response format (HTTP {status}; format: {format})."
        ));
    }
    parse_sse(BufReader::new(response))
        .map_err(|error| format!("{error} (HTTP {status}; format: {format})."))
}

fn content_type(headers: &reqwest::header::HeaderMap) -> &'static str {
    let Some(value) = headers.get(reqwest::header::CONTENT_TYPE) else {
        return "none";
    };
    let Ok(value) = value.to_str() else {
        return "other";
    };
    let value = value.split(';').next().unwrap_or("").trim();
    if value.eq_ignore_ascii_case("text/event-stream") {
        "text/event-stream"
    } else if value.eq_ignore_ascii_case("application/json") {
        "application/json"
    } else if value.eq_ignore_ascii_case("text/html") {
        "text/html"
    } else {
        "other"
    }
}

pub(crate) fn http_error(status: u16) -> String {
    match status {
        401|403 => "ChatGPT authorization was rejected. Reconnect and allow ChatGPT plan usage.",
        429 => "Your ChatGPT plan or this app's usage allowance has reached a limit. Manage usage in ChatGPT.",
        400|404 => "This model or message is not supported by your connected ChatGPT account.",
        _ => "ChatGPT could not complete the request. Check OpenAI service status and try again.",
    }.into()
}

fn failure(value: &Value) -> String {
    let code = value["response"]["error"]["code"]
        .as_str()
        .or_else(|| value["error"]["code"].as_str())
        .or_else(|| value["code"].as_str())
        .unwrap_or("");
    match code {
        "subscription_sharing_usage_limit_exceeded" => "This app's ChatGPT plan usage limit has been reached. Manage usage in ChatGPT.",
        "subscription_sharing_usage_unavailable" => "ChatGPT plan usage is unavailable for this connection. Manage usage or reconnect in ChatGPT.",
        "chatpass_v2_scope_not_authorized" => "Your connection does not allow ChatGPT plan usage. Reconnect and review the requested permission.",
        _ => "ChatGPT did not complete this response. Try again or check your account's usage limits.",
    }.into()
}

fn completed_text(value: &Value, accumulated: &str) -> Result<String, String> {
    if value["response"]["status"].as_str() != Some("completed") {
        return Err("ChatGPT did not confirm a completed response.".into());
    }
    if !value["response"]["error"].is_null() {
        return Err("ChatGPT did not confirm an error-free response.".into());
    }
    let output = &value["response"]["output"];
    if !output.is_null() && !output.is_array() {
        return Err("ChatGPT returned invalid completed output.".into());
    }
    let mut parts = Vec::new();
    let mut total = 0;
    if let Some(items) = value["response"]["output"].as_array() {
        for item in items {
            match item["type"].as_str() {
                Some("reasoning") => continue,
                Some("message") => {}
                _ => return Err("ChatGPT returned an unsupported response item.".into()),
            }
            if let Some(blocks) = item["content"].as_array() {
                for block in blocks {
                    if block["type"].as_str() == Some("refusal") {
                        return Err(
                            "ChatGPT did not provide a text response to this request.".into()
                        );
                    }
                    if block["type"].as_str() == Some("output_text") {
                        if let Some(text) = block["text"].as_str() {
                            total += text.len();
                            if total > MAX_REPLY {
                                return Err("ChatGPT reply exceeded the size limit.".into());
                            }
                            parts.push(text);
                        } else {
                            return Err("ChatGPT returned an invalid completed text block.".into());
                        }
                    } else {
                        return Err("ChatGPT returned an unsupported content block.".into());
                    }
                }
            } else {
                return Err("ChatGPT returned an invalid completed message.".into());
            }
        }
    }
    let mut text = parts.join("\n");
    if text.len() > MAX_REPLY {
        return Err("ChatGPT reply exceeded the size limit.".into());
    }
    // Some direct SIWC streams omit text from the terminal output. Deltas are
    // usable only after this completed event and its status have been validated.
    if text.trim().is_empty() {
        text = accumulated.into();
    }
    if text.trim().is_empty() {
        return Err("ChatGPT completed the response without any text.".into());
    }
    Ok(text)
}

pub(crate) fn parse_sse(mut reader: impl BufRead) -> Result<String, String> {
    let mut data = String::new();
    let mut accumulated = String::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let count = reader
            .by_ref()
            .take((MAX_EVENT + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| "The ChatGPT response stream was interrupted.")?;
        if count == 0 {
            return Err("ChatGPT response stream ended before response.completed.".into());
        }
        total += count;
        if line.len() > MAX_EVENT || total > MAX_STREAM {
            return Err("ChatGPT response stream exceeded the size limit.".into());
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| "ChatGPT returned an invalid response stream.")?
            .trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                return Err("ChatGPT response stream ended before response.completed.".into());
            }
            let value: Value = serde_json::from_str(&data)
                .map_err(|_| "ChatGPT returned an invalid stream event.")?;
            data.clear();
            match value["type"].as_str() {
                Some("response.completed") => return completed_text(&value, &accumulated),
                Some("response.output_text.delta") => {
                    let delta = value["delta"]
                        .as_str()
                        .ok_or("ChatGPT returned an invalid text fragment.")?;
                    if accumulated.len() + delta.len() > MAX_REPLY {
                        return Err("ChatGPT reply exceeded the size limit.".into());
                    }
                    accumulated.push_str(delta);
                }
                Some("response.failed" | "response.incomplete" | "error") => {
                    return Err(failure(&value))
                }
                _ => {}
            }
        } else if let Some(content) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(content.strip_prefix(' ').unwrap_or(content));
            if data.len() > MAX_EVENT {
                return Err("ChatGPT stream event exceeded the size limit.".into());
            }
        }
    }
}

#[cfg(test)]
#[path = "responses_adversarial.rs"]
mod independent_tests;
