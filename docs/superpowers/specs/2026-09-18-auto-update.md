# Auto-update do OpenJarvisBR com Tauri v2

Data: 2026-09-18 · Card: JRV-90

## Conclusão curta

O caminho certo é o plugin oficial `tauri-plugin-updater`. Ele consulta um
manifesto HTTPS, compara a versão publicada com a instalada, baixa um artefato
assinado, verifica a assinatura antes de instalar e substitui o bundle do app.
No macOS, a versão nova começa a rodar na próxima abertura; não é preciso abrir
o DMG nem arrastar o app novamente.

Isso **não é OTA de JavaScript como o Expo**. O EAS Update troca a camada de
JavaScript e assets compatível com um runtime nativo que já está instalado. O
updater do Tauri entrega o aplicativo nativo inteiro: frontend empacotado,
binário Rust, recursos e metadados. A atualização é maior, precisa de um novo
build por versão e só entra em execução depois de reiniciar o aplicativo.

Fontes oficiais:

- [Updater do Tauri v2](https://v2.tauri.app/plugin/updater/)
- [API Rust do updater](https://docs.rs/tauri-plugin-updater/latest/tauri_plugin_updater/)
- [Publicação com GitHub Actions](https://v2.tauri.app/distribute/pipelines/github/)
- [Assinatura de apps macOS](https://v2.tauri.app/distribute/sign/macos/)
- [Como o EAS Update funciona](https://docs.expo.dev/eas-update/how-it-works/)

## O que o plugin faz

1. Lê os `endpoints` e a `pubkey` de `plugins.updater`.
2. Busca o manifesto `latest.json` e compara seu SemVer com a versão instalada.
3. Seleciona a entrada da plataforma e arquitetura atuais.
4. Baixa o pacote de atualização e valida sua assinatura. Essa validação não
   pode ser desativada.
5. Instala o pacote. No macOS e Linux, o processo atual continua rodando e a
   versão nova é usada no próximo lançamento; no Windows, o instalador encerra
   o processo como parte da instalação.

Com `bundle.createUpdaterArtifacts: true`, o build do macOS passa a produzir,
além do `.app` e do `.dmg`, um `.app.tar.gz` e seu `.sig`. O arquivo `.tar.gz` é
o artefato consumido pelo updater; o `.dmg` continua sendo a opção de primeira
instalação.

## O que ele não faz

- Não envia somente `dist/`, JavaScript ou CSS.
- Não aplica hot swap no processo em execução.
- Não elimina o build nativo nem o incremento de versão.
- Não publica a release nem cria o `latest.json` sozinho em um build manual.
- Não substitui assinatura Apple, notarização ou o processo de release.
- Não permite atualização sem assinatura do artefato do updater.

## Duas assinaturas, dois objetivos

O bundle já usa uma identidade Apple estável. Ela deve continuar assinando o
`.app` em todas as versões; depois da atualização, `codesign --verify --strict`
precisa continuar válido.

O updater acrescenta outra assinatura, baseada no par de chaves do signer do
Tauri:

- a **chave pública** fica embutida em `tauri.conf.json` e pode ser publicada;
- a **chave privada** assina o artefato durante o build e nunca entra no repo;
- a senha da chave privada também fica somente com o dono ou no cofre do CI;
- perder a chave privada impede publicar atualizações aceitas pelas instalações
  existentes. Trocar apenas a identidade Apple não resolve isso.

Nenhum agente deve gerar, ler, digitar ou armazenar a chave privada. O passo de
criação pertence ao dono e deve ser executado uma única vez, em terminal
privado:

```sh
cd /Users/laschuk/Developer/bside/jarvis/apps/desktop
npm run tauri signer generate -- -w "$HOME/.tauri/openjarvisbr-updater.key"
```

O comando pergunta a senha diretamente ao dono e cria também o arquivo público
correspondente. Só o conteúdo da chave **pública** entra em
`plugins.updater.pubkey`.

Até esse passo acontecer, `tauri.conf.json` mantém o marcador explícito
`__OWNER_MUST_REPLACE_WITH_OPENJARVISBR_UPDATER_PUBLIC_KEY__`. Ele não é uma
chave e deixa a ativação final bloqueada de propósito; não publicar um build
com esse marcador.

## Comportamento no aplicativo

O fluxo está isolado em `src-tauri/src/updater.rs`, é iniciado pelo builder em
`main.rs` e mantém no próprio módulo os testes do agendamento.

- Registrar `tauri-plugin-updater` no builder do Tauri.
- Disparar uma checagem assíncrona ao abrir.
- Repetir a checagem a cada seis horas enquanto o app estiver vivo.
- Se houver versão nova, baixar e instalar silenciosamente o pacote verificado.
- Não interromper uma conversa reiniciando o app à força; registrar que a nova
  versão está pronta e deixá-la entrar em execução na próxima abertura.
- Falha de rede, manifesto inválido ou assinatura inválida não derruba o Jarvis:
  fica registrada sem dados sensíveis e é tentada novamente no próximo ciclo.

O endpoint estático será o asset `latest.json` da release pública mais recente.
Produção usa somente HTTPS. Um teste local por HTTP é aceitável apenas em build
de desenvolvimento, comportamento que o próprio plugin bloqueia em release.

## Publicar uma versão

1. Incrementar o mesmo SemVer em `tauri.conf.json`, `package.json` e
   `src-tauri/Cargo.toml`.
2. Construir o app com a identidade Apple estável e com as variáveis privadas
   do signer do Tauri disponíveis somente no terminal privado ou no CI.
3. Confirmar que o build gerou o `.app.tar.gz` e o `.app.tar.gz.sig`, além do
   `.dmg`, e validar o `.app` com `codesign --verify --strict`.
4. Criar a GitHub Release pública da tag e anexar o pacote de atualização, o
   `.sig` e o instalador inicial.
5. Criar `latest.json` com:
   - `version` em SemVer;
   - `notes` e `pub_date` opcionais;
   - uma entrada em `platforms` para cada alvo publicado, como
     `darwin-aarch64`;
   - `url` apontando para o `.app.tar.gz` da mesma release;
   - `signature` contendo o **texto do arquivo `.sig`**, não o caminho nem uma
     URL.
6. Anexar `latest.json` à mesma release. O endpoint configurado usa o asset da
   release pública mais recente.
7. Abrir uma versão anterior, observar a checagem/instalação e reiniciar o app.
   Confirmar a nova versão e repetir `codesign --verify --strict` no bundle
   instalado.

A configuração específica do Windows mantém `createUpdaterArtifacts: false`
para não quebrar a Action atual, que ainda não recebe o signer. Em trabalho
próprio, a Action deve receber os segredos, remover esse override e publicar as
entradas Windows no mesmo `latest.json`; até lá, a ativação fica limitada ao
macOS.

## Teste de aceitação

Para provar o fluxo antes da primeira release pública, use um par de chaves
criado pelo dono, construa uma versão assinada e sirva em desenvolvimento um
`latest.json` com versão maior, URL do `.app.tar.gz` e o conteúdo do `.sig`.
Abra a versão anterior: ela deve detectar, baixar e instalar sem DMG. Feche e
abra o app; a versão deve ser a nova e a assinatura Apple deve continuar
válida. Nunca habilite transporte inseguro no build de produção.
