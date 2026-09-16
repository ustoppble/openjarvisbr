//! Política de risco: decide se uma ferramenta executa direto (`Safe`) ou
//! pede confirmação (`Confirm`).

use super::{FullAccess, Risk, ToolSpec};

/// Verbos que só leem. Um tool MCP cujo nome traz um deles (e nenhum verbo de
/// escrita) é `Safe`.
const READ_VERBS: &[&str] = &["list", "get", "read", "search", "find", "status"];

/// Verbos que alteram algo. Vencem os de leitura: `list_delete` é `Confirm`.
const WRITE_VERBS: &[&str] = &[
    "create", "delete", "remove", "update", "write", "set", "send", "post", "spawn", "close",
    "claim", "deliver", "release", "submit", "run", "exec", "kill", "move", "rename", "put",
    "patch", "edit", "add", "start", "stop",
];

#[derive(Debug, Clone, Default)]
pub struct Policy {
    full_access: FullAccess,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    /// Política que segue o modo acesso total (ligado = tudo `Safe`).
    pub fn with_full_access(full_access: FullAccess) -> Self {
        Self { full_access }
    }

    /// Risco efetivo de uma spec. Em acesso total tudo é `Safe`; fora dele,
    /// tools `mcp.<server>.<tool>` usam a heurística por nome e as demais
    /// mantêm o risco declarado.
    pub fn risk(&self, spec: &ToolSpec) -> Risk {
        if self.full_access.get() {
            return Risk::Safe;
        }
        match mcp_tool_name(&spec.name) {
            Some(tool) => mcp_risk(tool),
            None => spec.risk,
        }
    }

    pub fn needs_confirmation(&self, spec: &ToolSpec) -> bool {
        self.risk(spec) == Risk::Confirm
    }
}

/// `mcp.overclock.pane_list` → `pane_list`.
fn mcp_tool_name(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("mcp.")?;
    let (_server, tool) = rest.split_once('.')?;
    Some(tool)
}

/// Heurística por nome para tools MCP: na dúvida, `Confirm`.
pub fn mcp_risk(tool: &str) -> Risk {
    let words = words(tool);
    if words.iter().any(|w| WRITE_VERBS.contains(&w.as_str())) {
        return Risk::Confirm;
    }
    if words.iter().any(|w| READ_VERBS.contains(&w.as_str())) {
        return Risk::Safe;
    }
    Risk::Confirm
}

/// Quebra `pane_list`, `task-get`, `listPanes`, `app.observe` em palavras
/// minúsculas.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str, risk: Risk) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: String::new(),
            parameters: json!({}),
            risk,
        }
    }

    #[test]
    fn local_tools_keep_declared_risk() {
        let policy = Policy::new();
        assert_eq!(policy.risk(&spec("fs.read", Risk::Safe)), Risk::Safe);
        assert_eq!(
            policy.risk(&spec("shell.run", Risk::Confirm)),
            Risk::Confirm
        );
        assert!(policy.needs_confirmation(&spec("fs.write", Risk::Confirm)));
        assert!(!policy.needs_confirmation(&spec("app.open", Risk::Safe)));
    }

    #[test]
    fn mcp_read_verbs_are_safe() {
        let policy = Policy::new();
        for name in [
            "mcp.overclick.task_list",
            "mcp.overclick.task_get",
            "mcp.overclock.pane_read",
            "mcp.overclick.task_search",
            "mcp.overclock.handoff_list",
            "mcp.overclock.overclock_status",
            "mcp.docs.findPages",
            "mcp.x.get-user",
        ] {
            // O risco declarado pelo cliente MCP é ignorado: vale o nome.
            assert_eq!(
                policy.risk(&spec(name, Risk::Confirm)),
                Risk::Safe,
                "{name}"
            );
        }
    }

    #[test]
    fn mcp_other_verbs_need_confirmation() {
        let policy = Policy::new();
        for name in [
            "mcp.overclick.task_create",
            "mcp.overclick.task_deliver",
            "mcp.overclock.pane_spawn",
            "mcp.overclock.pane_write",
            "mcp.overclock.handoff_submit",
            "mcp.overclick.list_delete",
            "mcp.overclock.overclock_mission",
            "mcp.overclock.listener",
        ] {
            assert_eq!(
                policy.risk(&spec(name, Risk::Safe)),
                Risk::Confirm,
                "{name}"
            );
        }
    }

    #[test]
    fn malformed_mcp_names_keep_declared_risk() {
        let policy = Policy::new();
        assert_eq!(policy.risk(&spec("mcp.list", Risk::Confirm)), Risk::Confirm);
        assert_eq!(
            policy.risk(&spec("mcpx.a.list", Risk::Confirm)),
            Risk::Confirm
        );
    }

    #[test]
    fn full_access_makes_everything_safe_and_follows_the_toggle() {
        let flag = FullAccess::new(true);
        let policy = Policy::with_full_access(flag.clone());
        for (name, risk) in [
            ("shell.run", Risk::Confirm),
            ("fs.write", Risk::Confirm),
            ("mcp.overclick.task_delete", Risk::Confirm),
            ("fs.read", Risk::Safe),
        ] {
            assert_eq!(policy.risk(&spec(name, risk)), Risk::Safe, "{name}");
        }
        flag.set(false);
        assert_eq!(
            policy.risk(&spec("shell.run", Risk::Confirm)),
            Risk::Confirm
        );
        assert!(policy.needs_confirmation(&spec("mcp.overclick.task_delete", Risk::Safe)));
    }

    #[test]
    fn word_splitting() {
        assert_eq!(words("pane_list"), ["pane", "list"]);
        assert_eq!(words("listPanes"), ["list", "panes"]);
        assert_eq!(words("get-user.info"), ["get", "user", "info"]);
    }
}
