//! `app.open`: abre um aplicativo pelo nome.

#[cfg(any(target_os = "macos", test))]
use std::collections::BTreeSet;
#[cfg(any(target_os = "macos", test))]
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use super::str_arg;
#[cfg(any(target_os = "macos", test))]
use crate::reflex::decide::normalize;
#[cfg(any(target_os = "macos", test))]
use crate::reflex::eye::Inventory;
#[cfg(target_os = "macos")]
use crate::reflex::eye::{default_app_dirs, scan_app_dirs};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

pub struct AppOpen;

#[async_trait]
impl Tool for AppOpen {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "app.open".into(),
            description: "Abre um aplicativo instalado no computador pelo nome \
                          (ex.: \"Safari\", \"Spotify\", \"Visual Studio Code\")."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Nome do aplicativo como aparece no sistema, ex.: \"Safari\"."
                    }
                },
                "required": ["name"]
            }),
            risk: Risk::Safe,
        }
    }

    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let name = validate_name(str_arg(&args, "name")?)?;
        if !open_app(name).await? {
            return Err(ToolError::Failed(format!(
                "não encontrei o aplicativo \"{name}\""
            )));
        }
        Ok(json!({ "opened": name }))
    }
}

fn validate_name(name: &str) -> Result<&str, ToolError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ToolError::InvalidArgs("nome do aplicativo vazio".into()));
    }
    if name.starts_with('-') || name.contains(['\n', '\r', '\0']) {
        return Err(ToolError::InvalidArgs(format!(
            "nome de aplicativo inválido: {name}"
        )));
    }
    Ok(name)
}

async fn command_succeeds(
    mut command: tokio::process::Command,
    name: &str,
) -> Result<bool, ToolError> {
    command
        .output()
        .await
        .map(|output| output.status.success())
        .map_err(|e| ToolError::Failed(format!("não consegui abrir \"{name}\": {e}")))
}

#[cfg(target_os = "macos")]
async fn open_app(name: &str) -> Result<bool, ToolError> {
    // Mantém o caminho rápido e o comportamento antigo para nomes canônicos.
    if command_succeeds(open_command(name)?, name).await? {
        return Ok(true);
    }
    let Some(path) = resolve_macos_app(name).await? else {
        return Ok(false);
    };
    command_succeeds(open_path_command(&path), name).await
}

#[cfg(not(target_os = "macos"))]
async fn open_app(name: &str) -> Result<bool, ToolError> {
    command_succeeds(open_command(name)?, name).await
}

#[cfg(target_os = "macos")]
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(target_os = "macos")]
async fn resolve_macos_app(name: &str) -> Result<Option<PathBuf>, ToolError> {
    let app_dirs = default_app_dirs();
    let scan_dirs = app_dirs.clone();
    let inventory =
        tokio::task::spawn_blocking(move || Inventory::new(scan_app_dirs(&scan_dirs), vec![]))
            .await
            .map_err(|e| {
                ToolError::Failed(format!(
                    "não consegui listar os aplicativos instalados: {e}"
                ))
            })?;
    let output = tokio::process::Command::new(LSREGISTER)
        .arg("-dump")
        .output()
        .await
        .map_err(|e| ToolError::Failed(format!("não consegui consultar o Launch Services: {e}")))?;
    if !output.status.success() {
        return Ok(None);
    }
    let dump = String::from_utf8_lossy(&output.stdout);
    Ok(resolve_from_inventory(name, &inventory, &app_dirs, &dump).filter(|path| path.is_dir()))
}

#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct LaunchServicesApp {
    path: PathBuf,
    name: String,
    localized_names: Vec<String>,
}

#[cfg(any(target_os = "macos", test))]
fn resolve_from_inventory(
    name: &str,
    inventory: &Inventory,
    app_dirs: &[PathBuf],
    dump: &str,
) -> Option<PathBuf> {
    // O registro também contém helpers e placeholders. Só um bundle que o
    // inventário encontrou diretamente numa raiz de apps pode ser escolhido.
    let requested = normalize(name);
    if requested.is_empty() {
        return None;
    }
    let installed: BTreeSet<String> = inventory
        .installed_apps
        .iter()
        .map(|app| normalize(&app.name))
        .collect();
    let mut exact = BTreeSet::new();
    let mut prefix = BTreeSet::new();

    for app in parse_launch_services_dump(dump) {
        let in_inventory_root = app_dirs
            .iter()
            .any(|root| app.path.parent() == Some(root.as_path()));
        let path_name = app
            .path
            .file_stem()
            .and_then(|name| name.to_str())
            .map(normalize)
            .unwrap_or_default();
        if !in_inventory_root
            || app.path.extension().and_then(|ext| ext.to_str()) != Some("app")
            || (!installed.contains(&normalize(&app.name)) && !installed.contains(&path_name))
        {
            continue;
        }
        for label in std::iter::once(&app.name).chain(app.localized_names.iter()) {
            let candidate = normalize(label);
            if candidate == requested {
                exact.insert(app.path.clone());
            } else if requested.chars().count() >= 3
                && (candidate.starts_with(&requested)
                    || candidate
                        .split_whitespace()
                        .any(|word| word.starts_with(&requested)))
            {
                prefix.insert(app.path.clone());
            }
        }
    }

    unique_path(exact).or_else(|| unique_path(prefix))
}

#[cfg(any(target_os = "macos", test))]
fn unique_path(paths: BTreeSet<PathBuf>) -> Option<PathBuf> {
    if paths.len() == 1 {
        paths.into_iter().next()
    } else {
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_launch_services_dump(dump: &str) -> Vec<LaunchServicesApp> {
    fn push_current(apps: &mut Vec<LaunchServicesApp>, current: &mut LaunchServicesApp) {
        if !current.name.is_empty() && !current.path.as_os_str().is_empty() {
            apps.push(std::mem::take(current));
        }
    }

    let mut apps = Vec::new();
    let mut current = LaunchServicesApp::default();
    for line in dump.lines() {
        if line.starts_with("---") {
            push_current(&mut apps, &mut current);
        } else if let Some(path) = registry_value(line, "path:") {
            current.path = PathBuf::from(path);
        } else if let Some(name) = registry_value(line, "name:") {
            current.name = name.to_string();
        } else if current.localized_names.is_empty() && line.starts_with("localizedNames:") {
            current.localized_names = localized_values(line);
        }
    }
    push_current(&mut apps, &mut current);
    apps
}

#[cfg(any(target_os = "macos", test))]
fn registry_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let value = line.strip_prefix(field)?.trim();
    Some(
        value
            .rsplit_once(" (0x")
            .map(|(value, _)| value)
            .unwrap_or(value),
    )
}

#[cfg(any(target_os = "macos", test))]
fn localized_values(line: &str) -> Vec<String> {
    let mut rest = line.strip_prefix("localizedNames:").unwrap_or_default();
    let mut values = Vec::new();
    while let Some(start) = rest.find(" = \"") {
        rest = &rest[start + 4..];
        let mut escaped = false;
        let Some(end) = rest.char_indices().find_map(|(index, ch)| {
            if escaped {
                escaped = false;
                None
            } else if ch == '\\' {
                escaped = true;
                None
            } else if ch == '"' {
                Some(index)
            } else {
                None
            }
        }) else {
            break;
        };
        values.push(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"));
        rest = &rest[end + 1..];
    }
    values
}

#[cfg(target_os = "macos")]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    let mut cmd = tokio::process::Command::new("open");
    cmd.arg("-a").arg(name);
    Ok(cmd)
}

#[cfg(target_os = "macos")]
fn open_path_command(path: &std::path::Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("open");
    cmd.arg(path);
    cmd
}

#[cfg(windows)]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    if name.contains(['&', '|', '<', '>', '^', '"']) {
        return Err(ToolError::InvalidArgs(format!(
            "nome de aplicativo inválido: {name}"
        )));
    }
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "start", ""]).arg(name);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_command(name: &str) -> Result<tokio::process::Command, ToolError> {
    let mut cmd = tokio::process::Command::new("gtk-launch");
    cmd.arg(name);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflex::eye::AppEntry;

    fn inventory(names: &[&str]) -> Inventory {
        Inventory::new(
            names.iter().map(|name| AppEntry::installed(name)).collect(),
            vec![],
        )
    }

    #[tokio::test]
    async fn recusa_nome_vazio_ou_flag() {
        for bad in [
            json!({}),
            json!({"name": "  "}),
            json!({"name": "-n Safari"}),
        ] {
            assert!(matches!(
                AppOpen.call(bad).await,
                Err(ToolError::InvalidArgs(_))
            ));
        }
    }

    #[test]
    fn resolve_nome_localizado_para_caminho_do_app_instalado() {
        let dump = r#"
--------------------------------------------------------------------------------
path:                       /System/Applications/Calculator.app (0x1338)
name:                       Calculator
localizedNames:             "en_GB" = "Calculator", "pt_BR" = "Calculadora"
identifier:                 com.apple.calculator
--------------------------------------------------------------------------------
path:                       /tmp/Placeholder/Calculator.app (0x9999)
name:                       Calculator
localizedNames:             "pt_BR" = "Calculadora"
"#;

        let installed = inventory(&["Calculator"]);
        let roots = [PathBuf::from("/System/Applications")];
        let target = Some(PathBuf::from("/System/Applications/Calculator.app"));
        assert_eq!(
            resolve_from_inventory("Calculadora", &installed, &roots, dump),
            target
        );
        assert_eq!(
            resolve_from_inventory("Calculad", &installed, &roots, dump),
            target
        );
    }

    #[test]
    fn nao_escolhe_prefixo_localizado_ambiguo_ou_app_fora_do_inventario() {
        let dump = r#"
--------------------------------------------------------------------------------
path:                       /Applications/Calculator One.app (0x1)
name:                       Calculator One
localizedNames:             "pt_BR" = "Calculadora Um"
--------------------------------------------------------------------------------
path:                       /Applications/Calculator Two.app (0x2)
name:                       Calculator Two
localizedNames:             "pt_BR" = "Calculadora Dois"
--------------------------------------------------------------------------------
path:                       /Applications/Calculator Rogue.app (0x3)
name:                       Calculator Rogue
localizedNames:             "pt_BR" = "Calculadora Secreta"
"#;
        let installed = inventory(&["Calculator One", "Calculator Two"]);

        let roots = [PathBuf::from("/Applications")];
        assert_eq!(
            resolve_from_inventory("Calculadora", &installed, &roots, dump),
            None
        );
        assert_eq!(
            resolve_from_inventory("Calculadora Secreta", &installed, &roots, dump),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "abre a aplicação real; execute manualmente"]
    async fn smoke_abre_nomes_localizado_e_canonico() {
        let localized = AppOpen.call(json!({"name": "Calculadora"})).await.unwrap();
        println!("app.open Calculadora => {localized}");
        assert_eq!(localized, json!({"opened": "Calculadora"}));

        let canonical = AppOpen.call(json!({"name": "Safari"})).await.unwrap();
        println!("app.open Safari => {canonical}");
        assert_eq!(canonical, json!({"opened": "Safari"}));

        let missing = AppOpen
            .call(json!({"name": "Aplicativo Que Não Existe JRV 84"}))
            .await;
        println!("app.open inexistente => {missing:?}");
        assert!(
            matches!(missing, Err(ToolError::Failed(message)) if message.contains("não encontrei"))
        );
    }
}
