# Agent CLI Session TUI — 過去の会話を検索して再開

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

CLI をまたぐ履歴から一度の検索で会話を探し、インストール済みの元の CLI で文脈を引き継いで会話を再開。Windows のみ、別のウィンドウを開く代わりに、一致する実行中の Windows Terminal タブへのフォーカスを試みます。

[操作の流れ](#first-workflow) · [Core をインストール](#install-core) · [対応範囲と制限](../../README.md#supported-providers)

既定のリモート分類はタイトルと作業パスを自動送信する場合があります。起動前に Core + wordmatch を選択。

<img src="../agent-session-benefits.png" alt="概念図：CLI をまたいで以前の会話を探して再開" width="780">

これは概念図であり、実際のアプリ画面ではありません。 英語のラベルと架空の行は説明用で、実際の個人の履歴ではありません。

- 以前の話題やプロジェクトを探す：覚えている語句で検索し、プロバイダー、タイトル、作業ディレクトリを確認。
- 正しい会話を再開：**Resumable** で `Enter`。保存済み履歴、インストールと認証を済ませた CLI、アクセス可能な元のディレクトリが必要です。
- 実行中の Windows タブを探す：Windows でのみ一致する Windows Terminal タブへのフォーカスを試みます。汎用的なプロセス接続ではなく、Linux/macOS では手動で切り替えます。

この TUI から始めた会話でなくても対象です。


<a id="install-core"></a>

## プライバシーを重視した最初の検索：Core + wordmatch

少なくとも1つの CLI に保存済みの会話が必要です。再開する CLI はインストール・認証済みで、設定したコマンドから実行でき、元の作業ディレクトリにアクセスできる必要があります。OS と CPU に合う版を対話型ターミナルで使います。

Windows の既定起動は `wt`（代替は `cmd`）、Linux は `gnome-terminal`、macOS は `osascript` と Terminal です。利用できなければ設定を変更します。Core は3 OS の x64/arm64 が対象ですが、セマンティック検索パッケージの対象は異なるのでリリースを確認してください。 ネイティブのタブフォーカスには Windows Terminal が必要で、Windows のみです。

[Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) から OS・CPU に合う **Core の完全なアーカイブ**を取得します。ソースコードのアーカイブではありません。全体を展開し、実行ファイル、`providers/`、`prompts/`、生成済み `config.toml.example` を同じ場所に保ちます。既存設定を上書きせず、`state_dir`、`command`、起動方法と以下の設定を確認します。ソースの `config.toml.template` は未置換のテンプレートなので単に改名しないでください。

起動前に対象配布ファイルのダイジェストを確認。一致は署名や安全性の保証ではありません。OS の警告と組織の規則に従ってください。 [SHA-256](../../README.md#verify-a-release-download).

**初回起動前の注意：** 既定の `remote` グループ分けは会話タイトルと作業ディレクトリを外部サービスへ送信し、初回の検出後に自動実行されることもあります。第三者向けの公開 API 契約はありません。通信しないグループ分けには、次の初回設定手順に従ってください：

Core の初回設定では、生成済みの `config.toml.example` を実行ファイル横の `config.toml` にコピーします。その中の `# [grouping]` と `# engine = "remote"` の両方から先頭の `#` を外し、値を `"wordmatch"` に変更して、下の2行を有効にします。他の分類設定はコメントのままにします。有効な `[grouping]` 表が既にある場合は、その `engine` だけを変更し、表を追加しないでください。保存してから起動し、履歴を読み込みます。

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` は単語の重なりによる分類で、ローカル AI ではありません。オプションのセマンティック検索プラグインは初期化時にモデルをダウンロードする場合があります。まず Core を使えばこのプラグインは読み込まれません。各 CLI 自体の通信を制御する設定ではありません。

<a id="first-workflow"></a>

## 最初の検索と期待結果

まずタイトルの一部やプロジェクト名をそのまま検索。Core はキーワード・曖昧検索に対応し、意味の近い言い換えのランキングには任意のプラグインが必要です。

展開先のターミナルで Windows PowerShell は `.\agent-session-tui.exe`、Linux/macOS は `./agent-session-tui` を実行します。履歴が表示されたら `/` を押し、自分の過去の会話にあるプロジェクト名や語句を入力します。矢印キーで詳細を確認し、CLI・タイトル・ディレクトリを確かめて `Enter`。

6種類の履歴定義と設定済みの再開経路を提供します。ここで実際にテストしたネイティブ CLI のバージョンは**不明**で、実環境の互換性認証ではありません。[対応範囲と証拠](../../README.md#supported-providers)を参照。

| UI | `Enter` |
|---|---|
| Resumable（再開可能な会話） | インストールと認証を済ませた CLI で再開。元のディレクトリが必要 |
| Running / Waiting / Idle | Windows のみ一致する実行中の Windows Terminal タブへのフォーカスを試行。それ以外は手動で切り替え |
| Archived / Stopped | インストールと認証を済ませた CLI で再開。元のディレクトリが必要; [制限](../../README.md#supported-providers) |



**Resumable** を選ぶと、対応する CLI がターミナルで選択した会話を開き、続けられるのが期待結果です。

`Esc` は検索を取り消し、`q` / `Ctrl+C` は履歴を削除せず終了します。他の CLI は停止しません。Kill 機能は未実装です。

- 一覧が空：保存済み履歴、プロバイダーの有効化、`state_dir` を確認。
- 再開できない：CLI のインストール、アカウント認証、元の作業ディレクトリを確認。
- 起動できない：アーカイブ全体を展開し、`providers/`、`prompts/`、生成済み設定例を保持して、起動方法を確認。

空の履歴、設定エラー、起動・フォーカス失敗、更新・削除・復元は [英語のトラブルシューティング](../../README.md#troubleshooting-update-and-removal) を参照してください。[英語版](../../README.md) が技術情報の基準です。[設定](../../README.md#configuration) と [送信内容](../../README.md#what-remote-sends) も確認してください。




プロバイダーと会話が分かっていれば、その CLI の履歴機能で十分です。複数の CLI やプロジェクトにまたがる場合や、セッション ID ではなく話題だけ覚えている場合に役立ちます。

Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI、Kimi の保存済み履歴を横断検索するターミナルのセッション管理ツールです。別々に起動した会話も対象です。標準対応はこの6種類で、他の CLI はプロバイダー定義による拡張が必要です。

起動前にインストール手順のプライバシー設定を行ってください。 既定のリモート分類は自動実行される場合があります。





### Windows Terminal

**Running/Waiting/Idle** は Windows の一致する Windows Terminal タブへの切り替えを試みます。 成功するとタブが選択され、ウィンドウが前面に出ます。 Linux/macOS では既存タブに切り替えられないため手動で移動します。 曖昧な語句や誤字の検索成功は保証されません。

<details>
<summary>↳ プライバシーを重視した最初の検索：Core + wordmatch</summary>

**架空の操作例です。実行確認済みでも同梱セッションでもありません：** `/` で `build failure` を検索し、Claude Code・タイトル `Build failure`・プロジェクト `demo-project` の行が見つかったとします。`↑`/`↓` で詳細と元のディレクトリを確認し、目的の **Resumable** 会話なら `Enter`。インストール・認証済みの Claude CLI で再開し、元のディレクトリへのアクセスも必要です。検索結果や互換性を保証する例ではありません。

</details>



保存済みの履歴を一覧で確認し、自分の会話を検索して再開する流れを示します。GIF は録画時のバージョンの英語 UI で、現行版や各 CLI の互換性を検証するものではありません。文書の翻訳は UI や保存済み会話の翻訳を意味しません。実行前にインストール手順のプライバシー設定を行ってください。 録画したバージョンと日付は不明です。 [Core をインストール](#install-core).

<img src="../agentTUI.gif" alt="履歴一覧・検索・会話再開の TUI デモ" width="700">
