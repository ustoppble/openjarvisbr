#!/usr/bin/env python3
"""MCP stdio que transforma o trace do Jarvis em findings acionáveis.

O parsing semântico pertence a ``tools.jarvis_trace_report``. Este módulo só
adapta aquele resultado para JSON, localiza linhas de evidência e agrupa
findings em contratos de tarefa.
"""

from __future__ import annotations

import json
import os
import re
import stat
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import IO, Any

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools import jarvis_trace_report as trace_report


DEFAULT_TRACE = Path("/tmp/jarvis-trace.log")
PROTOCOL_VERSION = "2025-06-18"

APP_MISSING = re.compile(
    r"unable to find application named\s+['\"]([^'\"]+)['\"]", re.I
)
URL = re.compile(r"https?://[^\s\"']+", re.I)
IPV4 = re.compile(r"\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}(?::[0-9]+)?\b")
IPV6 = re.compile(r"\b(?:[A-Fa-f0-9]{1,4}:){2,}[A-Fa-f0-9:]+\b")
HOSTNAME = re.compile(
    r"\b(?:[a-z0-9-]+\.)+(?:com|net|org|io|dev|app|sh|cloud|local|invalid)(?::[0-9]+)?\b",
    re.I,
)
BEARER = re.compile(r"\b(bearer\s+)[^\s,\"']+", re.I)
SENSITIVE_ASSIGNMENT = re.compile(
    r"\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|password|senha|secret|authorization|cookie|host)"
    r"\s*[:=]\s*(?:\"[^\"]*\"|'[^']*'|[^\s,]+)",
    re.I,
)
LONG_SECRET = re.compile(r"\b[A-Za-z0-9_+/=-]{24,}\b")
TOOL_IN_DUPLICATE = re.compile(r"`([^`]+)`")
MCP_BLOCK = re.compile(
    r"(?ms)^\s*\[\[mcp_servers\]\]\s*(.*?)(?=^\s*\[|\Z)"
)
OBSERVER_NAME = re.compile(r"(?m)^\s*name\s*=\s*['\"]observer['\"]\s*(?:#.*)?$")


class ObserverError(Exception):
    """Erro seguro para voltar pelo MCP, sem caminho nem conteúdo do log."""


@dataclass(frozen=True)
class SourceLine:
    number: int
    timestamp: str | None
    message: str
    traced: bool


@dataclass
class EventMetrics:
    reflex_actions: int = 0
    reflex_learns: int = 0
    deduplicated: int = 0
    latencies: list[int] = field(default_factory=list)


@dataclass
class Session:
    lines: list[str]
    turns: list[trace_report.Turn]
    source: list[SourceLine]
    events: EventMetrics


def redact(text: str) -> str:
    """Remove credenciais e endereços antes de qualquer valor virar output."""
    safe = trace_report.clean(trace_report.ANSI.sub("", text))
    safe = BEARER.sub(lambda match: f"{match.group(1)}***", safe)
    safe = SENSITIVE_ASSIGNMENT.sub(lambda match: f"{match.group(1)}=***", safe)
    safe = URL.sub("***", safe)
    safe = IPV4.sub("***", safe)
    safe = IPV6.sub("***", safe)
    safe = HOSTNAME.sub("***", safe)
    return LONG_SECRET.sub("***", safe)


def safe_tool(name: str | None) -> str:
    value = redact((name or "tool").strip())
    return re.sub(r"[^A-Za-z0-9._*:-]+", "_", value)[:80] or "tool"


def safe_log_path(value: str | Path) -> Path:
    try:
        path = Path(value).expanduser().resolve(strict=True)
    except (OSError, RuntimeError):
        raise ObserverError("não foi possível ler o log") from None
    lowered = str(path).lower()
    name = path.name.lower()
    if (
        "/.secrets/" in lowered
        or name.startswith(".env")
        or "credential" in name
        or name.endswith(".pem")
        or name.endswith(".key")
    ):
        raise ObserverError("caminho de log não permitido")
    if not path.is_file():
        raise ObserverError("não foi possível ler o log")
    return path


def read_lines(path: str | Path) -> list[str]:
    safe = safe_log_path(path)
    try:
        return safe.read_text(encoding="utf-8", errors="replace").splitlines(keepends=True)
    except OSError:
        raise ObserverError("não foi possível ler o log") from None


def index_source(lines: list[str]) -> list[SourceLine]:
    indexed: list[SourceLine] = []
    previous_timestamp: str | None = None
    for number, raw in enumerate(lines, 1):
        cleaned = trace_report.ANSI.sub("", raw.rstrip("\n"))
        match = trace_report.LINE.match(cleaned)
        if match:
            previous_timestamp = match.group("ts")
            indexed.append(
                SourceLine(
                    number=number,
                    timestamp=previous_timestamp,
                    message=trace_report.clean(match.group("msg")),
                    traced=True,
                )
            )
        else:
            indexed.append(
                SourceLine(
                    number=number,
                    timestamp=previous_timestamp,
                    message=trace_report.clean(cleaned),
                    traced=False,
                )
            )
    return indexed


def parse_events(lines: list[str]) -> EventMetrics:
    metrics = EventMetrics()
    for raw in lines:
        fields = raw.split()
        if len(fields) < 2:
            continue
        kind = fields[1]
        if kind == "reflex_act":
            metrics.reflex_actions += 1
            for value in reversed(fields):
                if value.endswith("ms") and value[:-2].isdigit():
                    metrics.latencies.append(int(value[:-2]))
                    break
        elif kind == "reflex_learn":
            metrics.reflex_learns += 1
        elif kind == "tool_dedup_reflex":
            metrics.deduplicated += 1
    return metrics


def load_session(
    trace_path: str | Path = DEFAULT_TRACE,
    events_path: str | Path | None = None,
) -> Session:
    lines = read_lines(trace_path)
    turns = trace_report.parse(lines, None)
    events = parse_events(read_lines(events_path)) if events_path else EventMetrics()
    return Session(lines=lines, turns=turns, source=index_source(lines), events=events)


def with_z(timestamp: str | None) -> str | None:
    if timestamp is None:
        return None
    return timestamp if timestamp.endswith("Z") else f"{timestamp}Z"


def entries_for_turn(session: Session, index: int) -> list[SourceLine]:
    turn = session.turns[index]
    next_start = (
        session.turns[index + 1].started if index + 1 < len(session.turns) else None
    )
    return [
        source
        for source in session.source
        if source.timestamp is not None
        and source.timestamp >= turn.started
        and (next_start is None or source.timestamp < next_start)
    ]


def line_for_tool(session: Session, turn_index: int, tool: trace_report.Tool) -> SourceLine | None:
    timestamp = tool.at_done or tool.at_request
    candidates = entries_for_turn(session, turn_index)
    if timestamp:
        exact = [source for source in candidates if source.timestamp == timestamp]
        if exact:
            return exact[0]
    for source in candidates:
        if "tool falhou" not in source.message and "tool concluída" not in source.message:
            continue
        fields = trace_report.parse_kv(source.message)
        if fields.get("id") == tool.id:
            return source
    return candidates[0] if candidates else None


def source_or_fallback(
    source: SourceLine | None, timestamp: str | None
) -> tuple[str | None, int]:
    if source is not None:
        return with_z(source.timestamp), source.number
    return with_z(timestamp), 0


def evidence(timestamp: str | None, line_number: int, description: str) -> str:
    return f"{timestamp or 'timestamp_indisponivel'} linha {line_number}: {description}"


def finding(
    *,
    kind: str,
    pattern: str,
    message: str,
    timestamp: str | None,
    line_number: int,
    tool: str | None = None,
) -> dict[str, Any]:
    description = redact(message)
    return {
        "timestamp": timestamp,
        "line_number": line_number,
        "kind": kind,
        "tool": safe_tool(tool) if tool else None,
        "pattern": pattern,
        "message": description,
        "evidence": evidence(timestamp, line_number, description),
    }


def error_category(error: str) -> str:
    lowered = error.lower()
    if APP_MISSING.search(error):
        return "aplicativo_nao_encontrado"
    if "timeout" in lowered or "tempo esgotado" in lowered or "não terminou" in lowered:
        return "timeout"
    if "permission" in lowered or "permiss" in lowered or "autoriz" in lowered:
        return "permissao"
    if "fora do perfil" in lowered or "não liberada" in lowered:
        return "fora_do_perfil"
    if "conect" in lowered or "connection" in lowered:
        return "conexao"
    return "erro_execucao"


def app_missing_name(text: str) -> str:
    match = APP_MISSING.search(text)
    return redact(match.group(1)) if match else "aplicativo"


def collect_findings(session: Session) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    structured_app_lines: set[int] = set()

    for turn_index, turn in enumerate(session.turns):
        turn_sources = entries_for_turn(session, turn_index)
        for tool in turn.tools.values():
            if tool.ok is not False:
                continue
            source = line_for_tool(session, turn_index, tool)
            timestamp, line_number = source_or_fallback(source, tool.at_done)
            category = error_category(tool.erro)
            name = safe_tool(tool.name)
            if category == "aplicativo_nao_encontrado":
                pattern = "app.open:aplicativo_nao_encontrado"
                message = f"aplicativo não encontrado: {app_missing_name(tool.erro)}"
                structured_app_lines.add(line_number)
            else:
                pattern = f"tool:{name}:{category}"
                message = f"tool falhou ferramenta={name} categoria={category}"
            findings.append(
                finding(
                    kind="tool_failure",
                    pattern=pattern,
                    message=message,
                    timestamp=timestamp,
                    line_number=line_number,
                    tool=name,
                )
            )

        model_text = "".join(turn.model)
        if trace_report.recusa(model_text):
            source = next(
                (item for item in turn_sources if item.message.startswith("modelo disse ")),
                turn_sources[0] if turn_sources else None,
            )
            timestamp, line_number = source_or_fallback(source, turn.started)
            findings.append(
                finding(
                    kind="model_refusal",
                    pattern="model:recusa",
                    message="recusa detectada no texto concatenado do turno",
                    timestamp=timestamp,
                    line_number=line_number,
                )
            )

        for duplicate in trace_report.duplicates(turn):
            match = TOOL_IN_DUPLICATE.search(duplicate)
            name = safe_tool(match.group(1) if match else "tool")
            completed = []
            for source in turn_sources:
                if not source.message.startswith("tool concluída"):
                    continue
                if trace_report.parse_kv(source.message).get("ferramenta") == name:
                    completed.append(source)
            source = completed[1] if len(completed) > 1 else completed[-1] if completed else None
            timestamp, line_number = source_or_fallback(source, turn.started)
            findings.append(
                finding(
                    kind="duplicate_action",
                    pattern="action:duplicada",
                    message=f"ação executada mais de uma vez no turno ferramenta={name}",
                    timestamp=timestamp,
                    line_number=line_number,
                    tool=name,
                )
            )

        acted = any(decision.startswith("Act") for _, decision, _ in turn.reflex)
        if turn.reflex and not acted:
            source = next(
                (
                    item
                    for item in turn_sources
                    if item.message.startswith("reflexo decision=")
                ),
                turn_sources[0] if turn_sources else None,
            )
            timestamp, line_number = source_or_fallback(source, turn.started)
            elapsed = sum(ms for _, _, ms in turn.reflex if ms >= 0)
            findings.append(
                finding(
                    kind="jev_without_action",
                    pattern="reflex:jev_sem_acao",
                    message=(
                        f"Jev consultado sem ação chamadas={len(turn.reflex)} latencia_ms={elapsed}"
                    ),
                    timestamp=timestamp,
                    line_number=line_number,
                    tool="reflex",
                )
            )

    for source in session.source:
        if source.traced or source.number in structured_app_lines:
            continue
        match = APP_MISSING.search(source.message)
        if not match:
            continue
        timestamp = with_z(source.timestamp)
        findings.append(
            finding(
                kind="tool_failure",
                pattern="app.open:aplicativo_nao_encontrado",
                message=f"aplicativo não encontrado: {redact(match.group(1))}",
                timestamp=timestamp,
                line_number=source.number,
                tool="app.open",
            )
        )

    findings.sort(key=lambda item: (item["line_number"], item["pattern"]))
    return findings


def latency_stats(values: list[int]) -> dict[str, int | None]:
    return {
        "samples": len(values),
        "average": int(statistics.mean(values)) if values else None,
        "min": min(values) if values else None,
        "max": max(values) if values else None,
    }


def summarize(session: Session) -> dict[str, Any]:
    tools = [tool for turn in session.turns for tool in turn.tools.values()]
    duplicates = [
        duplicate
        for turn in session.turns
        for duplicate in trace_report.duplicates(turn)
    ]
    refusals = [
        turn for turn in session.turns if trace_report.recusa("".join(turn.model))
    ]
    latencies = [
        ms
        for turn in session.turns
        for _, _, ms in turn.reflex
        if ms >= 0
    ]
    if not latencies:
        latencies = session.events.latencies
    reflex_actions = sum(
        decision.startswith("Act")
        for turn in session.turns
        for _, decision, _ in turn.reflex
    )
    if reflex_actions == 0:
        reflex_actions = session.events.reflex_actions
    waste = [
        turn
        for turn in session.turns
        if turn.reflex
        and not any(decision.startswith("Act") for _, decision, _ in turn.reflex)
    ]
    findings = collect_findings(session)
    tool_failures = sum(item["kind"] == "tool_failure" for item in findings)
    return {
        "started_at": with_z(session.turns[0].started) if session.turns else None,
        "ended_at": with_z(session.turns[-1].ended or session.turns[-1].started)
        if session.turns
        else None,
        "turns": sum(bool(turn.user) for turn in session.turns),
        "tool_requests": sum(bool(tool.at_request) for tool in tools),
        "actions": sum(tool.ok is not None for tool in tools),
        "failures": tool_failures,
        "duplicates": len(duplicates),
        "refusals": len(refusals),
        "reflex_actions": reflex_actions,
        "reflex_learns": sum(len(turn.learned) for turn in session.turns)
        or session.events.reflex_learns,
        "deduplicated_by_reflex": sum(len(turn.dedup) for turn in session.turns)
        or session.events.deduplicated,
        "jev_latency_ms": latency_stats(latencies),
        "jev_without_action": {
            "turns": len(waste),
            "calls": sum(len(turn.reflex) for turn in waste),
            "latency_ms": sum(
                ms for turn in waste for _, _, ms in turn.reflex if ms >= 0
            ),
        },
    }


def observe_session(
    trace_path: str | Path = DEFAULT_TRACE,
    events_path: str | Path | None = None,
) -> dict[str, Any]:
    return summarize(load_session(trace_path, events_path))


def list_failures(
    trace_path: str | Path = DEFAULT_TRACE,
    events_path: str | Path | None = None,
) -> list[dict[str, Any]]:
    return collect_findings(load_session(trace_path, events_path))


def confirmation(step: str, expected: str) -> dict[str, str]:
    return {"step": step, "expected": expected}


def task_contract(pattern: str, findings: list[dict[str, Any]]) -> dict[str, Any]:
    first = findings[0]
    if pattern == "app.open:aplicativo_nao_encontrado":
        what = "O Jarvis resolve nomes localizados de aplicativos antes de executar app.open."
        why = "Pedidos válidos falham quando o nome falado não coincide com o nome instalado."
        checks = [
            confirmation(
                "Repetir os pedidos de abrir aplicativo citados nas evidências.",
                "Cada aplicativo abre com uma única execução de app.open.",
            ),
            confirmation(
                "Consultar list_failures no novo trace.",
                "O padrão app.open:aplicativo_nao_encontrado não aparece.",
            ),
        ]
    elif pattern == "model:recusa":
        what = "O Jarvis executa pedidos cobertos por suas ferramentas sem responder com recusa genérica."
        why = "Uma recusa interrompe o roteiro mesmo quando a ação está disponível ao agente."
        checks = [
            confirmation(
                "Reexecutar os turnos associados às evidências.",
                "O modelo chama a ferramenta adequada ou explica uma limitação real e específica.",
            ),
            confirmation(
                "Consultar list_failures no novo trace.",
                "O padrão model:recusa não aparece.",
            ),
        ]
    elif pattern == "action:duplicada":
        what = "Cada ação externa é executada uma única vez por turno, inclusive quando o reflexo age antes do modelo."
        why = "Execução duplicada repete efeitos no computador e torna o Jarvis imprevisível."
        checks = [
            confirmation(
                "Repetir os turnos associados às evidências.",
                "Cada efeito ocorre uma vez e a chamada redundante é deduplicada.",
            ),
            confirmation(
                "Rodar o relatório de aceitação no novo trace.",
                "A contagem de duplicatas é zero.",
            ),
        ]
    elif pattern == "reflex:jev_sem_acao":
        what = "O reflexo evita consultar o Jev em turnos que não podem produzir uma ação."
        why = "Consultas sem ação aumentam a latência sem ajudar o usuário."
        checks = [
            confirmation(
                "Reexecutar os turnos associados às evidências.",
                "Turnos sem ação não consultam o Jev.",
            ),
            confirmation(
                "Chamar observe_session no novo trace.",
                "jev_without_action diminui sem perder ações válidas do reflexo.",
            ),
        ]
    else:
        tool = first.get("tool") or "ferramenta"
        what = f"A ferramenta {tool} conclui sem repetir o padrão {pattern}."
        why = "O padrão impede a ação solicitada de terminar."
        checks = [
            confirmation(
                f"Repetir o fluxo que aciona {tool}.",
                "A ferramenta conclui sem erro.",
            ),
            confirmation(
                "Consultar list_failures no novo trace.",
                f"O padrão {pattern} não aparece.",
            ),
        ]
    return {
        "pattern": pattern,
        "occurrences": len(findings),
        "o_que": what,
        "por_que": why,
        "como_confirmo": checks,
        "evidence": [item["evidence"] for item in findings],
    }


def propose_tasks(
    trace_path: str | Path = DEFAULT_TRACE,
    events_path: str | Path | None = None,
) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for item in list_failures(trace_path, events_path):
        grouped[item["pattern"]].append(item)
    return [task_contract(pattern, grouped[pattern]) for pattern in sorted(grouped)]


def paths_from_arguments(arguments: dict[str, Any]) -> tuple[str | Path, str | Path | None]:
    trace_path = arguments.get("trace_path", str(DEFAULT_TRACE))
    events_path = arguments.get("events_path")
    if not isinstance(trace_path, str) or (
        events_path is not None and not isinstance(events_path, str)
    ):
        raise ObserverError("trace_path e events_path devem ser strings")
    return trace_path, events_path


def call_tool(name: str, arguments: dict[str, Any]) -> Any:
    trace_path, events_path = paths_from_arguments(arguments)
    if name == "observe_session":
        return observe_session(trace_path, events_path)
    if name == "list_failures":
        return {"failures": list_failures(trace_path, events_path)}
    if name == "propose_tasks":
        return {"tasks": propose_tasks(trace_path, events_path)}
    raise ObserverError("tool desconhecida")


INPUT_SCHEMA = {
    "type": "object",
    "properties": {
        "trace_path": {
            "type": "string",
            "description": "Trace textual; padrão: /tmp/jarvis-trace.log",
        },
        "events_path": {
            "type": "string",
            "description": "events.log opcional produzido por --record",
        },
    },
    "additionalProperties": False,
}

TOOLS = [
    {
        "name": "observe_session",
        "description": "Resume turnos, ações, falhas, recusas, duplicatas e custo do Jev.",
        "inputSchema": INPUT_SCHEMA,
    },
    {
        "name": "list_failures",
        "description": "Lista findings com timestamp, linha, padrão e evidência mascarada.",
        "inputSchema": INPUT_SCHEMA,
    },
    {
        "name": "propose_tasks",
        "description": "Agrupa findings por padrão e escreve contratos prontos para o board.",
        "inputSchema": INPUT_SCHEMA,
    },
]


def rpc_result(request_id: Any, result: Any) -> dict[str, Any]:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def rpc_error(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def handle_rpc(request: dict[str, Any]) -> dict[str, Any] | None:
    request_id = request.get("id")
    method = request.get("method")
    if request_id is None:
        return None
    if method == "initialize":
        return rpc_result(
            request_id,
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "jarvis-observer", "version": "0.3.1"},
            },
        )
    if method == "ping":
        return rpc_result(request_id, {})
    if method == "tools/list":
        return rpc_result(request_id, {"tools": TOOLS})
    if method == "tools/call":
        params = request.get("params") or {}
        arguments = params.get("arguments") or {}
        try:
            if not isinstance(arguments, dict):
                raise ObserverError("arguments deve ser um objeto")
            structured = call_tool(str(params.get("name", "")), arguments)
            result = {
                "content": [
                    {
                        "type": "text",
                        "text": json.dumps(
                            structured, ensure_ascii=False, separators=(",", ":")
                        ),
                    }
                ],
                "structuredContent": structured,
                "isError": False,
            }
        except ObserverError as error:
            result = {
                "content": [{"type": "text", "text": str(error)}],
                "isError": True,
            }
        return rpc_result(request_id, result)
    return rpc_error(request_id, -32601, "método MCP desconhecido")


def serve(reader: IO[str] = sys.stdin, writer: IO[str] = sys.stdout) -> None:
    for raw in reader:
        if not raw.strip():
            continue
        try:
            request = json.loads(raw)
            if not isinstance(request, dict):
                raise TypeError
            response = handle_rpc(request)
        except (json.JSONDecodeError, TypeError):
            response = rpc_error(None, -32700, "JSON inválido")
        if response is not None:
            writer.write(json.dumps(response, ensure_ascii=False, separators=(",", ":")))
            writer.write("\n")
            writer.flush()


def default_config_path() -> Path:
    home = os.environ.get("HOME")
    if not home:
        raise ObserverError("não foi possível localizar o config.toml")
    return Path(home) / ".config" / "jarvis" / "config.toml"


def register_server(config_path: Path | None = None) -> str:
    path = config_path or default_config_path()
    try:
        existing = path.read_text(encoding="utf-8") if path.exists() else ""
    except OSError:
        raise ObserverError("não foi possível ler o config.toml") from None
    if any(OBSERVER_NAME.search(match.group(1)) for match in MCP_BLOCK.finditer(existing)):
        return "already_present"

    command = json.dumps(str(Path(sys.executable).resolve()), ensure_ascii=False)
    script = json.dumps(str(Path(__file__).resolve()), ensure_ascii=False)
    block = (
        "[[mcp_servers]]\n"
        'name = "observer"\n'
        f"command = {command}\n"
        f"args = [{script}]\n"
    )
    updated = existing
    if updated and not updated.endswith("\n"):
        updated += "\n"
    if updated:
        updated += "\n"
    updated += block

    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o600
        temporary = path.with_name(f".{path.name}.observer-{os.getpid()}.tmp")
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                stream.write(updated)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            if temporary.exists():
                temporary.unlink()
    except OSError:
        raise ObserverError("não foi possível gravar o config.toml") from None
    return "added"


def main(argv: list[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    if args == ["--install-config"]:
        result = register_server()
        print(
            "observador registrado no config.toml"
            if result == "added"
            else "observador já estava registrado"
        )
        return 0
    if args:
        print("uso: server.py [--install-config]", file=sys.stderr)
        return 2
    serve()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
