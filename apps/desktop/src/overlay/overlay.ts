import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { createSurrealScene, SurrealScene, SurrealState } from "./surreal";

interface EnginePayload {
    state?: string;
    reconnecting?: boolean;
    attempt?: number;
    from?: string;
    text?: string;
    mic?: number;
    model?: number;
}

const MIC_THRESHOLD = 0.08;
const HIDE_DELAY_MS = 4000;
const ERROR_HIDE_DELAY_MS = 6000;

function clamp01(value: number): number {
    if (Number.isNaN(value)) return 0;
    return Math.min(1, Math.max(0, value));
}

// "connecting" e o estado inicial (sem evento ainda) mapeiam para standby;
// os demais nomes já batem com os estados do engine.
function toSurrealState(state: string): SurrealState {
    if (state === "listening" || state === "speaking" || state === "muted" || state === "error") {
        return state;
    }
    return "standby";
}

class OverlayManager {
    private container: HTMLElement;
    private userTextEl: HTMLElement;
    private modelTextEl: HTMLElement;
    private modelLineEl: HTMLElement;
    private hideTimer: number | null = null;
    private currentState = "idle";
    private listeners: UnlistenFn[] = [];
    private scene: SurrealScene | null = null;

    constructor() {
        this.container = document.getElementById("app") || document.body;
        this.userTextEl = this.container.querySelector("#user-line .text") as HTMLElement;
        this.modelTextEl = this.container.querySelector("#model-line .text") as HTMLElement;
        this.modelLineEl = document.getElementById("model-line") as HTMLElement;
    }

    async initialize() {
        await this.initializeScene();
        this.listeners.push(
            await listen("engine://state", (event: any) => this.handleStateChange(event.payload))
        );
        this.listeners.push(
            await listen("engine://text", (event: any) => this.handleTextChange(event.payload))
        );
        this.listeners.push(
            await listen("engine://level", (event: any) => this.handleLevelChange(event.payload))
        );
        this.listeners.push(
            await listen("engine://error", (event: any) => this.handleError(event.payload))
        );
    }

    private async initializeScene() {
        let overlayStyle = "surreal";
        try {
            const settings = await invoke<{ overlay_style: string }>("get_settings");
            overlayStyle = settings.overlay_style;
        } catch (err) {
            console.error("[Overlay] Falha ao ler overlay_style, usando padrão", err);
        }

        const surrealWrap = document.getElementById("surreal-wrap") as HTMLElement;
        const orbWrap = document.getElementById("orb-wrap") as HTMLElement;

        if (overlayStyle === "orb") {
            orbWrap.hidden = false;
            surrealWrap.hidden = true;
            return;
        }

        orbWrap.hidden = true;
        surrealWrap.hidden = false;
        this.scene = createSurrealScene();
        this.scene.mount(surrealWrap);
    }

    private handleStateChange(payload: EnginePayload) {
        const state = payload.state;
        if (!state) return;

        console.log(`[Overlay] State: ${state}${payload.reconnecting ? " (reconectando)" : ""}`);

        this.currentState = state;
        this.updateStateClass(state);
        this.scene?.setState(toSurrealState(state));

        if (state === "speaking") {
            this.show();
        } else if (state === "error") {
            this.show();
            this.hideAfterDelay(ERROR_HIDE_DELAY_MS);
        } else if (payload.reconnecting) {
            this.setModelLine(`Reconectando… (tentativa ${payload.attempt ?? 1})`, "error");
            this.show();
        }
    }

    private handleTextChange(payload: EnginePayload) {
        if (payload.from === "user" && payload.text) {
            this.setLine(this.userTextEl, payload.text);
            this.show();
        } else if (payload.from === "model" && payload.text) {
            this.setModelLine(payload.text, "normal");
            this.show();
        } else if (payload.from === "turn_complete") {
            this.hideAfterDelay(HIDE_DELAY_MS);
        }
    }

    private handleLevelChange(payload: EnginePayload) {
        const mic = clamp01(payload.mic ?? 0);
        const model = clamp01(payload.model ?? 0);

        this.container.style.setProperty("--mic-level", String(mic));
        this.container.style.setProperty("--model-level", String(model));
        this.scene?.setLevel(mic, model);

        if (this.currentState === "listening" && mic > MIC_THRESHOLD) {
            this.show();
        }
    }

    private handleError(payload: any) {
        const message = typeof payload === "string" ? payload : payload?.message || "Erro desconhecido";
        console.error(`[Overlay] Error: ${message}`);

        this.setModelLine(message, "error");
        this.currentState = "error";
        this.updateStateClass("error");
        this.scene?.setState("error");
        this.show();
        this.hideAfterDelay(ERROR_HIDE_DELAY_MS);
    }

    private setLine(el: HTMLElement, text: string) {
        el.textContent = text;
        el.parentElement?.classList.add("has-text");
    }

    private setModelLine(text: string, tone: "normal" | "error") {
        this.setLine(this.modelTextEl, text);
        this.modelLineEl.classList.toggle("tone-error", tone === "error");
    }

    private show() {
        if (this.hideTimer !== null) {
            clearTimeout(this.hideTimer);
            this.hideTimer = null;
        }
        this.container.classList.add("visible");
    }

    private hide() {
        this.container.classList.remove("visible");
    }

    private hideAfterDelay(delayMs: number) {
        if (this.hideTimer !== null) {
            clearTimeout(this.hideTimer);
        }
        this.hideTimer = window.setTimeout(() => {
            this.hide();
            this.hideTimer = null;
        }, delayMs);
    }

    private updateStateClass(state: string) {
        this.container.classList.remove("listening", "speaking", "muted", "error", "connecting");
        this.container.classList.add(state);
    }

    async cleanup() {
        for (const unlisten of this.listeners) {
            await unlisten();
        }
        if (this.hideTimer !== null) {
            clearTimeout(this.hideTimer);
        }
        this.scene?.unmount();
    }
}

document.addEventListener("DOMContentLoaded", async () => {
    const overlay = new OverlayManager();
    await overlay.initialize();

    window.addEventListener("beforeunload", () => overlay.cleanup());
});
