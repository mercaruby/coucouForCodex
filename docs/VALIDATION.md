# Windows validation — 1 October 2026

Base commit: `Louis-CFM/coucou@5ae7bd946ab51493b5ddaebdc5f449f269ebb421`.

The comparison fork was reviewed at `iiZo7al/coucou-chatgpt@5b4ec94cc32684536e38a9b5bbfabe1bdf4ce43d`. This adaptation starts from the original rather than inheriting the entire external fork.

Passed during development: TypeScript no-emit check, Vite production build, Windows relay release build, 28 Rust workspace tests and npm audit (zero advisories in the retained frontend lockfile at review time).

A standalone Windows release was built with `tauri/custom-protocol`, so it embeds the frontend rather than requiring Vite. Native startup stayed running and created both WebView2 windows without a fresh settings-window error. Synthetic SessionStart/SessionEnd events reached the app through the per-user pipe. These checks did not modify the user's hooks.json and do not constitute a visual UI or authenticated chat test.

The protocol tests use a fixture process with no credentials or network calls. They check initialization, ChatGPT authentication type, text result processing and rejection of automatic approval requests. A legacy-schema fixture checks that a successful connection cannot silently waive the required read restriction. Security tests cover mixed hooks, corrupt settings, stale previews, backups, attachment confinement and n8n URLs. Relay regression tests reject oversized, unknown, ambiguous or visually incomplete permission requests. Known shell requests display every argument; other approvals remain in the official CLI.

The official npm Codex CLI downloaded for protocol inspection and the installed desktop CLI both report `codex-cli 0.159.2`. Their generated readOnly SandboxPolicy has only type and networkAccess, without restricted access. The application blocks session chat on those runtimes. Documentation describing a capability does not prove a particular installed runtime implements it.

No real OpenAI API/ChatGPT inference, OAuth login, production secret, real hook installation or live Allow/Deny action was used to validate this build. macOS is upstream and unverified for Codex. No installer is published. Fixture tests do not replace an authenticated end-to-end test with a compatible official runtime.

Sources: [hooks](https://learn.chatgpt.com/docs/hooks), [app-server](https://learn.chatgpt.com/docs/app-server), [authentication](https://learn.chatgpt.com/docs/auth).
