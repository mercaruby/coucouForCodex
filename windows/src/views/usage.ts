import { h, clear } from "./dom";
import { State } from "../core/state";
import { Usage, percent, resetLabel, windowLabel, type UsageWindow } from "../core/usage";
import { MANAGE_USAGE_URL } from "../core/chat-notice";
import type { ViewActions, ViewHost } from "./views";

function windowRow(window: UsageWindow): HTMLElement {
  const value = window.usedPercent;
  const known = value !== null && Number.isFinite(value) && value >= 0 && value <= 100;
  const meter = h("div", { class: "usage-meter" });
  if (known) {
    meter.setAttribute("role", "progressbar");
    meter.setAttribute("aria-label", `${windowLabel(window)} · porcentaje usado`);
    meter.setAttribute("aria-valuemin", "0");
    meter.setAttribute("aria-valuemax", "100");
    meter.setAttribute("aria-valuenow", String(value));
    meter.append(h("i", { style: `width:${value}%;background:${value >= 90 ? "#F5A524" : "#B6BDCC"}` }));
  } else meter.classList.add("unknown");
  return h("div", { class: "usage-window" },
    h("div", { class: "usage-line" }, h("span", { text: windowLabel(window) }), h("strong", { text: percent(value) })),
    meter, h("div", { class: "usage-muted", text: resetLabel(window.resetsAt) }));
}

/** CodeNotch-inspired windows and freshness, using only the official Codex RPC. */
export function buildUsage(actions: ViewActions): ViewHost {
  const refresh = h("button", { class: "link-btn", text: "Actualizar", onclick: () => void Usage.refresh(true) });
  const stamp = h("span", { class: "usage-muted" });
  const rows = h("div", { class: "usage-scroll", tabindex: "0", "aria-label": "Consumo por plataforma" });
  const el = h("div", { class: "view usage-view" }, h("div", { class: "card usage-card" },
    h("div", { class: "usage-heading" }, h("strong", { text: "Consumo" }), stamp, refresh), rows));
  let revision = -1;
  return {
    el,
    focus: () => { rows.focus(); void Usage.refresh(); },
    tick: () => { if (!State.paused && State.mode === "expanded") void Usage.refresh(); },
    sync() {
      if (revision === Usage.revision) return;
      revision = Usage.revision;
      refresh.disabled = Usage.busy;
      refresh.textContent = Usage.busy ? "Consultando…" : "Actualizar";
      const snapshot = Usage.snapshot;
      stamp.textContent = snapshot?.fetchedAt
        ? `${snapshot.status === "stale" ? "Lectura anterior · " : ""}${new Intl.DateTimeFormat("es", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(snapshot.fetchedAt * 1000))}`
        : "Porcentaje usado";
      clear(rows);
      const codex = h("section", { class: "usage-provider" },
        h("div", { class: "usage-line" }, h("strong", { text: "Codex" }), h("span", { class: "usage-muted", text: "Cliente oficial" })));
      if (snapshot?.buckets.length) {
        for (const bucket of snapshot.buckets) {
          if (bucket.id !== "codex") codex.append(h("div", { class: "usage-bucket", text: bucket.name ?? bucket.id }));
          codex.append(h("div", { class: "usage-windows" }, ...bucket.windows.map(windowRow)));
        }
      } else {
        codex.append(h("div", { class: "usage-muted usage-empty", text: Usage.busy ? "Consultando los límites de tu sesión de Codex…" : "Sin datos. Instala Codex e inicia sesión con ChatGPT en su cliente oficial." }));
      }
      if (snapshot?.status === "stale") codex.append(h("div", { class: "usage-warning", text: "No se pudo actualizar. Estos porcentajes son de la lectura anterior." }));
      const settings = h("button", { class: "link-btn", text: "Configurar Codex", onclick: () => actions.openSettingsWindow() });
      if (!snapshot?.buckets.length) codex.append(settings);
      rows.append(codex);

      const session = Usage.session;
      const status = Usage.limitReported ? "Límite comunicado por OpenAI" : Usage.sessionUnavailable ? "No se pudo comprobar la conexión"
        : session?.connected && session.sharing ? "Conectado con tu plan" : "Sin conexión con tu plan";
      rows.append(h("section", { class: "usage-provider chatgpt-usage" },
        h("div", { class: "usage-line" }, h("strong", { text: "ChatGPT en Coucou" }),
          h("button", { class: "link-btn", text: "Gestionar uso ↗", onclick: () => actions.openUrl(MANAGE_USAGE_URL) })),
        h("div", { class: Usage.limitReported ? "usage-warning" : "usage-status", text: status }),
        h("div", { class: "usage-muted", text: "Porcentaje no publicado para esta conexión. Su límite puede diferir del de Codex." })));
    },
  };
}
