# Chat default and error recovery validation

Date: 2 October 2026. This follow-up diagnoses a chat failure reported while Coucou already had permission to use the connected ChatGPT plan.

## Reproduction and correction

The previous empty model setting selected the first visible model in the account catalog. A fixed minimal request selected GPT-6 Astra and received the explicit provider code `subscription_sharing_usage_limit_exceeded`. The same protected OAuth registration accepted the fixed request to GPT-5.6 Luna, completing a two-byte reply. No repeated consent, credential imports, other billing path or quota bypass was used.

Automatic selection now prefers the exact eligible catalog entry `gpt-5.6-luna`; if absent, it retains the first available model. A manually chosen model is respected and there are no automatic retries with another model. Settings shows the resolved automatic model. The fixed automatic path was then executed through `Manager.respond` with an empty requested model and completed another two-byte response.

Three minimal requests were made in this diagnosis: the old default rejected, the explicit Luna request completed, and the corrected automatic request completed. Coucou was closed during these checks to prevent separate processes racing its rotating OAuth credentials. Only public connection flags, fixed model labels and completion sizes were printed; credentials, identities, reply text and provider message bodies were omitted. The helper is `windows/chatgpt-client/examples/check_connection.rs` and requires explicit `COUCOU_CHAT_ACCEPTANCE=1`.

## Error policy

Non-success HTTP responses now retain an allowlisted provider code, actual HTTP status and fixed content-format classification. Error bodies are read internally with a 64 KiB bound, default JSON recursion bound and at most four nested error/detail wrappers. Unknown codes, messages, detail strings and other body fields are never reflected. A structured JSON error with HTTP 200 still fails; it never becomes a successful text reply. SSE failures use the same error policy and never return partial success.

HTTP 403 alone no longer means reconnect. A generic 429 does not prove a sharing quota is exhausted. Eligibility, permissions, configuration, transient outages and unsupported capabilities receive distinct recovery notices. Request IDs are not retained by this minimal diagnostic, so it does not provide complete provider support correlation.

The model comparison establishes that this connection could complete a Luna request at the time of testing. It does not explain OpenAI's internal admission decision for Astra or promise future model availability, quota or service uptime. Numeric consumption for Coucou's own OAuth grant remains unavailable through the documented public interface.

## Independent regression evidence

`cargo test -p chatgpt-client --locked` passed **61 tests with zero failures**, including 15 new independent regressions: three model-selection cases in `windows/chatgpt-client/src/adversarial.rs` and 12 error-policy cases in `windows/chatgpt-client/src/responses_error_adversarial.rs`. Automatic selection prefers catalog-listed Luna over an earlier Astra entry, never invents an unavailable Luna model, respects an explicit eligible model and rejects unavailable explicit selections without switching models.

The error fixtures verify quota-specific HTTP 403, unknown 403 without a reconnect instruction, unavailable usage, known versus unknown HTTP 429, redaction of arbitrary messages/metadata/header values, malformed UTF-8 and I/O errors, and partial SSE text followed by a provider failure. A completed event containing a provider error never returns accumulated text. Exact 65,536-byte JSON is accepted for classification; an oversized payload has no semantic effect, and an unlimited synthetic reader is stopped after exactly 65,537 bytes. Four nested wrappers are supported, five remain unknown, and JSON nesting beyond the parser's recursion bound is rejected without reflection. Structured JSON codes can be recognized independently of the MIME header; the content-format diagnostic remains a fixed classification. No fixture sends HTTP requests or accesses real credentials.

`node tests/chat-notice.test.cjs` passed **16 frontend recovery mappings**, plus redaction, model guidance, the fixed official management link and explicit separation from API billing. The executable test file is `windows/tests/chat-notice.test.cjs`; it evaluates the actual TypeScript notice module using the project's existing TypeScript compiler and synthetic strings only. The Settings automatic label was inspected against the backend rule: exact `gpt-5.6-luna` when listed, otherwise the first eligible catalog entry.

The final frontend passed `tsc --noEmit` and the Vite production build after all notice changes. Only the production island and Settings entries were built; test pages and scripts are excluded. The acceptance helper's guard, fixed prompt, use of the original empty/manual model request and sanitized output were inspected; compilation of the example is included in the crate test command, but tests do not execute its network operation. The live checks above were coordinated and executed separately by the PM. This validation agent made no further inference requests, login attempts, credential refreshes or native application changes.

The PM built the final native application with `cargo build --release -p coucou --features tauri/custom-protocol --locked` successfully, then restarted that exact executable. The process reached input-idle and remained responsive. This confirms native startup separately from the fixed live chat check and frontend fixtures; it does not claim automated visual verification of the whole native UI.
