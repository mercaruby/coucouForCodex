//! Minimal local app-server client. Authentication remains with the official CLI.
//! No TCP listener, shell command strings, credential file reads or token handling.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const MAX_LINE: usize = 2_000_000;
const MAX_REPLY: usize = 1_000_000;
const READ_TIMEOUT: Duration = Duration::from_secs(25);

struct SchemaDir(PathBuf);
impl Drop for SchemaDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Older servers silently ignore unknown sandbox fields. Check their generated
/// wire schema instead of assuming a successful request enforced the restriction.
pub fn verify_restricted_reads(executable: &Path) -> Result<(), String> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err("Invalid Codex executable.".into());
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Could not initialize the compatibility check.")?
        .as_nanos();
    let dir = SchemaDir(std::env::temp_dir().join(format!(
        "codex-companion-schema-{}-{nonce}",
        std::process::id()
    )));
    std::fs::create_dir(&dir.0).map_err(|_| "Could not initialize the compatibility check.")?;
    let mut command = Command::new(executable);
    command
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(&dir.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Could not inspect Codex compatibility.")?;
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                return Err(
                    "This Codex CLI cannot report its protocol schema. Update the official CLI."
                        .into(),
                )
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not inspect Codex compatibility.".into());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex compatibility check timed out.".into());
            }
        }
    }
    let schema: Value = serde_json::from_slice(
        &std::fs::read(dir.0.join("ClientRequest.json"))
            .map_err(|_| "Codex did not report its protocol schema.")?,
    )
    .map_err(|_| "Invalid Codex protocol schema.")?;
    let mut read_only = false;
    let mut restricted = false;
    fn scan(v: &Value, read_only: &mut bool, restricted: &mut bool) {
        if let Some(props) = v.get("properties") {
            let tag = &props["type"];
            let matches = |name: &str| {
                tag["const"].as_str() == Some(name)
                    || tag["enum"]
                        .as_array()
                        .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(name)))
            };
            if matches("readOnly") && props.get("access").is_some() {
                *read_only = true;
            }
            if matches("restricted")
                && props.get("readableRoots").is_some()
                && props.get("includePlatformDefaults").is_some()
            {
                *restricted = true;
            }
        }
        match v {
            Value::Object(o) => {
                for child in o.values() {
                    scan(child, read_only, restricted);
                }
            }
            Value::Array(a) => {
                for child in a {
                    scan(child, read_only, restricted);
                }
            }
            _ => {}
        }
    }
    scan(&schema, &mut read_only, &mut restricted);
    if !read_only || !restricted {
        return Err("This Codex CLI does not support restricted file reads. ChatGPT chat is disabled for this version; use OpenAI API mode or a compatible official CLI.".into());
    }
    Ok(())
}

pub struct Client {
    child: Child,
    input: ChildStdin,
    output: Receiver<Result<Value, String>>,
    next_id: u64,
    executable: PathBuf,
}

impl Drop for Client {
    fn drop(&mut self) {
        // Kill before wait; neither unresponsive servers nor reader threads survive.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    pub fn start(executable: &Path, cwd: &Path) -> Result<Self, String> {
        if !executable.is_absolute() || !executable.is_file() {
            return Err("Choose the absolute path to the official Codex executable.".into());
        }
        let mut command = Command::new(executable);
        command
            .arg("app-server")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // A stale API key in the desktop environment must not switch billing.
        command
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_API_KEY");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Could not start Codex. Check its executable path.".to_string())?;
        let input = child.stdin.take().ok_or("Codex stdin unavailable")?;
        let output = child.stdout.take().ok_or("Codex stdout unavailable")?;
        let (tx, rx) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                match reader
                    .by_ref()
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut bytes)
                {
                    Ok(0) => {
                        let _ = tx.send(Err("Codex closed the connection.".into()));
                        break;
                    }
                    Ok(_) if bytes.len() > MAX_LINE => {
                        let _ = tx.send(Err("Codex message exceeded the size limit.".into()));
                        break;
                    }
                    Ok(_) => match serde_json::from_slice(&bytes) {
                        Ok(value) => {
                            if tx.send(Ok(value)).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = tx.send(Err("Invalid Codex protocol message.".into()));
                            break;
                        }
                    },
                    Err(_) => {
                        let _ = tx.send(Err("Could not read Codex output.".into()));
                        break;
                    }
                }
            }
        });
        let mut client = Self {
            child,
            input,
            output: rx,
            next_id: 1,
            executable: executable.to_path_buf(),
        };
        client.request("initialize", json!({"clientInfo": {
            "name": "coucou_codex_personal", "title": "Personal Codex companion", "version": "0.1.0"
        }}))?;
        client.write(&json!({"method":"initialized", "params":{}}))?;
        Ok(client)
    }

    fn write(&mut self, value: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.input, value)
            .map_err(|_| "Could not send to Codex.".to_string())?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| "Could not send to Codex.".to_string())
    }

    fn receive(&self, deadline: Instant) -> Result<Value, String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("Codex timed out.")?;
        self.output
            .recv_timeout(remaining)
            .map_err(|_| "Codex timed out or disconnected.".to_string())?
    }

    fn reject_server_request(&mut self, message: &Value) -> Result<(), String> {
        if message.get("id").is_some() && message.get("method").is_some() {
            // Embedded chat never silently approves commands, files, network access,
            // dynamic tools or requests for external authentication tokens.
            self.write(&json!({"id":message["id"],"error":{
                "code":-32601,"message":"This companion does not support agent actions or approval requests."
            }}))?;
        }
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + READ_TIMEOUT;
        loop {
            let message = self.receive(deadline)?;
            if message.get("method").is_some() {
                self.reject_server_request(&message)?;
                continue;
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if message.get("error").is_some() {
                // Do not forward arbitrary protocol errors that could contain tokens.
                return Err(format!(
                    "Codex rejected {method}. Update the official CLI and check its configuration."
                ));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "Missing Codex result.".into());
        }
    }

    pub fn is_chatgpt_authenticated(&mut self) -> Result<bool, String> {
        let result = self.request("account/read", json!({"refreshToken":false}))?;
        Ok(result["account"]["type"].as_str() == Some("chatgpt"))
    }

    pub fn chat(&mut self, model: &str, text: &str, cwd: &Path) -> Result<String, String> {
        verify_restricted_reads(&self.executable)?;
        if !self.is_chatgpt_authenticated()? {
            return Err("Sign in with ChatGPT in the official Codex CLI first. API-key login is not used in ChatGPT mode.".into());
        }
        let mut params = json!({
            "cwd":cwd,"approvalPolicy":"never","sandbox":"readOnly","ephemeral":true
        });
        if !model.is_empty() {
            params["model"] = json!(model);
        }
        let result = self.request("thread/start", params)?;
        let thread_id = result["thread"]["id"]
            .as_str()
            .ok_or("No Codex thread ID.")?
            .to_string();
        // Restrict reads to the app inbox, rather than the user's entire home.
        // Old CLI versions that reject this fail closed; no unrestricted retry.
        self.request(
            "turn/start",
            json!({
                "threadId":thread_id,"input":[{"type":"text","text":text}],
                "approvalPolicy":"never",
                "sandboxPolicy":{"type":"readOnly","access":{
                    "type":"restricted","includePlatformDefaults":false,"readableRoots":[cwd]
                }}
            }),
        )?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut texts = Vec::new();
        let mut reply_size = 0;
        loop {
            let message = self.receive(deadline)?;
            self.reject_server_request(&message)?;
            if message["params"]["threadId"]
                .as_str()
                .is_some_and(|id| id != thread_id)
            {
                continue;
            }
            match message["method"].as_str().unwrap_or("") {
                "item/completed" => {
                    let item = &message["params"]["item"];
                    if item["type"].as_str() == Some("agentMessage") {
                        if let Some(text) = item["text"].as_str() {
                            reply_size += text.len();
                            if reply_size > MAX_REPLY {
                                return Err("Codex reply exceeded the size limit.".into());
                            }
                            texts.push(text.to_string());
                        }
                    }
                }
                "turn/completed" => {
                    if message["params"]["turn"]["status"].as_str() != Some("completed") {
                        return Err("Codex could not complete this chat turn. Check the official client for account limits or errors.".into());
                    }
                    let reply = texts.join("\n\n");
                    if reply.trim().is_empty() {
                        return Err("Codex returned no text.".into());
                    }
                    return Ok(reply);
                }
                _ => {}
            }
        }
    }
}
