# Agent CLI Session TUI — 搜尋歷史對話，接續程式開發工作

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

記得做過的工作，卻忘了在哪個 CLI？這裡可搜尋 Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI 和 Kimi 保留的程式開發對話。若已知道工具與對話，用原 CLI 的紀錄功能就好。

<img src="../agent-session-benefits.png" alt="三步概念圖：跨 CLI 找回對話、接續原上下文、嘗試切換至相符的 Windows 分頁" width="780">

概念插圖，非實際應用程式截圖；英文標籤與虛構資料列僅供示意。

- **找回先前的工作：** 一次搜尋多個 CLI 的保留紀錄，包括從其他地方啟動的對話。
- **接續原對話：** 透過已安裝的原 CLI 恢復儲存的上下文。
- **找到執行中的 Windows 分頁：** 嘗試聚焦相符的 Windows Terminal 分頁；Linux/macOS 請手動切換。

<a id="install-core"></a>

## 私密首次搜尋：Core + wordmatch

須有已儲存的對話；對應 CLI 已安裝，帳戶已登入並完成驗證，能透過設定的 `command` 執行，且原工作目錄仍可存取。使用符合系統、CPU 的版本與互動式終端機。預設啟動器為 Windows `wt`（備援 `cmd`）、Linux `gnome-terminal`、macOS `osascript` 與 Terminal；缺少時調整[設定](../usage.md#configuration)。

**啟動前：** 預設遠端分組在探索紀錄後可能自動將**對話標題與工作目錄**傳至託管服務，沒有公開的第三方 API 契約。Core + `wordmatch` 可避免這種遠端分組，不會控制各 CLI 的網路行為。選用的語意外掛初始化時可能下載模型；第一次請使用不含外掛的 Core。

1. 從 [依系統與 CPU 選擇 Core 檔案](../usage.md#core-archive-filenames) 下載符合系統、CPU 的完整 **Core** 壓縮檔，而非原始碼壓縮檔。全部解壓，保留程式、`providers/`、`prompts/` 與產生好的 `config.toml.example`。[核對該檔案的摘要](../usage.md#verify-a-release-download)；相符不代表簽章或安全保證，仍須遵守系統警告與組織政策。
2. 首次安裝時，將產生好的範例複製為程式旁的 `config.toml`；**不要覆寫既有設定，也不要將原始碼的 `config.toml.template` 直接改名使用**。在副本中找到 `# [grouping]` 和 `# engine = "remote"`，移除**兩行**開頭的 `#`，將值改為：

```toml
[grouping]
engine = "wordmatch"
```

其他分組選項保持註解。若已有啟用的 `[grouping]` 區段，只改其中的 `engine`，不要新增重複區段。核對 `state_dir`、`command`，**儲存後才啟動**。`wordmatch` 只比對共同字詞，不是本機 AI。

3. 在解壓目錄的終端機執行：

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## 找到對話並接續

4. 等清單載入，按 `/` 輸入自己紀錄中的標題原詞或專案片語；用 `↑`/`↓` 檢視詳情，核對工具、標題及目錄。選 **Resumable** 再按 `Enter`。**預期：** 原 CLI 在終端機開啟選取的對話供你接續。搜尋可能遺漏；實際測試過的原生 CLI 版本未知，見[涵蓋範圍與證據](../usage.md#supported-providers)。

<a id="windows-terminal"></a>

**Running/Waiting/Idle**：僅 Windows 嘗試聚焦相符分頁，不是通用程序附加。短時間內對同一執行中的對話再按 `Enter`，可能改為**強制接續**；請先確認。[Windows 操作](../usage.md#secondary-workflow-find-a-running-windows-tab)。

**沒有結果？** 確認保留的紀錄、工具已啟用及 `state_dir`；啟動失敗時檢查驗證、原目錄與啟動器。[疑難排解](../usage.md#troubleshooting-update-and-removal)。

**結束：** `Esc` 取消搜尋後按 `q` 結束 TUI，或在清單或搜尋中按 `Ctrl+C`，不刪除紀錄或停止其他 CLI。終止工作階段功能尚未實作。[更新、還原與移除](../usage.md#troubleshooting-update-and-removal)。

[英文操作與證據指南](../usage.md)保留完整數值、負面結果與限制；[錄影](../usage.md#recorded-demo)的版本、日期未知，不是目前相容性證明。
