// Janela de configurações (JRV-33): chave, voz, dispositivos, efeito,
// barge-in e system prompt. A chave nunca é lida de volta do backend em
// texto — só um contador de caracteres quando já existe uma configurada.
//
// JRV-34: também mostra o erro que fez o backend abrir esta janela — chave
// inválida (401) no campo da chave, ou dispositivo ausente na lista certa.
// Chega de duas formas: `take_pending_error` no carregamento (janela recém
// criada) e o evento `engine://settings-error` (janela já aberta).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const VOICES = ["Puck", "Charon", "Kore", "Fenrir", "Aoede", "Leda", "Orus", "Zephyr"];

interface SettingsPayload {
  has_api_key: boolean;
  api_key_chars: number;
  voice: string;
  device_in: string | null;
  device_out: string | null;
  barge_in: boolean;
  voice_fx_amount: number;
  system_prompt: string;
  default_system_prompt: string;
  user_name: string;
  overlay_style: string;
}

interface DevicesPayload {
  input: string[];
  output: string[];
}

interface SettingsErrorPayload {
  kind: "invalid_key" | "no_input_device" | "no_output_device";
  message: string;
}

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`elemento #${id} não encontrado`);
  return el as T;
};

const userNameInput = $<HTMLInputElement>("user-name");
const apiKeyInput = $<HTMLInputElement>("api-key");
const apiKeyHint = $<HTMLDivElement>("api-key-hint");
const noKeyBanner = $<HTMLDivElement>("no-key-banner");
const voiceSelect = $<HTMLSelectElement>("voice");
const deviceInSelect = $<HTMLSelectElement>("device-in");
const deviceOutSelect = $<HTMLSelectElement>("device-out");
const fxAmountInput = $<HTMLInputElement>("fx-amount");
const fxAmountValue = $<HTMLSpanElement>("fx-amount-value");
const bargeInCheckbox = $<HTMLInputElement>("barge-in");
const overlayStyleSelect = $<HTMLSelectElement>("overlay-style");
const systemPromptTextarea = $<HTMLTextAreaElement>("system-prompt");
const restorePromptButton = $<HTMLButtonElement>("restore-prompt");
const saveButton = $<HTMLButtonElement>("save");
const statusEl = $<HTMLSpanElement>("status");

let defaultSystemPrompt = "";

function fillVoices(current: string) {
  voiceSelect.innerHTML = "";
  for (const voice of VOICES) {
    const option = document.createElement("option");
    option.value = voice;
    option.textContent = voice;
    voiceSelect.appendChild(option);
  }
  voiceSelect.value = current;
}

function fillDeviceSelect(select: HTMLSelectElement, devices: string[], current: string | null) {
  select.innerHTML = "";
  const auto = document.createElement("option");
  auto.value = "";
  auto.textContent = "Padrão do sistema";
  select.appendChild(auto);
  for (const device of devices) {
    const option = document.createElement("option");
    option.value = device;
    option.textContent = device;
    select.appendChild(option);
  }
  select.value = current ?? "";
}

function setStatus(text: string, kind: "" | "ok" | "error" = "") {
  statusEl.textContent = text;
  statusEl.className = `status ${kind}`.trim();
}

function applySettingsError(payload: SettingsErrorPayload) {
  if (payload.kind === "invalid_key") {
    apiKeyHint.textContent = payload.message;
    apiKeyHint.classList.add("error");
    apiKeyInput.classList.add("error");
    apiKeyInput.focus();
    return;
  }
  const select = payload.kind === "no_input_device" ? deviceInSelect : deviceOutSelect;
  select.classList.add("error");
  select.focus();
  select.scrollIntoView({ block: "center" });
  setStatus(payload.message, "error");
}

async function load() {
  const [settings, devices] = await Promise.all([
    invoke<SettingsPayload>("get_settings"),
    invoke<DevicesPayload>("list_devices"),
  ]);

  defaultSystemPrompt = settings.default_system_prompt;
  userNameInput.value = settings.user_name;

  noKeyBanner.classList.toggle("visible", !settings.has_api_key);
  apiKeyHint.textContent = settings.has_api_key
    ? `chave configurada (${settings.api_key_chars} caracteres)`
    : "nenhuma chave configurada";

  fillVoices(settings.voice);
  fillDeviceSelect(deviceInSelect, devices.input, settings.device_in);
  fillDeviceSelect(deviceOutSelect, devices.output, settings.device_out);

  fxAmountInput.value = String(settings.voice_fx_amount);
  fxAmountValue.textContent = settings.voice_fx_amount.toFixed(2);

  bargeInCheckbox.checked = settings.barge_in;
  systemPromptTextarea.value = settings.system_prompt;
  overlayStyleSelect.value = settings.overlay_style;

  const pendingError = await invoke<SettingsErrorPayload | null>("take_pending_error");
  if (pendingError) {
    applySettingsError(pendingError);
  }
}

listen<SettingsErrorPayload>("engine://settings-error", (event) => applySettingsError(event.payload));

apiKeyInput.addEventListener("input", () => {
  apiKeyHint.classList.remove("error");
  apiKeyInput.classList.remove("error");
});
deviceInSelect.addEventListener("change", () => deviceInSelect.classList.remove("error"));
deviceOutSelect.addEventListener("change", () => deviceOutSelect.classList.remove("error"));

fxAmountInput.addEventListener("input", () => {
  const amount = Number(fxAmountInput.value);
  fxAmountValue.textContent = amount.toFixed(2);
  // Aplica ao vivo na sessão em andamento; o valor final é persistido em Salvar.
  void invoke("set_fx_amount", { amount });
});

restorePromptButton.addEventListener("click", () => {
  systemPromptTextarea.value = defaultSystemPrompt;
});

saveButton.addEventListener("click", async () => {
  saveButton.disabled = true;
  setStatus("salvando…");
  try {
    const apiKey = apiKeyInput.value.trim();
    await invoke("save_settings", {
      payload: {
        user_name: userNameInput.value.trim(),
        api_key: apiKey.length > 0 ? apiKey : null,
        voice: voiceSelect.value,
        device_in: deviceInSelect.value.length > 0 ? deviceInSelect.value : null,
        device_out: deviceOutSelect.value.length > 0 ? deviceOutSelect.value : null,
        barge_in: bargeInCheckbox.checked,
        voice_fx_amount: Number(fxAmountInput.value),
        system_prompt: systemPromptTextarea.value,
        overlay_style: overlayStyleSelect.value,
      },
    });
    apiKeyInput.value = "";
    setStatus("salvo", "ok");
    await load();
  } catch (err) {
    setStatus(String(err), "error");
  } finally {
    saveButton.disabled = false;
  }
});

load().catch((err) => setStatus(String(err), "error"));
