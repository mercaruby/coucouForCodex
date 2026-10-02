import { Bridge, IS_TAURI, type ChatGPTSession } from "./bridge";
import { State } from "./state";

export interface UsageWindow {
  id: string;
  usedPercent: number | null;
  windowDurationMins: number | null;
  resetsAt: number | null;
}
export interface UsageBucket {
  id: string;
  name: string | null;
  planType: string | null;
  windows: UsageWindow[];
}
export interface UsageSnapshot {
  status: "available" | "stale" | "unavailable";
  source: "codex-app-server";
  fetchedAt: number | null;
  error: string | null;
  buckets: UsageBucket[];
}

export function percent(value: number | null): string {
  return value !== null && Number.isFinite(value) && value >= 0 && value <= 100
    ? `${new Intl.NumberFormat("es", { maximumFractionDigits: 1 }).format(value)} %` : "Sin dato";
}
export function windowLabel(window: UsageWindow): string {
  const minutes = window.windowDurationMins;
  if (minutes === 10080) return "Semana";
  if (minutes !== null && minutes > 0) {
    return minutes % 1440 === 0 ? `${minutes / 1440} días`
      : minutes % 60 === 0 ? `${minutes / 60} horas` : `${minutes} minutos`;
  }
  return window.id === "primary" ? "Ventana principal" : "Ventana secundaria";
}
export function resetLabel(epoch: number | null): string {
  if (epoch === null || !Number.isFinite(epoch) || epoch <= 0) return "Reinicio sin dato";
  const date = new Date(epoch * 1000);
  if (!Number.isFinite(date.getTime())) return "Reinicio sin dato";
  return `Reinicio · ${new Intl.DateTimeFormat("es", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(date)}`;
}

/** Memory-only UI data; credentials and raw RPC messages stay outside this module. */
export const Usage = {
  snapshot: null as UsageSnapshot | null,
  session: null as Pick<ChatGPTSession, "connected" | "sharing" | "generation"> | null,
  sessionUnavailable: false,
  limitReported: false,
  busy: false,
  checkedAt: 0,
  revision: 0,
  sessionGeneration: null as number | null,

  async refresh(force = false) {
    if (this.busy || State.paused || (!force && Date.now() - this.checkedAt < 300_000)) return;
    this.busy = true;
    const configuredPath = State.settings.codexPath;
    const sessionAtStart = this.sessionGeneration;
    this.checkedAt = Date.now();
    this.revision++;
    State.notify();
    if (IS_TAURI) {
      const results = await Promise.allSettled([Bridge.usageRead(force), Bridge.chatgptSession()]);
      const [codex, chatgpt] = results;
      if (configuredPath === State.settings.codexPath) {
        if (codex.status === "fulfilled") this.snapshot = codex.value;
        else this.snapshot = { status: this.snapshot?.buckets.length ? "stale" : "unavailable",
          source: "codex-app-server", fetchedAt: this.snapshot?.fetchedAt ?? null,
          buckets: this.snapshot?.buckets ?? [], error: "No se pudo consultar Codex. Revisa su instalación y sesión." };
      }
      // A login/logout event supersedes metadata requested before that event.
      if (this.sessionGeneration === sessionAtStart) {
        this.sessionUnavailable = chatgpt.status === "rejected";
        if (chatgpt.status === "fulfilled") this.updateSession(chatgpt.value);
      }
    }
    this.busy = false;
    this.revision++;
    State.notify();
  },
  reportLimit(limited: boolean) {
    this.limitReported = limited;
    this.revision++;
    State.notify();
  },
  invalidateCodex() {
    this.snapshot = null;
    this.checkedAt = 0;
    this.revision++;
    State.notify();
  },
  updateSession(session: Pick<ChatGPTSession, "connected" | "sharing" | "generation">) {
    if (this.sessionGeneration !== null && this.sessionGeneration !== session.generation) this.limitReported = false;
    this.sessionGeneration = session.generation;
    this.session = { connected: session.connected, sharing: session.sharing, generation: session.generation };
    this.sessionUnavailable = false;
    this.revision++;
    State.notify();
  },
};
