// Settings window — the place where anything that writes to disk is confirmed.
// Stage 2 covers the Codex hooks and the general preferences; API keys and
// integrations land here too in a later stage.

import "./settings.css";
import { Bridge, onEvent, type HookStatus, type ChatGPTSession, type ChatGPTModel } from "../core/bridge";
import { DEFAULT_SETTINGS, type Settings } from "../core/state";
import { h, clear } from "../views/dom";
import { MANAGE_USAGE_URL } from "../core/chat-notice";

let settings: Settings = { ...DEFAULT_SETTINGS };
let version = "";
let saveQueue: Promise<boolean> = Promise.resolve(true);
let updateChatControls = () => {};
let updateApiVisibility = () => {};

const root = document.getElementById("settings-root")!;

async function save() {
  const snapshot = { ...settings, activeIntegrations: [...settings.activeIntegrations] };
  saveQueue = saveQueue.then(async () => {
    try {
      await Bridge.saveSettings(snapshot);
      document.getElementById("save-error")?.remove();
      return true;
    } catch {
      let error = document.getElementById("save-error");
      if (!error) {
        error = h("div", { id: "save-error", class: "notice err", role: "alert" });
        root.prepend(error);
      }
      error.textContent = "Could not save preferences. Check your settings folder permissions and try again.";
      return false;
    }
  });
  return saveQueue;
}

// ── Reusable bits ─────────────────────────────────────────────────────────────

function toggle(on: boolean, onChange: (v: boolean) => void): HTMLElement {
  const el = h("button", { class: on ? "switch on" : "switch", "aria-pressed": on });
  el.addEventListener("click", () => {
    const next = !el.classList.contains("on");
    el.classList.toggle("on", next);
    onChange(next);
  });
  return el;
}

function statusDot(ok: boolean): HTMLElement {
  return h("i", { class: "dot", style: `background:${ok ? "#22c55e" : "#f4505e"}` });
}

function renderDiff(text: string): HTMLElement {
  const box = h("div", { class: "diff" });
  for (const line of text.split("\n")) {
    const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : "ctx";
    box.append(h("div", { class: cls, text: line }));
  }
  return box;
}

// ── Codex section ───────────────────────────────────────────────────────

function codexSection(status: HookStatus): HTMLElement {
  const body = h("div", { style: "display:flex;flex-direction:column;gap:12px" });
  const section = h(
    "section",
    {},
    h("h2", {}, statusDot(status.installed), h("span", { text: "Codex" })),
    body,
  );

  const rebuild = async () => {
    const fresh = await Bridge.hooksStatus();
    if (fresh) Object.assign(status, fresh);
    clear(body);
    draw();
    const head = section.querySelector("h2")!;
    clear(head);
    head.append(statusDot(status.installed), h("span", { text: "Codex" }));
  };

  function draw() {
    body.append(
      h("div", {
        class: "hint",
        text: status.installed
          ? "Coucou is hooked into your Codex sessions. Tool calls, questions and permission requests show up in the island, and you can answer them there."
          : "Install the hooks to see your Codex sessions in the island and respond to permission requests there. If Coucou is unavailable, Codex asks in its own interface.",
      }),
      h("div", { class: "row" },
        h("label", { text: "hooks.json" }),
        h("span", { class: "path", text: status.settingsPath }),
      ),
      h("div", { class: "row" },
        h("label", { text: "Relay" }),
        h("span", { class: "path", text: status.hookPath }),
        statusDot(status.hookReady),
      ),
    );
    body.append(h("div", {
      class: "hint",
      text: "After installation, review and trust the hooks with /hooks inside Codex. This updates only Codex hooks.json; your Claude configuration is untouched.",
    }));

    if (!status.hookReady) {
      body.append(h("div", {
        class: "notice warn",
        text: "coucou-hook.exe is not in place yet. Restart Coucou; if it still fails, build it with `cargo build -p coucou-hook`.",
      }));
    }

    const actions = h("div", { class: "row" });
    const install = h("button", {
      class: "primary",
      text: status.installed ? "Reinstall hooks…" : "Install hooks…",
      onclick: () => showPreview(true),
    });
    // Writing hook commands that point at a relay which isn't there would give
    // every Codex session a broken hook and nothing to show for it.
    if (!status.hookReady) {
      install.disabled = true;
      install.title = "The relay isn't installed yet.";
    }
    actions.append(install);
    if (status.installed) {
      actions.append(h("button", {
        class: "danger",
        text: "Uninstall hooks…",
        onclick: () => showPreview(false),
      }));
    }
    body.append(actions);
  }

  async function showPreview(install: boolean) {
    let preview;
    try {
      preview = await Bridge.hooksPreview(install);
    } catch (err) {
      // An unreadable or invalid hooks.json stops here rather than being
      // treated as empty and written over.
      clear(body);
      body.append(
        h("div", { class: "notice err", text: String(err).replace(/^Error:\s*/, "") }),
        h("div", { class: "row" }, h("button", {
          text: "Back",
          onclick: () => { clear(body); draw(); },
        })),
      );
      return;
    }
    if (!preview) return;
    clear(body);
    body.append(
      h("div", {
        class: "hint",
        text: install
          ? "This is exactly what will change in your hooks.json. Your own hooks are left untouched."
          : "This removes Coucou's entries only. Your own hooks are left untouched.",
      }),
      renderDiff(preview.diff),
      h("div", { class: "row" },
        h("span", { class: "path", text: `Backup → ${preview.backup}` }),
      ),
    );
    const confirm = h("button", {
      class: install ? "primary" : "danger",
      text: install ? "Back up and write" : "Back up and remove",
    });
    confirm.addEventListener("click", async () => {
      confirm.disabled = true;
      try {
        const backup = await Bridge.hooksApply(install, preview.fingerprint);
        clear(body);
        body.append(h("div", {
          class: "notice ok",
          text: `Done. Previous settings saved as ${backup}. Open Codex and review the hooks with /hooks before trusting them.`,
        }));
        window.setTimeout(() => void rebuild(), 2600);
      } catch (err) {
        confirm.disabled = false;
        body.append(h("div", { class: "notice err", text: `Could not write: ${String(err)}` }));
      }
    });
    body.append(h("div", { class: "row" }, confirm, h("button", {
      text: "Cancel",
      onclick: () => { clear(body); draw(); },
    })));
  }

  draw();
  return section;
}

// ── Chat backend ──────────────────────────────────────────────────────────────

function usageSection(): HTMLElement {
  const path = h("input", { type: "text", value: settings.codexPath, placeholder: "Automático · cliente oficial instalado", "aria-label": "Ruta del cliente oficial codex.exe" });
  const feedback = h("div", { class: "hint" });
  const apply = h("button", { text: "Guardar ruta", onclick: async () => {
    const next = path.value.trim();
    if (next && !/^(?:[A-Za-z]:\\|\\\\).+\\codex\.exe$/i.test(next)) {
      feedback.textContent = "Indica una ruta absoluta a codex.exe, sin comillas ni argumentos.";
      return;
    }
    const before = settings.codexPath;
    settings.codexPath = next;
    apply.disabled = true;
    const saved = await save();
    if (!saved) settings.codexPath = before;
    apply.disabled = false;
    feedback.textContent = saved ? "Ruta guardada. Abre Consumo para actualizar la lectura." : "No se pudo guardar la ruta.";
  } });
  return h("section", {}, h("h2", { text: "Consumo de plataformas" }),
    h("div", { class: "hint", text: "Codex publica sus porcentajes a través de su cliente oficial. Inicia sesión allí con ChatGPT. Coucou consulta los límites sin enviar mensajes y conserva la última lectura si falla la actualización." }),
    h("div", { class: "row" }, h("label", { text: "Cliente Codex" }), path, apply),
    h("div", { class: "hint", text: "Opcional: elige únicamente el codex.exe oficial. La conexión de ChatGPT de Coucou se gestiona por separado; OpenAI no publica su porcentaje mediante esta conexión." }),
    h("button", { text: "Gestionar uso de ChatGPT ↗", onclick: () => void Bridge.openUrl(MANAGE_USAGE_URL) }), feedback);
}

const MODELS: [string, string][] = [
  ["", "Default server model"],
  ["gpt-5.6-sol", "GPT-5.6 Sol"],
];

function chatgptChatSection(): HTMLElement {
  const backend = h("select", { "aria-label": "Chat backend" });
  backend.append(
    h("option", { value: "chatgpt", text: "ChatGPT · your connected plan" }),
    h("option", { value: "api", text: "OpenAI API · separate billing" }),
  );
  const hint = h("div", { class: "hint" });
  const model = h("select", { "aria-label": "Chat model" });
  const account = h("div", { class: "hint", text: "No ChatGPT account connected." });
  const connect = h("button", { class: "primary", text: "Continue with ChatGPT" });
  const cancel = h("button", { text: "Cancel sign-in" });
  const disconnect = h("button", { class: "danger", text: "Disconnect" });
  const check = h("button", { text: "Check connection" });
  const manage = h("button", { text: "Manage usage", onclick: () => void Bridge.openUrl("https://chatgpt.com/settings/usage") });
  const feedback = h("div", { role: "status", "aria-live": "polite" });
  const auth = h("div", { style: "display:flex;flex-direction:column;gap:12px" }, account,
    h("div", { class: "row" }, connect, cancel, disconnect, check, manage),
    h("div", { class: "hint", text: "Sign-in opens the official OpenAI page in your browser. Review and allow ChatGPT plan usage there. Your password and OAuth tokens never enter this interface. Coucou keeps its own protected connection in Windows Credential Manager." }),
  );
  let session: ChatGPTSession | null = null;
  let models: ChatGPTModel[] = [];
  let attempt: string | null = null;
  let revision = 0;
  let loading = false;

  function showError(error: unknown) {
    clear(feedback);
    feedback.append(h("div", { class: "notice err", text: String(error).replace(/^Error:\s*/, "") }));
  }

  function drawModels() {
    clear(model);
    model.append(h("option", { value: "", text: settings.chatBackend === "chatgpt" ? "Default · first available account model" : "Default · GPT-5.6 Sol" }));
    const choices = settings.chatBackend === "chatgpt"
      ? models.map((entry) => [entry.slug, entry.displayName] as [string, string])
      : MODELS.filter(([slug]) => slug);
    for (const [slug, label] of choices) model.append(h("option", { value: slug, text: label }));
    if (settings.model && !choices.some(([slug]) => slug === settings.model)) {
      model.append(h("option", { value: settings.model, text: `${settings.model} · unavailable for this connection`, disabled: true }));
    }
    model.value = settings.model;
    model.disabled = loading || attempt !== null || (settings.chatBackend === "chatgpt" && !session?.sharing);
  }

  updateChatControls = () => {
    backend.value = settings.chatBackend;
    backend.disabled = loading;
    const chatgpt = settings.chatBackend === "chatgpt";
    hint.textContent = chatgpt
      ? "Use eligible requests from your ChatGPT plan after authorizing this app. Limits and availability depend on your connected account. There is no automatic switch to API billing. Changing the backend or model starts a new conversation."
      : "Use your saved OpenAI API key. API requests are billed separately from your ChatGPT plan. Choosing this backend is explicit; ChatGPT sign-in never falls back to it.";
    auth.style.display = chatgpt ? "flex" : "none";
    account.textContent = session?.connected
      ? `${session.email ?? "ChatGPT account connected"} · ${session.sharing ? "ChatGPT plan usage allowed" : "ChatGPT plan usage has not been allowed"}`
      : "No ChatGPT account connected.";
    connect.textContent = session?.connected ? "Reconnect with ChatGPT" : "Continue with ChatGPT";
    connect.disabled = loading || attempt !== null;
    cancel.style.display = attempt ? "" : "none";
    disconnect.style.display = session?.connected ? "" : "none";
    disconnect.disabled = loading || attempt !== null;
    check.disabled = loading || attempt !== null;
    drawModels();
    updateApiVisibility();
  };

  async function refreshConnection(visible = false) {
    const current = ++revision;
    loading = true;
    if (visible) clear(feedback);
    updateChatControls();
    try {
      const nextSession = await Bridge.chatgptSession();
      const nextModels = nextSession.sharing ? await Bridge.chatgptModels() : [];
      if (current !== revision) return;
      session = nextSession;
      models = nextModels;
      if (visible) feedback.append(h("div", {
        class: session.sharing ? "notice ok" : "notice warn",
        text: session.sharing
          ? `ChatGPT connection verified. ${models.length} models available. Sending a message will use your connected plan allowance.`
          : "Continue with ChatGPT and allow plan usage in the official sign-in page before sending a message.",
      }));
    } catch (error) {
      if (current === revision) showError(error);
    } finally {
      if (current === revision) { loading = false; updateChatControls(); }
    }
  }

  backend.addEventListener("change", async () => {
    const before = settings.chatBackend;
    const previousModel = settings.model;
    const selected = backend.value as Settings["chatBackend"];
    const current = ++revision;
    loading = true;
    updateChatControls();
    try {
      if (selected === "api" && attempt) {
        const cancelledAttempt = attempt;
        await Bridge.chatgptLoginCancel(cancelledAttempt);
        if (attempt === cancelledAttempt) attempt = null;
      }
      settings.chatBackend = selected;
      settings.model = "";
      if (!(await save()) && current === revision) {
        settings.chatBackend = before;
        settings.model = previousModel;
      }
    } catch (error) { if (current === revision) showError(error); }
    finally { if (current === revision) { loading = false; updateChatControls(); } }
    if (current === revision && settings.chatBackend === "chatgpt") void refreshConnection();
  });
  model.addEventListener("change", async () => {
    const before = settings.model;
    settings.model = model.value;
    if (!(await save())) settings.model = before;
    updateChatControls();
  });
  check.addEventListener("click", () => void refreshConnection(true));
  connect.addEventListener("click", async () => {
    const current = ++revision;
    clear(feedback);
    loading = true;
    updateChatControls();
    let ownAttempt: string | null = null;
    try {
      const started = await Bridge.chatgptLoginStart();
      ownAttempt = started.attemptId;
      attempt = ownAttempt;
      loading = false;
      feedback.append(h("div", { class: "notice warn", text: "Complete sign-in and review ChatGPT plan usage in your browser. This window is waiting for OpenAI's callback." }));
      updateChatControls();
      const next = await Bridge.chatgptLoginFinish(ownAttempt);
      if (attempt !== ownAttempt) return;
      attempt = null;
      session = next;
      clear(feedback);
      feedback.append(h("div", { class: next.sharing ? "notice ok" : "notice warn", text: next.sharing
        ? "ChatGPT connected. This app can use your approved plan allowance."
        : "Account connected. ChatGPT plan usage was not granted; reconnect and review the requested permission to use chat." }));
      await refreshConnection();
    } catch (error) {
      if (ownAttempt === null || attempt === ownAttempt) showError(error);
    } finally {
      if (current === revision) {
        if (ownAttempt === null || attempt === ownAttempt) attempt = null;
        loading = false;
        updateChatControls();
      }
    }
  });
  cancel.addEventListener("click", async () => {
    const current = attempt;
    if (!current) return;
    cancel.disabled = true;
    try {
      await Bridge.chatgptLoginCancel(current);
      attempt = null;
      ++revision;
      loading = false;
      clear(feedback);
      feedback.append(h("div", { class: "notice warn", text: "Sign-in cancelled." }));
    } catch (error) { showError(error); }
    finally { cancel.disabled = false; updateChatControls(); }
  });
  disconnect.addEventListener("click", async () => {
    loading = true;
    updateChatControls();
    try {
      await Bridge.chatgptLogout();
      clear(feedback);
      feedback.append(h("div", { class: "notice ok", text: "ChatGPT disconnected. The app's registration is retained for reconnecting." }));
    } catch (error) { showError(error); }
    finally { loading = false; await refreshConnection(); }
  });
  void onEvent<ChatGPTSession>("chatgpt-session-changed", (next) => {
    session = next;
    models = [];
    updateChatControls();
    if (!attempt && !loading) void refreshConnection();
  });
  updateChatControls();
  void refreshConnection();
  return h("section", {}, h("h2", { text: "Chat" }),
    h("div", { class: "row" }, h("label", { text: "Backend" }), backend), hint, auth,
    h("div", { class: "row" }, h("label", { text: "Model" }), model), feedback,
  );
}

// ── Optional OpenAI API credentials ───────────────────────────────────────────

function apiSection(hasKey: boolean): HTMLElement {
  const dot = statusDot(hasKey);
  const state = h("span", { class: "hint", text: hasKey ? "Key saved in the Windows Credential Manager." : "No API key saved. Add one only if you choose the API backend." });

  const field = h("input", {
    type: "password",
    placeholder: hasKey ? "••••••••••••  (stored)" : "sk-...",
    style: "flex:1 1 auto;min-width:0",
    autocomplete: "off",
    spellcheck: "false",
  }) as HTMLInputElement;

  const saveBtn = h("button", { class: "primary", text: "Save key" });
  const clearBtn = h("button", { class: "danger", text: "Remove" });
  const feedback = h("div", {});

  async function refresh() {
    const present = (await Bridge.secretPresent("openai-api-key")) ?? false;
    dot.style.background = present ? "#22c55e" : "#f4505e";
    state.textContent = present
      ? "Key saved in the Windows Credential Manager."
      : "No API key saved. Add one only if you choose the API backend.";
    field.placeholder = present ? "••••••••••••  (stored)" : "sk-...";
    clearBtn.style.display = present ? "" : "none";
  }

  saveBtn.addEventListener("click", async () => {
    const value = field.value.trim();
    if (!value) return;
    clear(feedback);
    try {
      await Bridge.secretSet("openai-api-key", value);
      field.value = "";
      feedback.append(h("div", { class: "notice ok", text: "Saved in Windows Credential Manager." }));
      await refresh();
    } catch {
      feedback.append(h("div", { class: "notice err", text: "Could not save the key in Windows Credential Manager." }));
    }
  });

  clearBtn.addEventListener("click", async () => {
    clear(feedback);
    try {
      await Bridge.secretClear("openai-api-key");
      feedback.append(h("div", { class: "notice ok", text: "Key removed." }));
      await refresh();
    } catch (err) {
      feedback.append(h("div", { class: "notice err", text: `Could not remove: ${String(err)}` }));
    }
  });

  clearBtn.style.display = hasKey ? "" : "none";

  const section = h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: "OpenAI API (optional)" })),
    state,
    h("div", { class: "row" }, h("label", { text: "API key" }), field, saveBtn, clearBtn),
    h("div", { class: "hint", text: "Optional. Never paste your ChatGPT password, browser cookies or session tokens here." }),
    feedback,
  );
  updateApiVisibility = () => { section.style.display = settings.chatBackend === "api" ? "" : "none"; };
  updateApiVisibility();
  return section;
}

// ── Integrations section ──────────────────────────────────────────────────────

interface IntegrationDef {
  id: string;
  name: string;
  color: string;
  /** Credential Manager keys, in the order they are shown. */
  fields: { key: string; label: string; placeholder: string; secret: boolean }[];
}

const INTEGRATIONS: IntegrationDef[] = [
  { id: "integration_stripe", name: "Stripe", color: "#0570DE",
    fields: [{ key: "stripe-api-key", label: "Secret key", placeholder: "sk_live_…", secret: true }] },
  { id: "integration_github", name: "GitHub", color: "#F4505E",
    fields: [{ key: "github-token", label: "Token", placeholder: "ghp_…", secret: true }] },
  { id: "integration_vercel", name: "Vercel", color: "#7C5CFF",
    fields: [{ key: "vercel-token", label: "Token", placeholder: "…", secret: true }] },
  { id: "integration_n8n", name: "n8n", color: "#F29B38",
    fields: [
      { key: "n8n-url", label: "Instance URL", placeholder: "https://n8n.example.com", secret: false },
      { key: "n8n-api-key", label: "API key", placeholder: "…", secret: true },
    ] },
  { id: "integration_resend", name: "Resend", color: "#22C55E",
    fields: [{ key: "resend-api-key", label: "API key", placeholder: "re_…", secret: true }] },
  { id: "integration_notion", name: "Notion", color: "#8C8C8C",
    fields: [{ key: "notion-api-key", label: "Integration token", placeholder: "ntn_…", secret: true }] },
  { id: "integration_calcom", name: "Cal.com", color: "#C9956A",
    fields: [{ key: "calcom-api-key", label: "API key", placeholder: "cal_…", secret: true }] },
];

const MAX_ACTIVE = 4;

function integrationsSection(present: Record<string, boolean>): HTMLElement {
  const note = h("div", { class: "hint" });
  const list = h("div", { style: "display:flex;flex-direction:column;gap:14px" });

  function updateNote() {
    const used = settings.activeIntegrations.length;
    note.textContent = `Pick up to ${MAX_ACTIVE} pills to show next to Mochi — ${used}/${MAX_ACTIVE} in use. Keys are stored in the Windows Credential Manager, never on disk.`;
  }

  for (const def of INTEGRATIONS) {
    const active = settings.activeIntegrations.includes(def.id);
    const sw = h("button", { class: active ? "switch on" : "switch" });
    sw.addEventListener("click", () => {
      const on = settings.activeIntegrations.includes(def.id);
      if (on) {
        settings.activeIntegrations = settings.activeIntegrations.filter((x) => x !== def.id);
      } else {
        if (settings.activeIntegrations.length >= MAX_ACTIVE) return;
        settings.activeIntegrations = [...settings.activeIntegrations, def.id];
      }
      sw.classList.toggle("on", !on);
      updateNote();
      void save();
    });

    const rows = h("div", { style: "display:flex;flex-direction:column;gap:6px;flex:1 1 auto;min-width:0" });
    for (const field of def.fields) {
      const input = h("input", {
        type: field.secret ? "password" : "text",
        placeholder: present[field.key] ? "••••••••  (stored)" : field.placeholder,
        autocomplete: "off",
        spellcheck: "false",
        style: "flex:1 1 auto;min-width:0",
      }) as HTMLInputElement;
      const saveBtn = h("button", { text: "Save" });
      const dotEl = statusDot(present[field.key] ?? false);
      saveBtn.addEventListener("click", async () => {
        const value = input.value.trim();
        try {
          await Bridge.secretSet(field.key, value);
          present[field.key] = value.length > 0;
          input.value = "";
          input.placeholder = value ? "••••••••  (stored)" : field.placeholder;
          dotEl.style.background = value ? "#22c55e" : "#f4505e";
        } catch {
          dotEl.style.background = "#f5a524";
        }
      });
      rows.append(
        h("div", { class: "row" },
          h("label", { style: "min-width:104px", text: field.label }),
          input, saveBtn, dotEl,
        ),
      );
    }

    list.append(
      h("div", { style: "display:flex;gap:12px;align-items:flex-start" },
        h("div", { style: "display:flex;align-items:center;gap:8px;min-width:132px;padding-top:4px" },
          sw,
          h("i", { class: "dot", style: `background:${def.color}` }),
          h("span", { style: "font-size:12.5px", text: def.name }),
        ),
        rows,
      ),
    );
  }

  updateNote();
  return h("section", {}, h("h2", {}, h("span", { text: "Integrations" })), note, list);
}

// ── General section ───────────────────────────────────────────────────────────

function generalSection(): HTMLElement {
  const volume = h("input", {
    type: "range", min: "0", max: "0.2", step: "0.005",
    value: String(settings.soundVolume),
  }) as HTMLInputElement;
  volume.addEventListener("input", () => {
    settings.soundVolume = Number(volume.value);
    void save();
  });

  const autoClose = h("input", {
    type: "number", min: "5", max: "120", step: "1",
    value: String(Math.round(settings.autoCloseInterval)),
    style: "width:72px",
  }) as HTMLInputElement;
  autoClose.addEventListener("change", () => {
    settings.autoCloseInterval = Math.max(5, Math.min(120, Number(autoClose.value) || 15));
    autoClose.value = String(settings.autoCloseInterval);
    void save();
  });

  const screen = h("select", {}) as HTMLSelectElement;
  screen.append(
    h("option", { value: "primary", text: "Main display" }),
    h("option", { value: "cursor", text: "Display under the cursor" }),
  );
  screen.value = settings.screen;
  screen.addEventListener("change", () => {
    settings.screen = screen.value as Settings["screen"];
    void save();
  });

  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "General" })),
    h("div", { class: "row" },
      h("label", { text: "Sound" }),
      toggle(settings.soundEnabled, (v) => { settings.soundEnabled = v; void save(); }),
      volume,
    ),
    h("div", { class: "row" },
      h("label", { text: "Auto-close" }),
      autoClose,
      h("span", { class: "hint", text: "seconds after you leave the island" }),
    ),
    h("div", { class: "row" },
      h("label", { text: "Island lives on" }),
      screen,
    ),
    h("div", { class: "row" },
      h("label", { text: "Launch at startup" }),
      toggle(settings.autostart, (v) => { settings.autostart = v; void save(); }),
    ),
  );
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function main() {
  const boot = await Bridge.boot();
  if (boot) {
    settings = { ...settings, ...boot.settings };
    version = boot.version;
  }
  const status = (await Bridge.hooksStatus()) ?? {
    installed: false, settingsPath: "", hookPath: "", hookReady: false,
  };

  const hasKey = (await Bridge.secretPresent("openai-api-key")) ?? false;

  const keys = [
    "stripe-api-key", "github-token", "vercel-token",
    "n8n-url", "n8n-api-key", "resend-api-key", "notion-api-key", "calcom-api-key",
  ];
  const present: Record<string, boolean> = {};
  for (const k of keys) present[k] = (await Bridge.secretPresent(k)) ?? false;

  clear(root);
  root.append(
    h("h1", {}, h("span", { text: "Coucou" }), h("span", { class: "version", text: version })),
    codexSection(status),
    chatgptChatSection(),
    usageSection(),
    apiSection(hasKey),
    integrationsSection(present),
    generalSection(),
    h("div", {
      class: "hint",
      text: "No telemetry. Network requests only go to the services you configure yourself.",
    }),
  );

  void onEvent<Settings>("settings-changed", (s) => {
    settings = { ...settings, ...s };
    updateChatControls();
  });
}

void main();
