# Agent CLI Session TUI — Pesquisar o histórico e retomar sessões

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

Um gerenciador de sessões de terminal para o histórico existente do Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI e Kimi, inclusive conversas iniciadas separadamente. São seis provedores integrados; outros exigem novas definições, não há suporte automático a qualquer CLI.

## Antes de iniciar: requisitos, privacidade e instalação

Tenha conversas salvas de pelo menos um provedor. Para retomá-las, o CLI correspondente precisa estar instalado, autenticado e acessível pelo comando configurado; o diretório de trabalho original deve continuar acessível. Use um terminal interativo e a versão adequada ao sistema e à CPU.

**Aviso antes da primeira execução:** o agrupamento padrão `remote` envia títulos e diretórios de trabalho a um serviço hospedado e pode ser acionado automaticamente após a descoberta inicial. Não há contrato público de API para terceiros. Para agrupar sem rede, edite a tabela existente em `config.toml` antes de iniciar, sem duplicá-la:

Na primeira instalação do Core, copie o `config.toml.example` gerado para `config.toml` ao lado do executável. Nessa cópia, encontre `# [grouping]` e `# engine = "remote"`: remova o `#` inicial das duas linhas e altere o valor para `"wordmatch"`, como abaixo. Mantenha as outras opções comentadas. Se já houver uma tabela `[grouping]` ativa, altere o `engine` nela, sem criar outra tabela. Salve antes de iniciar ou ler o histórico.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compara palavras; não é IA local. O plugin semântico opcional pode baixar modelos na inicialização; comece com Core para não carregá-lo. Essa opção não controla a rede dos próprios CLIs.

Em [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), baixe o **arquivo Core completo** para seu sistema e CPU, não o pacote de código-fonte. Extraia tudo e mantenha juntos o executável, `providers/`, `prompts/` e o `config.toml.example` gerado. Copie o exemplo para `config.toml` ao lado do executável sem sobrescrever uma configuração existente. Confira `state_dir`, `command`, o lançador e a opção de privacidade acima. O `config.toml.template` do código-fonte contém marcadores e não pode simplesmente ser renomeado.

## Primeira pesquisa e resultado esperado

No terminal da pasta extraída, execute `.\agent-session-tui.exe` no PowerShell do Windows ou `./agent-session-tui` no Linux/macOS. Aguarde as sessões, pressione `/` e digite um projeto ou trecho de que se lembre do seu próprio histórico. Use as setas para conferir os detalhes e confirme provedor, título e diretório antes de `Enter`.

Em **Resumable**, espera-se que um terminal abra o CLI com a conversa escolhida, pronta para continuar. Em **Running/Waiting**, apenas no Windows, o programa tenta focar uma aba correspondente do Windows Terminal: se houver correspondência, a aba é selecionada e a janela vem para a frente. No Linux/macOS, mude manualmente para o terminal em execução. Consultas vagas ou com erros de digitação não garantem encontrar a conversa.

O lançador padrão é `wt` no Windows (alternativa: `cmd`), `gnome-terminal` no Linux e `osascript` com Terminal no macOS. Ajuste a configuração se estiverem ausentes. Core contempla x64/arm64 nos três sistemas; confira os arquivos publicados para a disponibilidade semântica. `Esc` cancela a pesquisa; `q` / `Ctrl+C` sai sem apagar o histórico nem encerrar outros CLIs. Encerrar sessões pelo TUI ainda não foi implementado.

Lista vazia, configuração inválida, falha ao abrir ou focar, atualização, remoção e restauração: [solução de problemas em inglês](../../README.md#troubleshooting-update-and-removal). A [documentação em inglês](../../README.md) é a referência técnica; veja [configuração](../../README.md#configuration) e [privacidade](../../README.md#what-remote-sends).
