use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read};

const ENDPOINT: &str = "https://api.openai.com/v1/responses";
const MAX_EVENT: usize = 2_000_000;
const MAX_STREAM: usize = 16_000_000;
const MAX_REPLY: usize = 1_000_000;
const MAX_ERROR_BODY: usize = 64 * 1024;

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
        return Err(response_error(response));
    }
    let status = response.status().as_u16();
    let format = content_type(response.headers());
    // SIWC may omit Content-Type on a valid event stream. Validation remains
    // bounded and requires the completed event; explicit other types fail.
    if !matches!(format, "text/event-stream" | "none") {
        if format == "application/json" {
            return Err(response_error(response));
        }
        return Err(
            StreamFailure::from("ChatGPT returned an unexpected response format.")
                .render(status, format),
        );
    }
    parse_sse_inner(BufReader::new(response)).map_err(|error| error.render(status, format))
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

pub(crate) fn response_error(response: reqwest::blocking::Response) -> String {
    let status = response.status().as_u16();
    let format = content_type(response.headers());
    error_from_reader(status, format, response)
}

fn error_from_reader(status: u16, format: &'static str, reader: impl Read) -> String {
    let mut body = Vec::new();
    let value = if reader
        .take((MAX_ERROR_BODY + 1) as u64)
        .read_to_end(&mut body)
        .is_ok()
        && body.len() <= MAX_ERROR_BODY
    {
        serde_json::from_slice::<Value>(&body).ok()
    } else {
        None
    };
    let code = value.as_ref().map(known_error_code).unwrap_or("unknown");
    provider_message(code, status).render(status, format)
}

fn known_error_code(value: &Value) -> &'static str {
    let mut detail = value
        .get("response")
        .filter(|value| value.is_object())
        .unwrap_or(value);
    for _ in 0..4 {
        if let Some(error) = detail.get("error").filter(|value| value.is_object()) {
            detail = error;
        } else if let Some(nested) = detail.get("detail").filter(|value| value.is_object()) {
            detail = nested;
        } else {
            break;
        }
    }
    let code = detail
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| detail.get("error").and_then(Value::as_str))
        .unwrap_or("");
    allowlisted_code(code)
}

fn allowlisted_code(code: &str) -> &'static str {
    match code {
        "subscription_sharing_user_not_eligible" => "subscription_sharing_user_not_eligible",
        "subscription_sharing_usage_limit_exceeded" => "subscription_sharing_usage_limit_exceeded",
        "subscription_sharing_usage_unavailable" => "subscription_sharing_usage_unavailable",
        "subscription_sharing_unsupported_capability" => {
            "subscription_sharing_unsupported_capability"
        }
        "subscription_sharing_route_not_supported" => "subscription_sharing_route_not_supported",
        "subscription_sharing_invalid_user" => "subscription_sharing_invalid_user",
        "subscription_sharing_user_unavailable" => "subscription_sharing_user_unavailable",
        "chatpass_v2_scope_not_authorized" => "chatpass_v2_scope_not_authorized",
        "chatpass_v2_invalid_authorization_context" => "chatpass_v2_invalid_authorization_context",
        // Earlier direct-route codes stay exact in diagnostics.
        "subscription_sharing_v2_user_not_eligible" => "subscription_sharing_v2_user_not_eligible",
        "subscription_sharing_v2_route_not_supported" => {
            "subscription_sharing_v2_route_not_supported"
        }
        "subscription_sharing_v2_invalid_user" => "subscription_sharing_v2_invalid_user",
        "subscription_sharing_v2_user_unavailable" => "subscription_sharing_v2_user_unavailable",
        "subscription_sharing_v2_client_not_enabled" => {
            "subscription_sharing_v2_client_not_enabled"
        }
        "invalid_client" => "invalid_client",
        "invalid_grant" => "invalid_grant",
        "invalid_refresh_token" => "invalid_refresh_token",
        "token_expired" => "token_expired",
        "refresh_token_expired" => "refresh_token_expired",
        "refresh_token_invalidated" => "refresh_token_invalidated",
        "refresh_token_reused" => "refresh_token_reused",
        "invalid_token" => "invalid_token",
        "invalid_api_key" => "invalid_api_key",
        "access_denied" => "access_denied",
        "model_not_found" => "model_not_found",
        "invalid_request_error" => "invalid_request_error",
        "invalid_request" => "invalid_request",
        _ => "unknown",
    }
}

struct StreamFailure {
    message: String,
    code: &'static str,
}
impl From<&str> for StreamFailure {
    fn from(message: &str) -> Self {
        Self {
            message: message.into(),
            code: "unknown",
        }
    }
}
impl StreamFailure {
    fn render(self, status: u16, format: &'static str) -> String {
        let code = allowlisted_code(self.code);
        let format = match format {
            "text/event-stream" => "text/event-stream",
            "none" => "none",
            "application/json" => "application/json",
            "text/html" => "text/html",
            _ => "other",
        };
        format!(
            "{} (HTTP {status}; code: {code}; format: {format}).",
            self.message
        )
    }
}

fn provider_message(code: &'static str, status: u16) -> StreamFailure {
    let message=match code {
        "subscription_sharing_usage_limit_exceeded"=>"OpenAI reported a ChatGPT sharing limit for this app or account. Check usage and app limits in ChatGPT Settings.",
        "subscription_sharing_usage_unavailable"=>"ChatGPT usage could not be checked right now. Try again shortly and keep your current connection.",
        "subscription_sharing_user_not_eligible"|"subscription_sharing_v2_user_not_eligible"=>"ChatGPT plan usage is unavailable for this account, workspace or policy. Check eligibility and the applicable restrictions.",
        "subscription_sharing_route_not_supported"|"subscription_sharing_v2_route_not_supported"=>"ChatGPT does not support this request route. Check the app's endpoint configuration.",
        "subscription_sharing_unsupported_capability"=>"This model or message uses a capability unsupported by ChatGPT plan usage. Choose a supported model or revise the request.",
        "subscription_sharing_invalid_user"|"subscription_sharing_v2_invalid_user"=>"ChatGPT could not validate the selected account context. Check the connection and granted permissions.",
        "subscription_sharing_user_unavailable"|"subscription_sharing_v2_user_unavailable"=>"Your ChatGPT account or workspace is temporarily unavailable. Try again shortly and keep your current connection.",
        "chatpass_v2_scope_not_authorized"|"chatpass_v2_invalid_authorization_context"=>"Your connection does not allow this ChatGPT plan request. Check the app's client and granted permissions.",
        "subscription_sharing_v2_client_not_enabled"=>"This app is not enabled for ChatGPT plan usage. Check the app's registration.",
        "invalid_client"=>"ChatGPT rejected this app registration. Check the saved client configuration.",
        "invalid_grant"|"invalid_refresh_token"|"token_expired"|"refresh_token_expired"|"refresh_token_invalidated"|"refresh_token_reused"=>"This ChatGPT connection can no longer be renewed. Continue with ChatGPT again in Settings.",
        "invalid_token"|"invalid_api_key"=>"ChatGPT did not accept the selected connection's credential. Check the account and granted permissions.",
        "access_denied"=>"ChatGPT sign-in was not completed. Continue with ChatGPT when you are ready.",
        "model_not_found"=>"This model is not available for your ChatGPT connection. Choose an available model in Settings.",
        "invalid_request_error"|"invalid_request"=>"ChatGPT rejected this request. Check the input, model and supported options.",
        _=>match status {
            400|422=>"ChatGPT rejected this request. Check the input, model and supported options.",
            401=>"ChatGPT did not accept the selected connection's signed identity or direct permission. Check the account and granted permissions.",
            403=>"A ChatGPT policy or permission restriction prevented this request. Check the app's configuration and permitted serving region.",
            404=>"This model or message is not supported by your connected ChatGPT account.",
            429=>"OpenAI reported a rate limit for this request. Wait before trying again and check account or app restrictions.",
            500..=599=>"ChatGPT is temporarily unavailable. Try again shortly and keep your current connection.",
            _=>"ChatGPT did not complete this response. Try again or check the connection.",
        },
    };
    StreamFailure {
        message: message.into(),
        code,
    }
}

#[cfg(test)]
fn http_error(status: u16) -> String {
    provider_message("unknown", status).render(status, "none")
}
#[cfg(test)]
fn failure(value: &Value) -> String {
    provider_message(known_error_code(value), 200).render(200, "text/event-stream")
}

fn completed_text(value: &Value, accumulated: &str) -> Result<String, StreamFailure> {
    if value["response"]["status"].as_str() != Some("completed") {
        return Err("ChatGPT did not confirm a completed response.".into());
    }
    if !value["response"]["error"].is_null() {
        return Err(provider_message(known_error_code(value), 200));
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

#[cfg(test)]
pub(crate) fn parse_sse(reader: impl BufRead) -> Result<String, String> {
    parse_sse_inner(reader).map_err(|error| error.render(200, "text/event-stream"))
}

fn parse_sse_inner(mut reader: impl BufRead) -> Result<String, StreamFailure> {
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
                    return Err(provider_message(known_error_code(&value), 200))
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

#[cfg(test)]
#[path = "responses_error_adversarial.rs"]
mod error_tests;

#[cfg(test)]
mod quota_tests {
    use super::*;

    #[test]
    fn sharing_limit_is_a_provider_signal_without_an_invented_percentage() {
        let message = failure(&json!({"response":{"error":{
            "code":"subscription_sharing_usage_limit_exceeded",
            "message":"synthetic-private-provider-detail"
        }}}));
        assert!(message.contains("OpenAI reported"));
        assert!(message.contains("app or account"));
        assert!(!message.contains('%'));
        assert!(!message.contains("synthetic-private-provider-detail"));
        let rate_limit = http_error(429);
        assert!(rate_limit.contains("rate limit"));
        assert!(!rate_limit.contains("sharing limit"));
        assert!(!rate_limit.contains("plan has"));
    }

    #[test]
    fn unavailable_usage_is_retryable_without_reauthorization_or_private_detail() {
        let message = failure(&json!({"error":{
            "code":"subscription_sharing_usage_unavailable",
            "detail":"synthetic-private-provider-detail"
        }}));
        assert!(message.contains("Try again shortly"));
        assert!(message.contains("keep your current connection"));
        assert!(!message.contains("reconnect"));
        assert!(!message.contains("synthetic-private-provider-detail"));
    }
}
