# OpenJarvisBR

Assistente de voz open source, em Rust, que conversa com você em tempo real usando o
**Gemini 3.8 Live** do Google. Roda no terminal do Mac hoje e no Windows em breve.

> Status: **v1 em construção**. O degrau 1 (só conversa) está sendo implementado card a
> card. Ainda não há binário pronto pra uso.

## O que ele faz (v1)

- Escuta o microfone e responde em áudio, com latência de conversa.
- Interrupção natural: fale por cima e ele para.
- Transcrição dos dois lados aparece no terminal.
- Reconecta sozinho quando a sessão da Live API cai ou expira.

## Roadmap por degraus

Cada degrau é uma entrega separada, com spec própria.

| Degrau | O que entra | Status |
|---|---|---|
| 1 | Conversa por voz no terminal | em construção |
| 2 | Ícone na bandeja e atalho global | planejado |
| 3 | Visão de tela (frames a 1 fps) | planejado |
| 4 | Ferramentas: rodar comandos com confirmação por voz | planejado |
| 5 | Integração com o Overclock (abrir panes, criar cards) | planejado |

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

MIT
