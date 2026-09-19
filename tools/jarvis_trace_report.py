#!/usr/bin/env python3
"""Relatório de observabilidade do Jarvis a partir do trace de tracing.

Uso:
    tools/jarvis_trace_report.py /tmp/jarvis-trace.log [--since 2026-09-18T19:37]

Lê o log gerado com RUST_LOG="info,openjarvisbr_core=debug,..." e agrupa em
turnos: o que o usuário disse, o que o modelo respondeu, que tools foram
pedidas/concluídas (origem modelo/reflexo, ms, erro), decisões do reflexo
(latência do Jev) e duplicatas (mesma ação executada pelas duas origens).

Mascara segredos e endereços também na saída, antes de imprimir ou gravar.
"""

from __future__ import annotations

import argparse
import re
import statistics
import sys
from dataclasses import dataclass, field

ANSI = re.compile(r"\x1b\[[0-9;]*m")
LINE = re.compile(
    r"(?P<ts>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+)Z\s+"
    r"(?P<level>[A-Z]+)\s+(?P<target>[a-z_:]+):\s+(?P<msg>.*)$"
)
KV = re.compile(r'(\w+)=("(?:[^"\\]|\\.)*"|\S+)')
SENSITIVE = re.compile(r"(api[_-]?key|token|secret|bearer|authorization)", re.I)
AIZA = re.compile(r"AIza[0-9A-Za-z_-]+")


def clean(text: str) -> str:
    return AIZA.sub("***", text)


def redact_output(text: str) -> str:
    # Aplicar depois da análise: mascarar destinos antes de comparar ações
    # confundiria URLs distintas e produziria duplicatas falsas.
    text = clean(text)
    text = re.sub(r"(?i)(?:https?|wss?|ssh)://[^\s<>\]\)\"'`]+", "***", text)
    text = re.sub(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b", "***", text)
    text = re.sub(r"(?i)\b(?:localhost|[A-Za-z0-9.-]+\.(?:internal|local|com|net|org|io|dev|ai|sh|br|app|test|invalid))(?::\d+)?\b", "***", text)
    text = re.sub(r"(?i)(bearer\s+)[^\s\"']+", r"\1***", text)
    text = re.sub(r'(?i)((?:api[_-]?key|token|secret|password|authorization|cookie)[\"\x27]?\s*[:=]\s*)(?:"[^"\n]*"|[^\s,;]+)', r'\1***', text)
    return text


def parse_kv(rest: str) -> dict[str, str]:
    out = {}
    for key, val in KV.findall(rest):
        if val.startswith('"') and val.endswith('"'):
            val = val[1:-1]
        out[key] = val
    return out


@dataclass
class Tool:
    id: str
    name: str
    origem: str = "modelo"
    risco: str = ""
    pedido: str = ""
    ms: int | None = None
    ok: bool | None = None
    erro: str = ""
    resultado: str = ""
    at_request: str = ""
    at_done: str = ""


@dataclass
class Turn:
    started: str
    user: list[str] = field(default_factory=list)
    model: list[str] = field(default_factory=list)
    tools: dict[str, Tool] = field(default_factory=dict)
    reflex: list[tuple[str, str, int]] = field(default_factory=list)  # ms=-1: Jev não consultado
    learned: list[str] = field(default_factory=list)
    dedup: list[str] = field(default_factory=list)
    pulos: list[str] = field(default_factory=list)  # reflexo mudo por falha de infra
    notes: list[str] = field(default_factory=list)
    ended: str = ""

    def first_user_ts(self) -> str:
        return self.started


def ts_ms(ts: str) -> float:
    h, m, s = ts.split("T")[1].split(":")
    return (int(h) * 3600 + int(m) * 60 + float(s)) * 1000


def parse(lines: list[str], since: str | None) -> list[Turn]:
    turns: list[Turn] = []
    cur: Turn | None = None

    def turn_for(ts: str) -> Turn:
        nonlocal cur
        if cur is None:
            cur = Turn(started=ts)
            turns.append(cur)
        return cur

    for raw in lines:
        raw = ANSI.sub("", raw.rstrip("\n"))
        # O CLI imprime fragmentos de fala sem quebra de linha; um evento
        # de tracing pode começar depois de "Feito." na saída combinada.
        m = LINE.search(raw)
        if not m:
            continue
        ts, msg = m.group("ts"), clean(m.group("msg"))
        if since and ts < since:
            continue
        if SENSITIVE.search(msg) and "ferramenta=" not in msg:
            continue

        if msg.startswith("usuário disse "):
            # `usuário disse texto=... via="voz"`: o campo via vem depois do texto
            text = re.sub(r'\s+via="[^"]*"$', "", msg.split("texto=", 1)[-1])
            # Cada envio de texto é um pedido completo, mesmo se o modelo
            # não emitiu TurnComplete. Só a voz pode chegar em fragmentos.
            if cur is None or cur.ended or parse_kv(msg).get("via") == "texto":
                cur = Turn(started=ts)
                turns.append(cur)
            cur.user.append(text)
        elif msg.startswith("modelo disse "):
            turn_for(ts).model.append(msg.split("texto=", 1)[-1])
        elif msg.startswith("tool pedida pelo modelo"):
            kv = parse_kv(msg)
            t = Tool(id=kv.get("id", "?"), name=kv.get("ferramenta", "?"), risco=kv.get("risco", ""),
                     pedido=msg.split("pedido=", 1)[-1], at_request=ts)
            turn_for(ts).tools[t.id] = t
        elif msg.startswith("reflexo agiu"):
            kv = parse_kv(msg)
            turn_for(ts).notes.append(f"reflexo agiu {kv.get('ferramenta')} (Jev {kv.get('ms')} ms)")
        elif msg.startswith("reflexo pediu confirmação"):
            kv = parse_kv(msg)
            turn_for(ts).notes.append(f"reflexo pediu confirmação {kv.get('ferramenta')}")
        elif msg.startswith("tool concluída") or msg.startswith("tool falhou"):
            kv = parse_kv(msg)
            tid = kv.get("id", "?")
            t = turn_for(ts).tools.get(tid)
            if t is None:
                t = Tool(id=tid, name=kv.get("ferramenta", "?"))
                turn_for(ts).tools[tid] = t
            t.origem = kv.get("origem", "modelo")
            t.ms = int(kv["ms"]) if kv.get("ms", "").isdigit() else None
            t.ok = msg.startswith("tool concluída")
            t.at_done = ts
            if not t.ok:
                t.erro = msg.split("erro=", 1)[-1].split(" tool falhou")[0]
            else:
                t.resultado = msg.split("resultado=", 1)[-1].split(" tool concluída")[0]
            if t.origem == "reflexo" and not t.pedido:
                t.pedido = t.name
        elif msg.startswith("reflexo decision="):
            dec = "Act" if "decision=Act" in msg else "Nothing" if "decision=Nothing" in msg else msg.split("decision=")[1].split(" ")[0]
            name = ""
            nm = re.search(r'name: "([^"]+)"', msg)
            if nm:
                name = nm.group(1)
            lat = re.search(r"latency_ms=(\d+)", msg)
            skipped = parse_kv(msg).get("skipped_judge") == "true"
            latency = -1 if skipped else int(lat.group(1)) if lat else 0
            turn_for(ts).reflex.append((ts, f"{dec}{(' ' + name) if name else ''}", latency))
        elif msg.startswith("reflexo pulou esta rodada"):
            # O reflexo ficou mudo por falha de infra (Jev fora do ar ou lento).
            # Sem isso no resumo, a rodada parece só "o modelo agiu mais" e o
            # produto degrada em silêncio.
            # O motivo é frase com espaços ("TypeSafe não respondeu a tempo"),
            # então não dá para usar parse_kv, que corta no primeiro espaço.
            motivo = msg.split("err=", 1)[-1].strip() if "err=" in msg else "motivo não dito"
            turn_for(ts).pulos.append(motivo)
        elif msg.startswith("reflexo aprendeu a ação"):
            kv = parse_kv(msg)
            turn_for(ts).learned.append(f"{kv.get('ferramenta')} ← \"{kv.get('frase', '')}\"")
        elif "já executada pelo reflexo" in msg:
            turn_for(ts).dedup.append(parse_kv(msg).get("ferramenta", "?"))
        elif msg.startswith("turno do modelo concluído"):
            if cur is not None:
                cur.ended = ts
        elif "fora do perfil" in msg or "não liberada neste perfil" in msg:
            turn_for(ts).notes.append(f"BLOQUEADA pelo perfil: {parse_kv(msg).get('ferramenta')}")
    return turns


RECUSAS = [
    "não é possível",
    "não consigo",
    "não posso",
    "não tenho como",
    "não sou capaz",
]


def recusa(texto: str) -> bool:
    """O modelo disse que não dá para fazer.

    A fala chega em pedaços (`modelo disse` várias vezes no mesmo turno) e o
    pedaço anterior costuma grudar no começo: "no históricoNão consigo
    realizar pesquisas". Por isso a busca é no texto do turno inteiro, já
    juntado, e sem diferenciar maiúscula — "Não consigo" é a forma mais comum
    e passava batido.
    """
    baixo = texto.lower()
    return any(p in baixo for p in RECUSAS)


def duplicates(turn: Turn) -> list[str]:
    # Chave = tool + resultado (o reflexo não tem "pedido"; o resultado das
    # duas origens é idêntico quando a ação é a mesma).
    seen: dict[str, list[Tool]] = {}
    for t in turn.tools.values():
        if t.ok is not True:
            continue
        seen.setdefault(f"{t.name}|{t.resultado}", []).append(t)
    out = []
    for key, ts in seen.items():
        if len(ts) > 1:
            origins = sorted({t.origem for t in ts})
            quem = "modelo+reflexo" if len(origins) > 1 else f"{origins[0]} sozinho"
            out.append(f"`{key.split('|')[0]}` {key.split('|')[1]} executada {len(ts)}x ({quem})")
    return out


def report(turns: list[Turn]) -> str:
    lines = ["# Relatório do trace do Jarvis", ""]
    jev = [ms for t in turns for (_, _, ms) in t.reflex if ms >= 0]
    acts = sum(1 for t in turns for (_, d, _) in t.reflex if d.startswith("Act"))
    tools = [t for tr in turns for t in tr.tools.values()]
    fails = [t for t in tools if t.ok is False]
    dups = [d for tr in turns for d in duplicates(tr)]
    by_origin = {}
    for t in tools:
        by_origin[t.origem] = by_origin.get(t.origem, 0) + 1
    fala_acao = []
    for tr in turns:
        reqs = [t.at_request or t.at_done for t in tr.tools.values() if (t.at_request or t.at_done)]
        if tr.user and reqs:
            fala_acao.append(ts_ms(min(reqs)) - ts_ms(tr.started))
    impossivel = [(tr, texto) for tr in turns
                  for texto in ["".join(tr.model)]
                  if recusa(texto)]

    lines += ["## Resumo", ""]
    lines.append(f"- Turnos com fala do usuário: {sum(1 for t in turns if t.user)}")
    lines.append(f"- Tools executadas: {len(tools)} " + ", ".join(f"{k}={v}" for k, v in sorted(by_origin.items())))
    lines.append(f"- Falhas de tool: {len(fails)}")
    lines.append(f"- Duplicatas (mesma ação executada mais de uma vez no turno): {len(dups)}")
    if jev:
        lines.append(f"- Chamadas ao Jev: {len(jev)}; Act: {acts} ({100 * acts // len(jev)}%); "
                     f"latência média {int(statistics.mean(jev))} ms, máx {max(jev)} ms")
        # "Quando NÃO usar o Jev": turno que consultou o juiz, gastou latência e
        # não virou ação nenhuma. É o desperdício que a regra do gate tem de zerar.
        gasto = 0
        desperdicio = []
        for tr in turns:
            chamadas = [ms for (_, _, ms) in tr.reflex if ms >= 0]
            agiu = any(d.startswith("Act") for (_, d, _) in tr.reflex)
            if chamadas and not agiu:
                gasto += sum(chamadas)
                desperdicio.append((' '.join(tr.user).strip() or '(sem fala)', len(chamadas), sum(chamadas)))
        mudo = [(tr, err) for tr in turns for err in tr.pulos]
        if mudo:
            lines.append(f"- REFLEXO MUDO: {len(mudo)} vez(es) — o Jev não respondeu e o reflexo nem decidiu")
            for tr, err in mudo:
                fala = ' '.join(tr.user).strip() or '(sem fala)'
                lines.append(f"  - \"{fala[:60]}\" — {err}")
        lines.append(f"- Jev gasto sem ação: {len(desperdicio)} de {len([t for t in turns if t.reflex])} turnos"
                     f" ({gasto} ms jogados fora)")
        for fala, n, ms in desperdicio:
            lines.append(f"  - \"{fala[:70]}{'…' if len(fala) > 70 else ''}\" — {n} chamada(s), {ms} ms, nenhuma ação")
    if fala_acao:
        lines.append(f"- Transcrição→primeira tool: média {int(statistics.mean(fala_acao))} ms, máx {int(max(fala_acao))} ms "
                     "(a transcrição chega junto com a chamada; o fim da fala real não está no trace)")
    lines.append(f"- Respostas 'não é possível/não consigo': {len(impossivel)}")
    for tr, texto in impossivel:
        n = turns.index(tr) + 1
        fala = ' '.join(tr.user).strip() or '(sem fala)'
        lines.append(f"  - turno {n} \"{fala}\" → \"{texto[:120]}{'…' if len(texto) > 120 else ''}\"")
    lines.append("")

    lines += ["## Turnos", ""]
    for i, tr in enumerate(turns, 1):
        if not (tr.user or tr.tools or tr.reflex):
            continue
        lines.append(f"### {i}. {tr.started.split('T')[1][:12]}  {' '.join(tr.user).strip() or '(sem fala)'}")
        for t in sorted(tr.tools.values(), key=lambda t: t.at_request or t.at_done):
            status = "ok" if t.ok else ("FALHOU " + t.erro if t.ok is False else "pendente")
            ms = f"{t.ms} ms" if t.ms is not None else "?"
            lines.append(f"- tool `{t.pedido or t.name}` origem={t.origem} {ms} {status}")
        for (ts, dec, ms) in tr.reflex:
            judge = f"Jev {ms} ms" if ms >= 0 else "Jev não consultado"
            lines.append(f"- reflexo {dec} ({judge})")
        for n in tr.notes:
            lines.append(f"- {n}")
        for d in tr.dedup:
            lines.append(f"- dedup: Gemini pediu `{d}` já feita pelo reflexo")
        for d in duplicates(tr):
            lines.append(f"- **DUPLICATA**: {d}")
        for l in tr.learned:
            lines.append(f"- aprendeu: {l}")
        model = "".join(tr.model).strip()
        if model:
            lines.append(f"- modelo: \"{model[:160]}{'…' if len(model) > 160 else ''}\"")
        lines.append("")
    if fails:
        lines += ["## Falhas", ""]
        for t in fails:
            lines.append(f"- `{t.pedido or t.name}` ({t.origem}): {t.erro}")
        lines.append("")
    if dups:
        lines += ["## Duplicatas", ""] + [f"- {d}" for d in dups] + [""]
    return redact_output("\n".join(lines))


def aceitacao(turns: list[Turn], runner_exit: int | None = None) -> tuple[list[tuple[bool, str]], bool]:
    """Os critérios da missão, checados por máquina.

    O loop de autoaprimoramento precisa saber sozinho quando parar. Olho humano
    lendo relatório não serve: foi assim que a rodada 2 passou por "0 recusas"
    tendo duas dentro.
    """
    dups = [d for tr in turns for d in duplicates(tr)]
    fails = [t for tr in turns for t in tr.tools.values() if t.ok is False or t.erro]
    recusas = [tr for tr in turns if recusa("".join(tr.model))]

    checks: list[tuple[bool, str]] = [
        (not dups, f"duplicatas = {len(dups)} (esperado 0)"),
        (not fails, f"falhas de tool = {len(fails)} (esperado 0)"),
        (not recusas, f"recusas do modelo = {len(recusas)} (esperado 0)"),
    ]
    if runner_exit is not None:
        checks.append((runner_exit == 0, f"runner exit = {runner_exit} (esperado 0)"))

    # Ausência de erros não prova execução: um log vazio também tem 0/0/0.
    # Exigir cada ação do roteiro e a ferramenta correspondente concluída.
    required = [
        ("abre o Safari", r"abre o safari", {"app.open"}),
        ("Google no Safari", r"abre o google\S* no safari", {"web.open", "browser.goto"}),
        ("abre o Chrome", r"abre o chrome", {"app.open"}),
        ("abre o youtube no Chrome", r"abre o youtube no chrome", {"web.open", "browser.goto"}),
        ("volta", r"volta", {"browser.back"}),
        ("pesquisa overclock", r"pesquisa overclock", {"browser.search"}),
        ("abre a Calculadora", r"abre a calculadora", {"app.open"}),
        ("abre a globo", r"abre a globo", {"web.open", "browser.goto"}),
        ("diminui o som", r"diminui o som", {"sys.volume"}),
        ("coloca o volume em 30", r"coloca o volume em 30", {"sys.volume"}),
    ]
    for label, phrase, names in required:
        matching = [tr for tr in turns if re.fullmatch(phrase, ' '.join(tr.user).strip(), re.I)]
        ok = bool(matching) and all(any(t.ok is True and t.name in names
                                       for t in tr.tools.values()) for tr in matching)
        checks.append((ok, f'ação "{label}" concluída'))

    times = [tr for tr in turns if ' '.join(tr.user).strip().casefold() == "que horas são?"]
    time_ok = bool(times) and all(tr.reflex and all(ms < 0 for _, _, ms in tr.reflex)
                                and not tr.pulos for tr in times)
    checks.append((time_ok, '"que horas são?" com Jev não consultado'))

    # Item 6 do roteiro: a mesma fala repetida tem de sair pelo reflexo na
    # segunda vez — é o que prova que a memória do reflexo está sendo usada.
    falas: dict[str, list[Turn]] = {}
    for tr in turns:
        fala = ' '.join(tr.user).strip().casefold()
        if fala:
            falas.setdefault(fala, []).append(tr)
    checks.append((len(falas.get("abre a globo", [])) >= 2, '"abre a globo" presente duas vezes'))
    for fala, repetidos in falas.items():
        if len(repetidos) < 2:
            continue
        segunda = repetidos[1]
        origens = {t.origem for t in segunda.tools.values() if t.ok is True
                   and (fala != "abre a globo" or t.name in {"web.open", "browser.goto"})}
        ok = "reflexo" in origens
        checks.append((ok, f'fala repetida "{fala[:40]}" — 2ª vez por {sorted(origens) or ["ninguém"]} (esperado reflexo)'))

    return checks, all(ok for ok, _ in checks)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("log")
    ap.add_argument("--since", help="ISO parcial, ex.: 2026-09-18T19:37")
    ap.add_argument("--aceitacao", action="store_true",
                    help="checa os critérios da missão e sai 1 se algum falhar")
    ap.add_argument("--runner-exit", type=int, help="código de saída do runner desta rodada")
    args = ap.parse_args()
    with open(args.log, encoding="utf-8", errors="replace") as f:
        turns = parse(f.readlines(), args.since)
    if args.aceitacao:
        checks, passou = aceitacao(turns, args.runner_exit)
        for ok, texto in checks:
            print(redact_output(f"[{'PASSA' if ok else 'FALHA'}] {texto}"))
        print("\nROTEIRO PASSOU" if passou else "\nROTEIRO NÃO PASSOU")
        return 0 if passou else 1
    print(report(turns))
    return 0


if __name__ == "__main__":
    sys.exit(main())
