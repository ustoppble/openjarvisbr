# Browser e UI locais — contratos `browser.*` e `ui.*` (JRV-86)

**Data:** 2026-09-18

**Status:** especificação para implementação posterior; este card não altera código de produção.

## Objetivo

Dar ao Jarvis duas famílias locais, com fronteira explícita:

- `browser.*` controla a página e a aba por Apple Events e pelo DOM;
- `ui.*` controla a interface acessível da janela em foco por `System Events`.

A primeira entrega deve cobrir Safari e Google Chrome no macOS. Os nomes de navegador reutilizam a mesma normalização de `web.open`. Nenhuma operação pede, lê, digita ou armazena senha, código de autenticação, token, cookie ou chave.

Esta spec define:

- nomes, argumentos e JSON Schema das tools;
- risco estático `Safe` ou `Confirm`;
- seleção e ciclo de vida do navegador;
- idempotência e erros;
- AppleScript/JavaScript equivalente para Safari e Chrome;
- permissões e preferências exigidas no macOS;
- a fronteira entre DOM (`browser.*`) e Acessibilidade (`ui.*`);
- os resultados observados nos testes manuais de 2026-09-18.

## Evidência que motivou o escopo

O roteiro de aceitação já exigia “volta” e “pesquisa overclock” na aba atual, mas hoje só existe `web.open`.

Durante a missão, o pedido para digitar um valor no campo em foco recebeu a resposta de que o Jarvis não conseguia digitar na tela. Por isso `ui.type` e `ui.click` entram nesta spec agora; `ui.*` não fica mais para uma fase opcional.

Às 19:46:58, a fala “abre o site google.com” gerou duas execuções de `web.open` (`call_112668` e `call_112675`). A deduplicação do turno pertence ao JRV-92, mas `browser.goto` precisa ser idempotente por contrato para não recarregar a mesma página nem acrescentar histórico quando receber duas chamadas iguais.

## Decisão de arquitetura

### Fronteira entre as famílias

| Alvo da intenção | Família que ganha | Motivo |
|---|---|---|
| aba, URL, histórico, conteúdo ou elemento dentro de uma página | `browser.*` | opera por semântica do navegador/DOM, não depende de coordenada nem do foco global |
| campo dentro de uma página, inclusive a caixa de pesquisa do Google | `browser.type` | encontra um campo semanticamente, suporta Unicode e dispara eventos DOM sem corrida de foco |
| botão/link dentro de uma página | `browser.click_text` | encontra o elemento no DOM e consegue detectar zero ou múltiplos candidatos |
| barra de endereço, menu, diálogo nativo ou extensão do navegador | `ui.*` | esses elementos não pertencem ao DOM da página |
| qualquer elemento de outro aplicativo | `ui.*` | `System Events` é a superfície comum entre aplicativos macOS |
| navegador sem driver DOM compatível ou com JavaScript via Apple Events desativado | `ui.*`, somente após nova chamada explícita | o fallback muda a superfície e o alvo; nunca ocorre silenciosamente |

“Digitar num campo do Google” usa `browser.type` quando o campo está na página. `ui.type` só ganha se a intenção aponta para a barra de endereço, um diálogo nativo ou se o DOM estiver indisponível e o usuário mantiver a intenção após o erro explicado.

Não há fallback automático de `browser.click_text`/`browser.type` para `ui.click`/`ui.type`. Um fallback automático poderia clicar ou digitar na janela errada se o foco mudasse entre a decisão e a execução.

### Por que não usar `System Events` para tudo

O DOM oferece nome acessível, papel, visibilidade e cardinalidade do alvo. Isso permite recusar um alvo ambíguo e bloquear campos sensíveis antes da ação. `System Events` depende da árvore de Acessibilidade da janela em foco e sofre corrida de foco; ele é necessário, mas é o segundo caminho.

### Forma futura no registro de tools

Cada operação será uma implementação de `Tool` com `ToolSpec { name, description, parameters, risk }`, como as tools existentes. `Registry` continuará apenas registrando e chamando as tools; não recebe regra específica de navegador.

Na implementação posterior:

1. a tabela hoje privada em `web.rs` deve virar um resolvedor compartilhado, para `web.open` e `browser.*` não divergirem;
2. `local::all` registra as novas tools;
3. o risco de cada tool local continua sendo o risco declarado no `ToolSpec`;
4. a política atual ainda torna tudo `Safe` quando o modo de acesso total está ligado; esta spec não muda essa regra;
5. os testes de `local::all` passam a conferir os novos nomes, schemas e riscos.

## Seleção do navegador

### Apelidos compartilhados com `web.open`

A normalização remove espaços externos e ignora maiúsculas/minúsculas.

| Entrada aceita | Nome oficial |
|---|---|
| `safari` | `Safari` |
| `chrome`, `google chrome` | `Google Chrome` |
| `firefox` | `Firefox` |
| `edge`, `microsoft edge` | `Microsoft Edge` |
| `arc` | `Arc` |
| `brave`, `brave browser` | `Brave Browser` |
| `opera` | `Opera` |

O resolvedor reconhece todos esses nomes para produzir a mesma normalização e o mesmo erro. A fase 1 dá suporte completo apenas a `Safari` e `Google Chrome`, que são os dois drivers especificados e testados. Firefox, Edge, Arc, Brave e Opera recebem `unsupported_browser` em `browser.*`; `web.open` continua podendo abrir endereços neles. Não existe fallback silencioso para Safari ou Chrome.

### Quando `browser` é omitido

1. Se o aplicativo em primeiro plano é Safari ou Google Chrome, ele é o alvo. Isso preserva o significado de “aba atual”.
2. Caso contrário, resolve-se o handler padrão de `https` do macOS, com `NSWorkspace` via JXA.
3. Se o handler padrão não possui driver nesta fase, retorna-se `unsupported_browser` e pede-se que a próxima intenção nomeie Safari ou Chrome.

Descoberta sem `System Events`:

```sh
osascript -l JavaScript \
  -e 'ObjC.import("AppKit"); const ws=$.NSWorkspace.sharedWorkspace; const front=ws.frontmostApplication; const handler=ws.URLForApplicationToOpenURL($.NSURL.URLWithString("https://example.invalid")); JSON.stringify({frontBundleId: front.bundleIdentifier.js, defaultApp: handler.lastPathComponent.js});'
```

O resultado é consumido em memória e não deve ser escrito em trace. O teste manual resolveu corretamente o navegador padrão instalado, com saída 0.

### Navegador fechado ou sem janela

| Tool | Aplicativo fechado | Aplicativo aberto, sem janela/aba |
|---|---|---|
| `browser.open_tab` | abre o navegador e cria a primeira janela/aba | cria a primeira janela/aba |
| `browser.goto` | abre o navegador e navega a primeira aba | cria a primeira janela/aba e navega |
| `browser.search` | igual a `browser.goto` | igual a `browser.goto` |
| `browser.back`, `browser.forward` | erro `browser_not_running`; não abre nada | erro `no_active_tab` |
| `browser.click_text`, `browser.type`, `browser.read` | erro `browser_not_running`; não abre nada | erro `no_active_tab` |

Uma tool nunca encerra o navegador. Quando o usuário nomeia um navegador, a operação não muda para outro em caso de erro.

## Catálogo de tools

### Resumo de argumentos e risco

Os valores abaixo são o enum `Risk` existente (`Risk::Safe` ou `Risk::Confirm`).

| Tool | Argumentos | Risco | Semântica |
|---|---|---|---|
| `browser.open_tab` | `browser?: string`, `url?: string` | `Safe` | abre uma nova aba; URL omitida vira `about:blank` |
| `browser.goto` | `url: string`, `browser?: string` | `Safe` | navega a aba atual; chamada repetida para a mesma URL é no-op |
| `browser.back` | `browser?: string` | `Safe` | volta uma entrada do histórico |
| `browser.forward` | `browser?: string` | `Safe` | avança uma entrada do histórico |
| `browser.search` | `query: string`, `browser?: string` | `Safe` | pesquisa no Google na aba atual; não cria aba extra |
| `browser.click_text` | `text: string`, `browser?: string`, `match?: "exact" | "contains"`, `occurrence?: integer` | `Confirm` | clica um único elemento interativo visível da página |
| `browser.type` | `text: string`, `field?: string`, `browser?: string`, `mode?: "replace" | "append"` | `Confirm` | digita em campo DOM; campo omitido usa o elemento editável já focado |
| `browser.read` | `browser?: string`, `max_chars?: integer` | `Safe` | lê título e texto visível da aba, sem HTML nem valores de formulário |
| `ui.type` | `text: string`, `expected_app?: string` | `Confirm` | envia texto ao elemento editável focado da janela em primeiro plano |
| `ui.click` | `target: string`, `by?: "text" | "identifier"`, `role?: string`, `match?: "exact" | "contains"`, `occurrence?: integer`, `expected_app?: string` | `Confirm` | executa `AXPress`/`click` num elemento acessível da janela em primeiro plano |

Navegar é `Safe`: abre ou altera página/histórico, sem submeter um controle arbitrário. Todo clique é `Confirm`, porque um texto aparentemente inocente pode ser um botão de compra, envio ou exclusão. Toda digitação é `Confirm`, porque pode alterar uma mensagem, comando ou formulário. Como o risco é estático em `ToolSpec`, não existe reclassificação depois de inspecionar o DOM.

### JSON Schemas normativos

Os schemas usam `additionalProperties: false`; argumentos inesperados são erro.

```json
{
  "browser.open_tab": {
    "type": "object",
    "properties": {
      "browser": { "type": "string", "minLength": 1 },
      "url": { "type": "string", "minLength": 1 }
    },
    "required": [],
    "additionalProperties": false
  },
  "browser.goto": {
    "type": "object",
    "properties": {
      "url": { "type": "string", "minLength": 1 },
      "browser": { "type": "string", "minLength": 1 }
    },
    "required": ["url"],
    "additionalProperties": false
  },
  "browser.back": {
    "type": "object",
    "properties": { "browser": { "type": "string", "minLength": 1 } },
    "required": [],
    "additionalProperties": false
  },
  "browser.forward": {
    "type": "object",
    "properties": { "browser": { "type": "string", "minLength": 1 } },
    "required": [],
    "additionalProperties": false
  },
  "browser.search": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "minLength": 1, "maxLength": 2000 },
      "browser": { "type": "string", "minLength": 1 }
    },
    "required": ["query"],
    "additionalProperties": false
  },
  "browser.click_text": {
    "type": "object",
    "properties": {
      "text": { "type": "string", "minLength": 1, "maxLength": 500 },
      "browser": { "type": "string", "minLength": 1 },
      "match": { "type": "string", "enum": ["exact", "contains"], "default": "exact" },
      "occurrence": { "type": "integer", "minimum": 1 }
    },
    "required": ["text"],
    "additionalProperties": false
  },
  "browser.type": {
    "type": "object",
    "properties": {
      "text": { "type": "string", "maxLength": 10000 },
      "field": { "type": "string", "minLength": 1, "maxLength": 500 },
      "browser": { "type": "string", "minLength": 1 },
      "mode": { "type": "string", "enum": ["replace", "append"], "default": "replace" }
    },
    "required": ["text"],
    "additionalProperties": false
  },
  "browser.read": {
    "type": "object",
    "properties": {
      "browser": { "type": "string", "minLength": 1 },
      "max_chars": { "type": "integer", "minimum": 1, "maximum": 50000, "default": 12000 }
    },
    "required": [],
    "additionalProperties": false
  },
  "ui.type": {
    "type": "object",
    "properties": {
      "text": { "type": "string", "maxLength": 10000 },
      "expected_app": { "type": "string", "minLength": 1 }
    },
    "required": ["text"],
    "additionalProperties": false
  },
  "ui.click": {
    "type": "object",
    "properties": {
      "target": { "type": "string", "minLength": 1, "maxLength": 500 },
      "by": { "type": "string", "enum": ["text", "identifier"], "default": "text" },
      "role": { "type": "string", "minLength": 1 },
      "match": { "type": "string", "enum": ["exact", "contains"], "default": "exact" },
      "occurrence": { "type": "integer", "minimum": 1 },
      "expected_app": { "type": "string", "minLength": 1 }
    },
    "required": ["target"],
    "additionalProperties": false
  }
}
```

### Contratos detalhados

#### `browser.open_tab`

- `url` usa a mesma validação HTTP(S) de `web.open`; `about:blank` só é o valor interno quando `url` é omitida.
- Com janela existente, cria e seleciona exatamente uma aba no fim.
- Sem janela, reutiliza a aba inicial criada pela nova janela; não cria uma aba vazia adicional.
- É deliberadamente não idempotente: duas intenções explícitas de “nova aba” produzem duas abas.
- O resultado não ecoa a URL: `{ "status": "opened", "browser": "Safari" }`.

#### `browser.goto`

- Aceita apenas HTTP(S), com a normalização já usada por `web.open`.
- Atua somente na aba atual; não cria outra aba quando já existe uma.
- Canonicaliza esquema e host em minúsculas, remove porta padrão e trata caminho vazio como `/`; preserva caminho, query e fragmento.
- Se a URL canonical atual é a pedida, retorna `{ "status": "already_there", "changed": false }` sem atribuir `URL`, sem recarregar e sem criar histórico.
- Mantém por três segundos um fingerprint em memória de `(navegador, janela, aba, URL canonical)` para absorver uma repetição enquanto a primeira navegação ainda está carregando ou redirecionando.
- A segunda chamada igual observada em sequência recebe `already_there`/`duplicate_ignored`; uma chamada posterior, após mudança da aba, navega normalmente.
- O resultado nunca ecoa URL nem query.

#### `browser.back` e `browser.forward`

- Movem uma posição por chamada; não são deduplicadas, porque “volta duas vezes” é uma intenção válida.
- Em limite de histórico, retornam `at_boundary` sem erro.
- Chrome usa os comandos nativos `go back`/`go forward`.
- Safari usa `history.back()`/`history.forward()` e, portanto, exige JavaScript via Apple Events habilitado.

#### `browser.search`

- A fase 1 usa Google de forma explícita e igual nos dois drivers.
- O core codifica `query` como componente de query UTF-8 e monta o endereço de busca; AppleScript nunca concatena texto cru do usuário.
- Delega a navegação a `browser.goto`, herdando a idempotência. Duas pesquisas consecutivas iguais não recarregam a página.
- Não pressiona Enter, não usa a barra de endereço e não cria aba extra.
- O resultado não ecoa a consulta.

#### `browser.click_text`

- Candidatos: `a`, `button`, `[role=button]`, `input[type=submit]` e `input[type=button]`.
- Ignora elementos sem retângulo, invisíveis, desabilitados ou com `aria-disabled=true`.
- O rótulo vem de `innerText`, `value` ou `aria-label`, nessa ordem; espaços são colapsados e a comparação padrão é exata, sem diferença entre maiúsculas/minúsculas.
- Sem `occurrence`, zero candidatos vira `target_not_found` e mais de um vira `ambiguous_target`; nada é clicado.
- Com `occurrence`, seleciona a ocorrência 1-based depois do filtro.
- Não atravessa iframe cross-origin nem shadow root fechado nesta fase.
- O resultado informa apenas tag/papel e cardinalidade; não repete o texto pedido.

#### `browser.type`

- Se `field` existe, procura `input`, `textarea` ou `[contenteditable=true]` por `aria-label`, `placeholder`, `name` ou texto de `<label>`.
- Sem `field`, exige que `document.activeElement` seja editável.
- Mais de um candidato sem resolução vira `ambiguous_target`.
- `replace` substitui o valor; `append` acrescenta ao valor atual.
- Usa o setter nativo de `value` e dispara `input` e `change`; não envia o formulário e não pressiona Enter.
- Recusa `type=password` e os `autocomplete` `current-password`, `new-password`, `one-time-code`, `cc-number` e `cc-csc`, com `sensitive_field`. O texto recusado não entra em resultado, erro ou log.

#### `browser.read`

- Retorna `{ browser, title, text, truncated }`.
- Não retorna URL, HTML, cookies, storage, cabeçalhos, atributos ocultos nem valores de `input`/`textarea`.
- Normaliza NUL e corta em `max_chars` caracteres Unicode; o padrão é 12.000.
- Safari usa as propriedades nativas `name` e `text`, sem depender de JavaScript.
- Chrome usa `document.title` e `document.body.innerText` via JavaScript.
- Leituras repetidas não alteram estado e são naturalmente idempotentes.

#### `ui.type`

- Atua no elemento focado do processo em primeiro plano; não ativa outro aplicativo.
- `expected_app`, quando presente, é uma guarda: divergência retorna `focus_changed` antes de qualquer tecla.
- Captura o PID em primeiro plano e o confere novamente imediatamente antes de `keystroke`.
- Exige papel editável (`AXTextField`, `AXTextArea`, `AXComboBox` ou `AXEditable=true`).
- Recusa `AXSecureTextField` e qualquer descrição de papel seguro.
- Digita somente o texto; não envia Return, Tab ou atalho adicional.
- O resultado é `{ "status": "typed", "app": "…" }`, sem ecoar texto nem tamanho.

#### `ui.click`

- Atua somente na árvore de Acessibilidade do processo em primeiro plano.
- `by=text` compara `AXTitle`/`name`, `AXDescription` e texto acessível; `by=identifier` compara `AXIdentifier` exatamente.
- `role` restringe o papel AX, sem torná-lo obrigatório.
- Sem `occurrence`, exige exatamente um candidato; nunca escolhe “o primeiro” em caso ambíguo.
- Prefere `AXPress`; usa `click` de `System Events` quando o elemento não expõe essa action.
- Não há clique por coordenada nesta fase.
- A travessia tem teto de 5.000 elementos e timeout de dois segundos.
- Confere novamente o PID em primeiro plano antes de pressionar o elemento.

## Transporte seguro de argumentos

Valores do usuário nunca são interpolados no fonte AppleScript nem montados por shell. A implementação usa `tokio::process::Command`, passa cada `-e` como argumento e passa os dados depois de `--`, recebidos por `on run argv`.

JavaScript é um payload constante. Quando precisa incorporar `text`, `field` ou opções, o core serializa um objeto com `serde_json` e o payload faz `JSON.parse`; não existe concatenação manual de aspas.

Os exemplos abaixo são reproduções de terminal para validação. Na aplicação não há shell.

## AppleScript por tool

### `browser.open_tab`

Safari:

```sh
osascript \
  -e 'on run argv' \
  -e 'set targetURL to item 1 of argv' \
  -e 'tell application "Safari"' \
  -e 'activate' \
  -e 'if (count of windows) = 0 then' \
  -e 'make new document with properties {URL:targetURL}' \
  -e 'else' \
  -e 'tell front window' \
  -e 'set createdTab to make new tab at end of tabs with properties {URL:targetURL}' \
  -e 'set current tab to createdTab' \
  -e 'end tell' \
  -e 'end if' \
  -e 'end tell' \
  -e 'end run' -- 'about:blank'
```

Chrome:

```sh
osascript \
  -e 'on run argv' \
  -e 'set targetURL to item 1 of argv' \
  -e 'tell application "Google Chrome"' \
  -e 'activate' \
  -e 'if (count of windows) = 0 then' \
  -e 'set createdWindow to make new window' \
  -e 'set URL of active tab of createdWindow to targetURL' \
  -e 'else' \
  -e 'tell front window' \
  -e 'make new tab at end of tabs with properties {URL:targetURL}' \
  -e 'set active tab index to count of tabs' \
  -e 'end tell' \
  -e 'end if' \
  -e 'end tell' \
  -e 'end run' -- 'about:blank'
```

### `browser.goto`

O core executa a comparação canonical/idempotente antes destes scripts. Safari:

```sh
osascript \
  -e 'on run argv' \
  -e 'set targetURL to item 1 of argv' \
  -e 'tell application "Safari"' \
  -e 'activate' \
  -e 'if (count of windows) = 0 then' \
  -e 'make new document with properties {URL:targetURL}' \
  -e 'else' \
  -e 'set URL of current tab of front window to targetURL' \
  -e 'end if' \
  -e 'end tell' \
  -e 'end run' -- 'https://example.com/'
```

Chrome:

```sh
osascript \
  -e 'on run argv' \
  -e 'set targetURL to item 1 of argv' \
  -e 'tell application "Google Chrome"' \
  -e 'activate' \
  -e 'if (count of windows) = 0 then' \
  -e 'set createdWindow to make new window' \
  -e 'set URL of active tab of createdWindow to targetURL' \
  -e 'else' \
  -e 'set URL of active tab of front window to targetURL' \
  -e 'end if' \
  -e 'end tell' \
  -e 'end run' -- 'https://example.com/'
```

### `browser.back` e `browser.forward`

Safari, voltar:

```sh
osascript \
  -e 'tell application "Safari"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'do JavaScript "history.back()" in current tab of front window' \
  -e 'end tell'
```

Safari, avançar:

```sh
osascript \
  -e 'tell application "Safari"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'do JavaScript "history.forward()" in current tab of front window' \
  -e 'end tell'
```

Chrome, voltar:

```sh
osascript \
  -e 'tell application "Google Chrome"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'go back active tab of front window' \
  -e 'end tell'
```

Chrome, avançar:

```sh
osascript \
  -e 'tell application "Google Chrome"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'go forward active tab of front window' \
  -e 'end tell'
```

### `browser.search`

O core aplica percent-encoding UTF-8 e monta a URL de busca. Depois usa exatamente o caminho de `browser.goto`. Exemplos equivalentes:

Safari:

```sh
osascript \
  -e 'on run argv' \
  -e 'tell application "Safari" to set URL of current tab of front window to item 1 of argv' \
  -e 'end run' -- 'https://www.google.com/search?q=overclock'
```

Chrome:

```sh
osascript \
  -e 'on run argv' \
  -e 'tell application "Google Chrome" to set URL of active tab of front window to item 1 of argv' \
  -e 'end run' -- 'https://www.google.com/search?q=overclock'
```

### Wrapper JavaScript de Safari e Chrome

`browser.click_text` e `browser.type` usam o mesmo payload JS nos dois navegadores. O payload completo é passado em `argv`; os wrappers não interpolam dados.

Safari:

```sh
osascript \
  -e 'on run argv' \
  -e 'set jsCode to item 1 of argv' \
  -e 'tell application "Safari"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'return do JavaScript jsCode in current tab of front window' \
  -e 'end tell' \
  -e 'end run' -- "$JS_PAYLOAD"
```

Chrome:

```sh
osascript \
  -e 'on run argv' \
  -e 'set jsCode to item 1 of argv' \
  -e 'tell application "Google Chrome"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'activate' \
  -e 'return execute active tab of front window javascript jsCode' \
  -e 'end tell' \
  -e 'end run' -- "$JS_PAYLOAD"
```

### Payload de `browser.click_text`

`ARGS_JSON` é produzido por `serde_json`, por exemplo com `text`, `match` e `occurrence`.

```js
(() => {
  const args = JSON.parse(ARGS_JSON);
  const norm = (value) => (value || "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
  const wanted = norm(args.text);
  const nodes = [...document.querySelectorAll(
    "a,button,[role=button],input[type=submit],input[type=button]"
  )].filter((element) => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 &&
      style.visibility !== "hidden" && style.display !== "none" &&
      !element.disabled && element.getAttribute("aria-disabled") !== "true";
  });
  const matches = nodes.filter((element) => {
    const label = norm(
      element.innerText || element.value || element.getAttribute("aria-label") || ""
    );
    return args.match === "contains" ? label.includes(wanted) : label === wanted;
  });
  if (matches.length === 0) return JSON.stringify({ ok: false, code: "target_not_found" });
  if (args.occurrence == null && matches.length !== 1) {
    return JSON.stringify({ ok: false, code: "ambiguous_target", count: matches.length });
  }
  const index = args.occurrence == null ? 0 : args.occurrence - 1;
  if (index < 0 || index >= matches.length) {
    return JSON.stringify({ ok: false, code: "occurrence_out_of_range", count: matches.length });
  }
  const target = matches[index];
  target.click();
  return JSON.stringify({ ok: true, tag: target.tagName.toLowerCase(), count: matches.length });
})()
```

### Payload de `browser.type`

```js
(() => {
  const args = JSON.parse(ARGS_JSON);
  const norm = (value) => (value || "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
  let matches;
  if (args.field == null) {
    matches = [document.activeElement].filter((element) =>
      element && element.matches("input,textarea,[contenteditable=true]")
    );
  } else {
    const wanted = norm(args.field);
    matches = [...document.querySelectorAll("input,textarea,[contenteditable=true]")].filter((element) => {
      const labels = element.labels ? [...element.labels].map((label) => label.innerText).join(" ") : "";
      return [
        element.getAttribute("aria-label"),
        element.getAttribute("placeholder"),
        element.getAttribute("name"),
        labels
      ].some((value) => norm(value) === wanted);
    });
  }
  if (matches.length === 0) return JSON.stringify({ ok: false, code: "target_not_found" });
  if (matches.length !== 1) {
    return JSON.stringify({ ok: false, code: "ambiguous_target", count: matches.length });
  }
  const target = matches[0];
  const autocomplete = (target.autocomplete || "").toLowerCase();
  const sensitive = target.matches("input[type=password]") || [
    "current-password", "new-password", "one-time-code", "cc-number", "cc-csc"
  ].includes(autocomplete);
  if (sensitive) return JSON.stringify({ ok: false, code: "sensitive_field" });

  const current = target.isContentEditable ? target.textContent : target.value;
  const next = args.mode === "append" ? current + args.text : args.text;
  target.focus();
  if (target.isContentEditable) {
    target.textContent = next;
  } else {
    const prototype = target.tagName === "TEXTAREA"
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value").set.call(target, next);
  }
  target.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: args.text
  }));
  target.dispatchEvent(new Event("change", { bubbles: true }));
  return JSON.stringify({ ok: true });
})()
```

### `browser.read`

Safari não precisa executar JavaScript:

```sh
osascript \
  -e 'tell application "Safari"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'set t to current tab of front window' \
  -e 'return (name of t) & linefeed & (text of t)' \
  -e 'end tell'
```

Chrome usa JavaScript e serializa o resultado:

```sh
osascript \
  -e 'on run argv' \
  -e 'set jsCode to item 1 of argv' \
  -e 'tell application "Google Chrome"' \
  -e 'if not running then error "browser_not_running"' \
  -e 'if (count of windows) = 0 then error "no_active_tab"' \
  -e 'return execute active tab of front window javascript jsCode' \
  -e 'end tell' \
  -e 'end run' -- '(() => { const limit=12000; const raw=(document.body?.innerText || "").replace(/\u0000/g, ""); return JSON.stringify({title: document.title, text: raw.slice(0, limit), truncated: raw.length > limit}); })()'
```

O corte real usa `max_chars`, passado no `ARGS_JSON`, e ocorre dentro do payload antes de o conteúdo cruzar Apple Events.

## AppleScript de `ui.*`

### `ui.type`

O script normativo recebe texto e `expected_app`. O exemplo usa uma guarda de aplicativo; o resultado nunca devolve o payload.

```sh
osascript \
  -e 'on run argv' \
  -e 'set payload to item 1 of argv' \
  -e 'set expectedApp to item 2 of argv' \
  -e 'tell application "System Events"' \
  -e 'set targetProcess to first application process whose frontmost is true' \
  -e 'set targetPid to unix id of targetProcess' \
  -e 'set targetName to name of targetProcess' \
  -e 'if expectedApp is not "" and targetName is not expectedApp then error "focus_changed"' \
  -e 'tell targetProcess' \
  -e 'set focusedElement to value of attribute "AXFocusedUIElement"' \
  -e 'set focusedRole to value of attribute "AXRole" of focusedElement' \
  -e 'try' \
  -e 'set focusedSubrole to value of attribute "AXSubrole" of focusedElement' \
  -e 'on error' \
  -e 'set focusedSubrole to ""' \
  -e 'end try' \
  -e 'if focusedSubrole is "AXSecureTextField" then error "sensitive_field"' \
  -e 'if focusedRole is not in {"AXTextField", "AXTextArea", "AXComboBox"} then error "target_not_editable"' \
  -e 'end tell' \
  -e 'set currentPid to unix id of first application process whose frontmost is true' \
  -e 'if currentPid is not targetPid then error "focus_changed"' \
  -e 'keystroke payload' \
  -e 'return targetName' \
  -e 'end tell' \
  -e 'end run' -- 'texto de teste' 'TextEdit'
```

Na implementação, a checagem também aceita `AXEditable=true`. Campos seguros são recusados mesmo em modo de acesso total.

### `ui.click`

O executor faz a coleta com timeout, lê somente metadados AX necessários e exige unicidade antes de pressionar. Este exemplo reproduz o caminho `by=text` usado no teste manual:

```sh
osascript \
  -e 'on run argv' \
  -e 'set wanted to item 1 of argv' \
  -e 'set expectedApp to item 2 of argv' \
  -e 'tell application "System Events"' \
  -e 'set targetProcess to first application process whose frontmost is true' \
  -e 'set targetPid to unix id of targetProcess' \
  -e 'set targetName to name of targetProcess' \
  -e 'if expectedApp is not "" and targetName is not expectedApp then error "focus_changed"' \
  -e 'tell targetProcess' \
  -e 'if (count of windows) = 0 then error "no_active_window"' \
  -e 'set candidates to entire contents of front window' \
  -e 'set matches to {}' \
  -e 'repeat with candidate in candidates' \
  -e 'try' \
  -e 'if (name of candidate as text) is wanted or (description of candidate as text) is wanted then set end of matches to candidate' \
  -e 'end try' \
  -e 'end repeat' \
  -e 'if (count of matches) = 0 then error "target_not_found"' \
  -e 'if (count of matches) is not 1 then error "ambiguous_target"' \
  -e 'set selectedElement to item 1 of matches' \
  -e 'end tell' \
  -e 'set currentPid to unix id of first application process whose frontmost is true' \
  -e 'if currentPid is not targetPid then error "focus_changed"' \
  -e 'tell targetProcess to click selectedElement' \
  -e 'return targetName' \
  -e 'end tell' \
  -e 'end run' -- 'close button' 'TextEdit'
```

O caminho `by=identifier` lê `value of attribute "AXIdentifier"`; `role`, `match` e `occurrence` são aplicados antes da exigência de unicidade.

## Permissões e preferências no macOS

| Recurso | Exigência | Situação atual / ação da implementação |
|---|---|---|
| Apple Events para Safari | Automação: OpenJarvisBR → Safari | adicionar alvo inofensivo de Safari em `TARGETS`; consentimento é por aplicativo alvo |
| Apple Events para Chrome | Automação: OpenJarvisBR → Google Chrome | adicionar alvo inofensivo de Chrome em `TARGETS`; consentimento é separado do Safari |
| `System Events` | Automação: OpenJarvisBR → System Events | já existe alvo `system_events` em `permissions.rs` |
| `ui.type`/`ui.click` | Acessibilidade para o OpenJarvisBR | `AXIsProcessTrusted*` e o prompt já existem em `permissions.rs` |
| JavaScript no Safari | Safari Settings → Developer → Allow JavaScript from Apple Events | preferência do navegador; não é TCC e não deve ser ligada automaticamente |
| JavaScript no Chrome | View → Developer → Allow JavaScript from Apple Events | preferência do navegador; não é TCC e não deve ser ligada automaticamente |

`permissions.rs` já possui o fluxo, estados e timeout para Acessibilidade e Automação, mas a lista atual de Automação contém `System Events`, Calendário, Lembretes, Música e Spotify; ela ainda não prepara Safari nem Chrome. A implementação de `browser.*` deve acrescentar esses alvos usando chamadas inofensivas, como obter o nome ou contar janelas, sem navegar nem ler páginas.

Nenhuma tool tenta contornar uma negativa, editar preferências do navegador com `defaults`, abrir o Keychain ou solicitar senha. Ao detectar JavaScript desabilitado, retorna `javascript_from_apple_events_disabled` com instrução curta para o usuário habilitar manualmente a opção do navegador.

## Erros e privacidade

Erros estáveis:

- `browser_not_installed`
- `browser_not_running`
- `unsupported_browser`
- `no_active_tab`
- `no_active_window`
- `automation_permission_denied`
- `accessibility_permission_denied`
- `javascript_from_apple_events_disabled`
- `target_not_found`
- `ambiguous_target`
- `occurrence_out_of_range`
- `target_not_editable`
- `sensitive_field`
- `focus_changed`
- `timeout`

Mensagens e resultados não incluem URL atual, query de busca, texto digitado, cookies, valores de formulário ou conteúdo de campos seguros. `browser.read` é a única tool que devolve conteúdo de página, limitado a texto visível e ao teto pedido. Stderr de `osascript` é traduzido para esses códigos antes de chegar ao modelo.

## Idempotência por operação

| Tool | Chamada repetida idêntica |
|---|---|
| `browser.goto` | segunda chamada é no-op e não recarrega |
| `browser.search` | herda a regra de `goto` |
| `browser.read` | somente lê; não altera estado |
| `browser.open_tab` | cria outra aba por definição |
| `browser.back`, `browser.forward` | move outra posição por definição |
| `browser.click_text`, `browser.type` | não deduplica; confirmação cobre cada efeito |
| `ui.click`, `ui.type` | não deduplica; foco/estado pode ter mudado |

O cache curto de `goto` não substitui o conserto do JRV-92: ele reduz o efeito da duplicação, enquanto o turno ainda deve emitir uma única chamada.

## Validação manual observada em 2026-09-18

Safari e Google Chrome estavam abertos. Os testes usaram janelas/abas temporárias identificáveis e foram fechados ao final; nenhuma aba pessoal foi lida.

| Superfície | Trecho testado | Resultado observado |
|---|---|---|
| resolvedor padrão | `NSWorkspace.URLForApplicationToOpenURL` via JXA | saída 0; resolveu o navegador padrão instalado |
| Chrome | `make new tab`, selecionar a última aba | saída 0; contagem aumentou exatamente em 1 |
| Chrome | atribuir URL e repetir `goto` com a mesma URL | saída 0; primeira mudou a aba e segunda virou no-op |
| Chrome | `go back` e `go forward` na aba de teste | saída 0; voltou e avançou entre os dois endereços esperados |
| Chrome | navegação para a URL de pesquisa codificada | saída 0; resultado continha a consulta esperada |
| Chrome | JavaScript de clique por texto | saída 0; evento `click` da página de teste foi disparado |
| Chrome | JavaScript de `browser.type` por `aria-label` | saída 0; campo recebeu o valor e eventos foram disparados |
| Chrome | JavaScript de `browser.read` | saída 0; título e texto visível esperados foram retornados |
| Safari | `make new tab`, selecionar `current tab` | saída 0; contagem aumentou exatamente em 1 |
| Safari | atribuir URL e repetir `goto` com a mesma URL | saída 0; primeira mudou a aba e segunda virou no-op |
| Safari | navegação para a URL de pesquisa codificada | saída 0; resultado continha a consulta esperada |
| Safari | propriedades nativas `name` e `text` | saída 0; título e texto visível esperados foram retornados |
| Safari | `history.back()` | saída 1; bloqueado porque “Allow JavaScript from Apple Events” está desativado |
| Safari | `history.forward()` | saída 1; mesmo bloqueio de preferência |
| Safari | JavaScript de clique por texto | saída 1; mesmo bloqueio de preferência |
| Safari | JavaScript de `browser.type` | saída 1; mesmo bloqueio de preferência |
| `ui.type` | `System Events` em `AXTextArea` de documento temporário do TextEdit | saída 0; texto conferido sem imprimir seu conteúdo |
| `ui.click` | busca por descrição e clique no botão de fechar da janela temporária | saída 0; uma janela virou zero |

O teste do Chrome também comprovou que JavaScript por Apple Events está habilitado nessa instalação. No Safari, os quatro trechos dependentes de JavaScript foram executados de verdade e falharam na pré-condição documentada; não se alterou a preferência de segurança para forçar um sucesso. A implementação deve preservar essa falha explícita até o usuário habilitar a opção.

## Fora desta fase

- clique por coordenada ou OCR;
- preenchimento de senha, código de autenticação ou cartão;
- leitura de HTML bruto, cookies, storage ou headers;
- iframe cross-origin e shadow root fechado;
- suporte completo a Firefox, Edge, Arc, Brave e Opera;
- ligar preferências de segurança automaticamente;
- deduplicação geral do turno, que pertence ao JRV-92.

## Pronto para implementar quando

- todas as tools acima mantiverem exatamente nomes, schemas e riscos desta spec;
- Safari e Chrome produzirem os mesmos resultados funcionais onde suas APIs permitem;
- a interface de permissões preparar Automação para Safari, Chrome e System Events e continuar preparando Acessibilidade;
- o erro de JavaScript desabilitado indicar a preferência manual correta;
- a segunda chamada idêntica de `browser.goto` não recarregar nem criar histórico;
- `browser.*` ganhar de `ui.*` para conteúdo da página, sem fallback silencioso;
- campos seguros forem recusados antes de qualquer digitação;
- testes unitários cobrirem normalização, risco, ambiguidade, idempotência, foco e redaction dos resultados.
