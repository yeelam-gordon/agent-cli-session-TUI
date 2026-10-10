# Agent CLI Session TUI — Frühere CLI-Sitzungen durchsuchen und fortsetzen

War das Debugging-Gespräch in Claude Code, Copilot CLI oder Codex? Durchsuchen Sie vorhandene Verläufe in einer Ansicht, prüfen Sie das passende Gespräch und setzen Sie es über die installierte zugehörige CLI fort. Es muss nicht in diesem TUI begonnen worden sein.

- Altes Thema oder Projekt finden: nach einem erinnerten Ausdruck suchen und Anbieter, Titel und Verzeichnis prüfen.
- Das richtige Gespräch fortsetzen: `Enter` bei **Resumable** benötigt erhaltenen Verlauf, installierte und authentifizierte CLI sowie ein zugängliches ursprüngliches Verzeichnis.
- Laufenden Windows-Tab finden: versucht nur unter Windows, einen passenden Windows-Terminal-Tab zu fokussieren; kein universelles Anhängen an Prozesse. Unter Linux/macOS manuell wechseln.

Wenn Anbieter und Sitzung bekannt sind, genügt der Verlauf der jeweiligen CLI. Dieser Katalog hilft bei mehreren CLIs oder Projekten, wenn Sie das Thema, aber nicht die Sitzungs-ID kennen.

Ein Terminal-Sitzungsmanager für gespeicherte Gespräche aus Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI und Kimi – auch für unabhängig gestartete Sitzungen. Diese sechs Anbieter sind integriert; weitere erfordern zusätzliche Anbieterdefinitionen. Nicht jede CLI wird automatisch unterstützt.

[Ablauf ansehen](#first-workflow) · [Core installieren](#install-core) · [Anbieter und Grenzen](../../README.md#supported-providers)

Konzeptillustration, kein tatsächlicher App-Screenshot. Englische Beschriftungen und fiktive Zeilen dienen als Beispiele, nicht als echte private Verläufe. Vor dem Start die Datenschutzschritte der Installation befolgen; die standardmäßige Remote-Gruppierung kann automatisch laufen.

<img src="../agent-session-benefits.png" alt="Konzept: ein altes Gespräch über CLI-Verläufe hinweg finden und fortsetzen" width="780">

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

<a id="install-core"></a>

## Vor dem Start: Voraussetzungen, Datenschutz und Installation

Mindestens ein Anbieter muss gespeicherte Gespräche haben. Zum Fortsetzen muss seine CLI installiert, authentifiziert und über den konfigurierten Befehl erreichbar sein; das ursprüngliche Arbeitsverzeichnis muss zugänglich bleiben. Verwenden Sie ein interaktives Terminal und die passende Version für Betriebssystem und CPU.

Laden Sie unter [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) das **vollständige Core-Archiv** für Ihr System und Ihre CPU herunter, nicht das Quellcode-Archiv. Entpacken Sie alles; Programm, `providers/`, `prompts/` und die generierte `config.toml.example` müssen zusammenbleiben. Überschreiben Sie keine vorhandene Konfiguration. Prüfen Sie `state_dir`, `command`, den Terminalstarter und die Datenschutzoption unten. Die Quelldatei `config.toml.template` enthält Platzhalter und darf nicht einfach umbenannt werden.

**Datenschutz vor dem ersten Start:** Die voreingestellte Gruppierung `remote` sendet Sitzungstitel und Arbeitsverzeichnisse an einen gehosteten Dienst und kann nach der ersten Erkennung automatisch laufen. Es gibt keinen öffentlichen API-Vertrag für Drittanbieter. Für Gruppierung ohne Netzwerk folgen Sie der folgenden Ersteinrichtung:

Kopieren Sie vor dem ersten Start von Core die generierte Datei `config.toml.example` als `config.toml` neben das Programm. Suchen Sie darin `# [grouping]` und `# engine = "remote"`: Entfernen Sie bei beiden Zeilen das führende `#` und ändern Sie den Wert zu `"wordmatch"`, wie unten gezeigt. Lassen Sie die anderen Optionen auskommentiert. Falls bereits eine aktive `[grouping]`-Tabelle existiert, ändern Sie dort `engine`, ohne eine zweite Tabelle anzulegen. Speichern Sie vor dem Start und dem Einlesen des Verlaufs.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` vergleicht Wörter und ist keine lokale KI. Das optionale semantische Plugin kann bei der Initialisierung Modelle herunterladen. Beginnen Sie mit Core, um dieses Plugin nicht zu laden. Diese Einstellung steuert nicht das Netzwerkverhalten der Anbieter-CLIs.

<a id="first-workflow"></a>

## Erste Suche und erwartetes Ergebnis

Starten Sie im Terminal des entpackten Ordners: `.\agent-session-tui.exe` in Windows PowerShell oder `./agent-session-tui` unter Linux/macOS. Warten Sie auf die Sitzungen, drücken Sie `/` und geben Sie einen Projektnamen oder eine Formulierung aus Ihrem eigenen Verlauf ein. Prüfen Sie mit den Pfeiltasten Anbieter, Titel und Verzeichnis in den Details, bevor Sie `Enter` drücken.

Bei **Resumable** sollte sich ein Terminal öffnen, in dem die Anbieter-CLI das gewählte Gespräch zum Fortsetzen lädt. Bei **Running/Waiting** versucht das Programm nur unter Windows, einen passenden Windows-Terminal-Tab zu fokussieren: Bei Erfolg wird der Tab ausgewählt und das Fenster in den Vordergrund gebracht. Unter Linux/macOS müssen Sie selbst zum laufenden Terminal wechseln. Vage oder falsch geschriebene Suchbegriffe garantieren keinen Treffer.

Standardstarter: Windows `wt` (Ausweichlösung `cmd`), Linux `gnome-terminal`, macOS `osascript` mit Terminal. Passen Sie die Konfiguration an, falls diese fehlen. Core hat x64/arm64-Ziele für alle drei Systeme; semantische Pakete sind nicht für jedes Ziel verfügbar. `Esc` beendet die Suche, `q` / `Ctrl+C` beendet das TUI ohne Verlaufslöschung oder Beenden anderer CLIs. Das Beenden von Sitzungen durch das TUI ist nicht implementiert.

Leere Liste, Konfigurationsfehler, Start- oder Fokusprobleme, Aktualisierung, Entfernen und Wiederherstellung: [englische Fehlerhilfe](../../README.md#troubleshooting-update-and-removal). Die [englische Dokumentation](../../README.md) ist die technische Referenz; siehe [Konfiguration](../../README.md#configuration) und [Datenschutz](../../README.md#what-remote-sends).

**Fiktives Beispiel, weder beobachteter Lauf noch mitgelieferte Sitzung:** Mit `/` nach `build failure` suchen; angenommen, eine Zeile zeigt Claude Code · Titel `Build failure` · Projekt `demo-project`. Mit `↑`/`↓` Anbieter, Titel und ursprüngliches Verzeichnis prüfen. Ist es das richtige **Resumable**-Gespräch, öffnet `Enter` es über die installierte, authentifizierte Claude-CLI; das Verzeichnis muss zugänglich sein. Keine Ergebnis- oder Kompatibilitätsgarantie.

- Leere Liste: gespeicherten Verlauf, aktivierten Anbieter und `state_dir` prüfen.
- Fortsetzen scheitert: CLI-Installation, Anmeldung und ursprüngliches Verzeichnis prüfen.
- Start scheitert: vollständiges Archiv mit `providers/`, `prompts/` und generiertem Beispiel entpacken; Terminalstarter prüfen.

Gespeicherte Gespräche in der Liste ansehen, im eigenen Verlauf suchen und ein Gespräch zum Fortsetzen auswählen. Das GIF zeigt die englische Oberfläche der aufgezeichneten Version; es belegt weder die aktuelle Version noch die Kompatibilität jeder CLI. Übersetzte Dokumentation bedeutet keine übersetzte Oberfläche oder Übersetzung gespeicherter Gespräche. Nehmen Sie vor dem Start die oben beschriebenen Datenschutzeinstellungen vor. Version und Datum der Aufnahme sind unbekannt. [Core installieren](#install-core).

<img src="../agentTUI.gif" alt="TUI-Demo: Verlaufsliste, Suche und Fortsetzen von Gesprächen" width="700">
