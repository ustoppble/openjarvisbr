import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface EngineState {
    state?: string;
    from?: string;
    text?: string;
    mic?: number;
    model?: number;
}

class OverlayManager {
    private container: HTMLElement;
    private hideTimer: number | null = null;
    private currentState: string = "idle";
    private listeners: UnlistenFn[] = [];

    constructor() {
        this.container = document.getElementById("app") || document.body;
    }

    async initialize() {
        console.log("[Overlay] Inicializando event listeners...");

        // Listen for engine state changes
        this.listeners.push(
            await listen("engine://state", (event: any) => this.handleStateChange(event.payload))
        );

        // Listen for text events (user/model/turn_complete)
        this.listeners.push(
            await listen("engine://text", (event: any) => this.handleTextChange(event.payload))
        );

        // Listen for audio levels
        this.listeners.push(
            await listen("engine://level", (event: any) => this.handleLevelChange(event.payload))
        );

        // Listen for errors
        this.listeners.push(
            await listen("engine://error", (event: any) => this.handleError(event.payload))
        );

        console.log("[Overlay] Event listeners registrados");
    }

    private handleStateChange(payload: EngineState) {
        const state = payload.state;
        const reconnecting = (payload as any).reconnecting;

        if (!state) return;

        console.log(`[Overlay] State: ${state}${reconnecting ? " (reconectando)" : ""}`);

        if (state === "listening" || state === "speaking") {
            this.currentState = state;
            this.show();
        } else if (state === "muted") {
            this.currentState = "muted";
            this.show();
        } else if (state === "error" || state === "connecting") {
            this.currentState = state;
            this.show();
        }

        this.updateStateClass(state);
    }

    private handleTextChange(payload: EngineState) {
        const from = payload.from;
        const text = payload.text;

        if (from === "user" && text) {
            console.log(`[Overlay] User: ${text}`);
            this.show();
        } else if (from === "model" && text) {
            console.log(`[Overlay] Model: ${text}`);
            this.show();
        } else if (from === "turn_complete") {
            console.log("[Overlay] TurnComplete — escondendo em 4s");
            this.hideAfterDelay(4000);
        }
    }

    private handleLevelChange(payload: EngineState) {
        const mic = payload.mic || 0;
        const model = payload.model || 0;

        console.log(`[Overlay] Levels: mic=${mic.toFixed(2)}, model=${model.toFixed(2)}`);

        // Dispatch custom event for UI to consume
        const event = new CustomEvent("overlay:levels", {
            detail: { mic, model },
        });
        this.container.dispatchEvent(event);
    }

    private handleError(payload: any) {
        const message = typeof payload === "string" ? payload : payload.message || "Unknown error";
        console.error(`[Overlay] Error: ${message}`);

        this.currentState = "error";
        this.updateStateClass("error");
        this.show();
        this.hideAfterDelay(6000);
    }

    private show() {
        // Clear any pending hide
        if (this.hideTimer !== null) {
            clearTimeout(this.hideTimer);
            this.hideTimer = null;
        }

        this.container.classList.add("visible");
        console.log("[Overlay] Visível");
    }

    private hide() {
        this.container.classList.remove("visible");
        console.log("[Overlay] Oculto");
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
        console.log(`[Overlay] Classe CSS atualizada: ${state}`);
    }

    async cleanup() {
        console.log("[Overlay] Limpando listeners...");
        for (const unlisten of this.listeners) {
            await unlisten();
        }
        if (this.hideTimer !== null) {
            clearTimeout(this.hideTimer);
        }
    }
}

// Initialize on page load
document.addEventListener("DOMContentLoaded", async () => {
    console.log("[Overlay] DOM pronto, inicializando...");

    const overlay = new OverlayManager();
    await overlay.initialize();

    // Cleanup on unload
    window.addEventListener("beforeunload", () => overlay.cleanup());
});

console.log("[Overlay] Script carregado");
