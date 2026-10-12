# Agent CLI Session TUI — Frühere CLI-Sitzungen durchsuchen und fortsetzen

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Sie erinnern sich an die Arbeit, aber nicht an die zugehörige CLI? Durchsuchen Sie Verläufe von Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI und Kimi. Sind Anbieter und Sitzung bekannt, genügt möglicherweise der Verlauf der jeweiligen CLI.

<img src="../agent-session-benefits.png" alt="Konzept: Die Stichwortsuche findet token expires im erhaltenen Gesprächskontext, nicht nur im Titel, und setzt das gespeicherte Gespräch in seiner ursprünglichen CLI fort. Copilot CLI, Claude Code, Codex und weitere werden unterstützt." width="700">

Konzeptillustration, kein tatsächlicher App-Screenshot. Englische Beschriftungen und fiktive Zeilen.

- **Frühere Arbeit wiederfinden:** eine Suche in mehreren erhaltenen CLI-Verläufen, auch für außerhalb dieses TUIs gestartete Gespräche.
- **Das ursprüngliche Gespräch fortsetzen:** gespeicherten Kontext über die installierte ursprüngliche CLI wieder öffnen.
- **Den laufenden Windows-Tab finden:** versuchen, den passenden Windows-Terminal-Tab auszuwählen; unter Linux/macOS manuell wechseln.

<a id="install-core"></a>

## Private erste Suche: Core + wordmatch

Erforderlich sind gespeicherter Verlauf, eine installierte und authentifizierte CLI, erreichbar über den konfigurierten `command`, und das zugängliche ursprüngliche Arbeitsverzeichnis. Nutzen Sie ein interaktives Terminal und eine passende OS/CPU-Version. Standardstarter: Windows `wt` (Ausweichlösung `cmd`), Linux `gnome-terminal`, macOS `osascript` mit Terminal. Fehlen sie, passen Sie die [Konfiguration](../usage.md#configuration) an.

**Vor dem Start:** Die voreingestellte Remote-Gruppierung kann nach der Erkennung automatisch **Titel und Arbeitsverzeichnisse** an einen gehosteten Dienst senden. Ein öffentlicher API-Vertrag für Drittanbieter fehlt. Core + `wordmatch` vermeidet diese Remote-Gruppierung, nicht das Netzwerkverhalten der CLIs selbst. Das optionale semantische Plugin kann bei der Initialisierung Modelle herunterladen; beginnen Sie mit Core ohne dieses Plugin.

1. Laden Sie unter [Core-Dateien für Betriebssystem und CPU](../usage.md#core-archive-filenames) das **vollständige Core-Archiv** für Ihr OS/Ihre CPU herunter, nicht das Quellcode-Archiv. Entpacken Sie alles und behalten Sie Programm, `providers/`, `prompts/` und generierte `config.toml.example` zusammen. [Prüfen Sie den Digest der genauen Datei](../usage.md#verify-a-release-download); Übereinstimmung ist weder Signatur noch Sicherheitsgarantie. Beachten Sie OS-Warnungen und Organisationsregeln.
2. Kopieren Sie bei einer Neuinstallation das generierte Beispiel als `config.toml` neben das Programm. **Überschreiben Sie keine Einstellungen und benennen Sie die Quellvorlage `config.toml.template` nicht einfach um.** Entfernen Sie in der Kopie das führende `#` aus **beiden Zeilen**, `# [grouping]` und `# engine = "remote"`, und ändern Sie den Wert:

```toml
[grouping]
engine = "wordmatch"
```

Lassen Sie andere Optionen auskommentiert. Existiert schon eine aktive `[grouping]`-Tabelle, ändern Sie dort `engine`, ohne sie zu duplizieren. Prüfen Sie `state_dir` und `command`; **speichern Sie vor dem Start**. `wordmatch` vergleicht gemeinsame Wörter; es ist keine lokale KI.

3. Starten Sie aus einem Terminal im entpackten Ordner:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Erste Suche und erwartetes Ergebnis

4. Warten Sie auf die Liste, drücken Sie `/` und geben Sie einen wörtlichen Titel- oder Projektausschnitt aus Ihrem Verlauf ein. Prüfen Sie mit `↑`/`↓` Anbieter, Titel und Verzeichnis; wählen Sie **Resumable**, dann `Enter`. **Erwartetes Ergebnis:** Die ursprüngliche CLI öffnet das gewählte Gespräch zum Fortsetzen. Die Suche kann Treffer übersehen; getestete native CLI-Versionen sind unbekannt. [Abdeckung und Belege](../usage.md#supported-providers).

<a id="windows-terminal"></a>

Unter Windows versucht das TUI bei **Running/Waiting/Idle**, den passenden Tab auszuwählen; kein universelles Anhängen an Prozesse. Ein rasches zweites `Enter` auf derselben laufenden Sitzung kann stattdessen **Fortsetzen erzwingen**. Prüfen Sie vorher die Auswahl. [Windows-Ablauf](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Keine Ergebnisse?** Prüfen Sie erhaltenen Verlauf, aktivierten Anbieter und `state_dir`; bei Startfehlern Authentifizierung, ursprüngliches Verzeichnis und Starter. [Fehlerhilfe](../usage.md#troubleshooting-update-and-removal).

**Beenden:** `Esc` bricht die Suche ab; danach beendet `q` das TUI, oder nutzen Sie `Ctrl+C` in der Liste oder Suche ohne Verlaufslöschung oder Stoppen anderer CLIs. Sitzungen zu beenden ist nicht implementiert. [Aktualisieren, Wiederherstellen und Entfernen](../usage.md#troubleshooting-update-and-removal).

Die [englische Betriebs- und Belegreferenz](../usage.md) bewahrt alle Zahlen, nachteiligen Ergebnisse und Grenzen. Version und Datum der [Aufnahme](../usage.md#recorded-demo) sind unbekannt; sie belegt keine aktuelle Kompatibilität.
