# Agent CLI Session TUI — Frühere CLI-Sitzungen durchsuchen und fortsetzen

[English](../../README.md) · [简体中文](README.zh-CN.md) · [日本語](README.ja.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md) · [Deutsch](README.de.md)

Ein Terminal-Sitzungsmanager für gespeicherte Gespräche aus Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI und Kimi – auch für unabhängig gestartete Sitzungen. Diese sechs Anbieter sind integriert; weitere erfordern zusätzliche Anbieterdefinitionen. Nicht jede CLI wird automatisch unterstützt.

## Vor dem Start: Voraussetzungen, Datenschutz und Installation

Mindestens ein Anbieter muss gespeicherte Gespräche haben. Zum Fortsetzen muss seine CLI installiert, authentifiziert und über den konfigurierten Befehl erreichbar sein; das ursprüngliche Arbeitsverzeichnis muss zugänglich bleiben. Verwenden Sie ein interaktives Terminal und die passende Version für Betriebssystem und CPU.

**Datenschutz vor dem ersten Start:** Die voreingestellte Gruppierung `remote` sendet Sitzungstitel und Arbeitsverzeichnisse an einen gehosteten Dienst und kann nach der ersten Erkennung automatisch laufen. Es gibt keinen öffentlichen API-Vertrag für Drittanbieter. Für Gruppierung ohne Netzwerk bearbeiten Sie vor dem Start die vorhandene Tabelle in `config.toml`, ohne sie doppelt anzulegen:

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` vergleicht Wörter und ist keine lokale KI. Das optionale semantische Plugin kann bei der Initialisierung Modelle herunterladen. Beginnen Sie mit Core, um dieses Plugin nicht zu laden. Diese Einstellung steuert nicht das Netzwerkverhalten der Anbieter-CLIs.

Laden Sie unter [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) das **vollständige Core-Archiv** für Ihr System und Ihre CPU herunter, nicht das Quellcode-Archiv. Entpacken Sie alles; Programm, `providers/`, `prompts/` und die generierte `config.toml.example` müssen zusammenbleiben. Kopieren Sie das Beispiel als `config.toml` neben das Programm, ohne vorhandene Einstellungen zu überschreiben. Prüfen Sie `state_dir`, `command`, den Terminalstarter und die Datenschutzoption oben. Die Quelldatei `config.toml.template` enthält Platzhalter und darf nicht einfach umbenannt werden.

## Erste Suche und erwartetes Ergebnis

Starten Sie im Terminal des entpackten Ordners: `.\agent-session-tui.exe` in Windows PowerShell oder `./agent-session-tui` unter Linux/macOS. Warten Sie auf die Sitzungen, drücken Sie `/` und geben Sie einen Projektnamen oder eine Formulierung aus Ihrem eigenen Verlauf ein. Prüfen Sie mit den Pfeiltasten Anbieter, Titel und Verzeichnis in den Details, bevor Sie `Enter` drücken.

Bei **Resumable** sollte sich ein Terminal öffnen, in dem die Anbieter-CLI das gewählte Gespräch zum Fortsetzen lädt. Bei **Running/Waiting** versucht das Programm nur unter Windows, einen passenden Windows-Terminal-Tab zu fokussieren: Bei Erfolg wird der Tab ausgewählt und das Fenster in den Vordergrund gebracht. Unter Linux/macOS müssen Sie selbst zum laufenden Terminal wechseln. Vage oder falsch geschriebene Suchbegriffe garantieren keinen Treffer.

Standardstarter: Windows `wt` (Ausweichlösung `cmd`), Linux `gnome-terminal`, macOS `osascript` mit Terminal. Passen Sie die Konfiguration an, falls diese fehlen. Core hat x64/arm64-Ziele für alle drei Systeme; semantische Pakete sind nicht für jedes Ziel verfügbar. `Esc` beendet die Suche, `q` / `Ctrl+C` beendet das TUI ohne Verlaufslöschung oder Beenden anderer CLIs. Das Beenden von Sitzungen durch das TUI ist nicht implementiert.

Leere Liste, Konfigurationsfehler, Start- oder Fokusprobleme, Aktualisierung, Entfernen und Wiederherstellung: [englische Fehlerhilfe](../../README.md#troubleshooting-update-and-removal). Die [englische Dokumentation](../../README.md) ist die technische Referenz; siehe [Konfiguration](../../README.md#configuration) und [Datenschutz](../../README.md#what-remote-sends).
