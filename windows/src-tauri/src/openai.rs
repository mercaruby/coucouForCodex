// OpenAI Responses API client for Coucou on Windows.
// Keeps the existing module name so the rest of the app does not need a risky refactor.

use crate::secrets;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;

const ENDPOINT: &str = "https://api.openai.com/v1/responses";
const MAX_OUTPUT_TOKENS: u32 = 4096;
const MAX_INLINE_TEXT: u64 = 200_000;

pub const DEFAULT_MODEL: &str = "gpt-5.6-sol";

const SYSTEM_PROMPT: &str = "You are Mochi, a personal AI assistant living at the top of the user's screen. \
You have web search access and can help with research, coding, finding places, recommendations, tasks, and questions. \
Respond in the user's language. Be thorough and complete. \
No markdown formatting (no **, no ##, no bullet dashes). Use plain text with line breaks.";

#[derive(Default, Clone)]
pub struct Chat {
    messages: std::sync::Arc<Mutex<Conversation>>,
    expected_revision: Option<u64>,
}

#[derive(Default)]
struct Conversation {
    revision: u64,
    messages: Vec<Value>,
}

impl Chat {
    pub fn reset(&self) {
        let mut conversation = self.messages.lock().unwrap();
        conversation.revision += 1;
        conversation.messages.clear();
    }
    pub(crate) fn request(&self) -> Self {
        Self {
            messages: self.messages.clone(),
            expected_revision: Some(self.messages.lock().unwrap().revision),
        }
    }
    pub(crate) fn snapshot(&self) -> Result<(u64, Vec<Value>), String> {
        let conversation = self.messages.lock().unwrap();
        if self
            .expected_revision
            .is_some_and(|revision| revision != conversation.revision)
        {
            return Err("The chat changed. Start a new message.".into());
        }
        Ok((conversation.revision, conversation.messages.clone()))
    }
    pub(crate) fn commit(&self, revision: u64, user: Value, reply: Value) -> Result<(), String> {
        let mut conversation = self.messages.lock().unwrap();
        if conversation.revision != revision {
            return Err("The chat changed. Start a new message.".into());
        }
        conversation.messages.extend([user, reply]);
        conversation.revision += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChatContext {
    File {
        name: String,
        path: String,
    },
    Window {
        app_name: String,
        title: String,
        url: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatReply {
    pub text: String,
}

pub async fn send(
    chat: &Chat,
    model: &str,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let key = secrets::get("openai-api-key")
        .ok_or_else(|| "OpenAI API key missing. Open settings.".to_string())?;

    let (revision, mut messages) = chat.snapshot()?;
    let mut content: Vec<Value> = Vec::new();
    if messages.is_empty() {
        match &context {
            Some(ChatContext::File { name, path }) => {
                content.push(file_block(name, path)?);
                content.push(json!({ "type": "input_text", "text": format!("File: {name}") }));
            }
            Some(ChatContext::Window {
                app_name,
                title,
                url,
            }) => {
                let mut text = format!("Context — App: {app_name}, Window: {title}");
                if let Some(url) = url {
                    text.push_str(&format!(", URL: {url}"));
                }
                content.push(json!({ "type": "input_text", "text": text }));
            }
            None => {}
        }
    }
    content.push(json!({ "type": "input_text", "text": query }));
    let user = json!({ "role": "user", "content": content });
    messages.push(user.clone());

    let body = json!({
        "model": if model.is_empty() { DEFAULT_MODEL } else { model },
        "store": false,
        "instructions": SYSTEM_PROMPT,
        "input": messages,
        "tools": [{ "type": "web_search" }],
        "max_output_tokens": MAX_OUTPUT_TOKENS
    });

    let response = match call(&key, &body).await {
        Ok(v) => v,
        Err(err) => {
            return Err(err);
        }
    };

    let text = extract_output_text(&response).ok_or_else(|| "No response text.".to_string())?;
    chat.commit(
        revision,
        user,
        json!({ "role": "assistant", "content": text }),
    )?;
    Ok(ChatReply { text })
}

fn extract_output_text(response: &Value) -> Option<String> {
    let mut parts = Vec::new();
    for item in response.get("output")?.as_array()? {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        if let Some(content) = item.get("content").and_then(Value::as_array) {
            for block in content {
                if block.get("type").and_then(Value::as_str) == Some("output_text") {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(text);
                    }
                }
            }
        }
    }
    let text = parts.join("\n").trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

async fn call(key: &str, body: &Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(ENDPOINT)
        .bearer_auth(key)
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        // Authentication errors can echo part of a bad key. Never forward a raw
        // server response to the frontend or its logs.
        let detail = match status.as_u16() {
            401 | 403 => "Authentication rejected. Check the project's API key and permissions.",
            429 => "Usage or rate limit reached. Check your API project's limits and billing.",
            400 | 404 => "Request or model unsupported. Check the selected model and attachment.",
            _ => "Request failed. Check OpenAI service status and try again.",
        };
        return Err(format!("OpenAI API {status}: {detail}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad API response: {e}"))
}

pub(crate) fn file_block(name: &str, path: &str) -> Result<Value, String> {
    let path = crate::files::checked_inbox_path(path)?;
    let path = path.to_str().ok_or("Attachment path cannot be encoded.")?;
    if std::fs::metadata(path)
        .map_err(|_| "Cannot read attachment.")?
        .len()
        > 20_000_000
    {
        return Err("Attachments must be no larger than 20 MB.".into());
    }
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext == "pdf" {
        let bytes = std::fs::read(path).map_err(|_| "Cannot read attachment.")?;
        return Ok(json!({
            "type": "input_file",
            "filename": name,
            "file_data": format!("data:application/pdf;base64,{}", base64(&bytes))
        }));
    }

    let media = match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    };
    if let Some(media) = media {
        let bytes = std::fs::read(path).map_err(|_| "Cannot read attachment.")?;
        return Ok(json!({
            "type": "input_image",
            "image_url": format!("data:{media};base64,{}", base64(&bytes))
        }));
    }

    let len = std::fs::metadata(path)
        .map_err(|_| "Cannot read attachment.")?
        .len();
    if len > MAX_INLINE_TEXT {
        return Err("Text attachments must be no larger than 200 KB.".into());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|_| "Only UTF-8 text, PDF and supported images can be attached.")?;
    Ok(json!({ "type": "input_text", "text": format!("File contents:\n{text}") }))
}

pub(crate) fn base64_for(bytes: &[u8]) -> String {
    base64(bytes)
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{base64, extract_output_text, Chat, ChatContext};
    use serde_json::json;
    #[test]
    fn submitted_window_context_matches_the_frontend_contract() {
        let context: ChatContext = serde_json::from_value(json!({
            "kind":"window","appName":"Browser","title":"Submitted page",
            "url":"https://example.com/page"
        }))
        .unwrap();
        match context {
            ChatContext::Window {
                app_name,
                title,
                url,
            } => {
                assert_eq!(app_name, "Browser");
                assert_eq!(title, "Submitted page");
                assert_eq!(url.as_deref(), Some("https://example.com/page"));
            }
            _ => panic!("Expected the submitted window context."),
        }
    }
    #[test]
    fn reset_invalidates_requests_started_before_it() {
        let chat = Chat::default();
        let request = chat.request();
        let (revision, _) = request.snapshot().unwrap();
        chat.reset();
        assert!(request.snapshot().is_err());
        assert!(request
            .commit(
                revision,
                json!({"role":"user"}),
                json!({"role":"assistant"})
            )
            .is_err());
        assert!(chat.snapshot().unwrap().1.is_empty());
    }
    #[test]
    fn concurrent_replies_cannot_corrupt_history() {
        let chat = Chat::default();
        let first = chat.request();
        let second = chat.request();
        let (revision, _) = first.snapshot().unwrap();
        first
            .commit(
                revision,
                json!({"role":"user","content":"hello"}),
                json!({"role":"assistant","content":"ok"}),
            )
            .unwrap();
        assert!(second.snapshot().is_err());
        assert_eq!(chat.snapshot().unwrap().1.len(), 2);
    }
    #[test]
    fn responses_extracts_only_message_text() {
        assert_eq!(
            extract_output_text(&json!({"output":[
                {"type":"web_search_call"},
                {"type":"message","content":[{"type":"output_text","text":"Hello"},{"type":"output_text","text":"world"}]}
            ]})),
            Some("Hello\nworld".into())
        );
        assert_eq!(extract_output_text(&json!({"output":[]})), None);
    }
    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
