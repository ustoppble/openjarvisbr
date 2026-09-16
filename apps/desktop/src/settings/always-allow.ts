// Lista "Sempre permitido" da aba Ferramentas (JRV-66): ferramentas que o
// usuário liberou por voz ("sempre pode", "não pergunta mais") e ficaram em
// `[tools].always_allow`. Remover grava o config e vale na hora no motor.
import { invoke } from "@tauri-apps/api/core";

const SECTION_ID = "always-allow-section";

/// Nomes amigáveis das ferramentas locais; as demais (MCP) aparecem pelo nome.
const LABELS: Record<string, string> = {
  "shell.run": "Rodar comandos no terminal",
  "fs.write": "Escrever arquivos",
  "calendar.create": "Criar eventos na agenda",
  "reminder.set": "Criar lembretes",
};

function ensureSection(anchor: HTMLElement): { list: HTMLUListElement; empty: HTMLDivElement } {
  let section = document.getElementById(SECTION_ID);
  if (!section) {
    section = document.createElement("div");
    section.className = "field";
    section.id = SECTION_ID;

    const title = document.createElement("p");
    title.className = "section-title";
    title.textContent = "Sempre permitido";
    const list = document.createElement("ul");
    list.className = "mcp-list";
    const empty = document.createElement("div");
    empty.className = "field-hint";
    empty.textContent = "Nada liberado. Diga \"sempre pode\" quando o Jarvis pedir confirmação.";

    section.append(title, list, empty);
    anchor.insertAdjacentElement("afterend", section);
  }
  return {
    list: section.querySelector("ul") as HTMLUListElement,
    empty: section.querySelector(".field-hint") as HTMLDivElement,
  };
}

/// Mostra a lista logo depois de `anchor`; `onError` recebe falhas ao remover.
export async function renderAlwaysAllow(anchor: HTMLElement, onError: (message: string) => void) {
  const { list, empty } = ensureSection(anchor);
  const names = await invoke<string[]>("get_always_allow");
  list.innerHTML = "";
  empty.hidden = names.length > 0;
  for (const name of names) {
    const item = document.createElement("li");
    item.className = "mcp-item";

    const head = document.createElement("div");
    head.className = "card-head";
    const label = document.createElement("span");
    label.className = "mcp-name";
    label.textContent = LABELS[name] ?? name;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Remover";
    remove.addEventListener("click", async () => {
      remove.disabled = true;
      try {
        await invoke("remove_always_allow", { name });
        await renderAlwaysAllow(anchor, onError);
      } catch (err) {
        onError(String(err));
        remove.disabled = false;
      }
    });
    head.append(label, remove);

    const meta = document.createElement("span");
    meta.className = "mcp-meta";
    meta.textContent = name;

    item.append(head, meta);
    list.appendChild(item);
  }
}
