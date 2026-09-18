//! Navegação na aba atual do Safari ou Google Chrome.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::str_arg;
use super::web::{normalize_browser, normalize_url};
use crate::tools::{Risk, Tool, ToolError, ToolSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserAction {
    Back,
    Forward,
    Goto,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserTarget {
    Focused,
    Safari,
    Chrome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserRequest {
    action: BrowserAction,
    target: BrowserTarget,
    destination: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserSnapshot {
    frontmost: String,
    window_owners: Vec<String>,
    running: Vec<String>,
}

impl BrowserSnapshot {
    fn choose(&self) -> Result<BrowserTarget, ToolError> {
        if let Some(target) = BrowserTarget::from_app_name(&self.frontmost) {
            return Ok(target);
        }
        for name in &self.window_owners {
            if let Some(target) = BrowserTarget::from_app_name(name) {
                return Ok(target);
            }
        }
        for name in &self.running {
            if let Some(target) = BrowserTarget::from_app_name(name) {
                return Ok(target);
            }
        }
        Err(ToolError::Failed(
            "nenhum navegador aberto (Safari ou Chrome)".into(),
        ))
    }
}

impl BrowserRequest {
    fn from_args(action: BrowserAction, args: &Value) -> Result<Self, ToolError> {
        let object = args
            .as_object()
            .ok_or_else(|| ToolError::InvalidArgs("esperava um objeto JSON".into()))?;
        let browser = match object.get("browser") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.as_str()),
            Some(_) => return Err(ToolError::InvalidArgs("`browser` deve ser texto".into())),
        };
        let target = match normalize_browser(browser)?.as_deref() {
            None => BrowserTarget::Focused,
            Some("Safari") => BrowserTarget::Safari,
            Some("Google Chrome") => BrowserTarget::Chrome,
            Some(other) => {
                return Err(ToolError::InvalidArgs(format!(
                    "navegação na aba atual suporta Safari ou Chrome; recebi {other:?}"
                )))
            }
        };
        let destination = match action {
            BrowserAction::Back | BrowserAction::Forward => None,
            BrowserAction::Goto => Some(normalize_url(str_arg(args, "url")?)?),
            BrowserAction::Search => Some(search_url(str_arg(args, "query")?)?),
        };
        Ok(Self {
            action,
            target,
            destination,
        })
    }
}

fn search_url(raw: &str) -> Result<String, ToolError> {
    let query = raw.trim();
    if query.is_empty() || query.chars().any(char::is_control) {
        return Err(ToolError::InvalidArgs(
            "a pesquisa precisa ter texto".into(),
        ));
    }
    let mut encoded = String::with_capacity(query.len());
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("escrever em String não falha");
        }
    }
    Ok(format!("https://www.google.com/search?q={encoded}"))
}

impl BrowserAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Goto => "goto",
            Self::Search => "search",
        }
    }
}

impl BrowserTarget {
    fn from_app_name(name: &str) -> Option<Self> {
        match name {
            "Safari" => Some(Self::Safari),
            "Google Chrome" => Some(Self::Chrome),
            _ => None,
        }
    }

    fn as_osascript_arg(self) -> &'static str {
        match self {
            Self::Focused => "",
            Self::Safari => "Safari",
            Self::Chrome => "Google Chrome",
        }
    }
}

const BROWSER_PICKER_SCRIPT: &str = r#"ObjC.import("AppKit");
ObjC.import("CoreGraphics");
const workspace = $.NSWorkspace.sharedWorkspace;
const frontmost = ObjC.unwrap(workspace.frontmostApplication.localizedName) || "";
const supported = name => name === "Safari" || name === "Google Chrome";
const windowOwners = [];
try {
	const windowRef = $.CGWindowListCopyWindowInfo(
		$.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements,
		$.kCGNullWindowID
	);
	const windows = ObjC.castRefToObject(windowRef);
	const windowCount = Number(ObjC.unwrap(windows.count));
	for (let index = 0; index < windowCount; index++) {
		const row = windows.objectAtIndex(index);
		const owner = ObjC.unwrap(row.objectForKey("kCGWindowOwnerName")) || "";
		if (supported(owner) && !windowOwners.includes(owner)) windowOwners.push(owner);
	}
} catch (error) {}
const running = [];
const applications = workspace.runningApplications;
const applicationCount = Number(ObjC.unwrap(applications.count));
for (let index = 0; index < applicationCount; index++) {
	const name = ObjC.unwrap(applications.objectAtIndex(index).localizedName) || "";
	if (supported(name) && !running.includes(name)) running.push(name);
}
JSON.stringify({ frontmost, windowOwners, running });"#;

const BROWSER_SCRIPT: &str = r#"on run argv
	set requestedBrowser to item 1 of argv
	set actionName to item 2 of argv
	set destination to item 3 of argv
	set chosenBrowser to requestedBrowser
	if chosenBrowser is not "Safari" and chosenBrowser is not "Google Chrome" then
		return "error:no_browser"
	end if
	tell application "System Events"
		set browserRunning to exists application process chosenBrowser
	end tell
	if browserRunning is false then return "error:not_running:" & chosenBrowser
	if chosenBrowser is "Safari" then
		tell application "Safari"
			if (count of windows) is 0 then return "error:no_window:Safari"
			activate
			if actionName is "goto" or actionName is "search" then
				set URL of current tab of front window to destination
			end if
		end tell
		if actionName is "back" then
			try
				tell application "Safari" to do JavaScript "history.back()" in current tab of front window
			on error
				delay 0.1
				tell application "System Events" to key code 123 using command down
			end try
		else if actionName is "forward" then
			try
				tell application "Safari" to do JavaScript "history.forward()" in current tab of front window
			on error
				delay 0.1
				tell application "System Events" to key code 124 using command down
			end try
		else if actionName is not "goto" and actionName is not "search" then
			return "error:invalid_action"
		end if
	else
		tell application "Google Chrome"
			if (count of windows) is 0 then return "error:no_window:Google Chrome"
			activate
			if actionName is "back" then
				go back active tab of front window
			else if actionName is "forward" then
				go forward active tab of front window
			else if actionName is "goto" or actionName is "search" then
				set URL of active tab of front window to destination
			else
				return "error:invalid_action"
			end if
		end tell
	end if
	return "ok:" & chosenBrowser
end run"#;

fn browser_picker_command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("osascript");
    command
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(BROWSER_PICKER_SCRIPT)
        .kill_on_drop(true);
    command
}

fn osascript_command(request: &BrowserRequest) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("osascript");
    command
        .arg("-e")
        .arg(BROWSER_SCRIPT)
        .arg("--")
        .arg(request.target.as_osascript_arg())
        .arg(request.action.as_str())
        .arg(request.destination.as_deref().unwrap_or(""))
        .kill_on_drop(true);
    command
}

#[cfg(target_os = "macos")]
async fn resolve_focused_browser() -> Result<BrowserTarget, ToolError> {
    use std::time::Duration;

    let output = tokio::time::timeout(Duration::from_secs(5), browser_picker_command().output())
        .await
        .map_err(|_| ToolError::Timeout)?
        .map_err(|error| {
            ToolError::Failed(format!("não consegui consultar os navegadores: {error}"))
        })?;
    if !output.status.success() {
        return Err(ToolError::Failed(
            "não consegui consultar os navegadores abertos".into(),
        ));
    }
    let snapshot: BrowserSnapshot = serde_json::from_slice(&output.stdout).map_err(|_| {
        ToolError::Failed("o sistema devolveu uma lista de navegadores inválida".into())
    })?;
    snapshot.choose()
}

#[cfg(not(target_os = "macos"))]
async fn resolve_focused_browser() -> Result<BrowserTarget, ToolError> {
    Err(ToolError::Failed(
        "browser.* não suportado ainda neste sistema (só macOS por enquanto)".into(),
    ))
}

#[cfg(target_os = "macos")]
async fn run_osascript(request: &BrowserRequest) -> Result<String, ToolError> {
    use std::time::Duration;

    let output = tokio::time::timeout(Duration::from_secs(15), osascript_command(request).output())
        .await
        .map_err(|_| ToolError::Timeout)?
        .map_err(|error| ToolError::Failed(format!("não consegui iniciar osascript: {error}")))?;
    if !output.status.success() {
        return Err(ToolError::Failed(format!(
            "osascript não conseguiu controlar o navegador (código {})",
            output.status.code().unwrap_or(-1)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

#[cfg(not(target_os = "macos"))]
async fn run_osascript(_request: &BrowserRequest) -> Result<String, ToolError> {
    Err(ToolError::Failed(
        "browser.* não suportado ainda neste sistema (só macOS por enquanto)".into(),
    ))
}

fn parse_outcome(request: &BrowserRequest, raw: &str) -> Result<Value, ToolError> {
    let raw = raw.trim();
    if let Some(browser) = raw.strip_prefix("ok:") {
        let browser = safe_browser_name(browser);
        if browser == "navegador" {
            return Err(ToolError::Failed(
                "osascript devolveu um navegador inesperado".into(),
            ));
        }
        return Ok(json!({
            "action": request.action.as_str(),
            "browser": browser,
        }));
    }
    if raw == "error:no_browser" {
        return Err(ToolError::Failed(
            "nenhum navegador aberto (Safari ou Chrome)".into(),
        ));
    }
    if let Some(browser) = raw.strip_prefix("error:not_running:") {
        return Err(ToolError::Failed(format!(
            "{} não está aberto",
            safe_browser_name(browser)
        )));
    }
    if let Some(browser) = raw.strip_prefix("error:no_window:") {
        return Err(ToolError::Failed(format!(
            "{} está sem janela aberta",
            safe_browser_name(browser)
        )));
    }
    if raw == "error:invalid_action" {
        return Err(ToolError::Failed("ação de navegador inválida".into()));
    }
    Err(ToolError::Failed(
        "osascript devolveu uma resposta inesperada".into(),
    ))
}

fn safe_browser_name(raw: &str) -> &'static str {
    match raw.trim() {
        "Safari" => "Safari",
        "Google Chrome" => "Google Chrome",
        _ => "navegador",
    }
}

fn tool_spec(action: BrowserAction) -> ToolSpec {
    let (name, description, properties, required) = match action {
        BrowserAction::Back => (
            "browser.back",
            "Volta uma página na aba atual do navegador indicado ou, se omitido, do navegador aberto mais à frente.",
            json!({ "browser": browser_parameter() }),
            json!([]),
        ),
        BrowserAction::Forward => (
            "browser.forward",
            "Avança uma página na aba atual do navegador indicado ou, se omitido, do navegador aberto mais à frente.",
            json!({ "browser": browser_parameter() }),
            json!([]),
        ),
        BrowserAction::Goto => (
            "browser.goto",
            "Navega a aba atual até um endereço, sem abrir outra aba.",
            json!({
                "url": {
                    "type": "string",
                    "description": "Endereço completo ou domínio, ex.: https://example.com."
                },
                "browser": browser_parameter()
            }),
            json!(["url"]),
        ),
        BrowserAction::Search => (
            "browser.search",
            "Pesquisa no Google usando a aba atual, sem abrir outra aba.",
            json!({
                "query": {
                    "type": "string",
                    "description": "Termos que o usuário quer pesquisar."
                },
                "browser": browser_parameter()
            }),
            json!(["query"]),
        ),
    };
    ToolSpec {
        name: name.into(),
        description: description.into(),
        parameters: json!({
            "type": "object",
            "properties": properties,
            "required": required
        }),
        risk: Risk::Safe,
    }
}

fn browser_parameter() -> Value {
    json!({
        "type": "string",
        "description": "Navegador citado pelo usuário. Omita para usar o navegador em foco ou o mais à frente entre Safari e Chrome abertos."
    })
}

async fn call_action(action: BrowserAction, args: Value) -> Result<Value, ToolError> {
    let mut request = BrowserRequest::from_args(action, &args)?;
    if request.target == BrowserTarget::Focused {
        request.target = resolve_focused_browser().await?;
    }
    let outcome = run_osascript(&request).await?;
    parse_outcome(&request, &outcome)
}

macro_rules! browser_tool {
    ($type:ident, $action:expr) => {
        pub struct $type;

        #[async_trait]
        impl Tool for $type {
            fn spec(&self) -> ToolSpec {
                tool_spec($action)
            }

            async fn call(&self, args: Value) -> Result<Value, ToolError> {
                call_action($action, args).await
            }
        }
    };
}

browser_tool!(BrowserBack, BrowserAction::Back);
browser_tool!(BrowserForward, BrowserAction::Forward);
browser_tool!(BrowserGoto, BrowserAction::Goto);
browser_tool!(BrowserSearch, BrowserAction::Search);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Risk, Tool};

    #[test]
    fn exposes_four_safe_browser_tools_with_expected_arguments() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(BrowserBack),
            Box::new(BrowserForward),
            Box::new(BrowserGoto),
            Box::new(BrowserSearch),
        ];
        let specs: Vec<_> = tools.iter().map(|tool| tool.spec()).collect();

        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.name.as_str())
                .collect::<Vec<_>>(),
            [
                "browser.back",
                "browser.forward",
                "browser.goto",
                "browser.search"
            ]
        );
        assert!(specs.iter().all(|spec| spec.risk == Risk::Safe));
        assert_eq!(specs[0].parameters["required"], serde_json::json!([]));
        assert_eq!(specs[1].parameters["required"], serde_json::json!([]));
        assert_eq!(specs[2].parameters["required"], serde_json::json!(["url"]));
        assert_eq!(
            specs[3].parameters["required"],
            serde_json::json!(["query"])
        );
        for spec in specs {
            assert_eq!(spec.parameters["properties"]["browser"]["type"], "string");
            assert!(!spec.description.is_empty());
        }
    }

    #[test]
    fn parses_shared_browser_aliases_and_defaults_to_focused_browser() {
        let focused = BrowserRequest::from_args(BrowserAction::Back, &json!({})).unwrap();
        assert_eq!(focused.target, BrowserTarget::Focused);

        let safari =
            BrowserRequest::from_args(BrowserAction::Back, &json!({"browser": "safari"})).unwrap();
        assert_eq!(safari.target, BrowserTarget::Safari);

        let chrome =
            BrowserRequest::from_args(BrowserAction::Forward, &json!({"browser": "Google Chrome"}))
                .unwrap();
        assert_eq!(chrome.target, BrowserTarget::Chrome);

        for bad in [json!({"browser": 7}), json!({"browser": "edge"})] {
            assert!(
                matches!(
                    BrowserRequest::from_args(BrowserAction::Back, &bad),
                    Err(ToolError::InvalidArgs(_))
                ),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn prepares_safe_current_tab_destinations() {
        let goto =
            BrowserRequest::from_args(BrowserAction::Goto, &json!({"url": " example.com/path "}))
                .unwrap();
        assert_eq!(
            goto.destination.as_deref(),
            Some("https://example.com/path")
        );

        let search =
            BrowserRequest::from_args(BrowserAction::Search, &json!({"query": " ação & café "}))
                .unwrap();
        assert_eq!(
            search.destination.as_deref(),
            Some("https://www.google.com/search?q=a%C3%A7%C3%A3o%20%26%20caf%C3%A9")
        );

        for (action, bad) in [
            (BrowserAction::Goto, json!({"url": "file:///etc/passwd"})),
            (BrowserAction::Search, json!({"query": "  "})),
            (BrowserAction::Search, json!({"query": 7})),
        ] {
            assert!(
                matches!(
                    BrowserRequest::from_args(action, &bad),
                    Err(ToolError::InvalidArgs(_))
                ),
                "deveria recusar {bad}"
            );
        }
    }

    #[test]
    fn builds_osascript_command_with_user_data_in_separate_arguments() {
        let request = BrowserRequest::from_args(
            BrowserAction::Goto,
            &json!({"browser": "Safari", "url": "example.com/a?x=1&y=2"}),
        )
        .unwrap();
        let command = osascript_command(&request);
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert_eq!(args[0], "-e");
        assert_eq!(
            &args[2..],
            ["--", "Safari", "goto", "https://example.com/a?x=1&y=2"]
        );
        assert!(!args[1].contains("example.com"));
    }

    #[test]
    fn maps_osascript_outcomes_without_echoing_destination() {
        let request = BrowserRequest::from_args(
            BrowserAction::Search,
            &json!({"query": "segredo que não deve voltar no resultado"}),
        )
        .unwrap();
        assert_eq!(
            parse_outcome(&request, "ok:Safari").unwrap(),
            json!({"action": "search", "browser": "Safari"})
        );

        for (raw, expected) in [
            ("error:no_browser", "nenhum navegador aberto"),
            ("error:not_running:Safari", "Safari não está aberto"),
            (
                "error:no_window:Google Chrome",
                "Google Chrome está sem janela aberta",
            ),
        ] {
            let err = parse_outcome(&request, raw).unwrap_err().to_string();
            assert!(err.contains(expected), "{raw}: {err}");
            assert!(!err.contains("segredo"));
        }
    }

    #[test]
    fn chooses_focused_then_frontmost_running_browser_then_clear_error() {
        let focused = BrowserSnapshot {
            frontmost: "Safari".into(),
            window_owners: vec!["Google Chrome".into(), "Safari".into()],
            running: vec!["Safari".into(), "Google Chrome".into()],
        };
        assert_eq!(focused.choose().unwrap(), BrowserTarget::Safari);

        let behind_calculator = BrowserSnapshot {
            frontmost: "Calculator".into(),
            window_owners: vec!["Google Chrome".into(), "Safari".into()],
            running: vec!["Safari".into(), "Google Chrome".into()],
        };
        assert_eq!(behind_calculator.choose().unwrap(), BrowserTarget::Chrome);

        let running_without_visible_window = BrowserSnapshot {
            frontmost: "Terminal".into(),
            window_owners: Vec::new(),
            running: vec!["Safari".into()],
        };
        assert_eq!(
            running_without_visible_window.choose().unwrap(),
            BrowserTarget::Safari
        );

        let none = BrowserSnapshot {
            frontmost: "Calculator".into(),
            window_owners: Vec::new(),
            running: Vec::new(),
        };
        let err = none.choose().unwrap_err().to_string();
        assert!(err.contains("nenhum navegador aberto"), "{err}");
    }
}
