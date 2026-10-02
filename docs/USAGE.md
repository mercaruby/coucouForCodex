# Consumption panel on Windows

Open **Consumo** (the bars icon in the expanded island). It shows percentages **used**, each reported window and its absolute reset time. Codex buckets stay separate; a missing principal window is not replaced by Spark or a weekly window. Unknown metrics display **Sin dato**, not 0%. The panel retains the previous successful reading on failure and marks it explicitly as old.

The interface is inspired by [CodeNotch](https://github.com/vinzdg/codenotch), reviewed at `6e8b0f828741233240d5fb10f7d52cf8eaf6efe4`. We adapted its consumption-window presentation and implemented a bounded native reader using the [official Codex app-server protocol](https://learn.chatgpt.com/docs/app-server). Its MIT attribution is preserved in [LICENSE-CODENOTCH.md](../LICENSE-CODENOTCH.md). We do not adopt its credential-import/private-HTTP/history fallbacks, LAN server or phone relay.

## Codex

The app starts its own trusted native `codex.exe` with private stdin/stdout, initializes the protocol and reads `account/rateLimits/read`. It does not start a thread, submit a turn, execute tools, install hooks or extract credentials. The official client manages its own sign-in. Model-catalog metadata work may also occur during official client startup; this is not an inference request.

Install the official Codex client and sign in there with ChatGPT. Automatic discovery is limited to supported official installation locations. If discovery is unavailable, enter the absolute path to the official native `codex.exe` in **Settings → Consumo de plataformas**. Command scripts, arguments and arbitrary executables found in the current project are not used for discovery.

The native reader bounds output, lines, messages and elapsed time, suppresses stderr and only returns the allowed quota fields. It removes inherited proxy/API-key/Node options, fixes provider destinations to official OpenAI endpoints and uses a neutral working directory. It terminates and waits for only its own child. An explicit executable path is trusted local configuration: select only the official binary. The installed client and operating system remain part of the trust boundary.

Refresh happens while the consumption view is expanded, at most once every five minutes automatically, with a manual **Actualizar** action. Pause or collapse stops automatic polling. Native caching groups concurrent reads and limits repeated manual requests. Statistics remain in memory; raw protocol output and authentication data are not shown or logged.

## ChatGPT in Coucou

This row describes Coucou's own official Sign in with ChatGPT connection. The SIWC documentation reviewed does not expose a public numeric usage getter for that grant. Therefore it shows connection status, a limit reported during a failed request when applicable, and **Gestionar uso**. It does not manufacture a percentage from an error or equate the Codex CLI account with Coucou's connected account.

OpenAI's `subscription_sharing_usage_limit_exceeded` can concern app or account limits. It does not establish that the entire plan is exhausted or provide a reset date. The error view links directly to [ChatGPT usage settings](https://chatgpt.com/settings/usage), preserves the typed message and offers **Volver al mensaje** and **Consumo**. A transient failure to check usage keeps the current connection and suggests trying later. No automatic switch to separately billed API requests occurs. See [official error recovery](https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery).

Managing a quota can require a human change to the app's allowance in ChatGPT or waiting for the provider's reset. The software cannot remove a provider limit. Changing the local code or reconnecting repeatedly is not a quota bypass.

An available catalog model is not a guarantee that a request to that model will be admitted. Automatic chat selection prefers `gpt-5.6-luna` when it is present in the connected account's catalog, otherwise the first available model. Settings displays the resolved automatic model. An explicit model selection is respected; a failed request never silently retries with a different model or billing path. A local comparison reproduced a sharing-limit rejection for the previous first-model default and completed a minimal request with Luna using the same connection.

For a local diagnostic check, close Coucou to serialize access to its rotating credentials. From `windows`, set `COUCOU_CHAT_ACCEPTANCE=1` and run `cargo run -p chatgpt-client --example check_connection --locked` (automatic selection), or append `-- gpt-5.6-luna` to select that eligible model explicitly. This sends one fixed, minimal request, uses the app's protected connection and prints only connection flags, model metadata and completion size or sanitized errors. It never prints tokens, account identifiers, reply text or provider message bodies, and does not start sign-in automatically.

Other CodeNotch providers are not enabled automatically. Their collectors use different credentials and interfaces; this adaptation covers the user's Codex and ChatGPT requirement.
