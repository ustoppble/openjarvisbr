//! Mensagens da Gemini Live API (WebSocket), serializadas e desserializadas
//! via `serde`. Este módulo não conecta a nada — só converte entre structs
//! Rust e o JSON trocado com o servidor, para ficar testável sem rede.
//!
//! Referência: <https://ai.google.dev/gemini-api/docs/live-api>
//!
//! `session.rs` (próximo card) é quem usa este módulo; até lá, itens ainda
//! não referenciados por fora não devem acender `dead_code`.
#![allow(dead_code)]

use std::fmt;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::tools::{ToolCall, ToolResult, ToolSpec};

/// Modelo padrão da v1 (spec: baixa latência).
pub const MODEL: &str = "models/gemini-3.8-live";
/// Voz padrão da v1.
pub const VOICE: &str = "Puck";
/// `mimeType` do áudio enviado ao servidor (mic, 16kHz).
pub const INPUT_AUDIO_MIME: &str = "audio/pcm;rate=16000";

// ---------------------------------------------------------------------
// Mensagens de saída (cliente -> servidor)
// ---------------------------------------------------------------------

/// `{"setup": {...}}` — primeira mensagem da sessão.
#[derive(Debug, Clone, Serialize)]
pub struct SetupRequest {
    pub setup: Setup,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<SystemInstruction>,
    pub generation_config: GenerationConfig,
    pub input_audio_transcription: AudioTranscriptionConfig,
    pub output_audio_transcription: AudioTranscriptionConfig,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolGroup>,
}

/// Um item de `setup.tools`: `{"functionDeclarations": [...]}`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGroup {
    pub function_declarations: Vec<FunctionDeclaration>,
}

/// Declaração de função. `parametersJsonSchema` aceita JSON Schema completo
/// (o campo `parameters` só aceita o subconjunto OpenAPI).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters_json_schema: Option<serde_json::Value>,
}

impl From<&ToolSpec> for FunctionDeclaration {
    fn from(spec: &ToolSpec) -> Self {
        FunctionDeclaration {
            name: spec.name.clone(),
            description: spec.description.clone(),
            parameters_json_schema: (!spec.parameters.is_null()).then(|| spec.parameters.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    pub response_modalities: Vec<String>,
    pub speech_config: SpeechConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechConfig {
    pub voice_config: VoiceConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfig {
    pub prebuilt_voice_config: PrebuiltVoiceConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrebuiltVoiceConfig {
    pub voice_name: String,
}

/// `systemInstruction`: identidade e regras do assistente.
#[derive(Debug, Clone, Serialize)]
pub struct SystemInstruction {
    pub parts: Vec<TextPart>,
}

/// Objeto vazio: a presença do campo já ativa a transcrição.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AudioTranscriptionConfig {}

impl SetupRequest {
    /// Setup padrão da v1: `gemini-3.8-live`, áudio, voz Puck, transcrição
    /// de entrada e saída ativas.
    pub fn new(voice: impl Into<String>) -> Self {
        SetupRequest {
            setup: Setup {
                model: MODEL.to_string(),
                system_instruction: None,
                generation_config: GenerationConfig {
                    response_modalities: vec!["AUDIO".to_string()],
                    speech_config: SpeechConfig {
                        voice_config: VoiceConfig {
                            prebuilt_voice_config: PrebuiltVoiceConfig {
                                voice_name: voice.into(),
                            },
                        },
                    },
                },
                input_audio_transcription: AudioTranscriptionConfig::default(),
                output_audio_transcription: AudioTranscriptionConfig::default(),
                tools: Vec::new(),
            },
        }
    }
}

impl SetupRequest {
    /// Define a instrução de sistema (identidade, idioma, regras).
    pub fn with_system_instruction(mut self, text: impl Into<String>) -> Self {
        self.setup.system_instruction = Some(SystemInstruction {
            parts: vec![TextPart { text: text.into() }],
        });
        self
    }

    /// Declara as ferramentas que o modelo pode pedir. Lista vazia não
    /// envia o campo `tools`.
    pub fn with_tools(mut self, specs: &[ToolSpec]) -> Self {
        self.setup.tools = if specs.is_empty() {
            Vec::new()
        } else {
            vec![ToolGroup {
                function_declarations: specs.iter().map(FunctionDeclaration::from).collect(),
            }]
        };
        self
    }
}

impl Default for SetupRequest {
    fn default() -> Self {
        SetupRequest::new(VOICE)
    }
}

/// `{"realtimeInput": {"audio": {...}}}` — um chunk de PCM do microfone.
#[derive(Debug, Clone, Serialize)]
pub struct RealtimeInputRequest {
    #[serde(rename = "realtimeInput")]
    pub realtime_input: RealtimeInput,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeInput {
    pub audio: AudioBlob,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioBlob {
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

impl RealtimeInputRequest {
    /// Empacota um chunk de PCM i16 little-endian (16kHz mono) em base64.
    pub fn from_pcm(samples: &[i16]) -> Self {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        RealtimeInputRequest {
            realtime_input: RealtimeInput {
                audio: AudioBlob {
                    data: BASE64.encode(bytes),
                    mime_type: INPUT_AUDIO_MIME.to_string(),
                },
            },
        }
    }
}

/// `{"clientContent": {...}}` — texto enviado como turno de usuário (usado
/// na reconexão, para reenviar contexto das últimas transcrições).
#[derive(Debug, Clone, Serialize)]
pub struct ClientContentRequest {
    #[serde(rename = "clientContent")]
    pub client_content: ClientContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientContent {
    pub turns: Vec<Turn>,
    pub turn_complete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Turn {
    pub role: String,
    pub parts: Vec<TextPart>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextPart {
    pub text: String,
}

impl ClientContentRequest {
    pub fn text(text: impl Into<String>) -> Self {
        ClientContentRequest {
            client_content: ClientContent {
                turns: vec![Turn {
                    role: "user".to_string(),
                    parts: vec![TextPart { text: text.into() }],
                }],
                turn_complete: true,
            },
        }
    }
}

/// `{"toolResponse": {"functionResponses": [...]}}` — resultados das
/// ferramentas pedidas em `toolCall`, casados pelo `id`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolResponseRequest {
    #[serde(rename = "toolResponse")]
    pub tool_response: ToolResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResponse {
    pub function_responses: Vec<FunctionResponse>,
}

/// `response` é sempre um objeto: `{"output": ...}` no sucesso e
/// `{"error": "..."}` na falha, como a API recomenda. `name` não está em
/// `ToolResult`; a sessão preenche a partir do `toolCall` recebido.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub response: serde_json::Value,
}

impl From<&ToolResult> for FunctionResponse {
    fn from(result: &ToolResult) -> Self {
        let response = match &result.error {
            Some(error) => serde_json::json!({ "error": error }),
            None => serde_json::json!({ "output": result.output }),
        };
        FunctionResponse {
            id: result.id.clone(),
            name: None,
            response,
        }
    }
}

impl ToolResponseRequest {
    pub fn new(results: &[ToolResult]) -> Self {
        ToolResponseRequest {
            tool_response: ToolResponse {
                function_responses: results.iter().map(FunctionResponse::from).collect(),
            },
        }
    }

    /// Preenche `name` de cada resposta a partir do `id` (sem nome
    /// conhecido, o campo não vai).
    pub fn with_names(mut self, mut name_of: impl FnMut(&str) -> Option<String>) -> Self {
        for response in &mut self.tool_response.function_responses {
            if response.name.is_none() {
                response.name = name_of(&response.id);
            }
        }
        self
    }
}

// ---------------------------------------------------------------------
// Mensagens de entrada (servidor -> cliente)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerMessage {
    #[serde(default)]
    setup_complete: Option<SetupComplete>,
    #[serde(default)]
    server_content: Option<ServerContent>,
    #[serde(default)]
    go_away: Option<GoAway>,
    #[serde(default)]
    tool_call: Option<ToolCallMessage>,
    #[serde(default)]
    tool_call_cancellation: Option<ToolCallCancellationMessage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallMessage {
    #[serde(default)]
    function_calls: Vec<FunctionCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct FunctionCall {
    #[serde(default)]
    id: String,
    name: String,
    #[serde(default)]
    args: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCallCancellationMessage {
    #[serde(default)]
    ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SetupComplete {}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerContent {
    #[serde(default)]
    model_turn: Option<ModelTurn>,
    #[serde(default)]
    interrupted: bool,
    #[serde(default)]
    input_transcription: Option<Transcription>,
    #[serde(default)]
    output_transcription: Option<Transcription>,
    #[serde(default)]
    turn_complete: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelTurn {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    #[serde(default)]
    inline_data: Option<InlineData>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    #[serde(default)]
    #[allow(dead_code)]
    mime_type: String,
    data: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Transcription {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoAway {
    #[serde(default)]
    #[allow(dead_code)]
    time_left: Option<String>,
}

/// Eventos que `session.rs` reage: áudio a tocar, interrupção do usuário,
/// transcrições e sinais de reconexão/fechamento.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    Audio(Vec<i16>),
    Interrupted,
    UserText(String),
    ModelText(String),
    /// O modelo terminou o turno (`serverContent.turnComplete`): a UI pode
    /// fechar a linha da transcrição corrente.
    TurnComplete,
    GoAway,
    /// O modelo pede a execução destas ferramentas (`toolCall`). Sem `args`
    /// no JSON, `args` vem como objeto vazio.
    ToolCall(Vec<ToolCall>),
    /// O servidor cancelou chamadas pendentes (`toolCallCancellation`), pelos
    /// ids — normalmente porque o usuário interrompeu.
    ToolCallCancellation(Vec<String>),
    /// Produzido por `session.rs` antes de cada tentativa de reconexão
    /// automática (1, 2, 3) — nunca vem de `parse`.
    Reconnecting(u32),
    /// Produzido por `session.rs` quando a reconexão automática deu certo.
    Reconnected,
    /// Produzido por `session.rs` quando o socket fecha — nunca vem de
    /// `parse`, que só lê mensagens JSON efetivamente recebidas.
    Closed,
}

#[derive(Debug)]
pub enum ProtocolError {
    Json(serde_json::Error),
    Audio(base64::DecodeError),
    /// A mensagem é uma mensagem de protocolo válida, mas não corresponde a
    /// nenhuma variante de `ServerEvent` (ex.: `setupComplete`, ou um
    /// `serverContent` vazio). Não é um erro de
    /// parsing — é sinal para quem chama ignorar esta mensagem.
    NoEvent,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::Json(err) => write!(f, "JSON inválido da Live API: {err}"),
            ProtocolError::Audio(err) => write!(f, "áudio base64 inválido: {err}"),
            ProtocolError::NoEvent => write!(f, "mensagem sem evento (ex.: setupComplete)"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<serde_json::Error> for ProtocolError {
    fn from(err: serde_json::Error) -> Self {
        ProtocolError::Json(err)
    }
}

/// Interpreta uma mensagem JSON recebida do servidor da Live API e devolve
/// TODOS os eventos que ela carrega, na ordem: interrupção, transcrições,
/// áudio, fim de turno. Uma mensagem pode trazer transcrição e áudio juntos;
/// devolver só o primeiro (comportamento antigo) descartava chunks de áudio
/// e a voz saía com pedaços faltando.
pub fn parse_all(raw: &str) -> Result<Vec<ServerEvent>, ProtocolError> {
    let message: ServerMessage = serde_json::from_str(raw)?;
    let mut events = Vec::new();

    if message.go_away.is_some() {
        events.push(ServerEvent::GoAway);
    }

    if let Some(cancel) = message.tool_call_cancellation {
        events.push(ServerEvent::ToolCallCancellation(cancel.ids));
    }

    if let Some(call) = message.tool_call {
        let calls: Vec<ToolCall> = call
            .function_calls
            .into_iter()
            .map(|fc| ToolCall {
                id: fc.id,
                name: fc.name,
                args: fc
                    .args
                    .filter(|args| !args.is_null())
                    .unwrap_or_else(|| serde_json::json!({})),
            })
            .collect();
        if !calls.is_empty() {
            events.push(ServerEvent::ToolCall(calls));
        }
    }

    if let Some(content) = message.server_content {
        if content.interrupted {
            events.push(ServerEvent::Interrupted);
        }
        if let Some(t) = content.input_transcription {
            events.push(ServerEvent::UserText(t.text));
        }
        if let Some(t) = content.output_transcription {
            events.push(ServerEvent::ModelText(t.text));
        }
        if let Some(turn) = content.model_turn {
            match decode_audio(&turn) {
                Ok(audio) => events.push(audio),
                Err(ProtocolError::NoEvent) => {}
                Err(err) => return Err(err),
            }
        }
        if content.turn_complete {
            events.push(ServerEvent::TurnComplete);
        }
    }

    if events.is_empty() {
        return Err(ProtocolError::NoEvent);
    }
    Ok(events)
}

/// Primeiro evento da mensagem. Mantido para os testes de fixture; o
/// caminho de produção usa `parse_all`.
pub fn parse(raw: &str) -> Result<ServerEvent, ProtocolError> {
    parse_all(raw).map(|mut events| events.remove(0))
}

fn decode_audio(turn: &ModelTurn) -> Result<ServerEvent, ProtocolError> {
    let mut bytes = Vec::new();
    for part in &turn.parts {
        if let Some(inline) = &part.inline_data {
            let decoded = BASE64.decode(&inline.data).map_err(ProtocolError::Audio)?;
            bytes.extend_from_slice(&decoded);
        }
    }

    if bytes.is_empty() {
        return Err(ProtocolError::NoEvent);
    }

    let samples = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair))
        .collect();
    Ok(ServerEvent::Audio(samples))
}

#[cfg(test)]
mod tests {
    #[test]
    fn transcription_and_audio_in_one_message_yield_both_events() {
        // 4 amostras i16 LE: 1, -1, 2, -2 → base64
        let raw = r#"{"serverContent":{"outputTranscription":{"text":"oi"},"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AQD//wIA/v8="}}]},"turnComplete":true}}"#;
        let events = parse_all(raw).unwrap();
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[0], ServerEvent::ModelText("oi".into()));
        assert_eq!(events[1], ServerEvent::Audio(vec![1, -1, 2, -2]));
        assert_eq!(events[2], ServerEvent::TurnComplete);
    }

    #[test]
    fn turn_complete_alone_is_an_event() {
        let raw = r#"{"serverContent":{"turnComplete":true}}"#;
        assert_eq!(parse(raw).unwrap(), ServerEvent::TurnComplete);
    }

    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("lendo {path}: {err}"))
    }

    #[test]
    fn parses_audio_fixture() {
        let raw = fixture("audio.json");
        match parse(&raw) {
            Ok(ServerEvent::Audio(samples)) => {
                assert_eq!(samples, vec![0, 100, -100, 32767, -32768]);
            }
            other => panic!("esperava ServerEvent::Audio, veio {other:?}"),
        }
    }

    #[test]
    fn parses_interrupted_fixture() {
        let raw = fixture("interrupted.json");
        assert_eq!(parse(&raw).unwrap(), ServerEvent::Interrupted);
    }

    #[test]
    fn parses_input_transcription_fixture() {
        let raw = fixture("input_transcription.json");
        match parse(&raw) {
            Ok(ServerEvent::UserText(text)) => assert_eq!(text, "oi, quem é você"),
            other => panic!("esperava ServerEvent::UserText, veio {other:?}"),
        }
    }

    #[test]
    fn parses_output_transcription_fixture() {
        let raw = fixture("output_transcription.json");
        match parse(&raw) {
            Ok(ServerEvent::ModelText(text)) => assert_eq!(text, "eu sou a OpenJarvisBR"),
            other => panic!("esperava ServerEvent::ModelText, veio {other:?}"),
        }
    }

    #[test]
    fn parses_go_away_fixture() {
        let raw = fixture("go_away.json");
        assert_eq!(parse(&raw).unwrap(), ServerEvent::GoAway);
    }

    #[test]
    fn setup_complete_fixture_is_recognized_with_no_event() {
        let raw = fixture("setup_complete.json");
        match parse(&raw) {
            Err(ProtocolError::NoEvent) => {}
            other => panic!("esperava ProtocolError::NoEvent, veio {other:?}"),
        }
    }

    #[test]
    fn setup_message_serializes_expected_fields() {
        let json = serde_json::to_string(&SetupRequest::default()).unwrap();
        assert!(json.contains("\"model\":\"models/gemini-3.8-live\""));
        assert!(json.contains("\"responseModalities\":[\"AUDIO\"]"));
        assert!(json.contains("\"voiceName\":\"Puck\""));
        assert!(json.contains("\"inputAudioTranscription\":{}"));
        assert!(json.contains("\"outputAudioTranscription\":{}"));
    }

    #[test]
    fn realtime_input_encodes_pcm_as_base64() {
        let req = RealtimeInputRequest::from_pcm(&[0, 100, -100, 32767, -32768]);
        assert_eq!(req.realtime_input.audio.data, "AABkAJz//38AgA==");
        assert_eq!(req.realtime_input.audio.mime_type, INPUT_AUDIO_MIME);
    }

    fn clock_spec() -> ToolSpec {
        ToolSpec {
            name: "clock.now".into(),
            description: "Retorna a hora atual local.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"timezone": {"type": "string"}},
                "required": ["timezone"],
                "additionalProperties": false
            }),
            risk: crate::tools::Risk::Safe,
        }
    }

    #[test]
    fn parses_tool_call_fixture() {
        // Capturado da Live API real (gemini-3.8-live) com `clock.now` declarada.
        let raw = fixture("tool_call.json");
        assert_eq!(
            parse_all(&raw).unwrap(),
            vec![ServerEvent::ToolCall(vec![ToolCall {
                id: "call_175775".into(),
                name: "clock.now".into(),
                args: serde_json::json!({"timezone": "America/Sao_Paulo"}),
            }])]
        );
    }

    #[test]
    fn tool_call_without_args_gets_empty_object() {
        let raw = r#"{"toolCall":{"functionCalls":[{"id":"call_1","name":"app.open"}]}}"#;
        match parse(raw).unwrap() {
            ServerEvent::ToolCall(calls) => assert_eq!(calls[0].args, serde_json::json!({})),
            other => panic!("esperava ToolCall, veio {other:?}"),
        }
    }

    #[test]
    fn parses_tool_call_cancellation_fixture() {
        let raw = fixture("tool_call_cancellation.json");
        assert_eq!(
            parse(&raw).unwrap(),
            ServerEvent::ToolCallCancellation(vec!["call_175775".into()])
        );
    }

    #[test]
    fn empty_and_usage_only_messages_have_no_event() {
        for raw in ["{}", r#"{"usageMetadata":{"totalTokenCount":10}}"#] {
            assert!(matches!(parse_all(raw), Err(ProtocolError::NoEvent)), "{raw}");
        }
    }

    #[test]
    fn setup_with_tools_serializes_function_declarations() {
        let request = SetupRequest::default().with_tools(&[clock_spec()]);
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            value["setup"]["tools"],
            serde_json::json!([{
                "functionDeclarations": [{
                    "name": "clock.now",
                    "description": "Retorna a hora atual local.",
                    "parametersJsonSchema": {
                        "type": "object",
                        "properties": {"timezone": {"type": "string"}},
                        "required": ["timezone"],
                        "additionalProperties": false
                    }
                }]
            }])
        );
    }

    #[test]
    fn setup_without_tools_omits_field_and_null_schema_is_omitted() {
        let json = serde_json::to_string(&SetupRequest::default().with_tools(&[])).unwrap();
        assert!(!json.contains("\"tools\""), "{json}");

        let mut spec = clock_spec();
        spec.parameters = serde_json::Value::Null;
        let value = serde_json::to_value(SetupRequest::default().with_tools(&[spec])).unwrap();
        let declaration = &value["setup"]["tools"][0]["functionDeclarations"][0];
        assert!(declaration.get("parametersJsonSchema").is_none(), "{declaration}");
    }

    #[test]
    fn tool_response_serializes_output_and_error() {
        let results = [
            ToolResult {
                id: "a".into(),
                output: serde_json::json!({"time": "14:32"}),
                error: None,
            },
            ToolResult {
                id: "b".into(),
                output: serde_json::Value::Null,
                error: Some("negado pelo usuário".into()),
            },
        ];
        let value = serde_json::to_value(ToolResponseRequest::new(&results)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"toolResponse": {"functionResponses": [
                {"id": "a", "response": {"output": {"time": "14:32"}}},
                {"id": "b", "response": {"error": "negado pelo usuário"}}
            ]}})
        );
    }

    #[test]
    fn tool_call_round_trip_answers_with_same_id_and_name() {
        // Ida: toolCall real do servidor. Volta: toolResponse com o mesmo id
        // e o nome recuperado do pedido — o formato que o servidor aceitou
        // no teste ao vivo.
        let ServerEvent::ToolCall(calls) = parse(&fixture("tool_call.json")).unwrap() else {
            panic!("fixture deveria ser toolCall");
        };
        let results: Vec<ToolResult> = calls
            .iter()
            .map(|call| ToolResult {
                id: call.id.clone(),
                output: serde_json::json!({"time": "14:32", "tz": call.args["timezone"]}),
                error: None,
            })
            .collect();
        let request = ToolResponseRequest::new(&results).with_names(|id| {
            calls.iter().find(|c| c.id == id).map(|c| c.name.clone())
        });
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"toolResponse": {"functionResponses": [{
                "id": "call_175775",
                "name": "clock.now",
                "response": {"output": {"time": "14:32", "tz": "America/Sao_Paulo"}}
            }]}})
        );
    }

    #[test]
    fn client_content_text_marks_turn_complete() {
        let req = ClientContentRequest::text("oi");
        assert!(req.client_content.turn_complete);
        assert_eq!(req.client_content.turns[0].parts[0].text, "oi");
    }
}
