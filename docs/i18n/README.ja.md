# Agent CLI Session TUI — 過去の会話を検索して再開

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI、Kimi の保存済み履歴を横断検索するターミナルのセッション管理ツールです。別々に起動した会話も対象です。標準対応はこの6種類で、他の CLI はプロバイダー定義による拡張が必要です。

## 起動前の確認・プライバシー・インストール

少なくとも1つの CLI に保存済みの会話が必要です。再開する CLI はインストール・認証済みで、設定したコマンドから実行でき、元の作業ディレクトリにアクセスできる必要があります。OS と CPU に合う版を対話型ターミナルで使います。

**初回起動前の注意：** 既定の `remote` グループ分けは会話タイトルと作業ディレクトリを外部サービスへ送信し、初回の検出後に自動実行されることもあります。第三者向けの公開 API 契約はありません。通信しないグループ分けには、起動前に `config.toml` の既存の表を編集します（重複させないでください）：

Core の初回設定では、生成済みの `config.toml.example` を実行ファイル横の `config.toml` にコピーします。その中の `# [grouping]` と `# engine = "remote"` の両方から先頭の `#` を外し、値を `"wordmatch"` に変更して、下の2行を有効にします。他の分類設定はコメントのままにします。有効な `[grouping]` 表が既にある場合は、その `engine` だけを変更し、表を追加しないでください。保存してから起動し、履歴を読み込みます。

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` は単語の重なりによる分類で、ローカル AI ではありません。オプションのセマンティック検索プラグインは初期化時にモデルをダウンロードする場合があります。まず Core を使えばこのプラグインは読み込まれません。各 CLI 自体の通信を制御する設定ではありません。

[Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) から OS・CPU に合う **Core の完全なアーカイブ**を取得します。ソースコードのアーカイブではありません。全体を展開し、実行ファイル、`providers/`、`prompts/`、生成済み `config.toml.example` を同じ場所に保ちます。例を実行ファイル横の `config.toml` にコピーし、既存設定を上書きせず、`state_dir`、`command`、起動方法と上記の設定を確認します。ソースの `config.toml.template` は未置換のテンプレートなので単に改名しないでください。

## 最初の検索と期待結果

展開先のターミナルで Windows PowerShell は `.\agent-session-tui.exe`、Linux/macOS は `./agent-session-tui` を実行します。履歴が表示されたら `/` を押し、自分の過去の会話にあるプロジェクト名や語句を入力します。矢印キーで詳細を確認し、CLI・タイトル・ディレクトリを確かめて `Enter`。

**Resumable** を選ぶと、対応する CLI がターミナルで選択した会話を開き、続けられるのが期待結果です。**Running/Waiting** は Windows の一致する Windows Terminal タブへの切り替えを試みます。成功するとタブが選択され、ウィンドウが前面に出ます。Linux/macOS では既存タブに切り替えられないため手動で移動します。曖昧な語句や誤字の検索成功は保証されません。

Windows の既定起動は `wt`（代替は `cmd`）、Linux は `gnome-terminal`、macOS は `osascript` と Terminal です。利用できなければ設定を変更します。Core は3 OS の x64/arm64 が対象ですが、セマンティック検索パッケージの対象は異なるのでリリースを確認してください。`Esc` は検索を取り消し、`q` / `Ctrl+C` は履歴を削除せず終了します。他の CLI は停止しません。Kill 機能は未実装です。

空の履歴、設定エラー、起動・フォーカス失敗、更新・削除・復元は [英語のトラブルシューティング](../../README.md#troubleshooting-update-and-removal) を参照してください。[英語版](../../README.md) が技術情報の基準です。[設定](../../README.md#configuration) と [送信内容](../../README.md#what-remote-sends) も確認してください。
