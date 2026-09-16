// Janela de configurações (JRV-33): chave, voz, dispositivos, efeito,
// barge-in e system prompt. A chave nunca é lida de volta do backend em
// texto — só um contador de caracteres quando já existe uma configurada.
import { invoke } from "@tauri-apps/api/core";

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
}

interface DevicesPayload {
  input: string[];
  output: string[];
}

const $ = <T extends HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`elemento #${id} não encontrado`);
  return el as T;
};

const apiKeyInput = $<HTMLInputElement>("api-key");
const apiKeyHint = $<HTMLDivElement>("api-key-hint");
const noKeyBanner = $<HTMLDivElement>("no-key-banner");
const voiceSelect = $<HTMLSelectElement>("voice");
const deviceInSelect = $<HTMLSelectElement>("device-in");
const deviceOutSelect = $<HTMLSelectElement>("device-out");
const fxAmountInput = $<HTMLInputElement>("fx-amount");
const fxAmountValue = $<HTMLSpanElement>("fx-amount-value");
const bargeInCheckbox = $<HTMLInputElement>("barge-in");
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

async function load() {
  const [settings, devices] = await Promise.all([
    invoke<SettingsPayload>("get_settings"),
    invoke<DevicesPayload>("list_devices"),
  ]);

  defaultSystemPrompt = settings.default_system_prompt;

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
}

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
        api_key: apiKey.length > 0 ? apiKey : null,
        voice: voiceSelect.value,
        device_in: deviceInSelect.value.length > 0 ? deviceInSelect.value : null,
        device_out: deviceOutSelect.value.length > 0 ? deviceOutSelect.value : null,
        barge_in: bargeInCheckbox.checked,
        voice_fx_amount: Number(fxAmountInput.value),
        system_prompt: systemPromptTextarea.value,
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
