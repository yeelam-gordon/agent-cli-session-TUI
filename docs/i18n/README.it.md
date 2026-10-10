# Agent CLI Session TUI — Cercare nella cronologia e riprendere le conversazioni

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

Un gestore di sessioni di terminale per le conversazioni conservate di Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI e Kimi, anche avviate separatamente. Sono sei fornitori integrati: la copertura dipende dai formati e dai filtri di rilevamento, non include automaticamente ogni CLI.

## Prima dell’avvio: installazione e privacy

Serve una conversazione salvata da almeno un fornitore. Per riprenderla, il suo CLI deve essere installato, autenticato e raggiungibile tramite il `command` configurato; la directory di lavoro originale deve essere accessibile. Usa un terminale interattivo e una versione adatta al sistema e alla CPU.

Da [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) scarica l’**archivio Core completo**, non quello del codice sorgente. Estrai tutto e mantieni insieme eseguibile, `providers/`, `prompts/` e `config.toml.example` generato. Copia l’esempio come `config.toml` accanto all’eseguibile senza sovrascrivere impostazioni esistenti. Controlla `state_dir`, `command` e il lanciatore. Il sorgente `config.toml.template` contiene segnaposto: non basta rinominarlo.

**Avviso prima del primo avvio:** il raggruppamento predefinito `remote` invia titoli e directory di lavoro a un servizio ospitato e può partire automaticamente dopo il rilevamento iniziale. Non esiste un contratto API pubblico per terze parti. Per raggruppare senza rete, nella copia trova `# [grouping]` e `# engine = "remote"`: togli il `#` iniziale da entrambe le righe e cambia il valore in `"wordmatch"`.

```toml
[grouping]
engine = "wordmatch"
```

Lascia commentate le altre opzioni. Se esiste già una tabella `[grouping]` attiva, modifica il suo `engine` senza aggiungerne un’altra. Salva prima di avviare o leggere la cronologia. `wordmatch` confronta parole, non è IA locale e non controlla le connessioni dei CLI. Il plugin semantico facoltativo può scaricare modelli all’inizializzazione; parti con Core per non caricarlo.

## Prima ricerca e risultato atteso

Dal terminale nella cartella estratta, su Windows PowerShell:

```powershell
.\agent-session-tui.exe
```

Su Linux/macOS:

```sh
./agent-session-tui
```

Attendi l’elenco, premi `/` e inserisci un progetto o una frase della tua cronologia. Con `↑`/`↓` controlla i dettagli: fornitore, titolo e directory, poi `Enter`. Con **Resumable**, ci si aspetta un terminale in cui il CLI carica la conversazione scelta per continuarla. Con **Running/Waiting**, solo su Windows si tenta di selezionare una scheda corrispondente di Windows Terminal: se riesce, la scheda è selezionata e la finestra va in primo piano. Su Linux/macOS passa manualmente al terminale attivo. Risultati e corrispondenze approssimative non sono garantiti.

Lanciatori predefiniti: Windows `wt` (alternativa `cmd`), Linux `gnome-terminal`, macOS `osascript` con Terminal. Adatta la configurazione se mancano. Core ha destinazioni x64/arm64 per i tre sistemi; i pacchetti semantici non coprono tutte le destinazioni, verifica gli allegati della release. `Esc` annulla la ricerca; `q` / `Ctrl+C` esce senza cancellare la cronologia né fermare altri CLI. La terminazione delle sessioni non è implementata.

Elenco vuoto, configurazione errata, problemi di avvio o selezione, aggiornamento, rimozione e ripristino: [risoluzione dei problemi in inglese](../../README.md#troubleshooting-update-and-removal). Il [manuale inglese](../../README.md) è il riferimento tecnico; vedi [configurazione](../../README.md#configuration) e [dati inviati](../../README.md#what-remote-sends).
