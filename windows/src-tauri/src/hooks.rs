// Codex hook installation.
//
// The rule from CLAUDE.md is strict and is followed to the letter:
// read %USERPROFILE%\.codex\hooks.json, take a dated backup, merge without
// touching anybody else's hooks, show the diff, and write only after an explicit
// click. Uninstall removes Coucou's entries and nothing else.
//
// The command is only the quoted exe path in forward slashes plus the event name:
// on Windows Codex runs hook commands through Git Bash, and anything with
// PowerShell or cmd in it breaks.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Manager};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::settings;

/// Supported Codex events, with the timeout written to hooks.json.
/// PermissionRequest waits for a human, so it gets the decision timeout + 10 s.
pub const HOOK_EVENTS: &[(&str, u64)] = &[
    ("SessionStart", 10),
    ("SessionEnd", 3),
    ("UserPromptSubmit", 10),
    ("PreToolUse", 10),
    ("PostToolUse", 10),
    ("PermissionRequest", 120),
    ("Stop", 10),
    ("SubagentStart", 10),
    ("SubagentStop", 10),
];

const MARKER: &str = "coucou-hook.exe";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    pub installed: bool,
    pub settings_path: String,
    pub hook_path: String,
    pub hook_ready: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPreview {
    pub diff: String,
    pub backup: String,
    pub settings_path: String,
    /// Identifies the bytes this diff was computed from; handed back to `write`
    /// so we only ever apply what the user actually looked at.
    pub fingerprint: String,
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn settings_path() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
        .join("hooks.json")
}

/// Reads `~/.codex/hooks.json`.
///
/// The only error that means "start from nothing" is the file not being there.
/// Everything else — a lock held by another process, a permission problem, JSON
/// we cannot parse — is reported, because the alternative is treating somebody's
/// unreadable settings as an empty object and then writing that back over them.
fn read_settings() -> Result<Value, String> {
    read_snapshot(&settings_path()).map(|(value, _)| value)
}

/// Keep the parsed content and its fingerprint bound to the same read.
fn read_snapshot(path: &Path) -> Result<(Value, String), String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok((
            parse_settings(&bytes, &path.display().to_string())?,
            format!("file:{}", fingerprint(&bytes)),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok((json!({}), "missing".into())),
        // A lock, a permission problem, a bad drive: all of them mean we do not
        // know what is in there, and not knowing is not the same as empty.
        Err(err) => Err(format!("Can't read {}: {err}", path.display())),
    }
}

/// The parsing half of `read_settings`, split out so it can be tested without a
/// home directory.
fn parse_settings(bytes: &[u8], path: &str) -> Result<Value, String> {
    // PowerShell writes a UTF-8 BOM with `Set-Content -Encoding utf8`, and
    // serde_json refuses it. Stripping it is safe and well defined; guessing at
    // anything else is not.
    let text = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    match serde_json::from_slice::<Value>(text) {
        Ok(v) if v.is_object() => {
            if let Some(hooks) = v.get("hooks") {
                let hooks = hooks.as_object().ok_or_else(|| format!("{path}: hooks must be an object; nothing will be overwritten."))?;
                if hooks.values().any(|event| !event.is_array()) {
                    return Err(format!("{path}: every hook event must contain an array; nothing will be overwritten."));
                }
            }
            Ok(v)
        },
        Ok(_) => Err(format!("{path} isn't a JSON object — Coucou won't touch it.")),
        Err(err) => Err(format!(
            "{path} isn't valid JSON ({err}). Fix or move it, then try again — Coucou won't overwrite it."
        )),
    }
}

/// The settings as they are, or an empty object when we cannot tell. Only for
/// read-only paths like `status()`, which must never fail loudly; anything that
/// writes uses `read_settings()` and surfaces the error instead.
fn read_settings_lossy() -> Value {
    read_settings().unwrap_or_else(|_| json!({}))
}

fn hook_command(event: &str) -> String {
    let exe = settings::hook_exe_path()
        .to_string_lossy()
        .replace('\\', "/");
    // Git Bash expands $ and backticks inside double quotes. A profile path is
    // data, never shell code; single quotes protect it, including apostrophes.
    format!("'{}' {event}", exe.replace('\'', "'\\''"))
}

fn handler_is_ours(handler: &Value) -> bool {
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    // Recognise one invocation of our executable, not a substring in a foreign
    // command (e.g. an audit script mentioning coucou-hook.exe).
    let command = command.trim();
    let (exe, args) = if let Some(tail) = command.strip_prefix('"') {
        let Some(end) = tail.find('"') else {
            return false;
        };
        (&tail[..end], &tail[end + 1..])
    } else if let Some(tail) = command.strip_prefix('\'') {
        let Some(end) = tail.find('\'') else {
            return false;
        };
        // Paths containing apostrophes are handled by exact current commands.
        if tail[end + 1..].starts_with('\\') {
            return HOOK_EVENTS
                .iter()
                .any(|(event, _)| command == hook_command(event));
        }
        (&tail[..end], &tail[end + 1..])
    } else {
        let end = command.find(char::is_whitespace).unwrap_or(command.len());
        (&command[..end], &command[end..])
    };
    let exe = exe.rsplit(['/', '\\']).next().unwrap_or("");
    let args = args.trim();
    exe.eq_ignore_ascii_case(MARKER)
        && (HOOK_EVENTS.iter().any(|(event, _)| args == *event)
            || ["PostToolUseFailure", "Notification", "StopFailure"].contains(&args))
}

fn entry_is_ours(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| hooks.iter().any(handler_is_ours))
        .unwrap_or(false)
}

/// Remove only our handlers and preserve the group's matcher and other fields.
fn clean_entry(entry: &Value) -> Option<Value> {
    let Some(handlers) = entry.get("hooks").and_then(Value::as_array) else {
        return Some(entry.clone());
    };
    if !handlers.iter().any(handler_is_ours) {
        return Some(entry.clone());
    }
    let kept: Vec<Value> = handlers
        .iter()
        .filter(|handler| !handler_is_ours(handler))
        .cloned()
        .collect();
    if kept.is_empty() {
        return None;
    }
    let mut cleaned = entry.clone();
    cleaned["hooks"] = Value::Array(kept);
    Some(cleaned)
}

/// Settings with Coucou's hooks added; everything else is left untouched.
fn merged(existing: &Value) -> Value {
    // Also remove our obsolete events from previous versions.
    let cleaned = without_ours(existing);
    let mut root = cleaned.as_object().cloned().unwrap_or_default();
    let mut hooks = root
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(Map::new);

    for (event, timeout) in HOOK_EVENTS {
        let mut list = hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        list = list.iter().filter_map(clean_entry).collect();
        list.push(json!({
            "hooks": [{
                "type": "command",
                "command": hook_command(event),
                "timeout": timeout,
            }]
        }));
        hooks.insert((*event).to_string(), Value::Array(list));
    }

    root.insert("hooks".into(), Value::Object(hooks));
    Value::Object(root)
}

/// Settings with every Coucou entry removed, and nothing else changed.
fn without_ours(existing: &Value) -> Value {
    let mut root = existing.as_object().cloned().unwrap_or_default();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object).cloned() else {
        return Value::Object(root);
    };
    let mut out = Map::new();
    for (event, value) in hooks {
        match value.as_array() {
            Some(list) => {
                let kept: Vec<Value> = list.iter().filter_map(clean_entry).collect();
                if !kept.is_empty() || list.is_empty() {
                    out.insert(event, Value::Array(kept));
                }
            }
            None => {
                out.insert(event, value);
            }
        }
    }
    if out.is_empty() {
        root.remove("hooks");
    } else {
        root.insert("hooks".into(), Value::Object(out));
    }
    Value::Object(root)
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Down to the second: installing then uninstalling in the same minute must not
/// quietly overwrite the first backup.
fn stamp() -> String {
    let t = unsafe { GetLocalTime() };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}-{nanos}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

/// Identifies the exact bytes a preview was computed from. FNV-1a is plenty:
/// the question is only "is this still the file I showed the user?".
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn status() -> HookStatus {
    let current = read_settings_lossy();
    let installed = current
        .get("hooks")
        .and_then(Value::as_object)
        .map(|hooks| {
            hooks
                .values()
                .filter_map(Value::as_array)
                .flatten()
                .any(entry_is_ours)
        })
        .unwrap_or(false);
    let hook_path = settings::hook_exe_path();
    HookStatus {
        installed,
        settings_path: settings_path().to_string_lossy().to_string(),
        hook_ready: hook_path.exists(),
        hook_path: hook_path.to_string_lossy().to_string(),
    }
}

pub fn preview(install: bool) -> Result<HookPreview, String> {
    preview_at(&settings_path(), install)
}

fn preview_at(path: &Path, install: bool) -> Result<HookPreview, String> {
    let (current, fingerprint) = read_snapshot(path)?;
    let next = if install {
        merged(&current)
    } else {
        without_ours(&current)
    };
    Ok(HookPreview {
        diff: unified_diff(&pretty(&current), &pretty(&next)),
        backup: path
            .with_file_name(format!("hooks.json.bak-{}", stamp()))
            .to_string_lossy()
            .to_string(),
        settings_path: path.to_string_lossy().to_string(),
        fingerprint,
    })
}

/// Writes the merged (or cleaned) settings after taking a dated backup.
///
/// `fingerprint` is the one the preview was computed from. If the file changed
/// in between — another tool, another window, the user's own editor — we stop
/// and make them look at a fresh diff, because the only thing worse than not
/// installing the hooks is silently reverting somebody else's edit.
pub fn write(install: bool, fingerprint: &str) -> Result<String, String> {
    write_at(&settings_path(), install, fingerprint)
}

fn write_at(path: &Path, install: bool, fingerprint: &str) -> Result<String, String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    // Read before the backup: an unreadable file must abort before we touch
    // anything at all.
    let (current, current_fingerprint) = read_snapshot(path)?;
    if current_fingerprint != fingerprint {
        return Err(format!(
            "{} changed since the preview. Nothing was written — review the new diff.",
            path.display()
        ));
    }

    let backup = path.with_file_name(format!("hooks.json.bak-{}", stamp()));
    if path.exists() {
        let mut source = std::fs::File::open(path).map_err(|e| format!("backup failed: {e}"))?;
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
            .map_err(|e| format!("backup failed: {e}"))?;
        std::io::copy(&mut source, &mut target).map_err(|e| format!("backup failed: {e}"))?;
        target
            .sync_all()
            .map_err(|e| format!("backup failed: {e}"))?;
    }

    let next = if install {
        merged(&current)
    } else {
        without_ours(&current)
    };
    let mut text = pretty(&next);
    text.push('\n');

    // Write beside the target and rename over it: a crash or a full disk leaves
    // the original settings.json intact rather than half a file.
    let temp = path.with_extension(format!("json.coucou-{}-{}", std::process::id(), stamp()));
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("write failed: {e}"))?;
    output
        .write_all(text.as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|e| format!("write failed: {e}"))?;
    drop(output);
    if read_snapshot(path)?.1 != fingerprint {
        let _ = std::fs::remove_file(&temp);
        return Err("hooks.json changed while preparing the write. Review a new preview.".into());
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("write failed: {err}"));
    }
    Ok(backup.to_string_lossy().to_string())
}

/// Copies coucou-hook.exe into %LOCALAPPDATA%\Coucou\bin on launch.
/// In a bundled install it comes from the app resources; in `tauri dev` it sits
/// next to coucou.exe in the workspace target directory.
///
/// Every candidate is tried rather than just the first, because getting this
/// wrong is silent and fatal: `resources` used to be a glob, which made NSIS
/// mirror the source path into `_up_\target\release\`, no candidate matched, and
/// the relay was simply never installed. It only looked healthy on a developer
/// machine, where a leftover copy from `tauri dev` was already sitting in bin/.
pub fn ensure_hook_exe(app: &AppHandle) {
    let dest = settings::hook_exe_path();
    let Some(dir) = dest.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(p) = app
        .path()
        .resolve("coucou-hook.exe", tauri::path::BaseDirectory::Resource)
    {
        candidates.push(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Installed build, then `tauri dev` (target/debug) next to the
            // release hook the pre-build step produces.
            candidates.push(parent.join("coucou-hook.exe"));
            candidates.push(parent.join("../release/coucou-hook.exe"));
            // Belt and braces: where the old glob form used to land it.
            candidates.push(parent.join("_up_/target/release/coucou-hook.exe"));
        }
    }

    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    let Some(src) = candidates.into_iter().find(|p| p.exists()) else {
        crate::log::line(format!(
            "coucou-hook.exe not found — Codex hooks cannot work. Looked in: {}",
            tried.join(", ")
        ));
        return;
    };

    let same = match (std::fs::metadata(&src), std::fs::metadata(&dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && a.modified().ok() == b.modified().ok(),
        _ => false,
    };
    if same {
        return;
    }
    // A hook may be running right now and hold the file open; keeping the old
    // copy is fine, it is the same relay.
    if let Err(err) = std::fs::copy(&src, &dest) {
        if !dest.exists() {
            crate::log::line(format!("could not install coucou-hook.exe: {err}"));
        }
    }
}

// ── Minimal unified diff (LCS) ────────────────────────────────────────────────

/// settings.json is short, so a plain O(n·m) LCS is the simplest honest diff.
fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let (n, m) = (a.len(), b.len());

    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out: Vec<String> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(format!("  {}", a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push(format!("- {}", a[i]));
            i += 1;
        } else {
            out.push(format!("+ {}", b[j]));
            j += 1;
        }
    }
    while i < n {
        out.push(format!("- {}", a[i]));
        i += 1;
    }
    while j < m {
        out.push(format!("+ {}", b[j]));
        j += 1;
    }

    // Keep three lines of context around each change so the panel stays readable.
    let changed: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with('+') || l.starts_with('-'))
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return "No change.".into();
    }
    let mut keep = vec![false; out.len()];
    for idx in changed {
        let lo = idx.saturating_sub(3);
        let hi = (idx + 4).min(out.len());
        for k in lo..hi {
            keep[k] = true;
        }
    }
    let mut result = String::new();
    let mut gap = false;
    for (idx, line) in out.iter().enumerate() {
        if keep[idx] {
            result.push_str(line);
            result.push('\n');
            gap = false;
        } else if !gap {
            result.push_str("  …\n");
            gap = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHERE: &str = "settings.json";

    #[test]
    fn a_utf8_bom_is_stripped_not_treated_as_corruption() {
        // PowerShell 5's `Set-Content -Encoding utf8` produces exactly this.
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"model":"opus","hooks":{}}"#);
        let parsed = parse_settings(&bytes, WHERE).expect("a BOM must not defeat the parser");
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn unreadable_content_is_an_error_never_an_empty_object() {
        // This is the whole bug: returning {} here meant `merged()` produced a
        // file containing nothing but Coucou's hooks, and the write replaced
        // everything the user had.
        for bad in [&b"{ not json"[..], &b"[1,2,3]"[..], &b"\"a string\""[..]] {
            assert!(
                parse_settings(bad, WHERE).is_err(),
                "content we cannot use must refuse, not come back empty"
            );
        }
    }

    #[test]
    fn empty_and_whitespace_files_start_from_nothing() {
        assert_eq!(parse_settings(b"", WHERE).unwrap(), json!({}));
        assert_eq!(
            parse_settings(
                b"  
	 ", WHERE
            )
            .unwrap(),
            json!({})
        );
    }

    #[test]
    fn merging_keeps_every_other_setting_and_every_foreign_hook() {
        let existing = serde_json::json!({
            "model": "claude-opus-5",
            "theme": "dark",
            "enabledPlugins": ["a", "b"],
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "someone-elses-tool.exe" }] }
                ],
                "SomeEventWeDoNotTouch": [
                    { "hooks": [{ "type": "command", "command": "keep-me.exe" }] }
                ]
            }
        });

        let after = merged(&existing);
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["enabledPlugins"], serde_json::json!(["a", "b"]));

        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre.iter().any(|e| serde_json::to_string(e)
                .unwrap()
                .contains("someone-elses-tool.exe")),
            "another tool's hook was dropped"
        );
        assert!(pre.iter().any(entry_is_ours), "our own hook was not added");
        assert!(after["hooks"]["SomeEventWeDoNotTouch"].is_array());

        // And removing ours puts it back exactly as it was.
        let cleaned = without_ours(&after);
        assert_eq!(cleaned, existing);
    }

    #[test]
    fn a_fingerprint_notices_any_change() {
        assert_eq!(fingerprint(b"{}"), fingerprint(b"{}"));
        assert_ne!(fingerprint(b"{}"), fingerprint(b"{ }"));
        assert_ne!(fingerprint(b""), fingerprint(b"{}"));
    }

    #[test]
    fn writing_backs_up_preserves_and_refuses_a_changed_file() {
        let tmp =
            std::env::temp_dir().join(format!("coucou-hooks-{}-{}", std::process::id(), stamp()));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("hooks.json");

        // A real-shaped file, written the way PowerShell 5 would: UTF-8 with BOM.
        let original = r#"{"model":"claude-opus-5","theme":"dark","tui":{"x":1},"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"other-tool.exe"}]}]}}"#;
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(original.as_bytes());
        std::fs::write(&path, &bytes).unwrap();

        // Install.
        let plan = preview_at(&path, true).expect("a BOM must not stop the preview");
        assert!(
            plan.diff.contains("coucou-hook"),
            "the diff must show what changes"
        );
        let backup = write_at(&path, true, &plan.fingerprint).expect("install should succeed");

        // The backup holds the original bytes, BOM and all.
        assert_eq!(std::fs::read(&backup).unwrap(), bytes);

        // Everything else survived, and so did the other tool's hook.
        let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["model"], "claude-opus-5");
        assert_eq!(after["theme"], "dark");
        assert_eq!(after["tui"]["x"], 1);
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(pre
            .iter()
            .any(|e| serde_json::to_string(e).unwrap().contains("other-tool.exe")));
        assert!(pre.iter().any(entry_is_ours));

        // A file that moved since the preview is refused, and left alone.
        let stale = preview_at(&path, false).unwrap();
        std::fs::write(&path, br#"{"model":"someone-else-edited-this"}"#).unwrap();
        let err = write_at(&path, false, &stale.fingerprint).unwrap_err();
        assert!(err.contains("changed since the preview"), "got: {err}");
        let untouched: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(untouched["model"], "someone-else-edited-this");

        // Content we cannot parse is refused before anything is written.
        std::fs::write(&path, b"{ broken").unwrap();
        assert!(preview_at(&path, true).is_err());
        assert!(write_at(&path, true, "whatever").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ broken");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mixed_groups_and_marker_mentions_survive_install_and_uninstall() {
        let mixed = json!({"matcher":"Bash","timeout":7,"hooks":[
            {"type":"command","command":"\"C:/old/coucou-hook.exe\" PreToolUse"},
            {"type":"command","command":"audit.exe --check coucou-hook.exe"},
            {"type":"prompt","prompt":"keep this validator"}
        ]});
        let expected = json!({"matcher":"Bash","timeout":7,"hooks":[
            {"type":"command","command":"audit.exe --check coucou-hook.exe"},
            {"type":"prompt","prompt":"keep this validator"}
        ]});
        assert_eq!(clean_entry(&mixed), Some(expected.clone()));
        let before = json!({"hooks":{"PreToolUse":[mixed],"EmptyEvent":[]}});
        let expected_root = json!({"hooks":{"PreToolUse":[expected],"EmptyEvent":[]}});
        assert_eq!(without_ours(&before), expected_root);
        assert_eq!(without_ours(&merged(&before)), expected_root);
        assert!(handler_is_ours(
            &json!({"type":"command","command":hook_command("PreToolUse")})
        ));
        for command in [
            "echo coucou-hook.exe",
            "coucou-hook.exe PreToolUse; other.exe",
            "different-coucou-hook.exe PreToolUse",
        ] {
            assert!(!handler_is_ours(
                &json!({"type":"command","command":command})
            ));
        }
    }

    #[test]
    fn malformed_hook_structures_refuse_instead_of_being_overwritten() {
        for invalid in [
            br#"{"hooks":[]}"#.as_slice(),
            br#"{"hooks":{"PreToolUse":{}}}"#.as_slice(),
        ] {
            assert!(parse_settings(invalid, WHERE).is_err());
        }
    }

    #[test]
    fn missing_and_empty_file_are_distinct_for_preview_integrity() {
        let tmp = std::env::temp_dir().join(format!(
            "coucou-fingerprint-{}-{}",
            std::process::id(),
            stamp()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("hooks.json");
        let plan = preview_at(&path, true).unwrap();
        std::fs::write(&path, b"").unwrap();
        assert!(write_at(&path, true, &plan.fingerprint).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}