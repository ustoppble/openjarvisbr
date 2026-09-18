# MCP observador do Jarvis

Servidor MCP stdio, somente leitura, que converte traces em findings para o
loop de autoaprimoramento.

O servidor importa `tools/jarvis_trace_report.py`: turnos, recusas,
duplicatas e latência do Jev têm uma única implementação. A camada MCP fica
responsável por localizar linhas de evidência, mascarar a saída e agrupar
ocorrências em contratos de tarefa.

## Tools

- `observe_session`: resumo da rodada, incluindo ações, falhas, recusas,
  duplicatas e custo do Jev.
- `list_failures`: findings com timestamp, linha, padrão e evidência
  mascarada.
- `propose_tasks`: uma tarefa por padrão, com `o_que`, `por_que`,
  `como_confirmo` e as evidências do grupo.

As três aceitam `trace_path` e, opcionalmente, `events_path`. Sem
`trace_path`, usam `/tmp/jarvis-trace.log`.

## Registro no Jarvis

```sh
python3 tools/observer/server.py --install-config
```

O instalador acrescenta `observer` ao `config.toml` com caminhos absolutos,
sem imprimir nem reserializar o conteúdo existente. Repetir o comando não
duplica o registro.

## Teste

```sh
python3 tools/observer/test_server.py -q
cargo test -p jarvis-observer-mcp
```

O teste usa o trace autônomo versionado em `docs/testes/rodada1-trace.log`.
