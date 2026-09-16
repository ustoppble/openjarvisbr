//! `fs.read`, `fs.write` e `fs.list`: arquivos dentro do home do usuário.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;
use tokio::io::AsyncReadExt;

use super::{resolve_in_root, str_arg};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

/// Bytes lidos por padrão em `fs.read`.
pub const READ_DEFAULT_BYTES: u64 = 64 * 1024;
/// Teto de bytes em `fs.read`, mesmo pedindo mais.
pub const READ_MAX_BYTES: u64 = 1024 * 1024;
/// Entradas devolvidas por `fs.list`.
pub const LIST_MAX_ENTRIES: usize = 500;

fn io_error(path: &std::path::Path, e: std::io::Error) -> ToolError {
    ToolError::Failed(format!("{}: {e}", path.display()))
}

pub struct FsRead {
    root: PathBuf,
}

impl FsRead {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for FsRead {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs.read".into(),
            description: "Lê um arquivo de texto dentro da pasta home do usuário e devolve o \
                          conteúdo. Caminho relativo é a partir do home; \"~/\" também vale. \
                          Arquivos grandes vêm truncados."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Arquivo a ler, ex.: \"Documentos/notas.txt\" ou \"~/.zshrc\"."
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Quantos bytes ler no máximo (padrão 65536, teto 1048576)."
                    }
                },
                "required": ["path"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let path = resolve_in_root(&self.root, str_arg(&args, "path")?)?;
        let max = args
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(READ_DEFAULT_BYTES)
            .clamp(1, READ_MAX_BYTES);
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| io_error(&path, e))?;
        if !meta.is_file() {
            return Err(ToolError::InvalidArgs(format!(
                "{} não é um arquivo",
                path.display()
            )));
        }
        let file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| io_error(&path, e))?;
        let mut bytes = Vec::new();
        file.take(max)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| io_error(&path, e))?;
        Ok(json!({
            "path": path.display().to_string(),
            "content": text_without_cut_char(&bytes),
            "size": meta.len(),
            "truncated": meta.len() > bytes.len() as u64,
        }))
    }
}

/// Texto do trecho lido; um caractere que o limite cortou no fim é descartado
/// em vez de virar caractere de substituição.
fn text_without_cut_char(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&bytes[..e.valid_up_to()]),
        _ => String::from_utf8_lossy(bytes),
    }
}

pub struct FsWrite {
    root: PathBuf,
}

impl FsWrite {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for FsWrite {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs.write".into(),
            description: "Escreve texto num arquivo dentro da pasta home do usuário, criando o \
                          arquivo e as pastas que faltarem. Por padrão substitui o conteúdo; \
                          com append=true acrescenta no fim. Pede confirmação antes."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Arquivo a escrever, ex.: \"Documentos/lista.md\"."
                    },
                    "content": {
                        "type": "string",
                        "description": "Texto a gravar no arquivo."
                    },
                    "append": {
                        "type": "boolean",
                        "description": "true acrescenta no fim do arquivo; false (padrão) substitui."
                    }
                },
                "required": ["path", "content"]
            }),
            risk: Risk::Confirm,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let path = resolve_in_root(&self.root, str_arg(&args, "path")?)?;
        let content = str_arg(&args, "content")?;
        let append = args
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if path.is_dir() {
            return Err(ToolError::InvalidArgs(format!(
                "{} é uma pasta",
                path.display()
            )));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| io_error(parent, e))?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&path)
            .await
            .map_err(|e| io_error(&path, e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, content.as_bytes())
            .await
            .map_err(|e| io_error(&path, e))?;
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|e| io_error(&path, e))?;
        Ok(json!({
            "path": path.display().to_string(),
            "bytes_written": content.len(),
            "appended": append,
        }))
    }
}

pub struct FsList {
    root: PathBuf,
}

impl FsList {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Tool for FsList {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "fs.list".into(),
            description: "Lista o que tem numa pasta dentro do home do usuário: nome, tipo \
                          (arquivo, pasta ou link) e tamanho. Sem caminho, lista o home."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Pasta a listar, ex.: \"Downloads\". Vazio ou \"~\" = home."
                    }
                },
                "required": []
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let path = resolve_in_root(&self.root, raw)?;
        let mut dir = tokio::fs::read_dir(&path)
            .await
            .map_err(|e| io_error(&path, e))?;
        let mut entries = Vec::new();
        while let Some(entry) = dir.next_entry().await.map_err(|e| io_error(&path, e))? {
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            let kind_name = if kind.is_symlink() {
                "link"
            } else if kind.is_dir() {
                "pasta"
            } else {
                "arquivo"
            };
            let size = if kind.is_file() {
                entry.metadata().await.map(|m| m.len()).ok()
            } else {
                None
            };
            entries.push((
                entry.file_name().to_string_lossy().into_owned(),
                kind_name,
                size,
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let total = entries.len();
        entries.truncate(LIST_MAX_ENTRIES);
        let items: Vec<_> = entries
            .into_iter()
            .map(|(name, kind, size)| json!({ "name": name, "kind": kind, "size": size }))
            .collect();
        Ok(json!({
            "path": path.display().to_string(),
            "entries": items,
            "total": total,
            "truncated": total > LIST_MAX_ENTRIES,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::TempDir;
    use super::super::OUTSIDE_HOME;
    use super::*;

    #[tokio::test]
    async fn write_read_list_ok() {
        let tmp = TempDir::new();
        let root = tmp.0.clone();
        let w = FsWrite::new(root.clone());
        w.call(json!({"path": "notas/dia.txt", "content": "olá"}))
            .await
            .unwrap();
        w.call(json!({"path": "~/notas/dia.txt", "content": " mundo", "append": true}))
            .await
            .unwrap();

        let r = FsRead::new(root.clone())
            .call(json!({"path": "notas/dia.txt"}))
            .await
            .unwrap();
        assert_eq!(r["content"], "olá mundo");
        assert_eq!(r["truncated"], false);

        let cut = FsRead::new(root.clone())
            .call(json!({"path": "notas/dia.txt", "max_bytes": 3}))
            .await
            .unwrap();
        assert_eq!(cut["content"], "ol");
        assert_eq!(cut["truncated"], true);

        let l = FsList::new(root.clone()).call(json!({})).await.unwrap();
        assert_eq!(l["entries"][0]["name"], "notas");
        assert_eq!(l["entries"][0]["kind"], "pasta");
        let l = FsList::new(root)
            .call(json!({"path": "notas"}))
            .await
            .unwrap();
        assert_eq!(l["entries"][0]["name"], "dia.txt");
        assert_eq!(l["entries"][0]["size"], "olá mundo".len());
    }

    #[tokio::test]
    async fn fora_do_home_e_recusado() {
        let tmp = TempDir::new();
        let inside = tmp.0.join("dentro");
        std::fs::create_dir(&inside).unwrap();
        std::fs::write(tmp.0.join("vizinho.txt"), "segredo").unwrap();

        let r = FsRead::new(inside.clone());
        let w = FsWrite::new(inside.clone());
        let l = FsList::new(inside);
        for bad in ["../vizinho.txt", "/etc/hosts"] {
            assert!(
                matches!(r.call(json!({"path": bad})).await, Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME))
            );
        }
        assert!(matches!(
            w.call(json!({"path": "../novo.txt", "content": "x"})).await,
            Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME)
        ));
        assert!(!tmp.0.join("novo.txt").exists());
        assert!(
            matches!(l.call(json!({"path": ".."})).await, Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME))
        );
        assert!(
            matches!(l.call(json!({"path": "/"})).await, Err(ToolError::InvalidArgs(ref m)) if m.starts_with(OUTSIDE_HOME))
        );
    }

    #[tokio::test]
    async fn erros_claros() {
        let tmp = TempDir::new();
        let r = FsRead::new(tmp.0.clone());
        assert!(matches!(
            r.call(json!({})).await,
            Err(ToolError::InvalidArgs(_))
        ));
        assert!(matches!(
            r.call(json!({"path": ""})).await,
            Err(ToolError::InvalidArgs(_))
        ));
        assert!(matches!(
            r.call(json!({"path": "nao-existe.txt"})).await,
            Err(ToolError::Failed(_))
        ));
        assert!(matches!(
            FsWrite::new(tmp.0.clone())
                .call(json!({"path": "", "content": "x"}))
                .await,
            Err(ToolError::InvalidArgs(_))
        ));
    }
}
