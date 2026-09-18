# OpenJarvisBR

Assistente de voz em Rust, que conversa com você em tempo real usando o
**Gemini 3.8 Live** do Google. Roda no terminal do Mac hoje e no Windows em breve.

> Status: **v4** — conversa, app de desktop, perfis, ferramentas (locais, Overclock e
> OverClick) com confirmação por voz e **reflexo** (ações seguras em ~300 ms com o Jev).

## Baixar

Os instaladores ficam na [página de releases](https://github.com/ustoppble/openjarvisbr/releases).

- **Windows (x64):** baixe o `.msi` ou o `-setup.exe` da última release. Eles são gerados
  pelo GitHub Actions (`.github/workflows/release.yml`) a cada tag `v*`. O instalador não
  é assinado: o SmartScreen pode avisar — clique em **Mais informações › Executar assim
  mesmo**.
- **Mac:** build local (`npm run tauri build`, ver abaixo); o `.dmg` é enviado à mão para
  a mesma release. O app não é assinado nem notarizado: na primeira vez, **botão direito
  no app › Abrir** (ou autorize em Ajustes › Privacidade e Segurança).

Para anexar o `.dmg` local a uma release que a Action já criou:

```sh
gh release upload v0.2.0 target/release/bundle/dmg/OpenJarvisBR_<versão>_aarch64.dmg
```

O CI (`.github/workflows/ci.yml`) roda em todo push e PR na `main`, só no Windows:
`cargo test`, `cargo clippy -D warnings` e `npm run tauri build` sem publicar (os
instaladores ficam como artefatos do run).

## O que o Jarvis faz

- **Conversa por voz** em tempo real: escuta o microfone, responde em áudio, aceita
  interrupção (fale por cima e ele para) e reconecta sozinho quando a sessão cai.
- **App de desktop** na bandeja, sempre ouvindo, com overlay flutuante estilo Siri e
  atalho global pra mutar.
- **Perfis:** cinco personalidades prontas — Assistente pessoal, Professor de inglês,
  Terapeuta de apoio, Mentor de negócios e Parceiro de código — trocadas pelo menu do
  ícone ou pelas Configurações. Cada perfil define voz, jeito de falar e **quais
  ferramentas pode usar** (o Professor e o Terapeuta só conversam).
- **Ferramentas locais:** abre apps (`app.open`) e sites (`web.open`), roda comandos
  no terminal (`shell.run`, timeout 30s, sem sudo), lê, escreve e lista arquivos
  (`fs.*`, só dentro do home), ajusta volume e mídia, cria eventos na agenda e
  lembretes.
- **Confirmação em ações arriscadas:** rodar comando, escrever arquivo, criar evento e
  ações que alteram algo no Overclock/OverClick pedem "confirma?" — responda "sim"/"não"
  por voz ou clique nos botões do overlay. Sem resposta em 20s, a ação é negada.
- **Overclock e OverClick por voz (MCP):** o Jarvis lista e abre panes, lê o que um
  pane entregou e cria cards no OverClick. Conecte em **Configurações › Ferramentas ›
  Servidores MCP** (botões **Conectar Overclock** e **Conectar OverClick**): o token
  colado ali fica no Keychain do Mac, nunca no config. A mesma aba mostra as permissões
  do macOS (Acessibilidade, Automação) com status ao vivo. Quem preferir arquivo usa
  `~/.config/jarvis/config.toml`, com o token só por nome de variável de ambiente:

  ```toml
  [tools]
  enabled = true

  [[mcp_servers]]
  name = "overclock"
  url = "http://127.0.0.1:${OVERCLOCK_MCP_PORT}/mcp"
  bearer_env = "OVERCLOCK_MCP_BEARER_TOKEN"

  [[mcp_servers]]
  name = "overclick"
  url = "https://cloud.overclock.sh/mcp"
  bearer_env = "OVERCLICK_MCP_BEARER_TOKEN"
  ```

Roteiro leigo das ferramentas: [`docs/testes/v3.md`](docs/testes/v3.md).

## Reflexo (Jev)

O **reflexo** é uma camada rápida na frente do Gemini: a cada fragmento do que você fala,
ele pergunta ao **Jev** (modelo de decisão da [TypeSafe.ai](https://typesafe.ai)) se aquilo
é uma ação segura e fechada e, se for, executa em ~300 ms, antes de o Jarvis terminar de
responder. O Gemini continua sendo o cérebro; o reflexo só escolhe entre opções listadas e
só dispara o que já não pede confirmação: abrir app, abrir site da lista, mídia, volume e
"sim"/"não" por voz num pedido pendente. Em dúvida, fica quieto. Sem chave, tudo funciona
como na v3.

**Como ligar.** Pela janela de Configurações › aba **Reflexo** (cole a chave, marque
**Reflexo ligado (Jev)**, cadastre os sites, **Salvar reflexo** e reconecte), ou por
arquivo. A chave vem do `config.toml` (`typesafe_api_key`) ou da variável de ambiente
`TYPESAFE_API_KEY` (a env vence). Nunca em log, evento, overlay ou commit.

```toml
typesafe_api_key = "..."        # ou export TYPESAFE_API_KEY="..."

[reflex]
enabled = true
act_threshold = 0.85            # confiança mínima para agir sozinho
confirm_threshold = 0.85        # confiança mínima para aprovar/negar por voz

[[reflex.sites]]
name = "YouTube"
url = "https://youtube.com"
```

**Privacidade.** Enquanto o reflexo estiver ligado, a transcrição do que você fala e os
nomes dos apps instalados/rodando e dos sites da lista são enviados à TypeSafe.ai. Com o
reflexo desligado ou sem chave, nada sai. `web.open` pelo reflexo só recebe URLs de
`[[reflex.sites]]`, nunca uma URL montada da fala.

**Diagnóstico no terminal.** Sem abrir microfone nem Gemini:

```sh
jarvis reflex "abre o safari"   # perguntas, probabilidades, decisão e latência
jarvis reflex --eye             # inventário: apps rodando, instalados e sites
jarvis reflex "pode ir" --pending   # simula confirmação pendente
```

Roteiro leigo do reflexo: [`docs/testes/v4.md`](docs/testes/v4.md).

## Roadmap por degraus

Cada degrau é uma entrega separada, com spec própria.

| Degrau | O que entra | Status |
|---|---|---|
| 1 | Conversa por voz no terminal | pronto (v1) |
| 2 | Ícone na bandeja e atalho global | pronto (v2) |
| 3 | Visão de tela (frames a 1 fps) | planejado |
| 4 | Ferramentas: rodar comandos com confirmação por voz | pronto (v3) |
| 5 | Integração com o Overclock (abrir panes, criar cards) | pronto (v3) |
| 6 | Reflexo: ações seguras em ~300 ms com o Jev (TypeSafe.ai) | pronto (v4) |

## Configurar a chave

Nunca coloque a chave no código nem em commits. Duas formas:

```sh
export GEMINI_API_KEY="..."
```

ou crie `~/.config/jarvis/config.toml`:

```toml
api_key = "..."
```

## Rodar

O repo é um workspace Cargo: `crates/core` (lib `openjarvisbr-core` — áudio, sessão Live,
config) e `crates/cli` (bin `jarvis`, usa o core).

```sh
. "$HOME/.cargo/env"
cargo run --release --bin jarvis
```

Flags: `--voice`, `--device-in`, `--device-out`, `--debug`.

Antes de testar de verdade, siga o roteiro leigo em [`docs/testes/v1.md`](docs/testes/v1.md) —
ele cobre desde criar a chave até conversa de 2 minutos, interrupção, mudo e reconexão de rede.

## App de desktop

`apps/desktop` é o app Mac/Windows: fica na bandeja, sempre conectado e ouvindo. Uma
janela flutuante estilo Siri surge no topo da tela quando você fala e some sozinha no
silêncio; uma janela de Configurações edita chave, voz, dispositivos, efeito, barge-in
e system prompt.

### Rodar em modo desenvolvimento

```sh
. "$HOME/.cargo/env"
cd apps/desktop
npm install
npm run tauri dev
```

- Com a chave configurada (`~/.config/jarvis/config.toml` ou `GEMINI_API_KEY`): conecta
  direto e o ícone da bandeja mostra o estado (conectando, ouvindo, falando, mudo, erro).
- Sem chave: abre a janela de Configurações com um aviso pra colar a chave, e o ícone
  fica em erro.
- Overlay: aparece no topo central da tela ao falar (sua fala e a resposta), some
  sozinho ~4s depois do silêncio, nunca rouba foco.
- Atalho global `Cmd+Shift+J` (`Ctrl+Shift+J` no Windows/Linux) muta e desmuta.
- Menu do ícone: Mutar/Desmutar, Reconectar, Configurações, Sair.
- Abrir o app uma segunda vez ativa a instância já aberta em vez de duplicar.

Antes de considerar uma mudança pronta, siga o roteiro leigo em
[`docs/testes/v2.md`](docs/testes/v2.md).

### Gerar o build (`.app`/`.dmg` no Mac, instalador no Windows)

```sh
. "$HOME/.cargo/env"
cd apps/desktop
npm install
npm run tauri build
```

- **Mac:** gera `OpenJarvisBR.app` e `OpenJarvisBR_<versão>_aarch64.dmg` em
  `target/release/bundle/macos/` e `target/release/bundle/dmg/` (raiz do workspace
  Cargo — o `.dmg` fica pronto pra abrir e arrastar pra `Aplicativos`).
- **Windows:** ver [`docs/build-windows.md`](docs/build-windows.md) — pré-requisitos
  (Rust, Node, WebView2) e o que testar a mais na primeira rodada.

Não há instalador assinado/notarizado nesta versão (fora de escopo do v2) — no Mac, o
Gatekeeper pode pedir para autorizar o app em Ajustes › Privacidade e Segurança na
primeira abertura.

## Stack

Rust stable · [cpal](https://github.com/RustAudio/cpal) · [rubato](https://github.com/HEnquist/rubato) ·
tokio · tokio-tungstenite + rustls · serde · clap · tracing

## Contribuir

O trabalho é organizado em cards no OverClick. A spec do degrau 1 está em
`docs/superpowers/specs/`. Um commit por card, `cargo clippy` sem warnings e
`cargo test` verde antes de entregar.

## Licença

Proprietário. Todos os direitos reservados. Instaladores públicos em
https://github.com/ustoppble/openjarvisbr-releases

## Publicar uma versão

1. Suba a versão em `apps/desktop/src-tauri/tauri.conf.json`, `package.json` e nos `Cargo.toml`; commit e `git tag vX.Y.Z && git push origin vX.Y.Z`.
2. A Action `release.yml` gera `.msi`/`.exe` (Windows) na release deste repo privado.
3. No Mac: `cd apps/desktop && npm run tauri build` gera o `.dmg` em `target/release/bundle/dmg/`.
4. Copie tudo para o repo público de downloads:
   `gh release download vX.Y.Z -D /tmp/rel && gh release create vX.Y.Z -R ustoppble/openjarvisbr-releases --title "OpenJarvisBR vX.Y.Z" --notes-file notas.md /tmp/rel/* target/release/bundle/dmg/*.dmg`
