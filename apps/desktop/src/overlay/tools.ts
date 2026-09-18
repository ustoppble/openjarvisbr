// Faixa de ferramentas do overlay (JRV-58), abaixo da cena: pedido de
// confirmação com Confirmar/Negar, "executando…" e o resumo do resultado.
//
// Contrato do evento `engine://tool` (spec v3 › engine.rs eventos):
// { kind: "requested" | "confirm_needed" | "result", id, name, summary, ok? }
//
// Contrato do evento `engine://reflex` (spec v4 › reflexo com Jev):
// { kind: "acted" | "confirmed", name?, summary?, latency_ms?, approve? }
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

export interface ToolEventPayload {
    kind: "requested" | "confirm_needed" | "result";
    id: string;
    name: string;
    summary: string;
    ok?: boolean;
}

export interface ReflexEventPayload {
    kind: "acted" | "confirmed";
    name?: string;
    summary?: string;
    latency_ms?: number;
    approve?: boolean;
}

export interface ToolStripHost {
    /** Mostra o overlay e cancela o auto-hide pendente. */
    show(): void;
    /** Faixa terminou: o overlay volta a poder se esconder sozinho. */
    release(): void;
}

type Mode = "idle" | "running" | "confirm" | "result";
type Tone = "" | "ok" | "fail" | "reflex";

const RESULT_VISIBLE_MS = 4000;
// Flash do reflexo: sem botões, some sozinho.
const REFLEX_VISIBLE_MS = 2500;
// O Engine nega sozinho em 20s; passou disso sem `result`, a faixa some.
const CONFIRM_STALE_MS = 25000;

export class ToolStrip {
    private mode: Mode = "idle";
    private currentId: string | null = null;
    private timer: number | null = null;
    private unlisten: UnlistenFn | null = null;
    private unlistenReflex: UnlistenFn | null = null;
    private readonly container: HTMLElement;
    private readonly strip: HTMLElement;
    private readonly textEl: HTMLElement;
    private readonly confirmButton: HTMLButtonElement;
    private readonly denyButton: HTMLButtonElement;

    constructor(container: HTMLElement, private readonly host: ToolStripHost) {
        this.container = container;
        this.strip = document.getElementById("tool-strip") as HTMLElement;
        this.textEl = this.strip.querySelector(".tool-text") as HTMLElement;
        this.confirmButton = this.strip.querySelector("#tool-confirm") as HTMLButtonElement;
        this.denyButton = this.strip.querySelector("#tool-deny") as HTMLButtonElement;
        this.confirmButton.addEventListener("click", () => void this.answer(true));
        this.denyButton.addEventListener("click", () => void this.answer(false));
    }

    async initialize() {
        this.unlisten = await listen<ToolEventPayload>("engine://tool", (event) => this.handle(event.payload));
        this.unlistenReflex = await listen<ReflexEventPayload>("engine://reflex", (event) =>
            this.handleReflex(event.payload)
        );
    }

    /** Enquanto há pedido, execução ou resultado na tela, o overlay não some. */
    holdsOverlay(): boolean {
        return this.mode !== "idle";
    }

    handle(payload: ToolEventPayload) {
        if (!payload || !payload.id) return;
        console.log(`[Overlay] Ferramenta ${payload.kind}: ${payload.name}`);

        switch (payload.kind) {
            case "requested":
                // Pedido já em confirmação para o mesmo id não volta a "executando".
                if (this.mode === "confirm" && this.currentId === payload.id) return;
                this.render(payload.id, "running", `Executando ${payload.summary}…`);
                break;
            case "confirm_needed":
                this.render(payload.id, "confirm", `Quer que eu execute ${payload.summary}?`);
                this.schedule(CONFIRM_STALE_MS);
                break;
            case "result": {
                const ok = payload.ok !== false;
                this.render(payload.id, "result", `${ok ? "✓" : "✕"} ${payload.summary}`, ok ? "ok" : "fail");
                this.schedule(RESULT_VISIBLE_MS);
                break;
            }
        }
    }

    /** Flash "⚡ reflexo": ação ou confirmação resolvida pelo Jev, sem botões. */
    handleReflex(payload: ReflexEventPayload) {
        if (!payload || !payload.kind) return;
        // Não atropela um pedido de confirmação que ainda espera o clique.
        if (this.mode === "confirm") return;
        console.log(`[Overlay] Reflexo ${payload.kind}: ${payload.summary ?? payload.name ?? ""}`);

        const text = payload.kind === "acted"
            ? `⚡ reflexo · ${payload.summary ?? payload.name ?? ""} · ${payload.latency_ms ?? 0} ms`
            : `⚡ reflexo · ${payload.approve ? "aprovado" : "negado"} por voz`;
        this.render(`reflex:${Date.now()}`, "result", text, "reflex");
        this.schedule(REFLEX_VISIBLE_MS);
    }

    private async answer(approve: boolean) {
        const id = this.currentId;
        if (this.mode !== "confirm" || id === null) return;
        this.render(id, "running", approve ? "executando…" : "cancelando…");
        try {
            await invoke("confirm_tool", { id, approve });
        } catch (err) {
            console.error("[Overlay] confirm_tool falhou", err);
            this.render(id, "result", "✕ não foi possível responder", "fail");
            this.schedule(RESULT_VISIBLE_MS);
        }
    }

    private render(id: string, mode: Exclude<Mode, "idle">, text: string, tone: Tone = "") {
        const wasIdle = this.mode === "idle";
        const wasInteractive = this.mode === "confirm";
        this.clearTimer();
        this.currentId = id;
        this.mode = mode;

        this.textEl.textContent = text;
        this.strip.dataset.mode = mode;
        this.strip.dataset.tone = tone;
        const interactive = mode === "confirm";
        this.confirmButton.hidden = !interactive;
        this.denyButton.hidden = !interactive;

        if (wasIdle || wasInteractive !== interactive) {
            void invoke("set_overlay_tool_strip", { visible: true, interactive }).catch((err) =>
                console.error("[Overlay] set_overlay_tool_strip falhou", err)
            );
        }
        this.container.classList.add("tool-active");
        this.host.show();
    }

    private clear() {
        this.clearTimer();
        this.mode = "idle";
        this.currentId = null;
        this.container.classList.remove("tool-active");
        void invoke("set_overlay_tool_strip", { visible: false, interactive: false }).catch((err) =>
            console.error("[Overlay] set_overlay_tool_strip falhou", err)
        );
        this.host.release();
    }

    private schedule(delayMs: number) {
        this.clearTimer();
        this.timer = window.setTimeout(() => {
            this.timer = null;
            this.clear();
        }, delayMs);
    }

    private clearTimer() {
        if (this.timer !== null) {
            clearTimeout(this.timer);
            this.timer = null;
        }
    }

    async cleanup() {
        this.clearTimer();
        if (this.unlisten) await this.unlisten();
        if (this.unlistenReflex) await this.unlistenReflex();
    }
}
