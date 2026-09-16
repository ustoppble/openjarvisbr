# Compilar o OpenJarvisBR no Windows

O app (`apps/desktop`) é um workspace Cargo + Tauri 2, então o mesmo código-fonte
compila em Mac e Windows sem mudança nenhuma — só falta instalar as ferramentas certas
e rodar `npm run tauri build`.

> Este documento não foi executado numa máquina Windows real nesta entrega — não há
> Windows/VM acessível a partir deste worker. **O build no Windows segue sem
> verificação.** Antes de considerar o degrau 2 pronto, alguém precisa rodar os passos
> abaixo numa máquina Windows de verdade e registrar o resultado (ou os ajustes que
> precisou fazer) neste arquivo.

## Pré-requisitos

1. **Rust** (via [rustup](https://rustup.rs)) — instala `cargo`, `rustc` e o toolchain
   `stable-x86_64-pc-windows-msvc`. Durante a instalação do rustup, aceite instalar
   também o **Build Tools for Visual Studio** quando ele pedir (precisa do linker MSVC
   e do Windows SDK — sem isso o `cargo build` falha no link).
2. **Node.js** LTS (18 ou mais recente) — https://nodejs.org. Traz o `npm`.
3. **WebView2 Runtime** — normalmente já vem instalado no Windows 10/11 atualizado
   (é o motor que a Microsoft usa no Edge/apps do sistema). Se o `npm run tauri build`
   reclamar de WebView2 ausente, baixe o "Evergreen Bootstrapper" em
   https://developer.microsoft.com/microsoft-edge/webview2/ e instale antes de tentar
   de novo.
4. **Tauri CLI** — não precisa instalar à parte; já está em
   `apps/desktop/package.json` como dependência de desenvolvimento e sobe com
   `npm install`.

## Passo a passo

Num PowerShell ou Prompt de Comando, na raiz do repositório:

```powershell
git clone https://github.com/ustoppble/openjarvisbr
cd openjarvisbr\apps\desktop
npm install
npm run tauri build
```

O build gera o instalador em
`openjarvisbr\target\release\bundle\` — no Windows, tipicamente um `.msi` (WiX) e/ou
um `.exe` (NSIS), dependendo do que o `tauri.conf.json` tiver habilitado em
`bundle.targets` (hoje está `"all"`, então os dois devem sair).

Depois de instalar, o app abre na bandeja do sistema (system tray), do mesmo jeito
que no Mac.

## O que é específico do macOS no código atual

Nada disso *impede* o build no Windows — a Tauri já resolve essas diferenças por
baixo dos panos — mas são os pontos que merecem um teste manual extra na primeira
rodada no Windows:

| Onde | O que é macOS-específico | O que verificar no Windows |
|---|---|---|
| `apps/desktop/src-tauri/src/main.rs:24` (`MUTE_SHORTCUT`) | `"CmdOrCtrl+Shift+J"` | A Tauri resolve `CmdOrCtrl` para `Ctrl` no Windows automaticamente — mas confirme que `Ctrl+Shift+J` não colide com um atalho já usado por outro app aberto na hora do teste. |
| `apps/desktop/src-tauri/src/main.rs:172-201` (`open_overlay_window`) | janela sem borda, transparente, sempre no topo, com `set_ignore_cursor_events(true)` | No Windows, janelas "always on top" e sem clique-através podem se comportar de forma sutilmente diferente (ex.: aparecer atrás da barra de tarefas em vez do topo, ou capturar clique mesmo com `ignore_cursor_events`). Confirme visualmente que o overlay aparece por cima de tudo e não rouba foco nem bloqueia cliques no que está atrás dele. |
| `apps/desktop/src-tauri/src/main.rs:150-170` (`overlay_position`) | cálculo de "monitor sob o cursor" via `cursor_position`/`monitor_from_point` | Testar com mais de um monitor conectado: confirme que o overlay abre no monitor onde o mouse está, não sempre no principal. |
| `apps/desktop/src-tauri/src/main.rs:5` (`windows_subsystem = "windows"`) | atributo que, em build de release no Windows, esconde a janela de console atrás do app | Confirme que, ao abrir o `.exe`/instalado, nenhuma janela de terminal preta aparece por trás. |
| `apps/desktop/src-tauri/Cargo.toml` (feature `macos-private-api`) | flag específica de macOS na dependência `tauri` | É ignorada em builds não-macOS pela própria Tauri; não deve gerar erro nem aviso no Windows. |
| Tray/menu (`TrayIconBuilder`, itens Mutar/Reconectar/Configurações/Sair) | ícones em `apps/desktop/icons/*.png` e `apps/desktop/src-tauri/icons/icon.ico` | O `.ico` já existe no repo para o bundle do Windows — confirme que o ícone aparece corretamente na bandeja (ícones de estado: conectando, ouvindo, falando, mudo, erro). |
| Áudio (`crates/core/src/audio`, via `cpal`) | a biblioteca `cpal` já abstrai o backend por plataforma (CoreAudio no Mac, WASAPI no Windows) — não há código específico de Mac aqui | Testar microfone e saída de verdade no Windows: listar dispositivos nas Configurações e confirmar que a conversa funciona com áudio de entrada e saída reais. |

## Registro de execução

_(preencher depois de rodar numa máquina Windows real)_

- Data:
- Máquina/VM usada:
- Versão do Windows:
- `npm run tauri build` terminou sem erro? —
- Ajustes que precisaram ser feitos:
- Resultado do roteiro `docs/testes/v2.md` no Windows:
