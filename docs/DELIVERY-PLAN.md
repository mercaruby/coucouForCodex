# OAuth subscription integration delivery plan

Owner: root orchestrator / PM. Scope: the Windows companion, official OpenAI OAuth and authorised ChatGPT plan usage. Codex lifecycle monitoring remains independent of chat authentication.

| Role | Assignment | Evidence |
| --- | --- | --- |
| PM / orchestrator | Resolve architecture, wire Tauri, coordinate acceptance and publish verified source | This plan, final validation and commit |
| Researcher | Verify official registration, auth, inference and subscription contracts | RESEARCH-OAUTH.md |
| Integrator | Native Rust SIWC client and frontend account/model controls | chatgpt-client and settings interface |
| Validator | Independent negative tests, build and end-to-end evidence | VALIDATION-OAUTH.md |
| Security auditor | Review credential boundaries and adversarial paths independently | SECURITY-OAUTH.md |

Four concurrent agent slots are available, including the PM. Research completed first; the independent validator then took that slot. The researcher remains available for follow-up questions.

## Architecture decision

Use the official dynamic-registration SIWC flow and public Responses API directly. The native client owns its registration and protected credentials. It does not import another application's login or grant the model access to Codex tools, global configuration, shell, MCP or local files. The app sends only chat messages and attachments explicitly submitted by the user. API billing is a separate, explicitly selected backend and is never a recovery path for failed plan usage.

## Acceptance

1. Browser OAuth uses OpenAI discovery with strict HTTPS origin checks, PKCE, unpredictable state and nonce, a prebound loopback callback, and validated ID-token signature/claims.
2. Issued client ID, granted scope and verified identity agree with the attempt. Identity-only consent does not enable plan inference.
3. Credentials remain in native code and protected Windows storage. Partial writes, failed refresh, cancellation and logout cannot revive an obsolete session or reveal tokens.
4. Models come from the authorised connection. A reply succeeds only after a completed Responses stream; account restrictions and exhausted usage remain visible errors.
5. Independent regression tests and Windows frontend/native builds pass. Native startup and synthetic Codex hook delivery remain functional.
6. Actual OAuth consent and one small inference using the user's eligible account are recorded separately from fixture evidence. Only the human completes sign-in. No passwords or tokens are requested in chat.
7. Documentation describes observed behaviour and remaining limits. No claim of zero risk or universal subscription eligibility. No derived binary publication with upstream restricted assets.

## Current milestones

- Research complete: official SIWC dynamic registration is available for local open-source apps.
- Integration complete: native OAuth, host-local protected storage, model discovery and Responses stream handling; frontend connect/cancel/disconnect and account models.
- Independent security audit accepts the implementation within its stated threat boundary. The initial OAuth delivery passed 73 workspace tests. The consumption and recovery extension passes 93: 46 OAuth/client, 37 application, 6 relay and 4 retained legacy protocol fixtures. Final TypeScript/Vite and relay production builds pass.
- The human user completed official consent. The native acceptance check confirmed plan scope, five available models and a completed response with gpt-5.6-luna. The final native release built and started successfully; synthetic SessionStart/SessionEnd delivery passed. This account-specific result does not establish universal plan eligibility.
- Verified source is published in this user-owned fork. No installer or derived binary is published.
- The 2026-10-02 extension adds the CodeNotch-inspired consumption panel using only the official native Codex metadata protocol, preserves unknown/stale values and separates Coucou's SIWC grant from Codex quotas. Real read-only quota acceptance passed without inference. Error recovery preserves the draft and attachment, provides official usage management and supports keyboard/scroll access. The final native release built, started and received synthetic lifecycle events successfully. See [independent evidence](VALIDATION-USAGE.md).
