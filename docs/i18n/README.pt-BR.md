# Agent CLI Session TUI — Pesquisar o histórico e retomar sessões

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Lembra do trabalho, mas não de qual CLI guardou a conversa? Pesquise históricos do Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI e Kimi. Se já sabe o provedor e a sessão, o histórico do próprio CLI pode bastar.

<img src="../agent-session-benefits.png" alt="Conceito: a busca por palavras encontra token expires no contexto preservado, não apenas no título, e retoma a conversa salva no CLI original. Suporta Copilot CLI, Claude Code, Codex e outros." width="700">

Ilustração conceitual, não uma captura real do aplicativo. Rótulos em inglês e linhas fictícias.

- **Encontre o trabalho anterior:** uma busca em vários históricos preservados, inclusive de conversas iniciadas fora deste TUI.
- **Continue a conversa original:** reabra o contexto salvo pelo CLI original instalado.
- **Encontre sua aba ativa no Windows:** tente selecionar uma aba correspondente do Windows Terminal; no Linux/macOS, alterne manualmente.

<a id="install-core"></a>

## Primeira pesquisa privada: Core + wordmatch

É preciso ter histórico salvo, CLI instalado e autenticado, acessível pelo `command` configurado, e diretório original disponível. Use um terminal interativo e uma versão para seu SO/CPU. Lançadores padrão: Windows `wt` (alternativa `cmd`), Linux `gnome-terminal`, macOS `osascript` com Terminal. Se faltarem, ajuste a [configuração](../usage.md#configuration).

**Antes de iniciar:** builds com os novos padrões de privacidade desativam o agrupamento automático por padrão e usam `wordmatch` manualmente. Versões antigas ainda podem executar agrupamento remoto automático. A escolha explícita de `remote` envia títulos e diretórios; ACP pode enviar resumos pelo CLI. [PRIVACY.md](../../PRIVACY.md) explica os caminhos e índices de conteúdo das conversas. O plugin semântico pode baixar modelos; Core não controla as conexões dos CLIs.

1. Baixe em [Arquivos Core por sistema e CPU](../usage.md#core-archive-filenames) o **arquivo Core completo** para seu SO/CPU, não o código-fonte. Extraia tudo e mantenha juntos executável, `providers/`, `prompts/` e o `config.toml.example` gerado. [Confira o digest do arquivo exato](../usage.md#verify-a-release-download); a correspondência não é assinatura nem garantia de segurança. Respeite avisos do SO e políticas da organização.
2. Na instalação nova, copie o exemplo gerado como `config.toml` ao lado do executável. **Não sobrescreva ajustes existentes nem renomeie o `config.toml.template` do código-fonte.** Na cópia, descomente `# [grouping]` e `# engine = "wordmatch"` (exemplos antigos: `# engine = "remote"`) e configure a seção como abaixo, incluindo desativar o agrupamento automático:

```toml
[grouping]
engine = "wordmatch"
auto_suggest = false
```

Deixe comentadas as demais opções. Se já houver uma tabela `[grouping]` ativa, altere seu `engine` sem duplicá-la. Confira `state_dir` e `command`; **salve antes de iniciar**. `wordmatch` compara palavras compartilhadas; não é IA local.

3. Execute em um terminal na pasta extraída:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Primeira pesquisa e resultado esperado

4. Aguarde a lista, pressione `/` e digite um trecho literal do título ou projeto do seu histórico. Com `↑`/`↓`, confira provedor, título e diretório; escolha **Resumable** e pressione `Enter`. **Resultado esperado:** o CLI original abre a conversa escolhida para continuar. A busca pode deixar resultados de fora; as versões nativas de CLI testadas são desconhecidas. [Cobertura e evidências](../usage.md#supported-providers).

<a id="windows-terminal"></a>

Em **Running/Waiting/Idle**, só o Windows tenta focar uma aba correspondente; não se conecta a qualquer processo. Repetir logo `Enter` na mesma sessão ativa pode **forçar a retomada** em vez de trocar de aba. Confira primeiro. [Fluxo no Windows](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Nenhum resultado?** Confira histórico preservado, provedor habilitado e `state_dir`; se não iniciar, verifique autenticação, diretório original e lançador. [Solução de problemas](../usage.md#troubleshooting-update-and-removal).

**Sair:** `Esc` cancela a busca; depois, `q` fecha o TUI, ou use `Ctrl+C` na lista ou na busca sem apagar históricos nem parar outros CLIs. Encerrar sessões não foi implementado. [Atualizar, restaurar e remover](../usage.md#troubleshooting-update-and-removal).

O [guia de uso e evidências em inglês](../usage.md) conserva números completos, resultados adversos e limites. A versão e a data da [gravação](../usage.md#recorded-demo) são desconhecidas; ela não comprova compatibilidade atual.
