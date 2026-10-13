# Agent CLI Session TUI — Cercare nella cronologia e riprendere le conversazioni

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Ricordi il lavoro, ma non il CLI che contiene la conversazione? Cerca nelle cronologie di Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI e Kimi. Se conosci già fornitore e sessione, può bastare la cronologia del CLI stesso.

<img src="../agent-session-benefits.png" alt="Concetto: la ricerca per parole trova token expires nel contesto conservato, non solo nel titolo, e riprende la conversazione salvata nella CLI originale. Supporta Copilot CLI, Claude Code, Codex e altri." width="700">

Illustrazione concettuale, non una schermata reale dell’app. Etichette inglesi e righe fittizie.

- **Ritrova il lavoro precedente:** una ricerca in più cronologie conservate, anche per conversazioni avviate fuori da questo TUI.
- **Riprendi la conversazione originale:** riapri il contesto salvato tramite il CLI originale installato.
- **Ritrova la sua scheda attiva su Windows:** tenta di selezionare la scheda corrispondente di Windows Terminal; su Linux/macOS passa manualmente al terminale.

<a id="install-core"></a>

## Prima ricerca privata: Core + wordmatch

Servono cronologia salvata, CLI installato e autenticato, accessibile tramite il `command` configurato, e directory originale disponibile. Usa un terminale interattivo e una versione per il tuo sistema/CPU. Lanciatori predefiniti: Windows `wt` (alternativa `cmd`), Linux `gnome-terminal`, macOS `osascript` con Terminal. Se mancano, modifica la [configurazione](../usage.md#configuration).

**Prima dell’avvio:** le build con i nuovi valori di privacy disattivano il raggruppamento automatico per impostazione predefinita e usano `wordmatch` manualmente. Le vecchie versioni possono ancora raggruppare automaticamente da remoto. Scegliere `remote` invia titoli e directory; ACP può inviare riassunti tramite il CLI. [PRIVACY.md](../../PRIVACY.md) descrive percorsi e indici del contenuto delle conversazioni. Il plugin semantico può scaricare modelli; Core non controlla le connessioni dei CLI.

1. Da [Archivi Core per sistema e CPU](../usage.md#core-archive-filenames) scarica l’**archivio Core completo** per il tuo sistema/CPU, non quello del codice sorgente. Estrai tutto e mantieni insieme eseguibile, `providers/`, `prompts/` e `config.toml.example` generato. [Verifica il digest del file esatto](../usage.md#verify-a-release-download); la corrispondenza non è firma né garanzia di sicurezza. Rispetta avvisi del sistema e regole dell’organizzazione.
2. Per una nuova installazione, copia l’esempio generato come `config.toml` accanto all’eseguibile. **Non sovrascrivere impostazioni esistenti e non rinominare il `config.toml.template` sorgente.** Nella copia, decommenta `# [grouping]` e `# engine = "wordmatch"` (vecchi esempi: `# engine = "remote"`) e configura la sezione come sotto, disattivando anche il raggruppamento automatico:

```toml
[grouping]
engine = "wordmatch"
auto_suggest = false
```

Lascia commentate le altre opzioni. Se esiste già una tabella `[grouping]` attiva, modifica il suo `engine` senza duplicarla. Controlla `state_dir` e `command`; **salva prima dell’avvio**. `wordmatch` confronta parole condivise; non è IA locale.

3. Avvia da un terminale nella cartella estratta:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Prima ricerca e risultato atteso

4. Attendi l’elenco, premi `/` e digita un frammento letterale del titolo o progetto della tua cronologia. Con `↑`/`↓` controlla fornitore, titolo e directory; scegli **Resumable**, poi `Enter`. **Risultato atteso:** il CLI originale apre la conversazione scelta per continuarla. La ricerca può tralasciare risultati; le versioni native dei CLI testate sono sconosciute. [Copertura e prove](../usage.md#supported-providers).

<a id="windows-terminal"></a>

Per **Running/Waiting/Idle**, solo Windows tenta di selezionare una scheda corrispondente; non è un collegamento universale ai processi. Ripetere subito `Enter` sulla stessa sessione attiva può **forzare la ripresa** anziché cambiare scheda. Verifica prima. [Percorso Windows](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Nessun risultato?** Verifica cronologia conservata, fornitore abilitato e `state_dir`; se non parte, controlla autenticazione, directory originale e lanciatore. [Risoluzione dei problemi](../usage.md#troubleshooting-update-and-removal).

**Uscire:** `Esc` annulla la ricerca; poi `q` chiude il TUI, oppure usa `Ctrl+C` nell’elenco o nella ricerca senza cancellare cronologie né fermare altri CLI. La terminazione delle sessioni non è implementata. [Aggiornamento, ripristino e rimozione](../usage.md#troubleshooting-update-and-removal).

La [guida d’uso e delle prove in inglese](../usage.md) conserva cifre complete, risultati negativi e limiti. Versione e data della [registrazione](../usage.md#recorded-demo) sono sconosciute; non dimostra compatibilità attuale.
