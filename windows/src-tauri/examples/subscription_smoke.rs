//! Optional local acceptance check. Never built into the desktop application.
//! Uses the same protected SIWC registration and native client as the app.
use chatgpt_client::Manager;
use serde_json::json;
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn main() {
    if let Err(error) = run() {
        eprintln!("Acceptance check failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let manager = Manager::new()?;
    if !manager.session().sharing {
        let start = manager.begin_login()?;
        let url =
            reqwest::Url::parse(&start.auth_url).map_err(|_| "Invalid official sign-in URL.")?;
        if url.scheme() != "https"
            || url.host_str() != Some("auth.openai.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() != Some(443)
        {
            manager.cancel_login(&start.attempt_id)?;
            return Err("Invalid official sign-in URL.".into());
        }
        let target: Vec<u16> = start
            .auth_url
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        let opened = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if opened.0 as isize <= 32 {
            manager.cancel_login(&start.attempt_id)?;
            return Err("Could not open the official sign-in browser.".into());
        }
        println!("OPENAI_SIGN_IN_WAITING: complete the official browser consent.");
        manager.finish_login(&start.attempt_id)?;
    }
    if !manager.session().sharing {
        return Err("The connection did not grant ChatGPT plan usage.".into());
    }
    println!("OFFICIAL_CONSENT_VERIFIED; plan usage granted.");
    let models = manager.models()?;
    let model = models
        .iter()
        .find(|model| model.slug.contains("luna"))
        .or_else(|| models.first())
        .ok_or("No eligible models were returned.")?;
    println!(
        "OFFICIAL_PLAN_CONNECTED; available models: {}",
        models.len()
    );
    let reply = manager.respond(
        &model.slug,
        json!([{"role":"user","content":"Responde únicamente con la palabra OK."}]),
        "Follow the user's request; use a brief text response.",
    )?;
    if reply.text.trim().is_empty() {
        return Err("The completed reply did not contain text.".into());
    }
    // Do not print credential, account, registration or prompt/history data.
    println!(
        "SUBSCRIPTION_INFERENCE_COMPLETED; model: {}; reply bytes: {}",
        model.slug,
        reply.text.len()
    );
    Ok(())
}
