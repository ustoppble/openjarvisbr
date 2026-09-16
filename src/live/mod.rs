//! Cliente da Live API do Gemini: protocolo e sessão WebSocket.
//!
//! `protocol.rs` traz as structs serde das mensagens (setup, realtimeInput,
//! serverContent, transcription, goAway) e o parser `ServerEvent`.
//! `session.rs` conecta, envia setup e áudio, recebe
//! eventos e reconecta em goAway ou queda.

pub mod protocol;
pub mod session;
