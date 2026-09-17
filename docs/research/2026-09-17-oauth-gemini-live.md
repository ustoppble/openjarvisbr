# Pesquisa: Gemini Live por login OAuth em vez de chave de API [JRV-70]

Data: 2026-09-17 · Autor: worker scout (Claude Opus 5) · Card: JRV-70

## Resumo executivo

1. **Não.** O "Login with Google" do Gemini CLI fala com o Code Assist (`cloudcode-pa.googleapis.com/v1internal`), que só tem `generateContent`/`streamGenerateContent`/`countTokens`. **Não há Live nem BidiGenerateContent nesse caminho.** O modo de voz do próprio Gemini CLI exige `GEMINI_API_KEY`.
2. Usar o OAuth do Gemini CLI/Code Assist/Antigravity num app de terceiros **viola os termos por escrito**. O Google já suspendeu contas em massa por isso em fev/2026: a primeira vez pede recertificação, a segunda **bane de vez**.
3. A assinatura Google AI Pro/Ultra **não dá cota de API para apps de terceiros**. O que ela dá e serve aqui são **US$ 10/mês (Pro) ou US$ 100/mês (Ultra) em créditos do Google Cloud**, usáveis no Vertex AI e na Gemini API.
4. O único jeito oficial de rodar a Live com **OAuth de usuário** é o **Vertex AI** (`gcloud auth application-default login` → Bearer). Mas ele exige projeto com faturamento ativo, cobra por uso e hoje só lista `gemini-live-2.5-flash-native-audio` (o `gemini-3.8-live` não aparece na doc do Vertex).
5. **Recomendação:** manter a chave da Gemini API, **usando o free tier** para testes. Em produção, ligar o faturamento no mesmo projeto e **abater a conta com os US$ 10/mês do AI Pro**. 1 h/dia de conversa sai por uns **US$ 20–40/mês**, e os créditos cobrem de 25 a 50% disso. Zero risco para a conta.

---

## 1. Como o Gemini CLI autentica com "Login with Google"

Fonte: código em `github.com/google-gemini/gemini-cli`, commit `6a466a7` (15/09/2026, v0.62.0-nightly).

| Item | O que o código faz | Onde |
|---|---|---|
| Fluxo | OAuth2 de "installed app" (loopback/navegador) usando o `OAuth2Client` do google-auth-library. O client id e o secret ficam embutidos no código (valores omitidos aqui). O comentário diz: "the client secret is obviously not treated as a secret". | [`packages/core/src/code_assist/oauth2.ts` L75-L86](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/oauth2.ts) |
| Escopos | `cloud-platform`, `userinfo.email`, `userinfo.profile` | `oauth2.ts` L88-L92 |
| Tipo de auth | `AuthType.LOGIN_WITH_GOOGLE = 'oauth-personal'`. Com esse tipo, o CLI **não lê** chave nenhuma. | [`packages/core/src/core/contentGenerator.ts` L63-L70, ~L168](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/core/contentGenerator.ts) |
| Endpoint | `CODE_ASSIST_ENDPOINT = 'https://cloudcode-pa.googleapis.com'`, `CODE_ASSIST_API_VERSION = 'v1internal'` (API **interna**, sem contrato público) | [`packages/core/src/code_assist/server.ts` L73-L74](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/code_assist/server.ts) |
| Métodos expostos | `streamGenerateContent` (L119), `generateContent` (L206), `onboardUser`, `loadCodeAssist`, `countTokens`, `retrieveUserQuota`, `listExperiments`, `fetchAdminControls`, `recordConversation*`, `recordCodeAssistMetrics`. **`embedContent` lança `Error()`** (L347). Tudo é POST/GET REST, sem WebSocket. | `server.ts` L93-L470 |
| Live/Bidi | Busca por `BidiGenerateContent` no pacote inteiro: **uma ocorrência só**, em `voice/geminiLiveTranscriptionProvider.ts`. Ela conecta em `wss://generativelanguage.googleapis.com/...BidiGenerateContent?key=<API_KEY>` com `gemini-3.1-flash-live-preview`. O comentário é "Connects to the Gemini Live API using raw WebSockets to support **API Key** authentication". | [`packages/core/src/voice/geminiLiveTranscriptionProvider.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/voice/geminiLiveTranscriptionProvider.ts) |
| Prova de que o OAuth não serve pra Live | O hook do modo de voz, logado com Google e sem chave, falha com: *"Cloud voice mode requires a GEMINI_API_KEY"*. | [`packages/cli/src/ui/hooks/useVoiceMode.ts` L167-L169](https://github.com/google-gemini/gemini-cli/blob/main/packages/cli/src/ui/hooks/useVoiceMode.ts) |
| Modelos | Família de texto: `gemini-2.5-pro`, `gemini-3-pro-preview`, `gemini-3.1-pro-preview`, `gemini-3.5-flash`, `gemini-3-flash`, `gemini-3.1-flash-lite`, Gemma 4, com roteamento `auto`. **Nenhum modelo Live.** | [`packages/core/src/config/models.ts` L54-L115](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/config/models.ts) |

**Cotas gratuitas/assinatura** ([`docs/resources/quota-and-pricing.md`](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/quota-and-pricing.md)):

| Conta | Requisições/usuário/dia |
|---|---|
| Code Assist para indivíduos (grátis) | 1.000 |
| Google AI Pro | 1.500 |
| Google AI Ultra | 2.000 |
| Chave Gemini API sem faturamento | 250 (só Flash) |

A doc avisa que "one prompt might result in multiple model requests" ([Code Assist quotas](https://docs.cloud.google.com/gemini/docs/quotas)). Os modelos são "as determined by Gemini CLI", ou seja, o usuário não escolhe o modelo livremente.

**Conclusão 1:** o caminho OAuth do Gemini CLI é uma API interna de **texto** para a ferramenta de código. Não existe Live, áudio nem WebSocket nele.

---

## 2. A Live API aceita OAuth de usuário em algum endpoint oficial?

### 2a. Gemini Developer API (AI Studio): não para a Live

- A doc de WebSocket da Live diz que a autenticação é a chave de API no parâmetro de query: `wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent?key=YOUR_API_KEY`. Ela oferece como alternativa só os **tokens efêmeros**, via `BidiGenerateContentConstrained?access_token=…` ([Get started WebSocket](https://ai.google.dev/gemini-api/docs/live-api/get-started-websocket), [referência Live](https://ai.google.dev/api/live)).
- Tokens efêmeros **são criados com a chave de API**, no backend (`client.auth_tokens.create()`). Eles duram 30 min por padrão, a nova sessão precisa abrir em até 1 min, e eles "are only compatible with Live API". **Não substituem a chave, só a escondem do cliente** ([Ephemeral tokens](https://ai.google.dev/gemini-api/docs/ephemeral-tokens)).
- O SDK oficial `python-genai` confirma. Quando `vertexai=False`, a Live **só tem ramo para chave de API ou token `auth_tokens/`**. O ramo de Bearer/credenciais ADC existe apenas quando `vertexai=True` ([`google/genai/live.py` ~L970-L1077](https://github.com/googleapis/python-genai/blob/main/google/genai/live.py)).
- Existe um [guia OAuth da Gemini API](https://ai.google.dev/gemini-api/docs/oauth). Ele pede projeto Cloud próprio, Generative Language API ativada, client OAuth desktop próprio e escopos `cloud-platform` + `generative-language.retriever`. A doc o chama de "appropriate for a testing environment", **não menciona a Live/WebSocket**, e a cota continua sendo a do **projeto** (limites são "per project, not per API key", [rate limits](https://ai.google.dev/gemini-api/docs/rate-limits)). Ou seja, mesmo funcionando, **não usa a cota da assinatura**: é o mesmo free tier/faturamento de uma chave. *Não testei se o WebSocket da Live aceita Bearer nesse endpoint; nada na documentação diz que sim.*

### 2b. Vertex AI: sim, com credenciais de usuário, mas pago

- Endpoint: `wss://{região}-aiplatform.googleapis.com/ws/google.cloud.aiplatform.v1beta1.LlmBidiService/BidiGenerateContent`, com header `Authorization: Bearer <access token>`. O modelo vai como `projects/{PROJETO}/locations/{região}/publishers/google/models/{modelo}` (demo oficial: [`geminilive.js` L125-L154](https://github.com/GoogleCloudPlatform/generative-ai/blob/main/gemini/multimodal-live-api/native-audio-websocket-demo-apps/plain-js-demo-app/frontend/geminilive.js)).
- O token vem de `gcloud auth application-default login` + `google.auth.default()` ([`server.py` L31-L39, L89](https://github.com/GoogleCloudPlatform/generative-ai/blob/main/gemini/multimodal-live-api/native-audio-websocket-demo-apps/plain-js-demo-app/server.py); tutorial [Vertex Live via WebSockets](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/live-api/get-started-websocket)).
- Modelos listados hoje na doc do Vertex: **`gemini-live-2.5-flash-native-audio` (GA)** e o preview `…-09-2025`, que sai em 19/03/2026 ([Vertex Live API overview](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/live-api)). **`gemini-3.8-live` não aparece na tabela do Vertex.** Hoje, trocar para o Vertex significa trocar de modelo.
- Custo: o Vertex cobra por uso no projeto. Relatos indicam que, sem faturamento, o setup é fechado com erro de política 1008 (não confirmado em doc oficial). Tentei ler a tabela de preços do Vertex para a linha da Live e **não consegui extrair** ([Vertex pricing](https://cloud.google.com/vertex-ai/generative-ai/pricing)). Uso a ordem de grandeza da Developer API como referência.
- Créditos: o Free Trial de US$ 300/90 dias vale para Vertex AI, mas é só para quem **nunca pagou** Google Cloud/Firebase/Maps, exige cartão e **não vale para "Gemini API in AI Studio costs"** ([Free Trial](https://docs.cloud.google.com/free/docs/free-cloud-features)). Os créditos mensais do AI Pro/Ultra também valem para o Vertex (seção 3).
- Limitação prática para um app desktop distribuído: cada usuário precisaria de `gcloud`, projeto próprio e faturamento. **Não é "login com Google" de consumidor.**

**Conclusão 2:** a Live por OAuth de usuário existe **só no Vertex AI**, e é cobrada. Não existe endpoint oficial em que a Live consuma a cota da assinatura ou do Code Assist.

---

## 3. O que a assinatura Google AI Pro/Ultra dá

- A ajuda do Google One lista para o AI Pro: app Gemini (inclui o Gemini Live **no app**), Google AI Studio (experimentar), Jules, Antigravity, Gemini in Android Studio, Colab e **Google Developer Program premium**, que inclui Code Assist com cota maior, **US$ 10/mês em créditos Cloud** e Firebase Studio ([Use Google AI Pro benefits](https://support.google.com/googleone/answer/14534406?hl=en)). **Não há cota de API para apps de terceiros.**
- O anúncio de 27/01/2026 diz: AI Pro "$10 per month in Google Cloud credits", AI Ultra "$100 per month". Os créditos podem ser usados na Gemini API e no Vertex AI/Cloud Run ([blog.google](https://blog.google/innovation-and-ai/technology/developers-tools/gdp-premium-ai-pro-ultra/)).
- FAQ do Developer Program: os créditos valem para "all Google Cloud products including Firebase, Vertex AI", **precisam ser ativados uma vez** numa conta de faturamento, expiram 1 ano após a concessão e têm as exclusões padrão ([Benefits FAQ](https://developers.google.com/profile/help/benefits)).
- Billing da Gemini API: numa conta **Prepay**, "you must add funds to your account before you can use promotional Cloud Credits". Depois de pôr saldo, os créditos promocionais são consumidos primeiro ([Gemini API billing](https://ai.google.dev/gemini-api/docs/billing)).
- A doc do Gemini CLI esclarece que os planos Gemini for Workspace "do not apply to the API usage" ([`quota-and-pricing.md`](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/quota-and-pricing.md)).

**Conclusão 3:** para o Jarvis, o valor aproveitável da assinatura é o **crédito Cloud mensal** (US$ 10 no Pro), não uma cota de Live.

---

## 4. Termos de uso e risco de bloqueio

- Texto oficial do Gemini CLI: *"Directly accessing the services powering Gemini CLI (for example, the Gemini Code Assist service) using third-party software, tools, or services (for example, using OpenClaw with Gemini CLI OAuth) is a violation of applicable terms and policies. Such actions may be grounds for suspension or termination of your account."* ([`docs/resources/tos-privacy.md`](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/tos-privacy.md))
- FAQ: *"Using third-party software, tools, or services to harvest or piggyback on Gemini CLI's OAuth authentication to access our backend services is a direct violation … may be grounds for immediate suspension or termination of your account. … the supported and secure method is to use a Vertex AI or Google AI Studio API key."* ([`docs/resources/faq.md`](https://github.com/google-gemini/gemini-cli/blob/main/docs/resources/faq.md))
- Precedente real: em fev/2026 houve uma onda de 403 "ToS" para quem usou OAuth do Antigravity/Gemini CLI no OpenClaw. O bloqueio atingiu **Gemini CLI e Code Assist juntos**, inclusive de assinantes pagos. Em 27/02/2026 o Google anunciou a política: 1ª violação gera e-mail + formulário de recertificação; **2ª violação gera banimento permanente** ([discussion #20632](https://github.com/google-gemini/gemini-cli/discussions/20632), [issue openclaw #14203](https://github.com/openclaw/openclaw/issues/14203), [fórum](https://discuss.ai.google.dev/t/urgent-mass-403-tos-bans-on-gemini-api-antigravity-for-open-source-cli-users-paid-tier/124508), [PCWorld](https://www.pcworld.com/article/3068842/whats-behind-the-openclaw-ban-wave.html)).
- Termos da Gemini API ([terms](https://ai.google.dev/gemini-api/terms)): no free tier, o conteúdo pode ser usado para melhorar produtos e revisado por humanos ("do not submit sensitive, confidential, or personal information to the Unpaid Services"). A idade mínima é 18 anos. Quem oferece o app a usuários do EEE/Reino Unido/Suíça precisa usar o serviço pago. O serviço é "for developers building … for professional or business purposes".

**Conclusão 4:** pegar carona no OAuth do Code Assist é violação explícita, e o Google já aplica bloqueio automático. O risco atinge a conta Google do Laschuk, que é a mesma do AI Pro, do Gemini CLI e do Antigravity. **Não vale a pena**, ainda mais porque nem destrava a Live.

---

## 5. Alternativas

### 5a. Híbrido "OAuth grátis pra texto + chave só pro áudio"
- Com o OAuth do Code Assist: **inviável** (seção 4, é a mesma violação).
- Versão legítima: o Jarvis já usa a Live para áudio e ferramentas **na mesma sessão** (function calling dentro do BidiGenerateContent). Separar o texto num segundo modelo acrescenta latência e complexidade, e só compensa se houver chamadas de texto grandes fora da conversa. Nesse caso, o free tier da chave (Flash) resolve sem OAuth nenhum.

### 5b. Vertex AI com créditos
- Viável tecnicamente, com `gcloud auth application-default login` + Bearer + endpoint regional.
- Custos: mesmo modelo de cobrança por uso. Os créditos do AI Pro (US$ 10/mês) valem aqui. O Free Trial de US$ 300 só vale se a conta nunca pagou Cloud.
- Contras: hoje o modelo documentado é o `gemini-live-2.5-flash-native-audio`, não o `gemini-3.8-live`. O protocolo muda (URI do modelo, `v1beta1.LlmBidiService`). Cada usuário final precisaria de projeto e gcloud. Serve para o Laschuk, não para distribuir o app.

### 5c. Free tier da Gemini API (chave sem faturamento)
- `gemini-3.8-live` e `gemini-3.1-flash-live-preview` aparecem com **Free Tier: "Free of charge"** para entrada e saída ([pricing](https://ai.google.dev/gemini-api/docs/pricing)).
- Limites da sessão ([session management](https://ai.google.dev/gemini-api/docs/live-session)): conexão de ~10 min; sessão só de áudio de 15 min sem compressão (com `contextWindowCompression`, a duração fica ilimitada); token de retomada válido por 2 h.
- Os limites numéricos do free tier (RPM/TPM/RPD/sessões simultâneas) da Live **não estão publicados na doc**. Eles ficam só no painel [AI Studio → Rate limit](https://aistudio.google.com/rate-limit), por projeto. Precisa conferir logado. O Firebase AI Logic diz apenas "Limits vary based on your project's Gemini Developer API usage tier" ([Firebase Live limits](https://firebase.google.com/docs/ai-logic/live-api/limits-and-specs)).
- Contras: dados usados para treino (não mandar nada sensível, e o Jarvis ouve o ambiente e tem ferramentas de arquivos/shell). Sem SLA. Limites podem mudar.

### 5d. Estimativa de custo — 1 h/dia de conversa (pago, `gemini-3.8-live`)

Preços da Developer API: entrada de áudio **US$ 0,005/min**, saída de áudio **US$ 0,018/min**, texto US$ 0,75/US$ 4,50 por 1M ([pricing](https://ai.google.dev/gemini-api/docs/pricing)).

| Cenário (por dia) | Cálculo | Dia | Mês (30 d) |
|---|---|---|---|
| Otimista: 60 min de mic transmitindo, 20 min de fala do Jarvis | 60×0,005 + 20×0,018 | US$ 0,66 | **~US$ 20** |
| Realista: + contexto reprocessado a cada turno, ferramentas e transcrição em texto (fator ~1,5–2×, **estimativa**) | — | US$ 1,00–1,30 | **~US$ 30–40** |
| Menos US$ 10/mês de crédito do AI Pro | — | — | **~US$ 10–30 do bolso** |

*A conta por minuto é uma aproximação. A cobrança real é por tokens de áudio (25 tokens/s, conforme a nota do Live Translate na mesma página) mais os tokens de contexto acumulados na sessão. Os fatores de 1,5–2× são inferência minha, não constam em doc. Confirmar com 1 semana de uso real no painel de billing.*

---

## Tabela de opções

| # | Opção | Viável? | Custo/mês (1 h/dia) | Esforço | Risco |
|---|---|---|---|---|---|
| 1 | **Chave Gemini API paga + créditos AI Pro (US$ 10) abatendo** | Sim | ~US$ 10–30 líquido | **Nenhum** (é o que o Jarvis já faz; só ativar os créditos na conta de faturamento) | Baixo |
| 2 | Chave Gemini API **free tier** | Sim, com limites não publicados | US$ 0 | Nenhum | Médio: dados usados para treino, limites/cortes, sem SLA |
| 3 | Vertex AI com `gcloud` ADC (OAuth de usuário) + créditos | Sim, para uso pessoal | ~o mesmo do item 1 (preço Vertex não confirmado), menos créditos | Médio: novo endpoint/protocolo, modelo `gemini-live-2.5-flash-native-audio`, projeto + billing | Baixo para a conta; alto para distribuição (cada usuário precisa de GCP) |
| 4 | Híbrido: Live por chave + texto via OAuth do Code Assist | **Não** (viola ToS) | "grátis" | Médio | **Alto**: suspensão, 2ª vez = ban permanente |
| 5 | Live via OAuth do Gemini CLI/Code Assist | **Impossível**: não existe Bidi/Live no `cloudcode-pa` | — | — | Alto + não funciona |
| 6 | Usar a Live "da assinatura" (app Gemini) | **Não**: só dentro do app do Google, sem API | — | — | — |

## Recomendação

**Não dá para rodar a Live por OAuth aproveitando a cota da assinatura ou do Code Assist.** Esse caminho não tem endpoint Live (`cloudcode-pa` só tem generate/stream de texto; o modo de voz do próprio Gemini CLI exige `GEMINI_API_KEY`) e é violação explícita de termos, já punida com bloqueio.

Caminho concreto:
1. **Agora:** continuar com a chave da Gemini API. Para testes, usar o free tier e conferir os limites reais da Live no painel [aistudio.google.com/rate-limit](https://aistudio.google.com/rate-limit).
2. **Uso diário:** ativar o benefício Developer Program do AI Pro ([developers.google.com/profile](https://developers.google.com/profile/help/benefits)), vincular os US$ 10/mês à mesma conta de faturamento do projeto da chave (se for Prepay, pôr saldo antes) e ligar o faturamento. Custo estimado: ~US$ 10–30/mês líquido para 1 h/dia. Os dados deixam de ir para treino.
3. **Opcional, só se quiser OAuth por preferência de segurança:** um card separado para suportar o Vertex (`LlmBidiService` + Bearer ADC), aceitando trocar para `gemini-live-2.5-flash-native-audio`. Não economiza nada em relação ao item 2.

## Não coberto / incertezas

- Não testei conexões reais (sem código de produção, por escopo). Em especial, não testei se o WebSocket `generativelanguage` aceita Bearer OAuth. A doc e o SDK dizem que não.
- Não consegui extrair a linha de preço da Live na [página de preços do Vertex](https://cloud.google.com/vertex-ai/generative-ai/pricing) (página truncada no fetch).
- Os números de RPM/RPD/sessões simultâneas do free tier da Live não estão publicados. Estão só no painel do AI Studio.
- A doc do Vertex pode estar desatualizada quanto a `gemini-3.8-live`. A navegação cita Live em modelos 3.x Flash, mas a tabela de modelos suportados não.
