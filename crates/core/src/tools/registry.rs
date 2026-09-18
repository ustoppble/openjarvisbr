//! Registro de ferramentas e allow-list por perfil (globs `fs.*`,
//! `mcp.overclock.*`).

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{Tool, ToolCall, ToolError, ToolResult, ToolSpec};

/// Ferramentas disponíveis, indexadas pelo nome. Clonar é barato: as tools
/// ficam atrás de `Arc`, e um registro filtrado compartilha as mesmas
/// instâncias.
#[derive(Clone, Default)]
pub struct Registry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra uma ferramenta. Um nome repetido substitui a anterior.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.spec().name;
        self.tools.insert(name, Arc::from(tool));
    }

    /// Specs de todas as ferramentas, em ordem alfabética de nome.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.values().map(|tool| tool.spec()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|tool| tool.as_ref())
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Registro só com as ferramentas cujo nome casa com algum glob do perfil.
    /// Lista vazia = nenhuma ferramenta.
    pub fn filter_for_profile<S: AsRef<str>>(&self, globs: &[S]) -> Registry {
        let tools = self
            .tools
            .iter()
            .filter(|(name, _)| globs.iter().any(|glob| glob_matches(glob.as_ref(), name)))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        Registry { tools }
    }

    /// Executa uma chamada e empacota o resultado para o modelo.
    pub async fn call(&self, call: &ToolCall) -> ToolResult {
        let Some(tool) = self.tools.get(&call.name) else {
            return ToolResult::err(&call.id, ToolError::NotFound(call.name.clone()).to_string());
        };
        match tool.call(call.args.clone()).await {
            Ok(output) => ToolResult::ok(&call.id, output),
            Err(error) => ToolResult::err(&call.id, error.to_string()),
        }
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Glob simples sobre nomes de ferramenta: `*` casa qualquer sequência
/// (inclusive vazia e com pontos); o resto é literal.
pub fn glob_matches(glob: &str, name: &str) -> bool {
    let glob = glob.trim();
    let parts: Vec<&str> = glob.split('*').collect();
    if parts.len() == 1 {
        return glob == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    if !name.ends_with(last) {
        return false;
    }
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(at) => rest = &rest[at + middle.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Risk;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct Echo(&'static str);

    #[async_trait]
    impl Tool for Echo {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.0.to_string(),
                description: "devolve os argumentos".to_string(),
                parameters: json!({"type": "object"}),
                risk: Risk::Safe,
            }
        }

        async fn call(&self, args: Value) -> Result<Value, ToolError> {
            if args.get("fail").is_some() {
                return Err(ToolError::InvalidArgs("fail".to_string()));
            }
            Ok(args)
        }
    }

    fn registry(names: &[&'static str]) -> Registry {
        let mut registry = Registry::new();
        for name in names {
            registry.register(Box::new(Echo(name)));
        }
        registry
    }

    const ALL: &[&str] = &[
        "app.open",
        "fs.list",
        "fs.read",
        "fs.write",
        "mcp.overclick.task_list",
        "mcp.overclock.pane_spawn",
        "shell.run",
    ];

    #[test]
    fn register_specs_and_get() {
        let registry = registry(&["fs.read", "app.open"]);
        assert_eq!(registry.len(), 2);
        let names: Vec<_> = registry.specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["app.open", "fs.read"]);
        assert_eq!(registry.get("fs.read").unwrap().spec().name, "fs.read");
        assert!(registry.get("fs.write").is_none());
    }

    #[test]
    fn register_same_name_replaces() {
        let registry = registry(&["fs.read", "fs.read"]);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn local_browser_tools_are_registered() {
        let mut registry = Registry::new();
        for tool in crate::tools::local::all(crate::tools::FullAccess::default()) {
            registry.register(tool);
        }
        for name in [
            "browser.back",
            "browser.forward",
            "browser.goto",
            "browser.search",
        ] {
            assert!(registry.get(name).is_some(), "{name} não foi registrado");
        }
    }

    #[test]
    fn filter_for_profile_by_globs() {
        let registry = registry(ALL);
        let code = registry.filter_for_profile(&["fs.*", "shell.*", "mcp.*"]);
        assert_eq!(
            code.names(),
            [
                "fs.list",
                "fs.read",
                "fs.write",
                "mcp.overclick.task_list",
                "mcp.overclock.pane_spawn",
                "shell.run"
            ]
        );
        let mentor = registry.filter_for_profile(&["agenda.*", "mcp.overclick.*"]);
        assert_eq!(mentor.names(), ["mcp.overclick.task_list"]);
        assert_eq!(registry.filter_for_profile(&["*"]).len(), ALL.len());
        assert!(registry.filter_for_profile::<&str>(&[]).is_empty());
        assert_eq!(
            registry.filter_for_profile(&["fs.read"]).names(),
            ["fs.read"]
        );
        // O original não muda.
        assert_eq!(registry.len(), ALL.len());
    }

    #[test]
    fn glob_rules() {
        assert!(glob_matches("fs.*", "fs.read"));
        assert!(!glob_matches("fs.*", "fsx.read"));
        assert!(!glob_matches("fs.*", "fs"));
        assert!(glob_matches("mcp.overclock.*", "mcp.overclock.pane_spawn"));
        assert!(!glob_matches("mcp.overclock.*", "mcp.overclick.task_list"));
        assert!(glob_matches("mcp.*.task_*", "mcp.overclick.task_list"));
        assert!(glob_matches("*.read", "fs.read"));
        assert!(glob_matches("*", "qualquer.coisa"));
        assert!(glob_matches("fs.read", "fs.read"));
        assert!(!glob_matches("fs.read", "fs.reader"));
        assert!(!glob_matches("a*a", "a"));
    }

    #[tokio::test]
    async fn call_wraps_ok_error_and_unknown() {
        let registry = registry(&["fs.read"]);
        let ok = registry
            .call(&ToolCall {
                id: "1".into(),
                name: "fs.read".into(),
                args: json!({"path": "x"}),
            })
            .await;
        assert_eq!(ok, ToolResult::ok("1", json!({"path": "x"})));

        let failed = registry
            .call(&ToolCall {
                id: "2".into(),
                name: "fs.read".into(),
                args: json!({"fail": true}),
            })
            .await;
        assert_eq!(failed.id, "2");
        assert!(failed.error.unwrap().contains("argumentos inválidos"));

        let unknown = registry
            .call(&ToolCall {
                id: "3".into(),
                name: "nope".into(),
                args: Value::Null,
            })
            .await;
        assert!(unknown.error.unwrap().contains("desconhecida"));
    }
}
