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
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { renderAlwaysAllow } from "./always-allow";

const VOICES = ["Puck", "Charon", "Kore", "Fenrir", "Aoede", "Leda", "Orus", "Zephyr"];

interface ProfileOption {
  id: string;
  name: string;
  description: string;
}

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
  profile: string;
  profiles: ProfileOption[];
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

const profileSelect = $<HTMLSelectElement>("profile");
const profileDescription = $<HTMLDivElement>("profile-description");
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
const toolsEnabledCheckbox = $<HTMLInputElement>("tools-enabled");
const fullAccessCheckbox = $<HTMLInputElement>("full-access");
const fullAccessHint = $<HTMLDivElement>("full-access-hint");
const mcpList = $<HTMLUListElement>("mcp-list");
const mcpEmpty = $<HTMLDivElement>("mcp-empty");
const saveButton = $<HTMLButtonElement>("save");
const statusEl = $<HTMLSpanElement>("status");

let defaultSystemPrompt = "";
let profiles: ProfileOption[] = [];

function fillProfiles(available: ProfileOption[], current: string) {
  profiles = available;
  profileSelect.innerHTML = "";
  for (const profile of available) {
    const option = document.createElement("option");
    option.value = profile.id;
    option.textContent = profile.name;
    profileSelect.appendChild(option);
  }
  profileSelect.value = current;
  updateProfileDescription();
}

function updateProfileDescription() {
  const active = profiles.find((p) => p.id === profileSelect.value);
  profileDescription.textContent = active?.description ?? "";
}

// Aba Ferramentas (JRV-58): toggle geral (salvo com o resto do formulário),
// permissões do macOS e servidores MCP. Tokens nunca voltam do backend: a
// lista só diz de onde o token vem (env ou Keychain).
type Grant = "concedida" | "pendente" | "negada" | "nao_instalado" | "erro";

const GRANT_LABEL: Record<Grant, string> = {
  concedida: "concedida",
  pendente: "pendente",
  negada: "negada",
  nao_instalado: "não instalado",
  erro: "erro",
};

interface AutomationTarget {
  id: string;
  label: string;
  status: Grant;
}

interface PermissionsPayload {
  macos: boolean;
  accessibility: Grant;
  automation: AutomationTarget[] | null;
  app_path: string;
  is_bundle: boolean;
  icon_data_url: string;
}

interface McpServerInfo {
  name: string;
  target: string;
  transport: "http" | "stdio";
  bearer_env: string | null;
  bearer_env_present: boolean;
  token_keychain: boolean;
}

interface ToolsSettingsPayload {
  enabled: boolean;
  full_access: boolean;
  mcp_servers: McpServerInfo[];
  overclock_env_present: boolean;
}

interface McpTestResult {
  ok: boolean;
  message: string;
}

const accessibilityStatus = $<HTMLSpanElement>("accessibility-status");
const dragIcon = $<HTMLImageElement>("drag-icon");
const appPathHint = $<HTMLSpanElement>("app-path-hint");
const automationList = $<HTMLUListElement>("automation-list");
const automationEmpty = $<HTMLDivElement>("automation-empty");
const requestPermissionsButton = $<HTMLButtonElement>("request-permissions");
const mcpForm = $<HTMLDivElement>("mcp-form");
const mcpName = $<HTMLInputElement>("mcp-name");
const mcpTarget = $<HTMLInputElement>("mcp-target");
const mcpToken = $<HTMLInputElement>("mcp-token");
const mcpTokenHint = $<HTMLDivElement>("mcp-token-hint");
const mcpBearerEnv = $<HTMLInputElement>("mcp-bearer-env");
const mcpAdvanced = $<HTMLDetailsElement>("mcp-advanced");

const ACCESSIBILITY_POLL_MS = 3000;
let accessibilityTimer: number | null = null;
let appPath = "";
let iconDataUrl = "";
let overclockEnvPresent = false;

function setBadge(el: HTMLElement, text: string, tone: string) {
  el.textContent = text;
  el.className = `badge ${tone}`.trim();
}

function setGrant(el: HTMLElement, grant: Grant) {
  setBadge(el, GRANT_LABEL[grant], grant);
}

async function refreshAccessibility() {
  setGrant(accessibilityStatus, await invoke<Grant>("check_accessibility"));
}

// Checa Acessibilidade a cada 3s só enquanto a aba está aberta.
function setAccessibilityPolling(active: boolean) {
  if (accessibilityTimer !== null) {
    clearInterval(accessibilityTimer);
    accessibilityTimer = null;
  }
  if (!active) return;
  void refreshAccessibility();
  accessibilityTimer = window.setInterval(() => void refreshAccessibility(), ACCESSIBILITY_POLL_MS);
}

function selectTab(tab: string) {
  document.body.classList.toggle("tab-tools", tab === "tools");
  document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((button) => {
    button.classList.toggle("active", button.dataset.tab === tab);
  });
  setAccessibilityPolling(tab === "tools");
}

document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((button) => {
  button.addEventListener("click", () => selectTab(button.dataset.tab ?? "general"));
});

function renderAutomation(targets: AutomationTarget[]) {
  automationEmpty.hidden = targets.length > 0;
  for (const target of targets) {
    let item = automationList.querySelector<HTMLLIElement>(`li[data-id="${CSS.escape(target.id)}"]`);
    if (!item) {
      item = document.createElement("li");
      item.dataset.id = target.id;
      const label = document.createElement("span");
      label.textContent = target.label;
      const badge = document.createElement("span");
      badge.className = "badge";
      item.append(label, badge);
      automationList.appendChild(item);
    }
    setGrant(item.querySelector(".badge") as HTMLElement, target.status);
  }
}

async function loadPermissions() {
  const permissions = await invoke<PermissionsPayload>("get_permissions");
  $<HTMLDivElement>("permissions-section").hidden = !permissions.macos;
  setGrant(accessibilityStatus, permissions.accessibility);
  appPath = permissions.app_path;
  iconDataUrl = permissions.icon_data_url;
  dragIcon.src = iconDataUrl;
  appPathHint.textContent = permissions.is_bundle
    ? ""
    : "(modo desenvolvimento: arrasta o binário do app)";
  if (permissions.automation) renderAutomation(permissions.automation);
}

listen<AutomationTarget>("tools://automation", (event) => renderAutomation([event.payload]));

// Drag-out nativo do bundle do app em execução (tauri-plugin-drag).
dragIcon.addEventListener("mousedown", (event) => {
  if (event.button !== 0 || !appPath) return;
  event.preventDefault();
  startDrag({ item: [appPath], icon: iconDataUrl }).catch((err) => setStatus(String(err), "error"));
});

requestPermissionsButton.addEventListener("click", async () => {
  requestPermissionsButton.disabled = true;
  try {
    renderAutomation(await invoke<AutomationTarget[]>("request_permissions"));
    await refreshAccessibility();
  } catch (err) {
    setStatus(String(err), "error");
  } finally {
    requestPermissionsButton.disabled = false;
  }
});

$<HTMLButtonElement>("request-accessibility").addEventListener("click", async () => {
  setGrant(accessibilityStatus, await invoke<Grant>("request_accessibility"));
});

$<HTMLButtonElement>("reveal-app").addEventListener("click", () => {
  invoke("reveal_app").catch((err) => setStatus(String(err), "error"));
});

for (const [id, pane] of [
  ["open-automation", "automation"],
  ["open-accessibility", "accessibility"],
] as const) {
  $<HTMLButtonElement>(id).addEventListener("click", () => {
    invoke("open_privacy_pane", { pane }).catch((err) => setStatus(String(err), "error"));
  });
}

function describeAuth(server: McpServerInfo): string {
  if (server.bearer_env) {
    return `token: env ${server.bearer_env} (${server.bearer_env_present ? "presente" : "ausente"})`;
  }
  return server.token_keychain ? "token: no Keychain" : "sem token";
}

function renderMcpServers(servers: McpServerInfo[]) {
  mcpList.innerHTML = "";
  mcpEmpty.hidden = servers.length > 0;
  for (const server of servers) {
    const item = document.createElement("li");
    item.className = "mcp-item";

    const head = document.createElement("div");
    head.className = "card-head";
    const name = document.createElement("span");
    name.className = "mcp-name";
    name.textContent = server.name;
    const badge = document.createElement("span");
    setBadge(badge, "desconhecido", "");
    head.append(name, badge);

    const target = document.createElement("span");
    target.className = "mcp-meta";
    target.textContent = `${server.transport === "http" ? "URL" : "comando"}: ${server.target}`;
    const auth = document.createElement("span");
    auth.className = "mcp-meta";
    auth.textContent = describeAuth(server);

    const buttons = document.createElement("div");
    buttons.className = "button-row";
    const test = document.createElement("button");
    test.type = "button";
    test.textContent = "Testar";
    test.addEventListener("click", async () => {
      test.disabled = true;
      setBadge(badge, "testando…", "");
      try {
        const result = await invoke<McpTestResult>("test_mcp_server", { name: server.name });
        setBadge(badge, result.ok ? `ok · ${result.message}` : `erro · ${result.message}`, result.ok ? "ok" : "fail");
      } catch (err) {
        setBadge(badge, `erro · ${String(err)}`, "fail");
      } finally {
        test.disabled = false;
      }
    });
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remover";
    remove.addEventListener("click", async () => {
      remove.disabled = true;
      try {
        await invoke("remove_mcp_server", { name: server.name });
        await loadTools();
      } catch (err) {
        setStatus(String(err), "error");
        remove.disabled = false;
      }
    });
    buttons.append(test, remove);

    item.append(head, target, auth, buttons);
    mcpList.appendChild(item);
  }
}

function openMcpForm(preset: { name?: string; target?: string; bearerEnv?: string; hint?: string }) {
  mcpForm.classList.add("open");
  mcpName.value = preset.name ?? "";
  mcpTarget.value = preset.target ?? "";
  mcpToken.value = "";
  mcpBearerEnv.value = preset.bearerEnv ?? "";
  mcpAdvanced.open = Boolean(preset.bearerEnv);
  mcpTokenHint.innerHTML = "";
  if (preset.hint) mcpTokenHint.textContent = preset.hint;
  (preset.name ? (preset.bearerEnv ? mcpName : mcpToken) : mcpName).focus();
}

$<HTMLButtonElement>("mcp-add").addEventListener("click", () => openMcpForm({}));

$<HTMLButtonElement>("mcp-connect-overclock").addEventListener("click", () => {
  openMcpForm({
    name: "overclock",
    target: "http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp",
    bearerEnv: "OVERCLOCK_MCP_BEARER_TOKEN",
    hint: overclockEnvPresent
      ? "env OVERCLOCK_MCP_BEARER_TOKEN encontrada neste processo — não precisa de token."
      : "env OVERCLOCK_MCP_BEARER_TOKEN ausente: abra o Jarvis de dentro do Overclock ou cole um token.",
  });
});

$<HTMLButtonElement>("mcp-connect-overclick").addEventListener("click", () => {
  openMcpForm({ name: "overclick", target: "https://cloud.overclock.sh/mcp" });
  mcpTokenHint.textContent = "Cole o token de API do OverClick. ";
  const link = document.createElement("a");
  link.className = "inline-link";
  link.textContent = "Onde pegar o token";
  link.addEventListener("click", () => {
    invoke("open_tools_link", { link: "overclick_tokens" }).catch((err) => setStatus(String(err), "error"));
  });
  mcpTokenHint.appendChild(link);
});

$<HTMLButtonElement>("mcp-cancel").addEventListener("click", () => {
  mcpToken.value = "";
  mcpForm.classList.remove("open");
});

$<HTMLButtonElement>("mcp-save").addEventListener("click", async () => {
  const saveServer = $<HTMLButtonElement>("mcp-save");
  saveServer.disabled = true;
  try {
    const token = mcpToken.value.trim();
    const bearerEnv = mcpBearerEnv.value.trim();
    await invoke("add_mcp_server", {
      server: {
        name: mcpName.value.trim(),
        target: mcpTarget.value.trim(),
        token: token.length > 0 ? token : null,
        bearer_env: bearerEnv.length > 0 ? bearerEnv : null,
      },
    });
    mcpToken.value = "";
    mcpForm.classList.remove("open");
    setStatus("servidor salvo", "ok");
    await loadTools();
  } catch (err) {
    setStatus(String(err), "error");
  } finally {
    saveServer.disabled = false;
  }
});

async function loadTools() {
  const tools = await invoke<ToolsSettingsPayload>("get_tools_settings");
  toolsEnabledCheckbox.checked = tools.enabled;
  setFullAccessChecked(tools.full_access);
  overclockEnvPresent = tools.overclock_env_present;
  renderMcpServers(tools.mcp_servers);
  await renderAlwaysAllow(fullAccessHint.parentElement ?? fullAccessHint, (message) => setStatus(message, "error"));
}

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
  fillProfiles(settings.profiles, settings.profile);

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
profileSelect.addEventListener("change", updateProfileDescription);
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
        profile: profileSelect.value,
        user_name: userNameInput.value.trim(),
        api_key: apiKey.length > 0 ? apiKey : null,
        voice: voiceSelect.value,
        device_in: deviceInSelect.value.length > 0 ? deviceInSelect.value : null,
        device_out: deviceOutSelect.value.length > 0 ? deviceOutSelect.value : null,
        barge_in: bargeInCheckbox.checked,
        voice_fx_amount: Number(fxAmountInput.value),
        system_prompt: systemPromptTextarea.value,
        overlay_style: overlayStyleSelect.value,
        tools_enabled: toolsEnabledCheckbox.checked,
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

// Acesso total (JRV-65): não espera o Salvar — grava e aplica no motor na hora.
function setFullAccessChecked(on: boolean) {
  fullAccessCheckbox.checked = on;
  fullAccessHint.classList.toggle("error", on);
}

fullAccessCheckbox.addEventListener("change", async () => {
  const on = fullAccessCheckbox.checked;
  fullAccessCheckbox.disabled = true;
  try {
    await invoke("set_full_access", { on });
    setFullAccessChecked(on);
    setStatus(on ? "acesso total ligado" : "acesso total desligado", "ok");
  } catch (err) {
    setFullAccessChecked(!on);
    setStatus(String(err), "error");
  } finally {
    fullAccessCheckbox.disabled = false;
  }
});

void listen<{ on: boolean }>("engine://full_access", (event) => setFullAccessChecked(event.payload.on));

load().catch((err) => setStatus(String(err), "error"));
loadTools().catch((err) => setStatus(String(err), "error"));
loadPermissions().catch((err) => setStatus(String(err), "error"));
