# Agent CLI Session TUI — 搜尋歷史對話，接續程式開發工作

那段除錯對話是在 Claude Code、Copilot CLI，還是 Codex？在同一個終端機畫面搜尋現有紀錄，查看符合的對話，再透過對應的已安裝 CLI 接續工作。對話不必從這個工具啟動。

- 找回舊話題或專案：搜尋記得的詞，再核對工具、標題和工作目錄。
- 接續正確的對話：`Enter` 恢復 **Resumable**；須保有紀錄、已安裝並完成驗證的 CLI，以及可存取的原目錄。
- 找到執行中的 Windows 分頁：僅在 Windows 嘗試聚焦相符的 Windows Terminal 分頁，不是通用的程序附加；Linux/macOS 請手動切換。

已知道工具和對話時，用原 CLI 的紀錄功能就好。跨多個 CLI 或專案、只記得話題而非工作階段 ID 時，這個目錄更適合。

這個終端機工作階段管理工具可搜尋 Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI 和 Kimi 已保留的對話紀錄，包括各自啟動的對話。內建支援這六種工具；是否找得到紀錄仍取決於格式與紀錄偵測時的篩選條件，不代表支援所有 CLI。

[查看操作流程](#first-workflow) · [安裝 Core](#install-core) · [支援範圍與限制](../../README.md#supported-providers)

這是概念插圖，不是實際應用程式截圖。英文標籤與模擬資料列僅供示意，不是真實的私人紀錄。啟動前請依安裝步驟設定隱私；預設遠端分組可能自動執行。

<img src="../agent-session-benefits.png" alt="概念圖：跨 CLI 紀錄找回並接續舊對話" width="780">

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

<a id="install-core"></a>

## 首次啟動前：安裝與隱私

至少要有一種工具的已儲存對話。要接續對話，對應 CLI 必須已安裝、完成驗證，能透過設定的 `command` 執行，且原本的工作目錄仍可存取。請使用互動式終端機及符合系統、CPU 的版本。

從 [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) 下載 **Core 完整壓縮檔**，不要選原始碼壓縮檔。全部解壓縮，讓執行檔、`providers/`、`prompts/` 與產生好的 `config.toml.example` 留在一起。將範例複製為執行檔旁的 `config.toml`，不要覆寫既有設定。確認 `state_dir`、`command` 與終端機啟動方式。原始碼中的 `config.toml.template` 含尚未替換的欄位，不能直接改名使用。

**先確認隱私再啟動：** 預設的 `remote` 分組會將對話標題與工作目錄傳送至託管服務，首次探索完成後可能自動執行。服務沒有公開的第三方 API 契約。若要分組時不連網，在剛複製的設定檔中找到 `# [grouping]` 和 `# engine = "remote"`，移除這兩行開頭的 `#`，將值改成 `"wordmatch"`：

```toml
[grouping]
engine = "wordmatch"
```

其他分組選項保持註解。若已有啟用的 `[grouping]` 區段，只改其中的 `engine`，不要再新增同名區段。儲存後才啟動、掃描紀錄。`wordmatch` 只是比對共同字詞，不是本機 AI，也不會控制各 CLI 的網路連線。選用的語意搜尋外掛初始化時可能下載模型；先用 Core 可避免載入外掛。

<a id="first-workflow"></a>

## 找到對話並接續

在解壓縮目錄的終端機執行：

```powershell
.\agent-session-tui.exe
```

Linux/macOS：

```sh
./agent-session-tui
```

等清單載入，按 `/`，輸入自己歷史對話中的專案名稱或片語。用 `↑`/`↓` 檢視詳情，確認工具、標題與目錄後按 `Enter`。選 **Resumable**：預期開啟終端機，由對應 CLI 載入選取的對話，繼續工作。選 **Running/Waiting**：僅 Windows 會嘗試切換至符合的 Windows Terminal 分頁；成功時分頁被選取，視窗移到前景。Linux/macOS 請自行切換至執行中的終端機。搜尋排序與模糊查詢不保證找到目標。

Windows 預設啟動器為 `wt`，備援為 `cmd`；Linux 使用 `gnome-terminal`，macOS 使用 `osascript` 與 Terminal。缺少時請調整設定。Core 提供三平台 x64/arm64 目標；語意外掛套件的供應範圍不同，請查看發行附件。`Esc` 取消搜尋，`q` / `Ctrl+C` 結束 TUI，不刪除原始紀錄，也不停止其他 CLI。終止工作階段功能尚未實作。

清單空白、設定錯誤、啟動或分頁切換失敗，以及更新、移除與還原，請參閱 [英文疑難排解](../../README.md#troubleshooting-update-and-removal)。[英文說明](../../README.md) 是技術依據；另見 [設定](../../README.md#configuration) 與 [傳送資料說明](../../README.md#what-remote-sends)。

**虛構操作範例，非實測，也不是隨附對話：** 按 `/` 搜尋 `build failure`；假設一列顯示 Claude Code、標題 `Build failure`、專案 `demo-project`。用 `↑`/`↓` 查看工具、標題和原目錄，確認是目標 **Resumable** 對話再按 `Enter`，由已安裝且完成驗證的 Claude CLI 接續；原目錄須可存取。這不是搜尋命中或相容性保證。

- 清單空白：確認已保留紀錄、工具已啟用及 `state_dir` 正確。
- 無法接續：確認 CLI 已安裝、帳戶已登入驗證及原工作目錄可存取。
- 無法啟動：完整解壓縮發行套件，保留 `providers/`、`prompts/` 與產生的設定範例，檢查啟動器。

從清單查看保留的對話，搜尋自己的紀錄，再選取工作階段接續。這份 GIF 錄製的是當時版本的英文介面，不是目前版本或各工具相容性的驗證；翻譯文件不代表介面或儲存的對話也已翻譯。執行前請先完成前述的隱私設定。 錄製版本與日期未知。 [安裝 Core](#install-core).

<img src="../agentTUI.gif" alt="歷史對話清單、搜尋與接續流程的終端機示範" width="700">
