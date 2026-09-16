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

```sh
. "$HOME/.cargo/env"
cargo run --release
```

Flags: `--voice`, `--device-in`, `--device-out`, `--debug`.

Antes de testar de verdade, siga o roteiro leigo em [`docs/testes/v1.md`](docs/testes/v1.md) —
ele cobre desde criar a chave até conversa de 2 minutos, interrupção, mudo e reconexão de rede.

## Stack

Rust stable · [cpal](https://github.com/RustAudio/cpal) · [rubato](https://github.com/HEnquist/rubato) ·
tokio · tokio-tungstenite + rustls · serde · clap · tracing

## Contribuir

O trabalho é organizado em cards no OverClick. A spec do degrau 1 está em
`docs/superpowers/specs/`. Um commit por card, `cargo clippy` sem warnings e
`cargo test` verde antes de entregar.

## Licença

MIT
