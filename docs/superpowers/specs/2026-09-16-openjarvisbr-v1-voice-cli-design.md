# OpenJarvisBR v1 — assistente de voz em Rust (degrau 1: só conversa)

Data: 2026-09-16
Status: aprovado pelo dono em conversa

## Objetivo

Um binário Rust, `jarvis` (crate `openjarvisbr`), que roda no terminal do Mac (e futuramente Windows),
abre uma sessão com o modelo `gemini-3.8-live` do Google e permite conversa
contínua por voz: você fala no microfone, o modelo responde em áudio pelo
alto-falante, com interrupção natural e transcrição visível no terminal.

Cada degrau seguinte (bandeja, visão de tela, ferramentas, Overclock) é uma
entrega separada com spec própria. Esta spec cobre apenas o degrau 1.

## Fora de escopo (v1)

- Ícone de bandeja / atalho global
- Envio de frames da tela
- Function calling / ferramentas
- Wake word
- Memória entre sessões
- Build Windows verificado (o código evita APIs específicas de plataforma, mas o
  teste em Windows é entrega futura)

## Decisões

| Decisão | Escolha | Motivo |
|---|---|---|
| Linguagem | Rust (edition 2021, stable) | binário único, portável Mac/Windows |
| Áudio | `cpal` | CoreAudio e WASAPI no mesmo crate |
| Resample | `rubato` | mic raramente é 16kHz nativo; saída é 24kHz |
| Rede | `tokio` + `tokio-tungstenite` + `rustls` | WebSocket async sem OpenSSL |
| JSON | `serde` + `serde_json` | |
| CLI | `clap` | flags `--voice`, `--device-in`, `--device-out`, `--debug` |
| Modelo | `gemini-3.8-live` | padrão de baixa latência (doc oficial) |
| Voz padrão | `Puck` | |
| VAD | automático (servidor) | menos código, interrupção nativa |
| Chave | env `GEMINI_API_KEY`, senão `~/.config/jarvis/config.toml` | nunca vai pro log |

## Arquitetura

```
src/
  main.rs        CLI (clap), carrega config, sobe App
  app.rs         estado (Idle/Connecting/Listening/Speaking/Error), loop principal
  config.rs      leitura de env + config.toml
  audio/
    mod.rs
    capture.rs   cpal input → PCM i16 16kHz mono, chunks de 20ms
    playback.rs  fila de PCM i16 24kHz → cpal output; descarta fila em interrupção
    resample.rs  rubato, f32 qualquer taxa → i16 taxa alvo
  live/
    mod.rs
    protocol.rs  structs serde das mensagens da Live API (setup, realtimeInput,
                 serverContent, transcription, goAway)
    session.rs   conecta, envia setup, envia áudio, recebe mensagens,
                 reconecta em goAway ou queda
```

### Fluxo de dados

1. `main` lê config, valida chave (só existência e tamanho, nunca imprime).
2. `App` abre `LiveSession` com modelo, voz, `response_modalities: ["AUDIO"]`,
   transcrição de entrada e saída ativadas.
3. `capture` produz chunks de 20ms num canal `mpsc`; `session` envia cada chunk
   como `realtimeInput.audio` (`audio/pcm;rate=16000`, base64).
4. `session` recebe:
   - áudio do modelo → empurra na fila de `playback`
   - `interrupted: true` → `playback.flush()`
   - transcrições → imprime no terminal (você em uma cor, OpenJarvisBR em outra)
   - `goAway` ou erro de socket → reconecta com backoff (1s, 2s, 4s, máx 3
     tentativas), reenviando as últimas transcrições como contexto de texto
5. Ctrl+C fecha o socket e para os streams de áudio.

### Interface entre módulos

- `capture::start(device, tx: Sender<Vec<i16>>) -> Result<CaptureHandle>`
- `playback::Player::new(device) -> Result<Player>`; `push(&[i16])`, `flush()`
- `resample::to_i16(input: &[f32], from_hz, to_hz) -> Vec<i16>`
- `session::LiveSession::connect(cfg) -> Result<LiveSession>`;
  `send_audio(&[i16])`, `next_event() -> Option<ServerEvent>`
- `ServerEvent` enum: `Audio(Vec<i16>)`, `Interrupted`, `UserText(String)`,
  `ModelText(String)`, `GoAway`, `Closed`

## Tratamento de erros

| Situação | Comportamento |
|---|---|
| Sem chave | mensagem clara com as duas formas de configurar, sai com código 2 |
| Sem mic / sem saída | lista dispositivos disponíveis, sai com código 3 |
| Permissão de mic negada (Mac) | mensagem apontando Ajustes › Privacidade |
| Socket cai | reconecta com backoff; após 3 falhas, avisa e sai com código 4 |
| Quota / 429 | mostra mensagem do servidor sem o corpo bruto, sai com código 5 |
| Chave inválida / 401 | mensagem, sai com código 2 |

Logs com `tracing`; `--debug` liga nível debug. Nenhum log inclui a chave,
headers de auth ou a URL completa com `key=`.

## Testes

- Unitários em `resample` (taxas 44.1k→16k e 48k→16k, tamanho e ausência de
  NaN) e em `protocol` (parse de fixtures JSON reais: audio, interrupted,
  transcription, goAway).
- Teste de integração opcional, ignorado por padrão, que conecta de verdade
  quando `GEMINI_API_KEY` existe e verifica que o setup é aceito.
- Roteiro manual (bateria de testes) na entrega: conversa de 2 minutos,
  interrupção no meio de uma resposta, forçar queda de rede e ver reconectar.

## Critério de pronto

- `cargo build --release` limpo, `cargo test` verde, `cargo clippy` sem warnings.
- Rodar `jarvis`, falar "oi, quem é você" e ouvir resposta em menos de 2s.
- Interromper a OpenJarvisBR falando por cima e ele parar de falar.
- Transcrição dos dois lados aparece no terminal.
