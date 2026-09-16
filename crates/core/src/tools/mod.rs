//! Framework de ferramentas: o contrato que as tools locais, de sistema e MCP
//! implementam, o `Registry` que as guarda e a política de risco que decide se
//! uma chamada executa direto ou pede confirmação.

pub mod local;
pub mod policy;
pub mod registry;
pub mod system;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use policy::Policy;
pub use registry::Registry;

/// Modo acesso total, compartilhado entre a política e as ferramentas que
/// mexem no disco: ligado, nada pede confirmação e `fs.*` aceita qualquer
/// caminho. Clonar compartilha o mesmo valor, então ligar/desligar vale na
/// hora, sem refazer o registro.
#[derive(Debug, Clone, Default)]
pub struct FullAccess(Arc<AtomicBool>);

impl FullAccess {
    pub fn new(on: bool) -> Self {
        Self(Arc::new(AtomicBool::new(on)))
    }

    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Relaxed);
    }
}

/// Risco de uma ferramenta. `Confirm` pede "confirma?" antes de executar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Safe,
    Confirm,
}

/// Descrição de uma ferramenta, no formato que vira `functionDeclarations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema dos argumentos.
    pub parameters: serde_json::Value,
    pub risk: Risk,
}

/// Pedido de execução vindo do modelo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

/// Resultado devolvido ao modelo. `error` preenchido = a chamada falhou.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(id: impl Into<String>, output: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            output,
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            output: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }
}

/// Falhas de uma ferramenta. As mensagens voltam ao modelo, então nunca
/// carregam tokens ou endereços.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("ferramenta desconhecida: {0}")]
    NotFound(String),
    #[error("ferramenta não permitida neste perfil: {0}")]
    NotAllowed(String),
    #[error("argumentos inválidos: {0}")]
    InvalidArgs(String),
    #[error("negado pelo usuário")]
    Denied,
    #[error("tempo esgotado")]
    Timeout,
    #[error("falha na execução: {0}")]
    Failed(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}
