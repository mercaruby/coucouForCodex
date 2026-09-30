//! coucou-hook — the relay Codex runs on every hook event.
//!
//! Reads the hook JSON on stdin, adds a little terminal context, and hands it to
//! Coucou over the named pipe `\\.\pipe\coucou-codex-<sid>`.
//!
//! Hard rule (docs/CLAUDE.md): **never block Codex.**
//! * If the pipe does not exist — Coucou is closed — we exit 0 immediately with
//!   nothing on stdout, and the session carries on untouched.
//! * Every step runs under a deadline enforced by the main thread, so a pipe that
//!   accepts the connection and then stops reading cannot wedge the session
//!   either: we abandon the worker and exit.
//! * Only `PermissionRequest` waits for an answer, because approving from the
//!   island is the whole point. No answer means empty stdout, and Codex
//!   asks in the terminal exactly as if Coucou were not installed.
//!
//! Usage: `coucou-hook <EventName>` (the name is also read from the JSON).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Budget for getting a pipe connection. Beyond this Codex wins, always.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Whole-run budget for an event nobody waits on: connect and write, no more.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission prompt may stay on screen before the terminal takes over.
const DECISION_BUDGET: Duration = Duration::from_secs(110);

/// `ERROR_PIPE_BUSY` — every instance is serving someone else right now. This is
/// the one error worth retrying: the server exists and a slot will free up.
const ERROR_PIPE_BUSY: i32 = 231;

/// Fields that are pointless to forward and can be enormous (a whole file read,
/// a full command output). The island never shows them.
const DROPPED_FIELDS: &[&str] = &["tool_response", "transcript_path"];
/// Longest string forwarded for any single field; the island truncates to far
/// less than this anyway.
const MAX_FIELD_LEN: usize = 2_000;
const MAX_STDIN_BYTES: usize = 64 * 1024;
const MAX_ANSWER_BYTES: usize = 32;
const MAX_APPROVAL_INPUT_BYTES: usize = 8 * 1024;

mod win;

/// `\\.\pipe\coucou-<sid>`. The SID keeps two accounts on the same machine from
/// ever meeting on the same pipe; the name falls back to the user name only if
/// the SID cannot be read at all, which should not happen.
fn pipe_path() -> String {
    let key = win::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\coucou-codex-{key}")
}

/// Opens the pipe. Retries only while the server is busy: any other error means
/// there is nothing to talk to, and waiting would only delay Codex.
fn connect() -> Option<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    let path = pipe_path();
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => {
                let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());
                // Somebody else's server on our pipe name gets nothing from us.
                return win::pipe_server_is_same_user(handle).then_some(file);
            }
            Err(err) => {
                if err.raw_os_error() != Some(ERROR_PIPE_BUSY) || Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        }
    }
}

fn main() {
    let began = Instant::now();
    // Stdin itself can stall. Keep it off the main thread and under a deadline,
    // just like the pipe, so a broken producer cannot wedge Codex.
    let (input_tx, input_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = input_tx.send(read_event());
    });
    let Ok(Some((payload, event))) = input_rx.recv_timeout(FIRE_AND_FORGET_BUDGET) else {
        std::process::exit(0)
    };

    let waits_for_answer = event == "PermissionRequest";
    let budget = if waits_for_answer {
        DECISION_BUDGET
    } else {
        FIRE_AND_FORGET_BUDGET
    };

    // The worker owns every blocking call. If it overruns the budget we simply
    // stop listening and exit: the process dying takes the pipe handle with it.
    // (No catch_unwind here — the release profile is panic = "abort", so it would
    // be dead code. `talk` is written to have nothing to panic on instead.)
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&payload, waits_for_answer));
    });

    if let Ok(Some(decision)) = rx.recv_timeout(budget.saturating_sub(began.elapsed())) {
        if let Some(json) = decision_json(&decision) {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{json}");
            let _ = out.flush();
        }
    }
    // Nothing printed: Codex asks in the terminal, as if we were not here.
    std::process::exit(0);
}

/// The documented PermissionRequest output. Anything we do not recognise prints
/// nothing at all rather than guessing — silence is the safe answer.
/// See https://learn.chatgpt.com/docs/hooks
fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        // "always" still answers a plain allow; remembering it is the island's
        // business, not Codex's.
        "allow" | "always" => r#"{"behavior":"allow"}"#.to_string(),
        "deny" => r#"{"behavior":"deny","message":"Denied from Coucou"}"#.to_string(),
        _ => return None,
    };
    Some(format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{behavior}}}}}"#
    ))
}

/// Reads stdin and returns the payload to forward plus the event name.
fn read_event() -> Option<(String, String)> {
    let mut raw = Vec::new();
    if std::io::stdin()
        .take((MAX_STDIN_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .is_err()
    {
        return None;
    }
    let arg_event = std::env::args().nth(1).unwrap_or_default();
    let (mut payload, event) = parse_event(&raw, &arg_event)?;
    let map = payload.as_object_mut()?;

    for field in DROPPED_FIELDS {
        map.remove(*field);
    }

    let cwd_missing = map
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::is_empty)
        .unwrap_or(true);
    if cwd_missing {
        if let Ok(cwd) = std::env::current_dir() {
            map.insert(
                "cwd".into(),
                serde_json::Value::String(cwd.to_string_lossy().to_string()),
            );
        }
    }

    for (key, var) in [
        ("term_program", "TERM_PROGRAM"),
        ("wt_session", "WT_SESSION"),
        ("term_session_id", "TERM_SESSION_ID"),
        ("vscode_pid", "VSCODE_PID"),
    ] {
        if !map.contains_key(key) {
            map.insert(
                key.into(),
                serde_json::Value::String(std::env::var(var).unwrap_or_default()),
            );
        }
    }
    // Observational activity may be shortened; permissions must stay verbatim.
    if event != "PermissionRequest" {
        truncate_strings(&mut payload);
    }
    let mut line = payload.to_string();
    if line.len() > MAX_STDIN_BYTES {
        return None;
    }
    line.push('\n');
    Some((line, event))
}

/// Parse independently of real stdin and home, so security cases are testable.
fn parse_event(raw: &[u8], arg_event: &str) -> Option<(serde_json::Value, String)> {
    if raw.is_empty() || raw.len() > MAX_STDIN_BYTES {
        return None;
    }
    // Some shells hand us a UTF-8 BOM; serde_json would choke on it.
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);

    let mut payload = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
    let map = payload.as_object_mut()?;

    // The event name is passed as argv[1] by the hook command; the JSON usually
    // carries it too. Trust argv when the JSON is missing it.
    let event = map
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| arg_event.to_string());
    if event.is_empty() || (!arg_event.is_empty() && arg_event != event) {
        return None;
    }
    map.insert(
        "hook_event_name".into(),
        serde_json::Value::String(event.clone()),
    );
    if event == "PermissionRequest" && !safe_permission_request(&payload) {
        return None;
    }
    Some((payload, event))
}

/// Approve only known shell shapes that the UI can show completely. Any unknown
/// tool, ambiguous command, truncation or invisible controls stays in Codex CLI.
fn safe_permission_request(payload: &serde_json::Value) -> bool {
    let Some(tool) = payload.get("tool_name").and_then(|v| v.as_str()) else {
        return false;
    };
    let tool = tool.strip_prefix("functions.").unwrap_or(tool);
    let Some(input) = payload.get("tool_input").and_then(|v| v.as_object()) else {
        return false;
    };
    let (field, array) = match tool {
        "Bash" | "PowerShell" | "shell_command" => ("command", false),
        "exec_command" => ("cmd", false),
        "shell" => ("command", true),
        _ => return false,
    };
    if input.contains_key(if field == "command" { "cmd" } else { "command" }) {
        return false;
    }
    let Some(command) = input.get(field) else {
        return false;
    };
    let command_ok = if array {
        command
            .as_array()
            .map(|args| {
                !args.is_empty()
                    && args
                        .iter()
                        .all(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
            })
            .unwrap_or(false)
    } else {
        command
            .as_str()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    };
    command_ok
        && command.to_string().len() < MAX_FIELD_LEN
        && serde_json::to_vec(input)
            .map(|bytes| bytes.len() <= MAX_APPROVAL_INPUT_BYTES)
            .unwrap_or(false)
        && complete_reviewable_strings(payload)
}

fn complete_reviewable_strings(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.len() < MAX_FIELD_LEN && !s.chars().any(|c|
            (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || matches!(c, '\u{00ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')),
        serde_json::Value::Array(items) => items.iter().all(complete_reviewable_strings),
        serde_json::Value::Object(map) => map.iter().all(|(key, value)| key.len() < MAX_FIELD_LEN && complete_reviewable_strings(&serde_json::Value::String(key.clone())) && complete_reviewable_strings(value)),
        serde_json::Value::Number(n) => n.as_f64().map(|number| number.abs() <= 9_007_199_254_740_991.0).unwrap_or(false),
        _ => true,
    }
}

/// Caps every string in the payload. A single Write can carry a whole file.
fn truncate_strings(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() > MAX_FIELD_LEN {
                // Cut on a char boundary; a lone byte index can split UTF-8.
                let mut end = MAX_FIELD_LEN;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(truncate_strings),
        serde_json::Value::Object(map) => map.values_mut().for_each(truncate_strings),
        _ => {}
    }
}

/// Connect, send, and — for a permission request — wait for the island's word.
fn talk(payload: &str, waits_for_answer: bool) -> Option<String> {
    let mut pipe = connect()?;

    if pipe.write_all(payload.as_bytes()).is_err() {
        return None;
    }
    let _ = pipe.flush();

    if !waits_for_answer {
        return None;
    }

    read_answer(&mut pipe)
}

fn read_answer(reader: &mut impl Read) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; MAX_ANSWER_BYTES + 1];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_ANSWER_BYTES {
                    return None;
                }
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let answer = String::from_utf8(buf).ok()?.trim().to_string();
    decision_json(&answer).map(|_| answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_json_matches_the_documented_shape() {
        assert_eq!(
            decision_json("allow").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
        );
        assert_eq!(
            decision_json("deny").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denied from Coucou"}}}"#
        );
        // "always" is an island concept; Codex just gets an allow.
        assert!(decision_json("always")
            .unwrap()
            .contains(r#""behavior":"allow""#));
    }

    #[test]
    fn anything_unrecognised_prints_nothing() {
        assert!(decision_json("").is_none());
        assert!(decision_json("maybe").is_none());
        // The shape the app used to send must not be mistaken for a decision.
        assert!(decision_json(r#"{"permissionDecision":"allow"}"#).is_none());
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        truncate_strings(&mut v);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
    }

    fn permission(tool: &str, input: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"hook_event_name":"PermissionRequest", "tool_name":tool, "tool_input":input, "cwd":"C:/project"})
    }

    #[test]
    fn permissions_keep_complete_input_and_known_shell_shapes() {
        for payload in [
            permission(
                "Bash",
                serde_json::json!({"command":"git status", "timeout":1000, "description":"status"}),
            ),
            permission("PowerShell", serde_json::json!({"command":"Get-ChildItem"})),
            permission(
                "exec_command",
                serde_json::json!({"cmd":"git status", "workdir":"C:/project"}),
            ),
            permission(
                "functions.shell",
                serde_json::json!({"command":["git", "status"], "workdir":"C:/project"}),
            ),
        ] {
            let original = payload.clone();
            let bytes = serde_json::to_vec(&payload).unwrap();
            let (parsed, event) = parse_event(&bytes, "PermissionRequest").unwrap();
            assert_eq!(event, "PermissionRequest");
            assert_eq!(
                parsed, original,
                "permission arguments must never be truncated or omitted"
            );
        }
    }

    #[test]
    fn long_unknown_ambiguous_and_hidden_permissions_fall_back_to_cli() {
        for payload in [
            permission(
                "Bash",
                serde_json::json!({"command":format!("{}; delete-secret", "a".repeat(MAX_FIELD_LEN))}),
            ),
            permission(
                "Bash",
                serde_json::json!({"command":"echo safe", "description":"x".repeat(MAX_FIELD_LEN)}),
            ),
            permission(
                "Write",
                serde_json::json!({"file_path":".env", "content":"changed"}),
            ),
            permission("UnknownTool", serde_json::json!({"command":"git status"})),
            permission(
                "Bash",
                serde_json::json!({"command":"git status", "cmd":"delete everything"}),
            ),
            permission("exec_command", serde_json::json!({"command":"git status"})),
            permission("shell", serde_json::json!({"command":["git", 123]})),
            permission("shell", serde_json::json!({"command":[]})),
            permission("Bash", serde_json::json!({"command":""})),
            permission(
                "Bash",
                serde_json::json!({"command":"echo safe\u{202e}delete"}),
            ),
        ] {
            assert!(
                parse_event(&serde_json::to_vec(&payload).unwrap(), "PermissionRequest").is_none(),
                "must defer {payload}"
            );
        }
        let mut payload = permission("Bash", serde_json::json!({"command":"git status"}));
        payload["tool_input"]["arguments"] = serde_json::json!(vec!["a".repeat(1000); 10]);
        assert!(!safe_permission_request(&payload));
    }

    #[test]
    fn oversized_stdin_mismatched_events_and_responses_are_rejected() {
        assert!(parse_event(&vec![b' '; MAX_STDIN_BYTES + 1], "Stop").is_none());
        let bytes = serde_json::to_vec(&permission(
            "Bash",
            serde_json::json!({"command":"git status"}),
        ))
        .unwrap();
        assert!(parse_event(&bytes, "PreToolUse").is_none());
        assert_eq!(
            read_answer(&mut std::io::Cursor::new(b"allow\n")),
            Some("allow".into())
        );
        assert!(read_answer(&mut std::io::Cursor::new(vec![b'a'; MAX_ANSWER_BYTES + 1])).is_none());
        assert!(read_answer(&mut std::io::Cursor::new(vec![0xff])).is_none());
        assert!(read_answer(&mut std::io::Cursor::new(b"maybe\n")).is_none());
    }
}
