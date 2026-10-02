// Coucou for Windows — app wiring and the commands the island calls.

mod chatgpt;
mod files;
mod hooks;
mod integrations;
mod island;
mod log;
mod openai;
mod pipe;
mod secrets;
mod settings;
mod tray;
mod usage;
mod win_user;

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use files::DroppedFile;
use hooks::{HookPreview, HookStatus};
use island::{PollGate, ScreenInfo};
use openai::{Chat, ChatContext, ChatReply};
use pipe::Pending;
use settings::Settings;

/// A credential-store failure disables subscription chat without preventing
/// local Codex monitoring. No credential or internal OAuth state crosses IPC.
struct Subscription(Result<Arc<chatgpt_client::Manager>, String>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginAttempt {
    attempt_id: String,
}

impl Subscription {
    fn manager(&self) -> Result<Arc<chatgpt_client::Manager>, String> {
        self.0.clone()
    }
}

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
    hook_path: String,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    let mut settings = shared.settings.lock().unwrap().clone();
    // The real state of ~/.codex/hooks.json wins over whatever we stored.
    settings.hooks_installed = hooks::status().installed;
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
        hook_path: settings::hook_exe_path().to_string_lossy().to_string(),
    }
}

#[tauri::command]
async fn usage_read(
    shared: State<'_, Shared>,
    cache: State<'_, Arc<usage::UsageCache>>,
    refresh: bool,
) -> Result<usage::UsageSnapshot, String> {
    let path = shared.settings.lock().unwrap().codex_path.clone();
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || cache.read(&path, refresh))
        .await
        .map_err(|_| "Could not read Codex usage.".into())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    shared: State<Shared>,
    chat: State<Chat>,
    settings: Settings,
) -> Result<(), String> {
    if !matches!(settings.chat_backend.as_str(), "chatgpt" | "api") {
        return Err("Choose ChatGPT plan or OpenAI API as the chat backend.".into());
    }
    if !settings.codex_path.is_empty() && !std::path::Path::new(&settings.codex_path).is_absolute()
    {
        return Err("Codex executable path must be absolute.".into());
    }
    if settings.model.chars().any(char::is_whitespace) || settings.model.len() > 128 {
        return Err("Model must be a model ID without spaces.".into());
    }
    let (screen_changed, autostart_changed) = {
        let mut current = shared.settings.lock().unwrap();
        settings::save(&settings).map_err(|_| "Could not save settings to disk.".to_string())?;
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        if current.chat_backend != settings.chat_backend
            || current.model != settings.model
            || current.codex_path != settings.codex_path
        {
            chat.reset();
        }
        *current = settings.clone();
        (screen_changed, autostart_changed)
    };
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart {
            manager.enable()
        } else {
            manager.disable()
        };
        if let Err(err) = result {
            eprintln!("[coucou] autostart: {err}");
        }
    }
    if screen_changed {
        let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
        island::apply_geometry(&app, &settings.screen, collapsed);
    }
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
    Ok(())
}

/// Hidden island → shrink the window to the invisible wake strip and park the
/// cursor poll; anything else → full panel and 60 Hz polling.
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, collapsed: bool) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
    // The wake strip must always take the mouse, and a resize invalidates the flag.
    island::set_ignore_cursor(&app, false);
    shared.gate.forget_ignore_state();
    shared.gate.set_active(!collapsed);
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect {
        x,
        y,
        w: width,
        h: height,
    });
}

#[tauri::command]
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else {
        return;
    };
    island::set_activating(&win, focused);
    if focused {
        let _ = win.set_focus();
    }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let pref = shared.settings.lock().unwrap().screen.clone();
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    island::apply_geometry(&app, &pref, collapsed);
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// "Open terminal" opens the working folder in VS Code when `code` is on PATH,
/// and falls back to Explorer otherwise.
#[tauri::command]
fn open_in_vscode(path: Option<String>) -> bool {
    // No `cmd /C` anywhere near this. The path is a project folder chosen by
    // whoever is using Codex, and cmd would happily read `&`, `^` and `%`
    // in a folder name as syntax. Finding the launcher ourselves and handing the
    // path over as a separate argument keeps it a path.
    if let Some(code) = find_on_path("code") {
        let mut cmd = Command::new(code);
        if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
            cmd.arg(p);
        }
        if cmd.creation_flags(CREATE_NO_WINDOW).spawn().is_ok() {
            return true;
        }
    }
    if let Some(p) = path.as_deref().filter(|p| !p.is_empty()) {
        let _ = Command::new("explorer").arg(p).spawn();
    }
    false
}

/// Our own `where`: walks %PATH% against %PATHEXT%, no shell involved.
/// Rust quotes arguments correctly for `.cmd`/`.bat` targets since 1.77, so
/// spawning `code.cmd` directly is safe.
fn find_on_path(stem: &str) -> Option<std::path::PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{stem}{}", ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause. Paused means paused: the pollers stop talking to the network,
/// not just the island stopping showing things.
#[tauri::command]
fn set_paused(paused: bool) {
    integrations::set_paused(paused);
}

// ── Codex hooks ─────────────────────────────────────────────────────────

#[tauri::command]
fn hooks_status() -> HookStatus {
    hooks::status()
}

/// Returns the diff the user has to look at before anything is written.
#[tauri::command]
fn hooks_preview(install: bool) -> Result<HookPreview, String> {
    hooks::preview(install)
}

/// Only ever called from an explicit click in the settings window.
#[tauri::command]
fn hooks_apply(
    app: AppHandle,
    shared: State<Shared>,
    install: bool,
    fingerprint: String,
) -> Result<String, String> {
    // The fingerprint comes from the preview the user actually looked at, so a
    // settings.json that changed in between is refused rather than overwritten.
    let backup = hooks::write(install, &fingerprint)?;
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        current.hooks_installed = install;
        let _ = settings::save(&current);
        current.clone()
    };
    let _ = app.emit("settings-changed", updated);
    Ok(backup)
}

#[tauri::command]
fn approval_decision(app: AppHandle, request_id: String, decision: String) {
    pipe::answer(&app, &request_id, &decision);
}

/// The island has the card on screen, so the long wait for a human may begin.
/// Until this arrives the relay only waits a few hundred milliseconds, which is
/// what stops a paused or unresponsive island from freezing Codex.
#[tauri::command]
fn approval_ack(app: AppHandle, request_id: String) {
    pipe::acknowledge(&app, &request_id);
}

/// Nobody can act on this request — the island is paused, or another card is
/// already up. Codex falls back to asking in the terminal immediately.
#[tauri::command]
fn approval_decline(app: AppHandle, request_id: String) {
    pipe::decline(&app, &request_id);
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// One chat turn. The API key and any file bytes stay on the Rust side.
#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    subscription: State<'_, Subscription>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let (prefs, request_chat) = {
        let prefs = shared.settings.lock().unwrap();
        (prefs.clone(), chat.request())
    };
    match prefs.chat_backend.as_str() {
        "chatgpt" => {
            let manager = subscription.manager()?;
            tauri::async_runtime::spawn_blocking(move || {
                chatgpt::send(&manager, &request_chat, &prefs.model, query, context)
            })
            .await
            .map_err(|_| "ChatGPT worker failed.".to_string())?
        }
        "api" => openai::send(&request_chat, &prefs.model, query, context).await,
        _ => {
            Err("Unknown chat backend. Open Settings and choose ChatGPT plan or OpenAI API.".into())
        }
    }
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// The island may only ask whether a key exists — never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
}

/// Opens the configured n8n instance — the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window must therefore ask
/// for the *same* arguments as the island (see `additionalBrowserArgs` in
/// tauri.conf.json) — a mismatch makes the second window come up blank, with no
/// error anywhere.
const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

/// The settings window is created hidden at launch and only ever shown and
/// hidden afterwards. A WebView2 window created later — on the main thread or
/// not — silently comes up blank in this app, so the window that works is the
/// one that exists before the island's webview does.
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Settings — Coucou")
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .visible(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

fn reset_subscription_chat(app: &AppHandle, chat: &Chat) {
    chat.reset();
    let _ = app.emit("chat-reset", ());
}

/// Open only the native client's freshly generated official authorization URL.
/// ShellExecute uses the registered browser; no shell or PATH lookup is involved.
fn open_authorization_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid OpenAI sign-in URL.")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("auth.openai.com")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.port_or_known_default() != Some(443)
    {
        return Err("Invalid OpenAI sign-in URL.".into());
    }
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let target: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let action: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(action.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("Could not open the browser for OpenAI sign-in.".into());
    }
    Ok(())
}

#[tauri::command]
async fn chatgpt_login_start(
    app: AppHandle,
    chat: State<'_, Chat>,
    subscription: State<'_, Subscription>,
) -> Result<LoginAttempt, String> {
    reset_subscription_chat(&app, &chat);
    let manager = subscription.manager()?;
    tauri::async_runtime::spawn_blocking(move || {
        let start = manager.begin_login()?;
        if let Err(err) = open_authorization_url(&start.auth_url) {
            let _ = manager.cancel_login(&start.attempt_id);
            return Err(err);
        }
        Ok(LoginAttempt {
            attempt_id: start.attempt_id,
        })
    })
    .await
    .map_err(|_| "OpenAI sign-in worker failed.".to_string())?
}

#[tauri::command]
async fn chatgpt_login_finish(
    app: AppHandle,
    chat: State<'_, Chat>,
    subscription: State<'_, Subscription>,
    attempt_id: String,
) -> Result<chatgpt_client::Session, String> {
    let manager = subscription.manager()?;
    let worker = manager.clone();
    let session = tauri::async_runtime::spawn_blocking(move || worker.finish_login(&attempt_id))
        .await
        .map_err(|_| "OpenAI sign-in worker failed.".to_string())??;
    if manager.session().generation != session.generation {
        return Err("The ChatGPT connection changed during sign-in.".into());
    }
    reset_subscription_chat(&app, &chat);
    let _ = app.emit("chatgpt-session-changed", &session);
    Ok(session)
}

#[tauri::command]
async fn chatgpt_login_cancel(
    app: AppHandle,
    chat: State<'_, Chat>,
    subscription: State<'_, Subscription>,
    attempt_id: String,
) -> Result<(), String> {
    reset_subscription_chat(&app, &chat);
    let manager = subscription.manager()?;
    tauri::async_runtime::spawn_blocking(move || manager.cancel_login(&attempt_id))
        .await
        .map_err(|_| "OpenAI sign-in worker failed.".to_string())?
}

#[tauri::command]
fn chatgpt_session(subscription: State<Subscription>) -> Result<chatgpt_client::Session, String> {
    Ok(subscription.manager()?.session())
}

#[tauri::command]
async fn chatgpt_models(
    subscription: State<'_, Subscription>,
) -> Result<Vec<chatgpt_client::Model>, String> {
    let manager = subscription.manager()?;
    tauri::async_runtime::spawn_blocking(move || manager.models())
        .await
        .map_err(|_| "ChatGPT model worker failed.".to_string())?
}

#[tauri::command]
async fn chatgpt_logout(
    app: AppHandle,
    chat: State<'_, Chat>,
    subscription: State<'_, Subscription>,
) -> Result<(), String> {
    reset_subscription_chat(&app, &chat);
    let manager = subscription.manager()?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = manager.logout();
        let _ = app.emit("chatgpt-session-changed", manager.session());
        result
    })
    .await
    .map_err(|_| "ChatGPT sign-out worker failed.".to_string())?
}

pub fn run() {
    let loaded = settings::load();
    let gate = Arc::new(PollGate::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Pending::default())
        .manage(Chat::default())
        .manage(Arc::new(usage::UsageCache::default()))
        .manage(Subscription(chatgpt_client::Manager::new().map(Arc::new)))
        .invoke_handler(tauri::generate_handler![
            boot,
            usage_read,
            save_settings,
            set_collapsed,
            set_island_rect,
            focus_window,
            reposition,
            open_url,
            open_in_vscode,
            quit_app,
            hooks_status,
            hooks_preview,
            hooks_apply,
            approval_decision,
            approval_ack,
            approval_decline,
            log_line,
            chat_send,
            chat_reset,
            chatgpt_login_start,
            chatgpt_login_finish,
            chatgpt_login_cancel,
            chatgpt_session,
            chatgpt_models,
            chatgpt_logout,
            ingest_file,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            open_settings_window,
            set_paused,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);

            if let Some(win) = island::window(&handle) {
                island::make_non_activating(&win);
                island::apply_geometry(&handle, &loaded.screen, false);
                let _ = win.show();
            }
            gate.collapsed.store(false, Ordering::Relaxed);
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());

            log::line(format!(
                "--- Coucou {} started ---",
                env!("CARGO_PKG_VERSION")
            ));
            hooks::ensure_hook_exe(&handle);
            pipe::start(handle.clone());
            integrations::start(handle.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Coucou");
}
