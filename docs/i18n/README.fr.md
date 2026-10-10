# Agent CLI Session TUI — Rechercher dans l’historique et reprendre les sessions

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Retrouvez la conversation dans les historiques CLI en une recherche. Reprenez son contexte avec le CLI d’origine installé. Sous Windows seulement, tentez de sélectionner l’onglet Windows Terminal correspondant en cours, sans autre fenêtre.

[Voir le parcours](#first-workflow) · [Installer Core](#install-core) · [Fournisseurs et limites](../../README.md#supported-providers)

Le regroupement distant par défaut peut envoyer automatiquement titres et chemins ; choisissez Core + wordmatch avant le lancement.

<img src="../agent-session-benefits.png" alt="Concept : retrouver et reprendre une conversation dans les historiques des CLI" width="780">

Illustration conceptuelle, pas une capture réelle de l’application. Les libellés anglais et les lignes fictives sont des exemples, pas de vrais historiques privés.

- Retrouvez un sujet ou projet : recherchez une expression mémorisée, puis vérifiez le fournisseur, le titre et le répertoire.
- Reprenez la bonne conversation : `Enter` sur **Resumable** exige un historique conservé, un CLI installé et authentifié et un répertoire d’origine accessible.
- Retrouvez l’onglet en cours d’exécution sous Windows : tente de sélectionner un onglet correspondant de Windows Terminal, uniquement sous Windows ; ce n’est pas un rattachement universel aux processus. Sous Linux/macOS, basculez manuellement.

Elle n’a pas besoin d’avoir été lancée depuis ce TUI.


<a id="install-core"></a>

## Première recherche privée : Core + wordmatch

Il faut des conversations enregistrées chez au moins un fournisseur. Pour les reprendre, son CLI doit être installé, authentifié et accessible par la commande configurée ; le répertoire de travail d’origine doit rester accessible. Utilisez un terminal interactif et une version adaptée au système et au processeur.

Le lanceur par défaut est `wt` sous Windows (repli : `cmd`), `gnome-terminal` sous Linux, `osascript` avec Terminal sous macOS ; adaptez la configuration si nécessaire. Core vise x64/arm64 sur les trois systèmes ; les archives sémantiques varient selon la cible. La sélection native des onglets nécessite Windows Terminal et se limite à Windows.

Dans [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), téléchargez l’**archive Core complète** adaptée au système et au processeur, et non l’archive du code source. Extrayez tout et conservez ensemble le binaire, `providers/`, `prompts/` et le fichier généré `config.toml.example`. Vérifiez `state_dir`, `command`, le lanceur et le réglage de confidentialité ci-dessous. Le fichier source `config.toml.template` contient des paramètres à substituer : ne le renommez pas simplement.

Vérifiez l’empreinte du fichier exact avant le lancement ; la concordance n’est ni signature ni garantie de sécurité. Respectez les alertes du système et les règles de votre organisation. [SHA-256](../../README.md#verify-a-release-download).

**Avant le premier lancement :** le regroupement par défaut `remote` envoie les titres et les répertoires de travail à un service hébergé et peut démarrer automatiquement après la découverte initiale. Il n’existe pas de contrat public d’API pour les tiers. Pour regrouper sans réseau, suivez la configuration initiale ci-dessous :

Pour une première installation de Core, copiez le fichier généré `config.toml.example` en `config.toml` à côté du binaire, sans écraser une configuration existante. Dans cette copie, repérez `# [grouping]` et `# engine = "remote"` : retirez le `#` initial des deux lignes et remplacez la valeur par `"wordmatch"`, comme ci-dessous. Laissez les autres options commentées. Si une table `[grouping]` active existe déjà, modifiez son `engine` sans ajouter une seconde table. Enregistrez avant de lancer le programme ou de lire l’historique.

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` compare les mots ; ce n’est pas une IA locale. Le module sémantique facultatif peut télécharger des modèles à l’initialisation ; commencez par Core pour ne pas le charger. Ce réglage ne contrôle pas les connexions des CLI eux-mêmes.

<a id="first-workflow"></a>

## Première recherche et résultat attendu

Commencez par un fragment littéral du titre ou du projet. Core offre recherche par mots et approximative ; les paraphrases sémantiques nécessitent le plugin facultatif.

Depuis un terminal dans le dossier extrait, lancez `.\agent-session-tui.exe` sous PowerShell Windows ou `./agent-session-tui` sous Linux/macOS. Attendez la liste, appuyez sur `/`, puis saisissez un projet ou une expression de votre propre historique. Parcourez les détails avec les flèches ; vérifiez fournisseur, titre et répertoire avant `Enter`.

Six définitions d’historique et des chemins de reprise configurés sont fournis ; les versions des CLI natifs testées ici sont **inconnues**. Ce n’est pas une certification de compatibilité en conditions réelles. Voir [couverture et preuves](../../README.md#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable | Reprendre via le CLI installé et authentifié, avec le répertoire d’origine accessible |
| Running / Waiting / Idle | Uniquement sous Windows, tenter de sélectionner l’onglet en cours d’exécution correspondant de Windows Terminal ; sinon basculer manuellement |
| Archived / Stopped | Reprendre via le CLI installé et authentifié, avec le répertoire d’origine accessible; [limits](../../README.md#supported-providers) |



Avec **Resumable**, le résultat attendu est un terminal où le CLI ouvre la conversation choisie pour la poursuivre.

`Esc` annule la recherche ; `q` / `Ctrl+C` quitte sans effacer l’historique ni arrêter les autres CLI. L’arrêt des sessions par le TUI n’est pas implémenté.

- Liste vide : vérifiez l’historique enregistré, le fournisseur activé et `state_dir`.
- Reprise impossible : vérifiez l’installation du CLI, l’authentification et le répertoire d’origine.
- Démarrage impossible : extrayez l’archive complète avec `providers/`, `prompts/` et l’exemple généré ; vérifiez le lanceur.

Liste vide, configuration invalide, échec de lancement ou de sélection, mise à jour, suppression et restauration : [dépannage en anglais](../../README.md#troubleshooting-update-and-removal). La [version anglaise](../../README.md) fait référence pour les détails techniques ; voir [configuration](../../README.md#configuration) et [confidentialité](../../README.md#what-remote-sends).




Si vous connaissez déjà le fournisseur et la session, l’historique du CLI suffit. Ce catalogue aide lorsque vos conversations couvrent plusieurs CLI ou projets et que vous vous souvenez du sujet, pas de l’identifiant.

Un gestionnaire de sessions de terminal pour les conversations enregistrées de Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI et Kimi, y compris celles lancées séparément. Six fournisseurs sont intégrés ; les autres nécessitent des définitions supplémentaires, sans prise en charge universelle des CLI.

Suivez les étapes de confidentialité avant le lancement ; le regroupement distant par défaut peut se déclencher automatiquement.





### Windows Terminal

Avec **Running/Waiting/Idle**, seulement sous Windows, le programme tente de sélectionner un onglet Windows Terminal correspondant : en cas de succès, l’onglet est sélectionné et la fenêtre passe au premier plan. Sous Linux/macOS, rejoignez manuellement le terminal actif. Une requête vague ou mal orthographiée ne garantit pas de retrouver la session.

<details>
<summary>↳ Première recherche privée : Core + wordmatch</summary>

**Exemple fictif, ni observé en exécution ni fourni comme session :** appuyez sur `/`, cherchez `build failure` et imaginez une ligne Claude Code · titre `Build failure` · projet `demo-project`. Avec `↑`/`↓`, vérifiez fournisseur, titre et répertoire d’origine. Si c’est la bonne conversation **Resumable**, `Enter` la rouvre via le CLI Claude installé et authentifié, avec le répertoire accessible. Aucun résultat ni compatibilité garantis.

</details>



Consultez les conversations conservées, recherchez dans votre historique et sélectionnez une conversation à reprendre. Le GIF montre l’interface anglaise de la version enregistrée ; il ne vérifie ni la version actuelle ni la compatibilité de chaque CLI. La traduction des documents ne traduit pas l’interface ou les conversations enregistrées. Réglez la confidentialité dans l’installation avant de lancer le programme. La version et la date de l’enregistrement sont inconnues. [Installer Core](#install-core).

<img src="../agentTUI.gif" alt="Démonstration TUI : liste de l’historique, recherche et reprise" width="700">
