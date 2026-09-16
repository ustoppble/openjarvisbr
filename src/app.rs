//! Estado da aplicação e loop principal da OpenJarvisBR.

use tracing::info;

/// Estados possíveis do assistente durante uma sessão. As variantes além de
/// `Idle` entram em uso quando `live::session` for implementado.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Connecting,
    Listening,
    Speaking,
    Error,
}

/// Aplicação principal: mantém o estado atual da sessão.
pub struct App {
    state: State,
}

impl App {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Executa o loop principal. Nos próximos degraus isso abre a sessão
    /// Live e os streams de áudio; por ora apenas confirma que o scaffold
    /// está pronto, sem nunca logar a chave — só seu tamanho em bytes.
    pub fn run(&mut self, api_key_len: usize) {
        self.state = State::Idle;
        info!(chave_bytes = api_key_len, "jarvis pronta (scaffold)");
    }

    #[allow(dead_code)]
    pub fn state(&self) -> State {
        self.state
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
