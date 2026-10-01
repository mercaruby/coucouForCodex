# Personal Codex companion — Windows source fork

This personal source fork adapts [Louis-CFM/coucou](https://github.com/Louis-CFM/coucou) for **Codex monitoring and official Sign in with ChatGPT on Windows**. Claude is not required. The macOS directory remains upstream Claude code; it is not migrated or verified for this integration.

## ChatGPT plan chat

Choose **ChatGPT plan** in Settings and **Continue with ChatGPT**. Your system browser opens OpenAI's official authorization page. Select your account/workspace and review permission to use your ChatGPT plan. OpenAI registers this local app during consent; no API key or client secret is required. [Official integration](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt).

The native Rust client owns this app's OAuth registration. It validates PKCE, state, nonce and signed identity claims, and keeps credentials in a separate Windows Credential Manager service. It does not import Codex's auth.json, browser cookies or another app's tokens. Tokens never return to the webview.

Chat uses the public Responses API directly, with store:false, streaming and no tools. The model cannot run local commands, browse local files, use MCP/plugins or inherit Codex configuration. The app sends the messages and attachments you submit. Models come from the connected account; success requires a completed stream. Limits or declined consent remain errors and never switch to API billing.

Identity sign-in alone does not grant plan usage. Your account must grant that scope and meet OpenAI's current plan/workspace eligibility. Settings provides **Manage usage**. Provider policies still apply to submitted content; store:false does not remove every retention policy. See [plan usage](https://developers.openai.com/siwc/token-sharing-open-source).

The optional **OpenAI API** backend is a separate explicit choice with its own key and project billing. A ChatGPT subscription does not supply that key. Its default model is gpt-5.6-sol, subject to project access.

## Codex monitoring and approvals

Lifecycle hooks use $CODEX_HOME/hooks.json (default ~/.codex/hooks.json), a separate per-user pipe, and explicit Allow/Deny buttons for recognised shell requests whose arguments can be displayed completely. Unknown, oversized or ambiguous requests stay in the official terminal. The island shows the latest session in an aggregated card.

Inspect the installation diff in Settings before applying it. Installation backs up existing files, refuses corrupt settings and stale previews, and preserves other handlers in mixed groups. Then review and trust the exact definitions with /hooks in the official Codex CLI. Chat authentication and hook monitoring are independent; installing hooks does not import a login or grant permissions automatically.

Attachments are copied to the inbox. Text is limited to 200 KB; inline images/PDFs to 20 MB and the selected model's capabilities. Submit only content you intend to send to OpenAI. Clearing chat, changing backend/model or changing the OAuth connection invalidates pending replies.

## Build locally

Requires Windows, Node 20+, npm, Rust stable, MSVC C++ Build Tools and WebView2:

```powershell
git clone https://github.com/mercaruby/coucouForCodex.git
cd coucouForCodex/windows
npm ci
npm run build
cargo test --workspace --locked
cargo build --release -p coucou --features tauri/custom-protocol --locked
```

The standalone app is windows/target/release/coucou.exe; keep coucou-hook.exe beside it. npm run tauri dev runs the development app. CI checks builds/tests and does not publish installers or binaries. Upstream Claude releases do not contain this adaptation.

For an optional local acceptance check, close Coucou first and run `cargo run -p coucou --example subscription_smoke --locked` from `windows`. This opens official consent if needed and sends one brief request using the connected plan. It prints only public result flags and model metadata; it does not print tokens or account details. Do not run it alongside the desktop app because both use the same protected registration.

## Security and validation

Preferences, inbox, log, pipe and credential namespaces are isolated from upstream. OAuth uses fixed official HTTPS origins, rejects redirects and inherited HTTP proxies, confines callbacks to loopback, bounds inputs and rejects incomplete results. Hook IPC verifies the Windows user identity and limits messages, connections and waiting time. Remote n8n requires HTTPS; integrations need their own configured credentials.

These controls do not protect against malware or administrators already acting as your Windows account, a compromised browser/OS or an upstream provider compromise. No review guarantees zero risk. The model has no automatic path to unsubmitted local data in subscription chat.

See [research](docs/RESEARCH-OAUTH.md), [security acceptance](docs/SECURITY-OAUTH.md), [independent OAuth validation](docs/VALIDATION-OAUTH.md), [original/fork comparison](docs/REVIEW.md) and [delivery plan](docs/DELIVERY-PLAN.md). Fixture tests, builds and actual consent/inference are separate evidence. Consult validation for what has actually run.

If the protected ChatGPT profile is damaged, chat stops while Codex monitoring remains available. Follow the recovery instructions in [security acceptance](docs/SECURITY-OAUTH.md); do not paste credentials into an issue or chat.

## License and attribution

Code stays [MIT](LICENSE); upstream copyright is preserved. The initial API conversion uses MIT-licensed work from [iiZo7al/coucou-chatgpt](https://github.com/iiZo7al/coucou-chatgpt), with integration and security fixes. SIWC applies the published protocol with cryptographic libraries; the separately licensed official devkit is not copied or bundled.

Names, character, icons, sounds and media retain restrictions in [LICENSE-ASSETS.md](LICENSE-ASSETS.md). This is a personal source fork. Replace restricted assets or obtain written permission before distributing derived binaries. No binaries are published here.
