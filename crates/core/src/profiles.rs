//! Perfis de personalidade: cada um reaproveita a identidade padrão
//! (`default_system_prompt`, ver `config.rs`) e acrescenta uma persona em
//! cima — nome do usuário e tom continuam vindo da base; só o papel muda.

use serde::{Deserialize, Serialize};

use crate::config::default_system_prompt;

/// Perfil usado quando nenhum outro está configurado, ou quando o id salvo
/// não corresponde a nenhum perfil (embutido ou customizado).
pub const DEFAULT_PROFILE_ID: &str = "assistant";

fn default_voice() -> String {
    "Puck".to_string()
}

fn default_fx_amount() -> f32 {
    0.35
}

/// Um perfil: identidade, prompt de sistema completo, voz e intensidade do
/// efeito. `voice`/`fx_amount`/`description` têm padrão para perfis
/// customizados minimalistas no config.toml (só `id`, `name` e
/// `system_prompt` obrigatórios).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub system_prompt: String,
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_fx_amount")]
    pub fx_amount: f32,
    /// Allow-list de ferramentas (globs: `fs.*`, `mcp.overclock.*`, `*`).
    /// Ausente num perfil próprio = nenhuma ferramenta.
    #[serde(default)]
    pub tools: Vec<String>,
}

fn globs(list: &[&str]) -> Vec<String> {
    list.iter().map(|g| g.to_string()).collect()
}

/// Os cinco perfis embutidos, com o prompt já resolvido para `user_name`
/// (nome, tom e regra de não responder à live vêm da base padrão).
pub fn builtin_profiles(user_name: Option<&str>) -> Vec<Profile> {
    let base = default_system_prompt(user_name);
    vec![
        Profile {
            id: "assistant".to_string(),
            name: "Assistente pessoal".to_string(),
            description: "O padrão: conversa, ajuda no dia a dia, lembra do que você contou."
                .to_string(),
            system_prompt: base.clone(),
            voice: "Puck".to_string(),
            fx_amount: 0.35,
            tools: globs(&["*"]),
        },
        Profile {
            id: "english_teacher".to_string(),
            name: "Professor de inglês".to_string(),
            description:
                "Ensina inglês do zero, uma frase por vez, só a frase-alvo em inglês."
                    .to_string(),
            system_prompt: format!(
                "{base}\n\n\
Papel agora: você é professor(a) de inglês, ensinando alguém que começa do zero. \
Vá uma frase por vez: diga a frase-alvo SOMENTE em inglês, depois explique em \
português o que ela significa e quando usar, peça para a pessoa repetir em voz \
alta, corrija a pronúncia se precisar e só então avance para a próxima frase. \
Nunca avance sem ela repetir a frase. Não dê aulas longas nem liste várias \
frases de uma vez — isto é voz, uma frase de cada vez."
            ),
            voice: "Aoede".to_string(),
            fx_amount: 0.0,
            tools: Vec::new(),
        },
        Profile {
            id: "therapist".to_string(),
            name: "Terapeuta de apoio".to_string(),
            description:
                "Escuta ativa e perguntas abertas; não diagnostica e orienta ajuda em crise."
                    .to_string(),
            system_prompt: format!(
                "{base}\n\n\
Papel agora: você acolhe com escuta ativa. Faça perguntas abertas, valide o que \
a pessoa sente, dê mais espaço para ela falar do que você. Deixe claro, se ela \
perguntar ou parecer confusa sobre isso, que você NÃO é terapeuta de verdade: \
nunca dê diagnóstico, nunca prescreva remédio ou tratamento, e sempre deixe \
claro que uma conversa com você não substitui acompanhamento profissional. Se \
perceber qualquer sinal de risco (falar em se machucar, em desistir da vida, \
crise grave), acolha sem minimizar e oriente com calma a buscar ajuda agora: no \
Brasil, o CVV atende de graça, 24 horas, pelo 188 (ligação, chat ou e-mail em \
cvv.org.br) — diga esse número com clareza e sugira também procurar alguém de \
confiança ou um serviço de saúde por perto."
            ),
            voice: "Kore".to_string(),
            fx_amount: 0.0,
            tools: Vec::new(),
        },
        Profile {
            id: "business_mentor".to_string(),
            name: "Mentor de negócios".to_string(),
            description: "Direto, provoca com perguntas, foca em decisão e próximo passo."
                .to_string(),
            system_prompt: format!(
                "{base}\n\n\
Papel agora: você é mentor(a) de negócios, direto e sem rodeios. Provoque com \
perguntas que forcem clareza (qual é o número? quem é o cliente? o que trava a \
venda?), desafie suposição fraca e corte enrolação. Sempre termine puxando para \
uma decisão e um próximo passo concreto — nunca deixe a conversa acabar sem um \
combinado do que fazer a seguir."
            ),
            voice: "Charon".to_string(),
            fx_amount: 0.35,
            tools: globs(&["calendar.*", "reminder.*", "mcp.overclick.*"]),
        },
        Profile {
            id: "pair_programmer".to_string(),
            name: "Parceiro de código".to_string(),
            description:
                "Pair programming em voz: pensa alto, pede pra ler o erro, sugere o próximo comando."
                    .to_string(),
            system_prompt: format!(
                "{base}\n\n\
Papel agora: você faz pair programming em voz com quem está codando. Pense em \
voz alta como um par sênior faria, pergunte o que a pessoa já tentou antes de \
sugerir algo, peça para ela ler a mensagem de erro em voz alta quando travar, e \
proponha o próximo comando ou passo concreto a testar — um de cada vez, curto, \
sem ditar bloco de código inteiro em voz."
            ),
            voice: "Fenrir".to_string(),
            fx_amount: 0.35,
            tools: globs(&["fs.*", "shell.*", "mcp.*"]),
        },
    ]
}

/// Resolve o perfil ativo: primeiro entre os customizados do config.toml (o
/// último com o id pedido vence, caso haja repetição), senão entre os
/// embutidos. Sem correspondência em nenhum dos dois, cai no padrão e avisa
/// — nunca deixa a sessão sem persona.
pub fn resolve_profile(id: &str, user_name: Option<&str>, custom: &[Profile]) -> Profile {
    if let Some(found) = custom.iter().rev().find(|p| p.id == id) {
        return found.clone();
    }
    let builtins = builtin_profiles(user_name);
    if let Some(found) = builtins.iter().find(|p| p.id == id) {
        return found.clone();
    }
    tracing::warn!(perfil = id, "perfil desconhecido; usando o padrão");
    builtins
        .into_iter()
        .find(|p| p.id == DEFAULT_PROFILE_ID)
        .expect("o perfil padrão está sempre entre os embutidos")
}

/// Junta embutidos e customizados para listar na UI (menu, select da janela
/// de configurações, `--list-profiles`): um customizado com o mesmo id de um
/// embutido o sobrescreve, mantendo a posição original na lista; ids novos
/// vão para o final, na ordem do config.toml.
pub fn all_profiles(user_name: Option<&str>, custom: &[Profile]) -> Vec<Profile> {
    let mut result = builtin_profiles(user_name);
    for profile in custom {
        match result.iter_mut().find(|p| p.id == profile.id) {
            Some(slot) => *slot = profile.clone(),
            None => result.push(profile.clone()),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_builtin_profiles_load_with_expected_ids() {
        let profiles = builtin_profiles(Some("Maria"));
        assert_eq!(profiles.len(), 5);
        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "assistant",
                "english_teacher",
                "therapist",
                "business_mentor",
                "pair_programmer"
            ]
        );
        for profile in &profiles {
            assert!(profile.system_prompt.contains("Maria"), "{}", profile.id);
            assert!(!profile.name.is_empty());
            assert!(!profile.description.is_empty());
        }
    }

    #[test]
    fn therapist_profile_mentions_no_diagnosis_and_crisis_helpline() {
        let profiles = builtin_profiles(None);
        let therapist = profiles.iter().find(|p| p.id == "therapist").unwrap();
        assert!(therapist.system_prompt.to_lowercase().contains("não substitui"));
        assert!(therapist.system_prompt.contains("188"));
        assert!(therapist.system_prompt.to_lowercase().contains("cvv"));
    }

    #[test]
    fn english_teacher_profile_teaches_one_sentence_at_a_time_in_english() {
        let profiles = builtin_profiles(None);
        let teacher = profiles.iter().find(|p| p.id == "english_teacher").unwrap();
        assert!(teacher.system_prompt.contains("uma frase por vez") || teacher.system_prompt.contains("frase-alvo"));
    }

    #[test]
    fn resolve_profile_finds_builtin_by_id() {
        let profile = resolve_profile("business_mentor", None, &[]);
        assert_eq!(profile.id, "business_mentor");
    }

    #[test]
    fn resolve_profile_prefers_custom_profile_with_same_id() {
        let custom = vec![Profile {
            id: "assistant".to_string(),
            name: "Meu assistente".to_string(),
            description: "versão minha".to_string(),
            system_prompt: "prompt customizado".to_string(),
            voice: "Kore".to_string(),
            fx_amount: 0.1,
            tools: Vec::new(),
        }];
        let profile = resolve_profile("assistant", None, &custom);
        assert_eq!(profile.name, "Meu assistente");
        assert_eq!(profile.system_prompt, "prompt customizado");
    }

    #[test]
    fn resolve_profile_falls_back_to_default_for_unknown_id() {
        let profile = resolve_profile("perfil-que-nao-existe", Some("Maria"), &[]);
        assert_eq!(profile.id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn all_profiles_merges_custom_into_builtins_by_id() {
        let custom = vec![
            Profile {
                id: "assistant".to_string(),
                name: "Assistente v2".to_string(),
                description: String::new(),
                system_prompt: "x".to_string(),
                voice: default_voice(),
                fx_amount: default_fx_amount(),
                tools: Vec::new(),
            },
            Profile {
                id: "meu_perfil".to_string(),
                name: "Perfil novo".to_string(),
                description: String::new(),
                system_prompt: "y".to_string(),
                voice: default_voice(),
                fx_amount: default_fx_amount(),
                tools: Vec::new(),
            },
        ];
        let all = all_profiles(None, &custom);
        assert_eq!(all.len(), 6);
        assert_eq!(all[0].name, "Assistente v2");
        assert_eq!(all.last().unwrap().id, "meu_perfil");
    }

    #[test]
    fn profile_deserializes_from_minimal_toml_with_defaults() {
        let profile: Profile = toml::from_str(
            "id = \"x\"\nname = \"X\"\nsystem_prompt = \"seja X\"\n",
        )
        .unwrap();
        assert_eq!(profile.voice, "Puck");
        assert_eq!(profile.fx_amount, 0.35);
        assert_eq!(profile.description, "");
        assert!(profile.tools.is_empty());
    }

    #[test]
    fn builtin_tool_allow_lists_follow_spec() {
        let tools = |id: &str| resolve_profile(id, None, &[]).tools;
        assert_eq!(tools("assistant"), ["*"]);
        assert_eq!(tools("pair_programmer"), ["fs.*", "shell.*", "mcp.*"]);
        assert_eq!(
            tools("business_mentor"),
            ["calendar.*", "reminder.*", "mcp.overclick.*"]
        );
        assert!(tools("english_teacher").is_empty());
        assert!(tools("therapist").is_empty());
        let custom: Profile = toml::from_str(
            "id = \"x\"\nname = \"X\"\nsystem_prompt = \"y\"\ntools = [\"fs.*\"]\n",
        )
        .unwrap();
        assert_eq!(custom.tools, ["fs.*"]);
    }
}
