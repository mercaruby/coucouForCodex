export const MANAGE_USAGE_URL = "https://chatgpt.com/settings/usage";

export interface ChatNotice {
  kind: "limit" | "temporary" | "connection" | "model" | "other";
  title: string;
  detail: string;
  action: "usage" | "settings" | "chat";
}

/** Allowlisted recovery messages. Never render arbitrary transport/provider errors. */
export function chatNotice(error: unknown, backend: "chatgpt" | "api"): ChatNotice {
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : "";
  if (backend === "chatgpt") {
    if (/^(?:OpenAI reported a ChatGPT sharing limit for this app or account\.|This app's ChatGPT plan usage limit has been reached\.)/.test(message)) {
      return { kind: "limit", title: "Límite de uso de ChatGPT",
        detail: "OpenAI ha limitado el uso de esta conexión. Revisa el límite de Coucou en ChatGPT. Tu mensaje está guardado.", action: "usage" };
    }
    if (message.startsWith("OpenAI reported a usage or rate limit for this request.")) {
      return { kind: "temporary", title: "OpenAI ha limitado esta solicitud",
        detail: "Revisa el uso en ChatGPT o inténtalo más tarde. Tu mensaje está guardado.", action: "usage" };
    }
    if (/^(?:ChatGPT usage could not be checked right now\.|ChatGPT plan usage is unavailable)/.test(message)) {
      return { kind: "temporary", title: "No se pudo comprobar el uso",
        detail: "ChatGPT no puede comprobar tu cuota ahora. Conservamos la conexión y tu mensaje; inténtalo más tarde.", action: "chat" };
    }
    if (message.startsWith("Could not connect to ChatGPT.")) {
      return { kind: "temporary", title: "No se pudo conectar con ChatGPT",
        detail: "Comprueba la conexión e inténtalo más tarde. Tu mensaje está guardado.", action: "chat" };
    }
    if (/connect|sign in|authorization|permission|scope|not eligible|not_eligible|identity|revoked/i.test(message)) {
      return { kind: "connection", title: "Revisa la conexión con ChatGPT",
        detail: "Comprueba el consentimiento y la disponibilidad de tu cuenta en Ajustes. Tu mensaje está guardado.", action: "settings" };
    }
    if (/model|unsupported/i.test(message)) {
      return { kind: "model", title: "Este modelo no está disponible",
        detail: "Elige un modelo disponible para tu conexión en Ajustes. Tu mensaje está guardado.", action: "settings" };
    }
  }
  return { kind: "other", title: "No se pudo enviar el mensaje",
    detail: "Tu mensaje está guardado. Comprueba la conexión e inténtalo de nuevo más tarde.", action: "chat" };
}
