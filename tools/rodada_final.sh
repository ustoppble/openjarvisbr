#!/usr/bin/env bash
# Rodada de aceitação da missão "Autonomia do Jarvis", num comando só.
#
#   tools/rodada_final.sh [ROTEIRO] [NOME] [OPÇÕES DO JARVIS...]
#
# 1. fecha o app de desktop (ele ouve o mic e responderia ao áudio do CLI);
# 2. compila o CLI em release;
# 3. roda o roteiro em modo texto com o trace ligado;
# 4. gera o relatório e aplica a régua (--aceitacao);
# 5. grava trace + relatório em docs/superpowers/briefs/<data>-<nome>.md.
#
# Sai com 0 se a régua passou (duplicatas=0, falhas=0, recusas=0), 1 se não.
# Nunca imprime chaves: o trace já sai mascarado pelo engine.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ROTEIRO="${1:-$ROOT/docs/testes/roteiro-autonomia.txt}"
NOME="${2:-rodada-final}"
if [ "$#" -ge 2 ]; then shift 2; else shift "$#"; fi
STAMP="$(date +%Y-%m-%d-%H%M)"
LOG="/tmp/jarvis-${NOME}-${STAMP}.log"
DOC="$ROOT/docs/superpowers/briefs/${STAMP}-${NOME}.md"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target-5834}"

echo "== 1/5 fechando o app de desktop (se aberto)"
pkill -f "openjarvisbr-desktop" 2>/dev/null || true

echo "== 2/5 compilando o CLI (release)"
(cd "$ROOT" && cargo build -q --release --bin jarvis)

echo "== 3/5 rodando $ROTEIRO (trace em $LOG)"
INICIO="$(date -u +%Y-%m-%dT%H:%M:%S)"
set +e
RUST_LOG="info,openjarvisbr_core=debug,openjarvisbr_core::audio=warn" \
  "$CARGO_TARGET_DIR/release/jarvis" --script "$ROTEIRO" "$@" >"$LOG" 2>&1 </dev/null
RC=$?
set -e
echo "   runner exit=$RC"

echo "== 4/5 relatório + régua"
set +e
python3 "$ROOT/tools/jarvis_trace_report.py" "$LOG" --since "$INICIO" --aceitacao --runner-exit "$RC" >"/tmp/jarvis-${NOME}-${STAMP}.aceitacao.txt" 2>&1
PASSOU=$?
set -e
cat "/tmp/jarvis-${NOME}-${STAMP}.aceitacao.txt"

echo "== 5/5 gravando $DOC"
{
  echo "# Medição — ${NOME} — ${STAMP}"
  echo
  echo "- HEAD: $(git -C "$ROOT" rev-parse --short HEAD)"
  echo "- Roteiro: ${ROTEIRO#$ROOT/}"
  echo "- Runner exit: $RC"
  echo "- Régua: $([ "$PASSOU" -eq 0 ] && echo PASSOU || echo 'NÃO PASSOU')"
  echo
  echo '```'
  cat "/tmp/jarvis-${NOME}-${STAMP}.aceitacao.txt"
  echo '```'
  echo
  python3 "$ROOT/tools/jarvis_trace_report.py" "$LOG" --since "$INICIO"
} >"$DOC"
echo "   medição em ${DOC#$ROOT/}"

exit "$PASSOU"
