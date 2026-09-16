//! Ferramentas locais: as mãos básicas do Jarvis no computador — abrir app e
//! site, rodar comando e mexer em arquivos. Tudo que toca o disco fica preso
//! a uma raiz (o home do usuário em produção, um diretório temporário nos
//! testes): caminho que resolve para fora dela é recusado.

use std::path::{Component, Path, PathBuf};

use super::{Tool, ToolError};

mod app;
mod fs;
mod shell;
mod web;

pub use app::AppOpen;
pub use fs::{FsList, FsRead, FsWrite};
pub use shell::ShellRun;
pub use web::WebOpen;

/// Todas as ferramentas locais, presas ao home do usuário.
pub fn all() -> Vec<Box<dyn Tool>> {
    let home = home_dir();
    vec![
        Box::new(AppOpen),
        Box::new(WebOpen),
        Box::new(ShellRun::new(home.clone())),
        Box::new(FsRead::new(home.clone())),
        Box::new(FsWrite::new(home.clone())),
        Box::new(FsList::new(home)),
    ]
}

/// Home do usuário (`HOME`, ou `USERPROFILE` no Windows). Sem nenhum dos dois,
/// devolve um caminho vazio: toda resolução falha e as ferramentas recusam.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Lê um argumento string obrigatório.
pub(crate) fn str_arg<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArgs(format!("faltou o parâmetro \"{key}\" (texto)")))
}

/// Mensagem de caminho recusado por sair da raiz.
pub(crate) const OUTSIDE_HOME: &str = "só posso mexer dentro da pasta home";

/// Resolve `input` dentro de `root` e garante que o resultado não escapa dela.
///
/// Aceita caminho relativo (à raiz), absoluto e `~`/`~/...`. O trecho que já
/// existe é canonicalizado (resolve symlinks); o que ainda não existe (arquivo
/// a criar) não pode conter `..`. O caminho final precisa começar pela raiz
/// canonicalizada.
pub(crate) fn resolve_in_root(root: &Path, input: &str) -> Result<PathBuf, ToolError> {
    let root = root
        .canonicalize()
        .map_err(|_| ToolError::Failed("pasta home não encontrada".into()))?;
    let input = input.trim();
    let joined = if input.is_empty() || input == "~" {
        root.clone()
    } else if let Some(rest) = input.strip_prefix("~/") {
        root.join(rest)
    } else {
        root.join(input)
    };

    // Sobe até o ancestral que existe e canonicaliza só ele.
    let mut existing = joined.as_path();
    let mut pending: Vec<&std::ffi::OsStr> = Vec::new();
    let canonical = loop {
        match existing.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                let name = existing
                    .file_name()
                    .ok_or_else(|| ToolError::InvalidArgs(format!("caminho inválido: {input}")))?;
                pending.push(name);
                existing = existing
                    .parent()
                    .ok_or_else(|| ToolError::InvalidArgs(format!("caminho inválido: {input}")))?;
            }
        }
    };
    if joined
        .strip_prefix(existing)
        .map(|rest| {
            rest.components()
                .any(|c| !matches!(c, Component::Normal(_)))
        })
        .unwrap_or(true)
    {
        return Err(ToolError::InvalidArgs(format!("caminho inválido: {input}")));
    }
    let mut resolved = canonical;
    for name in pending.into_iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(&root) {
        return Err(ToolError::InvalidArgs(format!(
            "{OUTSIDE_HOME}: \"{input}\""
        )));
    }
    Ok(resolved)
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Diretório temporário único, apagado no drop.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ojbr-tools-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir.canonicalize().unwrap())
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDir;
    use super::*;
    use crate::tools::Risk;

    #[test]
    fn resolve_aceita_relativo_til_e_inexistente() {
        let tmp = TempDir::new();
        std::fs::create_dir(tmp.0.join("a")).unwrap();
        assert_eq!(resolve_in_root(&tmp.0, "a").unwrap(), tmp.0.join("a"));
        assert_eq!(resolve_in_root(&tmp.0, "~/a").unwrap(), tmp.0.join("a"));
        assert_eq!(resolve_in_root(&tmp.0, "").unwrap(), tmp.0);
        assert_eq!(
            resolve_in_root(&tmp.0, "a/novo/x.txt").unwrap(),
            tmp.0.join("a/novo/x.txt")
        );
        let abs = tmp.0.join("a");
        assert_eq!(resolve_in_root(&tmp.0, abs.to_str().unwrap()).unwrap(), abs);
    }

    #[test]
    fn resolve_recusa_fora_da_raiz() {
        let tmp = TempDir::new();
        std::fs::create_dir(tmp.0.join("a")).unwrap();
        for bad in [
            "..",
            "../fora.txt",
            "/etc/hosts",
            "/",
            "a/../../x",
            "a/nao/../../../x",
        ] {
            assert!(
                matches!(resolve_in_root(&tmp.0, bad), Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME) || m.starts_with("caminho inválido")),
                "deveria recusar {bad}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolve_recusa_symlink_para_fora() {
        let tmp = TempDir::new();
        std::os::unix::fs::symlink("/", tmp.0.join("raiz")).unwrap();
        assert!(matches!(
            resolve_in_root(&tmp.0, "raiz/etc"),
            Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME) || m.starts_with("caminho inválido")
        ));
    }

    #[test]
    fn all_tem_os_seis_nomes_e_riscos() {
        let specs: Vec<_> = all().iter().map(|t| t.spec()).collect();
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "app.open",
                "web.open",
                "shell.run",
                "fs.read",
                "fs.write",
                "fs.list"
            ]
        );
        for s in &specs {
            let want = matches!(s.name.as_str(), "shell.run" | "fs.write");
            assert_eq!(s.risk == Risk::Confirm, want, "{}", s.name);
            assert_eq!(s.parameters["type"], "object", "{}", s.name);
            assert!(!s.description.is_empty());
        }
    }
}
