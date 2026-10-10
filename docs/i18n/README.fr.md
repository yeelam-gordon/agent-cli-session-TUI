# Agent CLI Session TUI — Rechercher l’historique et reprendre les sessions

[English](../../README.md) · [简体中文](README.zh-CN.md) · [日本語](README.ja.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md) · [Deutsch](README.de.md)

Un gestionnaire de sessions de terminal pour les conversations enregistrées de Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI et Kimi, y compris celles lancées séparément. Six fournisseurs sont intégrés ; les autres nécessitent des définitions supplémentaires, sans prise en charge universelle des CLI.

## Avant de lancer : prérequis, confidentialité et installation

Il faut des conversations enregistrées chez au moins un fournisseur. Pour les reprendre, son CLI doit être installé, authentifié et accessible par la commande configurée ; le répertoire de travail d’origine doit rester accessible. Utilisez un terminal interactif et une version adaptée au système et au processeur.

**Avant le premier lancement :** le regroupement par défaut `remote` envoie les titres et les répertoires de travail à un service hébergé et peut démarrer automatiquement après la découverte initiale. Il n’existe pas de contrat public d’API pour les tiers. Pour regrouper sans réseau, modifiez la table existante de `config.toml` avant le lancement, sans la dupliquer :

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compare les mots ; ce n’est pas une IA locale. Le module sémantique facultatif peut télécharger des modèles à l’initialisation ; commencez par Core pour ne pas le charger. Ce réglage ne contrôle pas les connexions des CLI eux-mêmes.

Dans [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), téléchargez l’**archive Core complète** adaptée au système et au processeur, et non l’archive du code source. Extrayez tout et conservez ensemble le binaire, `providers/`, `prompts/` et le fichier généré `config.toml.example`. Copiez ce dernier en `config.toml` à côté du binaire sans écraser une configuration existante. Vérifiez `state_dir`, `command`, le lanceur et le réglage de confidentialité ci-dessus. Le fichier source `config.toml.template` contient des paramètres à substituer : ne le renommez pas simplement.

## Première recherche et résultat attendu

Depuis un terminal dans le dossier extrait, lancez `.\agent-session-tui.exe` sous PowerShell Windows ou `./agent-session-tui` sous Linux/macOS. Attendez la liste, appuyez sur `/`, puis saisissez un projet ou une expression de votre propre historique. Parcourez les détails avec les flèches ; vérifiez fournisseur, titre et répertoire avant `Enter`.

Avec **Resumable**, le résultat attendu est un terminal où le CLI ouvre la conversation choisie pour la poursuivre. Avec **Running/Waiting**, seulement sous Windows, le programme tente de sélectionner un onglet Windows Terminal correspondant : en cas de succès, l’onglet est sélectionné et la fenêtre passe au premier plan. Sous Linux/macOS, rejoignez manuellement le terminal actif. Une requête vague ou mal orthographiée ne garantit pas de retrouver la session.

Le lanceur par défaut est `wt` sous Windows (repli : `cmd`), `gnome-terminal` sous Linux, `osascript` avec Terminal sous macOS ; adaptez la configuration si nécessaire. Core vise x64/arm64 sur les trois systèmes ; les archives sémantiques varient selon la cible. `Esc` annule la recherche ; `q` / `Ctrl+C` quitte sans effacer l’historique ni arrêter les autres CLI. L’arrêt des sessions par le TUI n’est pas implémenté.

Liste vide, configuration invalide, échec de lancement ou de sélection, mise à jour, suppression et restauration : [dépannage en anglais](../../README.md#troubleshooting-update-and-removal). La [version anglaise](../../README.md) fait référence pour les détails techniques ; voir [configuration](../../README.md#configuration) et [confidentialité](../../README.md#what-remote-sends).
