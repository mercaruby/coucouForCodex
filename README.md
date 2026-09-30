# Personal Codex companion — Windows source fork

This personal source fork adapts [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou) for **Codex notifications, explicit approvals and OpenAI chat on Windows**. It does not require Claude. The macOS directory remains upstream Claude code; it is not migrated or verified for Codex.

## Capabilities

- Codex lifecycle hooks use `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`), a separate local pipe and explicit Allow/Deny buttons. After installation, review and trust the commands with `/hooks` in the official CLI. The island shows the latest session in an aggregated card, not a separate card per concurrent chat.
- API chat uses the official Responses endpoint, `store: false` and a separate Credential Manager service. API billing is independent of ChatGPT subscriptions. Default model is `gpt-5.6-sol`, subject to project access.
- ChatGPT session chat uses the official local `codex app-server` over stdin/stdout. This app never reads `auth.json`, copies OAuth tokens or accepts your ChatGPT password. **It rejects CLIs whose generated schema cannot restrict file reads. The official npm CLI 0.159.2 inspected during development lacks that capability and is blocked.** No successful authenticated inference is claimed. Choose API mode for chat with that runtime; observing hooks needs no API key.
- ChatGPT mode supports UTF-8 attachments up to 200 KB. API mode also supports images/PDFs up to 20 MB. Attachments are copied to the inbox and sent to OpenAI when submitted. Do not attach secrets.

## Build locally

Requires Windows, Node 20+, npm, Rust stable, MSVC C++ Build Tools and WebView2:

```powershell
git clone https://github.com/mercaruby/coucouForCodex.git
cd coucouForCodex/windows
npm ci
npm run build
cargo test --workspace --locked
cargo build -p coucou --locked
```

The application is `windows/target/debug/coucou.exe`. `npm run tauri dev` runs the native development app. CI checks builds/tests and does not publish installers or binary artifacts. Do not download the Claude app from the upstream releases expecting this adaptation.

## Setup and security

Open the tray menu → Settings. Install hooks only after inspecting the diff; the installer backs up existing files, refuses corrupt settings and stale previews, and preserves other handlers within mixed groups. It never edits Claude settings. Review the new definitions through `/hooks` in the official CLI.

Choose ChatGPT session or API explicitly. For ChatGPT, sign in through the official CLI and select its absolute native `codex.exe` path if absent from PATH, then Check connection. `.cmd` wrappers are not executed. Older CLIs show a compatibility error. For API, use a separate limited project key and monitor usage; only then enable integrations you need. Remote n8n requires HTTPS.

Preferences, inbox, log, pipe and credential service are isolated from upstream. API keys never return through IPC; raw authentication errors are not forwarded because they may echo keys. Chat history stays in this app's memory. Clearing chat or switching backend/model invalidates pending replies. Local IPC verifies the user's identity, bounds messages/connections and times out stalled senders. This does not protect against malware already running as your account. The official app-server inherits your Codex configuration, including configured integrations, which still requires your review.

See [comparison review](docs/REVIEW.md) and [validation](docs/VALIDATION.md). Builds and fixture tests do not certify future dependencies, downloaded installers or every Codex version. No production keys or paid inference were used in development.

## License and attribution

Code stays [MIT](LICENSE); upstream copyright is preserved. The initial Responses conversion uses MIT-licensed work from [iiZo7al/coucou-chatgpt](https://github.com/iiZo7al/coucou-chatgpt), with integration and security fixes added here.

Upstream names, character, icons, sounds and media have separate restrictions in [LICENSE-ASSETS.md](LICENSE-ASSETS.md). This is a personal source fork, not an independently branded app release. Replace restricted assets or obtain written permission before distributing derived binaries. No binaries are published here.
