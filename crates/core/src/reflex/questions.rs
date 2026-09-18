//! Tipos do protocolo TypeSafe (`POST /v1/systemone`): perguntas tipadas
//! (choice, score, noul) e respostas com probabilidades.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Uma pergunta ao Jev. `criteria` muda de forma conforme o tipo.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
    Noul {
        instructions: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

/// O que "sim" e "não" significam num `noul`, quando vale explicar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}

impl Question {
    pub fn choice<I, K, V>(instructions: &str, criteria: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::Choice {
            instructions: instructions.to_string(),
            criteria: criteria
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn score<I, S>(instructions: &str, levels: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Score {
            instructions: instructions.to_string(),
            criteria: levels.into_iter().map(Into::into).collect(),
        }
    }

    pub fn noul(instructions: &str, criteria: Option<NoulCriteria>) -> Self {
        Self::Noul {
            instructions: instructions.to_string(),
            criteria,
        }
    }
}

/// Conjunto de perguntas de uma chamada, por id.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Questions(pub BTreeMap<String, Question>);

impl Questions {
    pub fn insert(&mut self, id: &str, question: Question) {
        self.0.insert(id.to_string(), question);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Corpo do pedido.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub state: String,
    pub model: String,
    pub questions: Questions,
}

/// Uma resposta do Jev.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f32>,
        #[serde(default)]
        confidence: f32,
    },
    Score {
        score: f32,
        #[serde(default)]
        probabilities: BTreeMap<String, f32>,
        #[serde(default)]
        confidence: f32,
    },
    Noul {
        noul: f32,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Answers {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub answers: BTreeMap<String, Answer>,
    #[serde(default)]
    pub usage: Usage,
}

impl Answers {
    /// Opção escolhida e a confiança, se a resposta `id` for um `choice`.
    pub fn choice(&self, id: &str) -> Option<(&str, f32)> {
        match self.answers.get(id)? {
            Answer::Choice {
                choice, confidence, ..
            } => Some((choice.as_str(), *confidence)),
            _ => None,
        }
    }

    /// Probabilidade de uma opção específica num `choice`.
    pub fn probability(&self, id: &str, option: &str) -> Option<f32> {
        match self.answers.get(id)? {
            Answer::Choice { probabilities, .. } => probabilities.get(option).copied(),
            _ => None,
        }
    }

    pub fn noul(&self, id: &str) -> Option<f32> {
        match self.answers.get(id)? {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pedido_serializa_no_formato_da_typesafe() {
        let mut q = Questions::default();
        q.insert(
            "intent",
            Question::choice(
                "O que o usuário pede?",
                [
                    ("open_app", "Abrir um aplicativo"),
                    ("none", "Nenhuma ação"),
                ],
            ),
        );
        q.insert(
            "approve",
            Question::noul("O usuário está aprovando um pedido pendente", None),
        );
        q.insert(
            "frustration",
            Question::score("Quão irritado", ["calmo", "irritado"]),
        );
        let body = Request {
            state: "abre o safari".into(),
            model: "jev-latest".into(),
            questions: q,
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["model"], "jev-latest");
        assert_eq!(v["state"], "abre o safari");
        assert_eq!(v["questions"]["intent"]["type"], "choice");
        assert_eq!(
            v["questions"]["intent"]["criteria"]["open_app"],
            "Abrir um aplicativo"
        );
        assert_eq!(v["questions"]["approve"]["type"], "noul");
        assert!(v["questions"]["approve"].get("criteria").is_none());
        assert_eq!(v["questions"]["frustration"]["criteria"][1], "irritado");
    }

    #[test]
    fn resposta_da_doc_faz_parse() {
        let raw = json!({
            "model": "jev-latest",
            "answers": {
                "intent": { "type": "choice", "choice": "open_app",
                            "probabilities": { "open_app": 0.93, "none": 0.07 }, "confidence": 0.93 },
                "approve": { "type": "noul", "noul": 0.12 },
                "frustration": { "type": "score", "score": 1.035,
                                 "legend": {"0": "calmo", "1": "irritado"},
                                 "probabilities": {"0": 0.2, "1": 0.8}, "confidence": 0.8 }
            },
            "usage": { "input_tokens": 312, "output_tokens": 48 }
        });
        let a: Answers = serde_json::from_value(raw).unwrap();
        assert_eq!(a.choice("intent"), Some(("open_app", 0.93)));
        assert_eq!(a.noul("approve"), Some(0.12));
        assert_eq!(a.choice("approve"), None);
        assert_eq!(a.usage.input_tokens, 312);
        assert!(matches!(
            a.answers.get("frustration"),
            Some(Answer::Score { score, .. }) if (*score - 1.035).abs() < 1e-6
        ));
    }

    #[test]
    fn probabilidade_de_uma_opcao() {
        let raw = json!({ "model": "jev-latest", "answers": { "app": { "type": "choice",
            "choice": "Safari", "probabilities": {"Safari": 0.6, "Spotify": 0.4}, "confidence": 0.6 } },
            "usage": { "input_tokens": 1, "output_tokens": 1 } });
        let a: Answers = serde_json::from_value(raw).unwrap();
        assert_eq!(a.probability("app", "Spotify"), Some(0.4));
        assert_eq!(a.probability("app", "Zoom"), None);
    }
}
