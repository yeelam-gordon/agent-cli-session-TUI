# Agent CLI Session TUI — Pesquisar o histórico e retomar sessões

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Busque conversas nos históricos de vários CLIs de uma vez. Retome a conversa e seu contexto pelo CLI original instalado. Só no Windows, tente focar sua aba correspondente já em execução no Windows Terminal em vez de abrir outra janela.

[Ver o fluxo](#first-workflow) · [Instalar Core](#install-core) · [Provedores e limites](../../README.md#supported-providers)

O agrupamento remoto padrão pode enviar títulos e caminhos automaticamente; escolha Core + wordmatch antes de iniciar.

<img src="../agent-session-benefits.png" alt="Conceito: encontrar e continuar uma conversa nos históricos dos CLI" width="780">

Ilustração conceitual, não uma captura real do aplicativo. Os rótulos em inglês e as linhas fictícias são exemplos, não históricos privados reais.

- Encontre um assunto ou projeto antigo: pesquise uma expressão de que se lembra e confira provedor, título e diretório.
- Continue a conversa certa: `Enter` em **Resumable** exige histórico preservado, CLI instalado e autenticado e diretório original acessível.
- Encontre a aba em execução no Windows: tenta focar uma aba correspondente do Windows Terminal, apenas no Windows; não é conexão universal a processos. No Linux/macOS, alterne manualmente.

Ela não precisa ter sido iniciada neste TUI.


<a id="install-core"></a>

## Primeira pesquisa privada: Core + wordmatch

Tenha conversas salvas de pelo menos um provedor. Para retomá-las, o CLI correspondente precisa estar instalado, autenticado e acessível pelo comando configurado; o diretório de trabalho original deve continuar acessível. Use um terminal interativo e a versão adequada ao sistema e à CPU.

O lançador padrão é `wt` no Windows (alternativa: `cmd`), `gnome-terminal` no Linux e `osascript` com Terminal no macOS. Ajuste a configuração se estiverem ausentes. Core contempla x64/arm64 nos três sistemas; confira os arquivos publicados para a disponibilidade semântica. O foco nativo de abas exige Windows Terminal e só está disponível no Windows.

Em [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), baixe o **arquivo Core completo** para seu sistema e CPU, não o pacote de código-fonte. Extraia tudo e mantenha juntos o executável, `providers/`, `prompts/` e o `config.toml.example` gerado. Confira `state_dir`, `command`, o lançador e a opção de privacidade abaixo. O `config.toml.template` do código-fonte contém marcadores e não pode simplesmente ser renomeado.

Confira o digest do arquivo exato antes de iniciar; a correspondência não é assinatura nem garantia de segurança. Respeite avisos do sistema e políticas da organização. [SHA-256](../../README.md#verify-a-release-download).

**Aviso antes da primeira execução:** o agrupamento padrão `remote` envia títulos e diretórios de trabalho a um serviço hospedado e pode ser acionado automaticamente após a descoberta inicial. Não há contrato público de API para terceiros. Para agrupar sem rede, siga a configuração inicial abaixo:

Na primeira instalação do Core, copie o `config.toml.example` gerado para `config.toml` ao lado do executável, sem sobrescrever configurações existentes. Nessa cópia, encontre `# [grouping]` e `# engine = "remote"`: remova o `#` inicial das duas linhas e altere o valor para `"wordmatch"`, como abaixo. Mantenha as outras opções comentadas. Se já houver uma tabela `[grouping]` ativa, altere o `engine` nela, sem criar outra tabela. Salve antes de iniciar ou ler o histórico.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compara palavras; não é IA local. O plugin semântico opcional pode baixar modelos na inicialização; comece com Core para não carregá-lo. Essa opção não controla a rede dos próprios CLIs.

<a id="first-workflow"></a>

## Primeira pesquisa e resultado esperado

Comece com um trecho literal do título ou projeto. Core oferece pesquisa por palavras e aproximada; paráfrases semânticas exigem o plugin opcional.

No terminal da pasta extraída, execute `.\agent-session-tui.exe` no PowerShell do Windows ou `./agent-session-tui` no Linux/macOS. Aguarde as sessões, pressione `/` e digite um projeto ou trecho de que se lembre do seu próprio histórico. Use as setas para conferir os detalhes e confirme provedor, título e diretório antes de `Enter`.

Há seis definições de histórico e rotas de retomada configuradas; as versões dos CLI nativos testadas aqui são **desconhecidas**. Não é certificação de compatibilidade em uso real. Veja [cobertura e evidências](../../README.md#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable (conversa que pode ser retomada) | Retomar com CLI instalado e autenticado e diretório original acessível |
| Running / Waiting / Idle | Só no Windows, tentar focar a aba em execução correspondente do Windows Terminal; nos demais sistemas, alternar manualmente |
| Archived / Stopped | Retomar com CLI instalado e autenticado e diretório original acessível; [limites](../../README.md#supported-providers) |



Em **Resumable**, espera-se que um terminal abra o CLI com a conversa escolhida, pronta para continuar.

`Esc` cancela a pesquisa; `q` / `Ctrl+C` sai sem apagar o histórico nem encerrar outros CLIs. Encerrar sessões pelo TUI ainda não foi implementado.

- Lista vazia: confira histórico salvo, provedor habilitado e `state_dir`.
- Falha ao retomar: confira instalação do CLI, autenticação e diretório original.
- Não inicia: extraia o pacote completo com `providers/`, `prompts/` e exemplo gerado; confira o lançador.

Lista vazia, configuração inválida, falha ao abrir ou focar, atualização, remoção e restauração: [solução de problemas em inglês](../../README.md#troubleshooting-update-and-removal). A [documentação em inglês](../../README.md) é a referência técnica; veja [configuração](../../README.md#configuration) e [privacidade](../../README.md#what-remote-sends).




Se você já sabe o provedor e a sessão, o histórico do próprio CLI basta. Este catálogo ajuda com vários CLI ou projetos, quando você lembra o assunto, mas não o ID.

Um gerenciador de sessões de terminal para o histórico existente do Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI e Kimi, inclusive conversas iniciadas separadamente. São seis provedores integrados; outros exigem novas definições, não há suporte automático a qualquer CLI.

Siga os passos de privacidade antes de iniciar; o agrupamento remoto padrão pode executar automaticamente.





### Windows Terminal

Em **Running/Waiting/Idle**, apenas no Windows, o programa tenta focar uma aba correspondente do Windows Terminal: se houver correspondência, a aba é selecionada e a janela vem para a frente. No Linux/macOS, mude manualmente para o terminal em execução. Consultas vagas ou com erros de digitação não garantem encontrar a conversa.

<details>
<summary>↳ Primeira pesquisa privada: Core + wordmatch</summary>

**Exemplo fictício, não observado em execução nem incluído como sessão:** pressione `/`, busque `build failure` e imagine uma linha Claude Code · título `Build failure` · projeto `demo-project`. Com `↑`/`↓`, confira provedor, título e diretório original. Se for a conversa **Resumable** correta, `Enter` a reabre pelo CLI do Claude instalado e autenticado, com o diretório acessível. Não garante resultados nem compatibilidade.

</details>



Veja as conversas salvas na lista, pesquise no seu histórico e escolha uma para continuar. O GIF mostra a interface em inglês da versão gravada; não verifica a versão atual nem a compatibilidade de cada CLI. A tradução da documentação não traduz a interface ou as conversas salvas. Faça a configuração de privacidade na instalação antes de iniciar. A versão e a data da gravação são desconhecidas. [Instalar Core](#install-core).

<img src="../agentTUI.gif" alt="Demonstração do TUI: lista do histórico, pesquisa e retomada" width="700">
