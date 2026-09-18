//! Reflexo: decisões rápidas (~300 ms) com o Jev (TypeSafe.ai) na frente do
//! Gemini Live. Spec: docs/superpowers/specs/2026-09-18-openjarvisbr-v4-reflexo-jev-design.md
//!
//! Regra de ouro: Gemini pensa, Jev julga. O reflexo só executa ações que hoje
//! já não pedem confirmação e só escolhe entre opções listadas.

pub mod decide;
pub mod eye;
pub mod judge;
pub mod questions;
