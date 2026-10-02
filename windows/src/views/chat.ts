// Chat view — DOM port of PromptView / ChatBubble / TypingDotsView from
// IslandViewContent.swift.

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, onEvent, type ChatContext } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type ChatMessage } from "../core/state";
import type { ViewHost } from "./views";
import { chatNotice, MANAGE_USAGE_URL } from "../core/chat-notice";
import { Usage } from "../core/usage";
import { ChatModelCatalog } from "../core/chat-models";

let nextId = 1;

function bubble(message: ChatMessage): HTMLElement {
  if (message.role === "user") {
    return h(
      "div",
      { class: "chat-row user" },
      h("div", { class: "bubble", text: message.content }),
    );
  }
  return h("div", { class: "chat-row" }, h("div", { class: "reply", text: message.content }));
}

function typingDots(): HTMLElement {
  return h(
    "div",
    { class: "chat-row" },
    h("div", { class: "typing" }, h("i"), h("i"), h("i")),
  );
}

/** The coloured chip showing what the question is about (a dropped file). */
function contextChip(label: string): HTMLElement {
  const chip = h("div", { class: "chip" }, h("i", { class: "chip-dot" }), h("span", { text: label }));
  requestAnimationFrame(() => chip.classList.add("settled"));
  return chip;
}

export function buildPrompt(onHeightChange: () => void): ViewHost {
  const chipRow = h("div", { class: "chip-row" });
  const log = h("div", { class: "chat-log" });
  const input = h("input", {
    type: "text",
    class: "chat-input",
    placeholder: "Ask me anything…",
    spellcheck: "false",
  }) as HTMLInputElement;
  const send = h("button", { class: "send-btn", title: "Send" }, svg(ICONS.arrowUp, 11));
  const bar = h("div", { class: "chat-bar" }, input, send);
  const modelSelect = h("select", { class: "chat-model-select", "aria-label": "Modelo del chat" });
  const modelRow = h("div", { class: "chat-model-row" }, h("span", { text: "Modelo" }), modelSelect);
  const modelFeedback = h("span", { "aria-live": "polite" });
  const retryModels = h("button", { class: "link-btn", text: "Reintentar" });
  const modelStatus = h("div", { class: "chat-model-feedback" }, modelFeedback, retryModels);
  const planLabel = h("span");
  const manage = h("button", { class: "link-btn", text: "Gestionar uso ↗", onclick: () => void Bridge.openUrl(MANAGE_USAGE_URL) });
  const plan = h("div", { class: "chat-plan" }, planLabel, manage);

  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card wash chat-card" }, h("div", { class: "chat-body" }, chipRow, log, modelRow, modelStatus, bar, plan)),
  );
  (el.querySelector(".card") as HTMLElement).style.setProperty("--wash", "rgba(99,102,241,0.5)");

  let sending = false;
  let modelSaving = false;
  let saveError: string | null = null;
  let optionKey = "";
  let renderedCount = -1;
  const catalog = new ChatModelCatalog(() => {
    State.notify();
    onHeightChange();
  });
  // Both windows share native settings/account events. Hidden views discard stale catalogs too.
  let observedModel = State.settings.model;
  State.subscribe(() => {
    const invalidated = catalog.observe();
    if (invalidated || observedModel !== State.settings.model) saveError = null;
    observedModel = State.settings.model;
  });
  void onEvent<boolean>("chatgpt-login-state", (busy) => catalog.setLoginBusy(busy));

  function readyToSend() {
    return !sending && !modelSaving && !catalog.loginBusy && (State.settings.chatBackend === "api" || catalog.available);
  }

  function syncModels() {
    const api = State.settings.chatBackend === "api";
    const visible = State.view === "prompt" && el.isConnected && !el.closest("[inert]");
    catalog.ensure(visible);
    const choices = api ? [{ slug: "gpt-5.6-sol", displayName: "gpt-5.6-sol" }] : catalog.models;
    const automatic = api ? "gpt-5.6-sol" : catalog.automatic?.displayName;
    const selected = State.settings.model;
    const unavailable = selected !== "" && !choices.some((model) => model.slug === selected);
    const key = JSON.stringify([api, automatic, choices, unavailable]);
    if (key !== optionKey) {
      optionKey = key;
      clear(modelSelect);
      modelSelect.append(h("option", { value: "", text: automatic ? `Automático · ${automatic}` : "Automático" }));
      for (const model of choices) modelSelect.append(h("option", { value: model.slug, text: model.displayName }));
      if (unavailable) modelSelect.append(h("option", { value: "__unavailable", text: "Modelo guardado no disponible", disabled: true }));
    }
    modelSelect.value = unavailable ? "__unavailable" : selected;
    modelSelect.disabled = sending || modelSaving || catalog.loginBusy || (!api && !catalog.available);
    const feedback = saveError ?? (modelSaving ? "Guardando modelo…" : !api && catalog.loginBusy ? "Conexión en curso…"
      : !api && catalog.loading ? "Cargando modelos…" : !api && catalog.error ? catalog.error
      : !api && !catalog.connected ? "Conecta ChatGPT en Ajustes para elegir un modelo."
      : !api && !catalog.sharing ? "Activa el permiso de uso de ChatGPT en Ajustes."
      : unavailable ? "Elige un modelo disponible antes de enviar." : null);
    modelFeedback.textContent = feedback ?? "";
    modelStatus.hidden = feedback === null;
    retryModels.hidden = api || !catalog.error || modelSaving || sending || catalog.loginBusy;
    send.disabled = !readyToSend() || unavailable;
  }

  async function chooseModel() {
    const selected = modelSelect.value;
    const api = State.settings.chatBackend === "api";
    if (sending || modelSaving || modelSelect.disabled || selected === State.settings.model ||
      (selected !== "" && !(api ? selected === "gpt-5.6-sol" : catalog.models.some((model) => model.slug === selected)))) {
      syncModels();
      return;
    }
    const before = State.settings;
    const epoch = State.chatEpoch;
    const generation = api ? null : Usage.sessionGeneration;
    modelSaving = true;
    saveError = null;
    syncModels();
    State.notify();
    try {
      const settings = await Bridge.setChatModel(selected, before.chatBackend, before.model, generation);
      // The native event normally commits first. A later external preference/account change wins.
      if (State.settings === before && (api || Usage.sessionGeneration === generation) && State.chatEpoch === epoch) {
        State.settings = settings;
        State.resetChat();
      }
    } catch {
      saveError = "No se pudo cambiar el modelo. Revisa la conexión y vuelve a elegirlo.";
    } finally {
      modelSaving = false;
      syncModels();
      State.notify();
      onHeightChange();
    }
  }
  modelSelect.addEventListener("change", () => void chooseModel());
  modelSelect.addEventListener("keydown", (event) => event.stopPropagation());
  retryModels.addEventListener("click", () => {
    saveError = null;
    catalog.retry();
  });

  async function submit() {
    const query = input.value.trim();
    if (!query || !readyToSend() || (State.settings.model !== "" &&
      !(State.settings.chatBackend === "api" ? State.settings.model === "gpt-5.6-sol"
        : catalog.models.some((model) => model.slug === State.settings.model)))) return;
    input.value = "";
    sending = true;
    const epoch = State.chatEpoch;
    Sound.play("send");

    const submittedId = nextId++;
    State.chatHistory.push({ id: submittedId, role: "user", content: query });
    State.stateOverride = "thinking";
    State.notify();
    onHeightChange();

    const file = State.droppedFile;
    const context: ChatContext | null =
      State.chatHistory.length === 1 && file ? { kind: "file", name: file.name, path: file.path } : null;

    try {
      const reply = await Bridge.chatSend(query, context);
      if (epoch !== State.chatEpoch) return;
      State.chatHistory.push({ id: nextId++, role: "assistant", content: reply.text });
      if (State.settings.chatBackend === "chatgpt") Usage.reportLimit(false);
      State.stateOverride = null;
      Sound.play("finish");
    } catch (err) {
      if (epoch !== State.chatEpoch) return;
      State.chatHistory = State.chatHistory.filter((message) => message.id !== submittedId);
      input.value = query;
      State.stateOverride = null;
      const notice = chatNotice(err, State.settings.chatBackend);
      State.noteChat = notice;
      State.noteMessage = notice.title;
      if (notice.kind === "limit") Usage.reportLimit(true);
      State.view = "note";
      Sound.play("error");
    } finally {
      sending = false;
      State.notify();
      onHeightChange();
      if (State.view === "prompt") input.focus();
    }
  }

  send.addEventListener("click", () => void submit());
  input.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") {
      e.preventDefault();
      void submit();
    }
    e.stopPropagation(); // Escape closes the island, not the chat
  });

  return {
    el,
    sync() {
      const file = State.droppedFile;
      const wantChip = file?.name ?? "";
      if (chipRow.dataset.label !== wantChip) {
        chipRow.dataset.label = wantChip;
        clear(chipRow);
        if (wantChip) chipRow.append(contextChip(wantChip));
      }

      const thinking = State.stateOverride === "thinking";
      const count = State.chatHistory.length + (thinking ? 0.5 : 0);
      if (count !== renderedCount) {
        renderedCount = count;
        clear(log);
        for (const m of State.chatHistory) log.append(bubble(m));
        if (thinking) log.append(typingDots());
        log.scrollTop = log.scrollHeight;
      }

      input.placeholder = State.chatHistory.length === 0 ? "Ask me anything…" : "Continue…";
      input.disabled = sending;
      syncModels();
      planLabel.textContent = State.settings.chatBackend === "chatgpt" ? "Usando tu plan de ChatGPT" : "OpenAI API · facturación independiente";
      manage.hidden = State.settings.chatBackend !== "chatgpt";
    },
    focus() {
      input.focus();
      input.select();
    },
  };
}
