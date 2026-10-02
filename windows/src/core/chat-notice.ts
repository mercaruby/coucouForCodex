export const MANAGE_USAGE_URL = "https://chatgpt.com/settings/usage";

export interface ChatNotice {
  kind: "limit" | "temporary" | "restriction" | "connection" | "model" | "other";
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
        detail: "OpenAI ha rechazado el uso de este modelo por un límite. Revisa el uso en ChatGPT; si sólo falla un modelo, elige otro en Ajustes. Tu mensaje está guardado.", action: "usage" };
    }
    if (/^OpenAI reported a (?:usage or )?rate limit for this request\./.test(message)) {
      return { kind: "temporary", title: "OpenAI ha limitado esta solicitud",
        detail: "Revisa el uso en ChatGPT o inténtalo más tarde. Tu mensaje está guardado.", action: "usage" };
    }
    if (message.startsWith("ChatGPT usage could not be checked right now.")) {
      return { kind: "temporary", title: "No se pudo comprobar el uso",
        detail: "ChatGPT no puede comprobar tu cuota ahora. Conservamos la conexión y tu mensaje; inténtalo más tarde.", action: "chat" };
    }
    if (/^(?:Your ChatGPT account or workspace is temporarily unavailable\.|ChatGPT is temporarily unavailable\.)/.test(message)) {
      return { kind: "temporary", title: "ChatGPT no está disponible temporalmente",
        detail: "Conservamos tu conexión y tu mensaje. Inténtalo de nuevo más tarde.", action: "chat" };
    }
    if (message.startsWith("Could not connect to ChatGPT.")) {
      return { kind: "temporary", title: "No se pudo conectar con ChatGPT",
        detail: "Comprueba la conexión e inténtalo más tarde. Tu mensaje está guardado.", action: "chat" };
    }
    if (/^(?:ChatGPT plan usage is unavailable for this account, workspace or policy\.|A ChatGPT policy or permission restriction prevented this request\.)/.test(message)) {
      return { kind: "restriction", title: "Uso del plan restringido por OpenAI",
        detail: "La cuenta, el espacio de trabajo o sus permisos impiden esta solicitud. Revisa su disponibilidad en Ajustes. Tu mensaje está guardado.", action: "settings" };
    }
    if (/^(?:ChatGPT does not support this request route\.|This app is not enabled for ChatGPT plan usage\.|ChatGPT rejected this app registration\.)/.test(message)) {
      return { kind: "restriction", title: "Revisa la integración de Coucou",
        detail: "OpenAI ha rechazado la configuración de la aplicación. Este error requiere revisar la integración. Tu mensaje está guardado.", action: "chat" };
    }
    if (/^(?:ChatGPT could not validate the selected account context\.|Your connection does not allow this ChatGPT plan request\.)/.test(message)) {
      return { kind: "connection", title: "Revisa la conexión con ChatGPT",
        detail: "OpenAI no ha aceptado el contexto o los permisos de esta conexión. Comprueba tu cuenta en Ajustes. Tu mensaje está guardado.", action: "settings" };
    }
    if (message.startsWith("This model or message uses a capability unsupported by ChatGPT plan usage.")) {
      return { kind: "model", title: "Modelo o adjunto no compatible",
        detail: "Revisa el modelo y el adjunto elegidos antes de volver a enviar. Tu mensaje está guardado.", action: "settings" };
    }
    if (message.startsWith("ChatGPT rejected this request.")) {
      return { kind: "model", title: "Revisa el mensaje, el modelo y el adjunto",
        detail: "OpenAI no ha aceptado esta solicitud. Revisa lo que envías antes de intentarlo de nuevo. Tu mensaje está guardado.", action: "settings" };
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
