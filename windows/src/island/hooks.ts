// Codex Code hook events → island state.
// Port of HookServer.processEvent / processPermissionRequest from the macOS app.
// Difference from macOS: no terminal filter. On Windows the hook fires from any
// terminal (Windows Terminal, VS Code, PowerShell…) and all of them are handled.

import { Bridge, onEvent } from "../core/bridge";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { Island } from "./island";

const CODEX_ID = "integration_codex";

/** Clears the approval card if no decision was made before the hook gave up. */
let pendingTimeout: number | null = null;
let activityRevision = 0;
let currentSessionId = "";

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  message?: string;
  /** UserPromptSubmit carries `prompt`; `message` belongs to Notification/Stop. */
  prompt?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
}

const PROJECT_ALIASES: Record<string, string> = {
  "notch-buddy": "Notch Buddy",
  notchbuddy: "Notch Buddy",
  notch_buddy: "Notch Buddy",
};

function aliasProjectName(name: string): string {
  return PROJECT_ALIASES[name.toLowerCase()] ?? name;
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

/** frenchStep() — same labels as the macOS app. */
const TOOL_LABELS: Record<string, string> = {
  Bash: "Exécute",
  Read: "Lit",
  Write: "Écrit",
  Edit: "Modifie",
  Glob: "Cherche",
  Grep: "Recherche",
  WebSearch: "Recherche web",
  WebFetch: "Récupère",
  TodoWrite: "Tâches",
  Task: "Agent",
  LS: "Liste",
  MultiEdit: "Modifie",
  NotebookEdit: "Notebook",
  PowerShell: "Exécute",
};

function stepLabel(tool: string, input: Record<string, unknown>): string {
  const label = TOOL_LABELS[tool] ?? tool;
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  const cmd = str("command");
  if (cmd) return `${label} · ${cmd.slice(0, 40)}`;
  const path = str("path");
  if (path) return `${label} · ${lastPathComponent(path)}`;
  const file = str("file_path");
  if (file) return `${label} · ${lastPathComponent(file)}`;
  const query = str("query");
  if (query) return `${label} · ${query.slice(0, 40)}`;
  return label;
}

/** The relay validates too; defend the UI if an older relay is still installed. */
function reviewablePermission(payload: HookPayload): boolean {
  const tool = (payload.tool_name ?? "").replace(/^functions\./, "");
  const input = payload.tool_input;
  if (!input || typeof input !== "object" || Array.isArray(input)) return false;
  let field: "command" | "cmd";
  let array = false;
  switch (tool) {
    case "Bash": case "PowerShell": case "shell_command": field = "command"; break;
    case "exec_command": field = "cmd"; break;
    case "shell": field = "command"; array = true; break;
    default: return false;
  }
  if (Object.hasOwn(input, field === "command" ? "cmd" : "command")) return false;
  const command = input[field];
  if (array) {
    if (!Array.isArray(command) || !command.length || !command.every((v) => typeof v === "string" && v.length > 0)) return false;
  } else if (typeof command !== "string" || !command.trim()) return false;
  const bytes = (s: string) => new TextEncoder().encode(s).length;
  const complete = (v: unknown): boolean => {
    if (typeof v === "string") return bytes(v) < 2000 && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f\u00ad\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/u.test(v);
    if (Array.isArray(v)) return v.every(complete);
    if (v && typeof v === "object") return Object.entries(v).every(([k, value]) => complete(k) && complete(value));
    if (typeof v === "number") return Number.isFinite(v) && Math.abs(v) <= Number.MAX_SAFE_INTEGER;
    return true;
  };
  return bytes(JSON.stringify(command)) < 2000 && bytes(JSON.stringify(input)) <= 8192 && complete(payload);
}

function approvalTarget(payload: HookPayload): string {
  // Every argument and working directory is displayed verbatim, with JSON
  // escaping for control characters; never authorise a summary of the request.
  return JSON.stringify({ tool: payload.tool_name, cwd: payload.cwd ?? "", tool_input: payload.tool_input }, null, 2);
}

function upsert(projectName: string, cwd: string) {
  const t = State.tasks.find((x) => x.id === CODEX_ID);
  if (!t) return;
  t.name = projectName;
  if (cwd) t.sessionCwd = cwd;
}

function clearSession() {
  const t = State.tasks.find((x) => x.id === CODEX_ID);
  if (!t) return;
  t.steps = [];
  t.stepIndex = 0;
  t.name = "VS Code";
  t.pillBadge = null;
}

export function registerHookHandlers(island: Island) {
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  if (State.paused) {
    // Silence here used to cost Codex Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately.
    if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
    return;
  }

  const name = payload.hook_event_name ?? "";
  const cwd = payload.cwd ?? "";
  const raw = lastPathComponent(cwd);
  const projectName = aliasProjectName(raw || "Session");
  const focused = State.focusId === CODEX_ID;
  const sessionId = payload.session_id ?? cwd;
  const startsActivity = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "PermissionRequest"].includes(name);
  if (State.pendingApproval && sessionId && State.pendingApproval.sessionId && State.pendingApproval.sessionId !== sessionId) {
    if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
    return;
  }
  if (startsActivity) currentSessionId = sessionId;
  else if (sessionId && currentSessionId && sessionId !== currentSessionId) return;
  const revision = ++activityRevision;

  /** Alerts force the island open; work events only reveal the compact island. */
  const surface = (view: Parameters<Island["alert"]>[0], isAlert: boolean) => {
    if (State.mode === "expanded") {
      if (isAlert) island.setView(view);
    } else if (isAlert) {
      island.alert(view);
    } else if (State.mode === "hidden") {
      island.reveal();
    }
  };

  switch (name) {
    case "SessionStart":
      upsert(projectName, cwd);
      surface("overview", false);
      Sound.play("work");
      break;

    case "UserPromptSubmit": {
      upsert(projectName, cwd);
      State.updateTask(CODEX_ID, "thinking");
      // The field is `prompt`; reading `message` meant this step was always blank.
      const asked = payload.prompt ?? payload.message;
      if (asked) State.appendStep(CODEX_ID, asked.slice(0, 60));
      surface("overview", false);
      break;
    }

    case "PreToolUse": {
      upsert(projectName, cwd);
      State.updateTask(CODEX_ID, "working");
      const tool = payload.tool_name ?? "Tool";
      State.appendStep(CODEX_ID, stepLabel(tool, payload.tool_input ?? {}));
      surface("overview", false);
      break;
    }

    case "PostToolUse":
      State.updateTask(CODEX_ID, "working");
      break;

    case "PostToolUseFailure":
      State.updateTask(CODEX_ID, "working");
      State.appendStep(CODEX_ID, "⚠ failed");
      break;

    case "Notification": {
      const message = payload.message ?? "";
      const lower = message.toLowerCase();
      if (lower.includes("rate limit") || lower.includes("limite d")) {
        State.updateTask(CODEX_ID, "ratelimit");
        Sound.play("rate");
      } else if (message.endsWith("?")) {
        State.updateTask(CODEX_ID, "question");
        State.appendStep(CODEX_ID, message);
      }
      break;
    }

    case "Stop":
      State.updateTask(CODEX_ID, "finished");
      if (payload.message) State.appendStep(CODEX_ID, payload.message.slice(0, 60));
      Sound.play("finish");
      if (focused) surface("finished", true);
      else State.setPillBadge(CODEX_ID, "finished");
      window.setTimeout(() => {
        if (revision !== activityRevision) return;
        State.updateTask(CODEX_ID, "idle");
        State.setPillBadge(CODEX_ID, null);
      }, 5200);
      break;

    case "StopFailure":
      State.updateTask(CODEX_ID, "error");
      Sound.play("error");
      if (focused) surface("error", true);
      else State.setPillBadge(CODEX_ID, "error");
      break;

    case "SessionEnd":
      State.updateTask(CODEX_ID, "idle");
      clearSession();
      break;

    case "SubagentStart":
      State.appendStep(CODEX_ID, "+ subagent");
      break;

    case "SubagentStop":
      State.appendStep(CODEX_ID, "• subagent done");
      break;

    case "PermissionRequest": {
      const requestId = payload.request_id ?? "";
      if (!requestId || !reviewablePermission(payload)) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      // One card, one request. A second one must never quietly replace the first
      // — that would leave a human staring at request B while request A waits for
      // a decision nobody can give. Hand it straight back to the terminal.
      if (State.pendingApproval && State.pendingApproval.requestId !== requestId) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      upsert(projectName, cwd);
      if (pendingTimeout != null) window.clearTimeout(pendingTimeout);
      const tool = payload.tool_name ?? "Tool";
      State.pendingApproval = {
        requestId,
        sessionId: payload.session_id ?? "",
        tool,
        command: approvalTarget(payload),
      };
      // The relay's short ack window closes in 800 ms; everything below this
      // line is synchronous, so the card really is up by the time it lands.
      if (requestId) void Bridge.approvalAck(requestId);
      State.updateTask(CODEX_ID, "approval");
      State.isPinned = true;
      Sound.play("approval");
      if (focused) {
        island.alert("approval");
      } else {
        // Another agent holds the view, so the card would yank it away. The badge
        // is the signal instead — but it has to be on screen for that to mean
        // anything, hence the reveal. We just told the relay a human can act.
        State.setPillBadge(CODEX_ID, "approval");
        island.reveal();
      }
      // Coucou answers within 108 s or not at all; after that the terminal has
      // taken over and the card would be lying.
      pendingTimeout = window.setTimeout(() => {
        pendingTimeout = null;
        if (!State.pendingApproval) return;
        State.pendingApproval = null;
        State.isPinned = false;
        island.dropPin();
        State.updateTask(CODEX_ID, "working");
        State.setPillBadge(CODEX_ID, null);
        if (State.view === "approval") island.setView(State.defaultView());
        State.notify();
      }, 110_000);
      break;
    }

    default:
      break;
  }
  State.notify();
}
