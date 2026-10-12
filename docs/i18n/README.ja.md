# Agent CLI Session TUI — 過去の会話を検索して再開

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

作業内容は覚えているのに、どの CLI の会話だったか分からない？ Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI、Kimi の保存済み会話を探せます。CLI と会話が分かっているなら、その CLI の履歴機能で十分です。

<img src="../agent-session-benefits.png" alt="概念図：キーワード検索はタイトルだけでなく保持された会話の文脈で token expires を見つけ、元の CLI で保存済み会話を再開する。Copilot CLI、Claude Code、Codex などに対応。" width="700">

概念図であり、実際のアプリ画面ではありません。英語のラベルと架空の行は説明用です。

- **以前の作業を見つける：** 複数の CLI の保存済み履歴を一度に検索。この TUI 以外から始めた会話も対象です。
- **元の会話を続ける：** インストール済みの元の CLI で保存された文脈を引き継ぎます。
- **実行中の Windows タブを探す：** 一致する Windows Terminal タブへの切り替えを試みます。Linux/macOS では手動で切り替えます。

<a id="install-core"></a>

## プライバシーを重視した最初の検索：Core + wordmatch

保存済み会話と、インストール済みで、ログイン・認証を済ませて設定の `command` から実行できる CLI、アクセス可能な元の作業ディレクトリが必要です。OS・CPU に合う版を対話型ターミナルで使います。既定の起動方法は Windows `wt`（代替 `cmd`）、Linux `gnome-terminal`、macOS `osascript` と Terminal。なければ[設定](../usage.md#configuration)を変更します。

**起動前の注意：** 既定のリモート分類は、履歴の検出後に**会話タイトルと作業ディレクトリ**を外部サービスへ自動送信する場合があります。第三者向けの公開 API 契約はありません。Core + `wordmatch` はこの通信を避けますが、各 CLI の通信は制御しません。任意のセマンティック検索プラグインは初期化時にモデルを取得する場合があります。まずプラグインなしの Core を使ってください。

1. [OS・CPU 別の Core ファイル](../usage.md#core-archive-filenames) から OS・CPU に合う **Core の完全なアーカイブ**を取得します。ソースコードのアーカイブではありません。すべて展開し、実行ファイル、`providers/`、`prompts/`、生成済み `config.toml.example` を一緒に保ちます。[対象ファイルのダイジェストを確認](../usage.md#verify-a-release-download)してください。一致は署名や安全性の保証ではなく、OS の警告と組織の規則にも従います。
2. 初回は生成済みの例を実行ファイル横の `config.toml` にコピーします。**既存設定を上書きせず、ソースの `config.toml.template` を単に改名しないでください。** コピー内の `# [grouping]` と `# engine = "remote"` の**両方**から先頭の `#` を外し、値を変更します：

```toml
[grouping]
engine = "wordmatch"
```

他の分類設定はコメントのままにします。有効な `[grouping]` が既にあれば、その `engine` のみ変更し、同名の表を追加しません。`state_dir` と `command` を確認して**保存してから起動**します。`wordmatch` は単語の重なりの比較で、ローカル AI ではありません。

3. 展開先のターミナルで実行します：

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## 最初の検索と期待結果

4. 一覧が出たら `/` を押し、自分の履歴のタイトルやプロジェクト名の一部をそのまま入力します。`↑`/`↓` で詳細を見て CLI、タイトル、ディレクトリを確認し、**Resumable** を選んで `Enter`。**期待結果：** 元の CLI が選択した会話を開き、続けられます。検索が見逃す場合もあり、実際にテストしたネイティブ CLI の版は不明です。[対応範囲と証拠](../usage.md#supported-providers)。

<a id="windows-terminal"></a>

**Running/Waiting/Idle** は Windows の一致するタブへの切り替えを試みるだけで、汎用的なプロセス接続ではありません。同じ実行中の会話ですぐ再び `Enter` を押すと、代わりに**強制再開**する場合があります。先に対象を確認してください。[Windows の操作](../usage.md#secondary-workflow-find-a-running-windows-tab)。

**見つからない？** 保存履歴、有効なプロバイダー、`state_dir` を確認。起動できなければ認証、元のディレクトリ、起動方法を確認します。[トラブルシューティング](../usage.md#troubleshooting-update-and-removal)。

**終了：** `Esc` で検索を取り消してから `q` で終了するか、一覧または検索で `Ctrl+C` で終了できます。履歴は削除しません。他の CLI は停止しません。Kill は未実装です。[更新・復元・削除](../usage.md#troubleshooting-update-and-removal)。

[英語の操作・証拠ガイド](../usage.md)に数値、悪化した結果、制限を保存しています。[録画](../usage.md#recorded-demo)の版と日付は不明で、現行版の互換性証明ではありません。
