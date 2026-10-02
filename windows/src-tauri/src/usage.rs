//! Read-only Codex quota metadata through the official native app-server.
//! No account credentials, conversation files, private HTTP endpoints or tools.
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TIMEOUT: Duration = Duration::from_secs(20);
const TTL: Duration = Duration::from_secs(60);
const MIN_REFRESH: Duration = Duration::from_secs(15);
const MAX_LINE: usize = 256 * 1024;
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
const MAX_MESSAGES: usize = 64;
const SOURCE: &str = "codex-app-server";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub status: String,
    pub source: String,
    pub fetched_at: Option<u64>,
    pub error: Option<String>,
    pub buckets: Vec<UsageBucket>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub id: String,
    pub name: Option<String>,
    pub plan_type: Option<String>,
    pub windows: Vec<UsageWindow>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    pub used_percent: Option<f64>,
    pub window_duration_mins: Option<u64>,
    pub resets_at: Option<u64>,
}

struct Cached {
    path: String,
    finished: Option<Instant>,
    snapshot: UsageSnapshot,
}

pub struct UsageCache(Mutex<Cached>);

impl Default for UsageCache {
    fn default() -> Self {
        Self(Mutex::new(Cached {
            path: String::new(),
            finished: None,
            snapshot: UsageSnapshot {
                status: "unavailable".into(),
                source: SOURCE.into(),
                fetched_at: None,
                error: None,
                buckets: Vec::new(),
            },
        }))
    }
}

impl UsageCache {
    /// One worker owns the child at a time. Concurrent refreshes receive the
    /// completed result; routine reads reuse successes or errors for one minute.
    pub fn read(&self, configured_path: &str, refresh: bool) -> UsageSnapshot {
        let requested = Instant::now();
        let mut cached = self.0.lock().unwrap();
        if cached.path != configured_path {
            cached.path = configured_path.into();
            cached.finished = None;
            cached.snapshot = UsageCache::default().0.into_inner().unwrap().snapshot;
        }
        if cached.finished.is_some_and(|finished| {
            finished >= requested
                || finished.elapsed() < MIN_REFRESH
                || (!refresh && finished.elapsed() < TTL)
        }) {
            return cached.snapshot.clone();
        }
        let result = resolve_executable(configured_path).and_then(|path| read_native(&path));
        match result {
            Ok(buckets) => {
                cached.snapshot = UsageSnapshot {
                    status: "available".into(),
                    source: SOURCE.into(),
                    fetched_at: Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    ),
                    error: None,
                    buckets,
                }
            }
            Err(error) => {
                cached.snapshot.status = if cached.snapshot.fetched_at.is_some() {
                    "stale"
                } else {
                    "unavailable"
                }
                .into();
                cached.snapshot.error = Some(error);
            }
        }
        cached.finished = Some(Instant::now());
        cached.snapshot.clone()
    }
}

fn bounded_label(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .filter(|text| {
            !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn metric<'a>(value: &'a Value, camel: &str, snake: &str) -> &'a Value {
    // Explicit null is authoritative, and never replaced by a legacy alias.
    value
        .get(camel)
        .or_else(|| value.get(snake))
        .unwrap_or(&Value::Null)
}

fn parse_window(value: &Value, id: &str) -> UsageWindow {
    UsageWindow {
        id: id.into(),
        used_percent: metric(value, "usedPercent", "used_percent")
            .as_f64()
            .filter(|number| number.is_finite() && (0.0..=100.0).contains(number)),
        window_duration_mins: metric(value, "windowDurationMins", "window_duration_mins")
            .as_u64()
            .filter(|number| *number > 0 && *number <= 525_600_000),
        // These are absolute Unix seconds. Milliseconds and invalid values stay unknown.
        resets_at: metric(value, "resetsAt", "resets_at")
            .as_u64()
            .filter(|number| *number > 0 && *number <= 253_402_300_799),
    }
}

fn parse_bucket(id: &str, value: &Value) -> Result<UsageBucket, String> {
    if !value.is_object() || id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        return Err("Codex returned invalid usage metadata.".into());
    }
    Ok(UsageBucket {
        id: id.into(),
        name: bounded_label(&value["limitName"], 256),
        plan_type: bounded_label(&value["planType"], 64),
        windows: vec![
            parse_window(&value["primary"], "primary"),
            parse_window(&value["secondary"], "secondary"),
        ],
    })
}

fn parse_usage(value: &Value) -> Result<Vec<UsageBucket>, String> {
    if !value.is_object() {
        return Err("Codex returned invalid usage metadata.".into());
    }
    if let Some(by_id) = value
        .get("rateLimitsByLimitId")
        .filter(|value| !value.is_null())
    {
        let map = by_id
            .as_object()
            .ok_or("Codex returned invalid usage buckets.")?;
        if map.len() > 64 {
            return Err("Codex usage exceeded the bucket limit.".into());
        }
        // Even an empty authoritative map suppresses the legacy view. Never
        // rename the principal bucket to Codex, Spark, or a ChatGPT quota.
        return map
            .iter()
            .map(|(id, bucket)| parse_bucket(id, bucket))
            .collect();
    }
    let Some(legacy) = value.get("rateLimits").filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let id = bounded_label(&legacy["limitId"], 128).unwrap_or_else(|| "legacy".into());
    Ok(vec![parse_bucket(&id, legacy)?])
}

fn native_executable(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if !canonical.is_file() {
        return None;
    }
    // Reject command scripts and Node wrappers. An explicit native executable
    // is trusted by the user; autodetection is restricted to known installers.
    let mut file = std::fs::File::open(&canonical).ok()?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header).ok()?;
    if header[..2] != *b"MZ" {
        return None;
    }
    let pe_offset = u32::from_le_bytes(header[60..64].try_into().ok()?) as u64;
    if !(64..=1_048_576).contains(&pe_offset) {
        return None;
    }
    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut signature = [0u8; 4];
    file.read_exact(&mut signature).ok()?;
    (signature == *b"PE\0\0").then_some(canonical)
}

fn package_candidates(base: &Path, candidates: &mut Vec<PathBuf>) {
    let (package, triple) = if cfg!(target_arch = "aarch64") {
        ("codex-win32-arm64", "aarch64-pc-windows-msvc")
    } else {
        ("codex-win32-x64", "x86_64-pc-windows-msvc")
    };
    for package in [package, "codex"] {
        for subdir in ["bin", "codex"] {
            candidates.push(
                base.join("@openai")
                    .join(package)
                    .join("vendor")
                    .join(triple)
                    .join(subdir)
                    .join("codex.exe"),
            );
        }
    }
}

fn desktop_candidates(base: &Path, candidates: &mut Vec<PathBuf>) {
    // The official Windows desktop updater extracts its native CLI here. This
    // is an installed-app location, not a recursive search of arbitrary folders.
    let bin = base.join("OpenAI/Codex/bin");
    let mut versioned = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&bin) {
        for entry in entries.take(64).flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() != 16 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let path = entry.path().join("codex.exe");
            if let Ok(metadata) = path.metadata() {
                if metadata.is_file() {
                    versioned.push((metadata.modified().unwrap_or(UNIX_EPOCH), path));
                }
            }
        }
    }
    versioned.sort_by(|left, right| right.0.cmp(&left.0));
    candidates.extend(versioned.into_iter().map(|(_, path)| path));
    candidates.push(bin.join("codex.exe"));
}

fn resolve_executable(configured: &str) -> Result<PathBuf, String> {
    if !configured.is_empty() {
        return native_executable(Path::new(configured)).ok_or_else(|| {
            "Choose an absolute path to the trusted native Codex executable in Settings.".into()
        });
    }
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        desktop_candidates(&PathBuf::from(local), &mut candidates);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        package_candidates(
            &PathBuf::from(appdata).join("npm/node_modules"),
            &mut candidates,
        );
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        candidates.push(PathBuf::from(home).join(".codex/bin/codex.exe"));
    }
    // Development CLI installed and authorized for this build task. This is a
    // fixed build-workspace location, never a scan of the user's current folder.
    #[cfg(debug_assertions)]
    package_candidates(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.tools/codex/node_modules"),
        &mut candidates,
    );
    candidates.into_iter().find_map(|path|native_executable(&path)).ok_or_else(||
        "Native Codex was not found. Install the official Codex CLI, sign in there, or choose its native executable in Settings.".into())
}

struct NativeServer {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    pending: Vec<u8>,
    total_bytes: usize,
    deadline: Instant,
    messages: usize,
}

impl Drop for NativeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl NativeServer {
    fn start(path: &Path) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let deadline = Instant::now() + TIMEOUT;
        let mut command = Command::new(path);
        // Remove inherited provider overrides, while allowing the official CLI
        // to choose its normal endpoint for its own signed-in account.
        command
            .args([
                "-c",
                "model_provider=\"openai\"",
                "-c",
                "openai_base_url=\"\"",
                "-c",
                "chatgpt_base_url=\"https://chatgpt.com/backend-api/\"",
                "app-server",
                "--listen",
                "stdio://",
            ])
            .current_dir(trusted_cwd()?)
            .creation_flags(super::CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        // Only OS paths and the user's explicit Codex home are inherited. API
        // keys, proxy/provider overrides and Node options are never forwarded.
        for name in [
            "SystemRoot",
            "WINDIR",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
            "CODEX_HOME",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Could not start the native Codex usage reader.")?;
        let input = child.stdin.take();
        let output = child.stdout.take();
        let (Some(input), Some(output)) = (input, output) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Could not connect to the native Codex usage reader.".into());
        };
        Ok(Self {
            child,
            input,
            output,
            pending: Vec::new(),
            total_bytes: 0,
            deadline,
            messages: 0,
        })
    }

    fn receive(&mut self) -> Result<Value, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::Pipes::PeekNamedPipe};
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or("Codex usage read timed out.")?;
            if let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
                if end + 1 > MAX_LINE {
                    return Err("Codex usage output exceeded the size limit.".into());
                }
                let line = self.pending.drain(..=end).collect::<Vec<_>>();
                return serde_json::from_slice(&line)
                    .map_err(|_| "Codex returned an invalid usage protocol message.".into());
            }
            if self.pending.len() > MAX_LINE {
                return Err("Codex usage output exceeded the size limit.".into());
            }
            let mut available = 0;
            unsafe {
                PeekNamedPipe(
                    HANDLE(self.output.as_raw_handle()),
                    None,
                    0,
                    None,
                    Some(&mut available),
                    None,
                )
            }
            .map_err(|_| "Codex closed the usage connection.")?;
            if available > 0 {
                let mut buffer = [0u8; 4096];
                let count = (available as usize).min(buffer.len());
                let count = self
                    .output
                    .read(&mut buffer[..count])
                    .map_err(|_| "Could not read the Codex usage connection.")?;
                if count == 0 {
                    return Err("Codex closed the usage connection.".into());
                }
                self.total_bytes += count;
                if self.total_bytes > MAX_OUTPUT {
                    return Err("Codex usage output exceeded the size limit.".into());
                }
                self.pending.extend_from_slice(&buffer[..count]);
            } else {
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
            }
        }
    }

    fn write(&mut self, value: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.input, value)
            .map_err(|_| "Could not request Codex usage.")?;
        self.input
            .write_all(b"\n")
            .and_then(|_| self.input.flush())
            .map_err(|_| "Could not request Codex usage.".into())
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        self.write(&json!({"id":id,"method":method,"params":params}))?;
        loop {
            let message = self.receive()?;
            self.messages += 1;
            if self.messages > MAX_MESSAGES {
                return Err("Codex usage exceeded the message limit.".into());
            }
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    return Err(
                        "Codex requested an unsupported operation during usage reading.".into(),
                    );
                }
                continue;
            }
            if message["id"].as_u64() != Some(id) {
                continue;
            }
            if message.get("error").is_some() {
                return Err(
                    "Codex could not read usage. Check the official CLI sign-in and configuration."
                        .into(),
                );
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "Codex returned no usage result.".into());
        }
    }
}

fn trusted_cwd() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = [0u16; 32768];
    let length = unsafe {
        windows::Win32::System::SystemInformation::GetWindowsDirectoryW(Some(&mut buffer))
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Could not choose a trusted folder for the Codex usage reader.".into());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..length],
    )))
}

fn read_native(path: &Path) -> Result<Vec<UsageBucket>, String> {
    let mut server = NativeServer::start(path)?;
    server.request(1,"initialize",json!({"clientInfo":{"name":"coucou_usage_reader","title":"Coucou Codex usage","version":env!("CARGO_PKG_VERSION")}}))?;
    server.write(&json!({"method":"initialized","params":{}}))?;
    let result = server.request(
        2,
        "account/rateLimits/read",
        json!({"excludeResetCreditDetails":true}),
    )?;
    parse_usage(&result)
}

#[cfg(test)]
#[path = "usage_adversarial.rs"]
mod independent_tests;
