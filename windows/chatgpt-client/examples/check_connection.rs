//! Explicit local acceptance check. Sends one fixed, minimal message through
//! Coucou's own protected OAuth connection; never prints credentials or replies.
use chatgpt_client::Manager;
use serde_json::json;

fn main() {
    if std::env::var("COUCOU_CHAT_ACCEPTANCE").as_deref() != Ok("1") {
        eprintln!("Set COUCOU_CHAT_ACCEPTANCE=1 to send one fixed test using your ChatGPT plan.");
        std::process::exit(2);
    }
    if let Err(error) = check() {
        // Manager returns fixed public errors, never raw provider bodies.
        eprintln!("Check failed: {error}");
        std::process::exit(1);
    }
}

fn check() -> Result<(), String> {
    let manager = Manager::new()?;
    let session = manager.session();
    println!(
        "Connection: connected={}, planPermission={}",
        session.connected, session.sharing
    );
    let models = manager.models()?;
    let requested = std::env::args().nth(1).unwrap_or_default();
    let selected = if requested.is_empty() {
        models
            .iter()
            .find(|model| model.slug == "gpt-5.6-luna")
            .or_else(|| models.first())
    } else {
        models.iter().find(|model| model.slug == requested)
    }
    .ok_or("The requested model is not in this connection's catalog.")?;
    let model_label = match selected.slug.as_str() {
        "gpt-5.6-luna" => "gpt-5.6-luna",
        "gpt-6-astra" => "gpt-6-astra",
        _ => "selected catalog model",
    };
    println!(
        "Catalog verified: {} models; testing {}",
        models.len(),
        model_label
    );
    let reply = manager.respond(
        &requested,
        json!([{"role":"user","content":[{"type":"input_text","text":"Reply only OK."}]}]),
        "Reply with two letters: OK.",
    )?;
    println!("Completed: replyBytes={}", reply.text.len());
    Ok(())
}
