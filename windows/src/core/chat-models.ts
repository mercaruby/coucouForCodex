import { Bridge, type ChatGPTModel } from "./bridge";
import { State } from "./state";
import { Usage } from "./usage";

function sessionFlags(): string {
  return Usage.session ? `${Usage.session.connected}/${Usage.session.sharing}` : "unknown";
}

/** Public account metadata only. Each session/backend change invalidates pending reads. */
export class ChatModelCatalog {
  models: ChatGPTModel[] = [];
  loading = false;
  loginBusy = false;
  connected = false;
  sharing = false;
  error: string | null = null;
  private attempted = false;
  private revision = 0;
  private backend = State.settings.chatBackend;
  private generation = Usage.sessionGeneration;
  private flags = sessionFlags();

  constructor(private readonly changed: () => void) {}

  observe(): boolean {
    const flags = sessionFlags();
    if (this.backend === State.settings.chatBackend && this.generation === Usage.sessionGeneration && this.flags === flags) return false;
    this.backend = State.settings.chatBackend;
    this.generation = Usage.sessionGeneration;
    this.flags = flags;
    this.invalidate();
    return true;
  }

  private invalidate() {
    this.revision++;
    this.models = [];
    this.loading = false;
    this.connected = false;
    this.sharing = false;
    this.error = null;
    this.attempted = false;
  }

  setLoginBusy(busy: boolean) {
    this.loginBusy = busy;
    this.invalidate();
    this.changed();
  }

  ensure(visible: boolean) {
    this.observe();
    if (!visible || this.backend !== "chatgpt" || this.loginBusy || this.attempted) return;
    void this.load();
  }

  retry() {
    if (this.loading || this.loginBusy) return;
    this.attempted = false;
    this.ensure(true);
  }

  get available(): boolean {
    return !this.loginBusy && !this.loading && this.connected && this.sharing && this.models.length > 0;
  }

  get automatic(): ChatGPTModel | undefined {
    return this.models.find((model) => model.slug === "gpt-5.6-luna") ?? this.models[0];
  }

  private async load() {
    const ownRevision = ++this.revision;
    const valid = () => {
      this.observe();
      return ownRevision === this.revision && State.settings.chatBackend === "chatgpt" && !this.loginBusy;
    };
    this.attempted = true;
    this.loading = true;
    this.error = null;
    this.changed();
    try {
      const session = await Bridge.chatgptSession();
      if (!valid()) return;
      this.generation = session.generation;
      this.flags = `${session.connected}/${session.sharing}`;
      this.connected = session.connected;
      this.sharing = session.sharing;
      Usage.updateSession(session);
      if (!valid()) return;
      if (!session.connected || !session.sharing) return;
      const models = await Bridge.chatgptModels();
      if (!valid()) return;
      // Native validation provides the catalog; do not create choices from free text.
      this.models = models;
      if (models.length === 0) this.error = "No hay modelos disponibles para esta conexión.";
    } catch {
      if (!valid()) return;
      this.error = "No se pudieron cargar los modelos. Revisa la conexión en Ajustes.";
    } finally {
      if (valid()) {
        this.loading = false;
        this.changed();
      }
    }
  }
}
