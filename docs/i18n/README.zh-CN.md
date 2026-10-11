# Agent CLI Session TUI — 搜索并恢复 AI 编程助手的历史会话

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

记得做过什么，却想不起在哪个 CLI 里？这里可搜索 Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI 和 Kimi 保留的编程对话。已知道工具和会话时，原 CLI 的历史功能通常就够用。

<img src="../agent-session-benefits.png" alt="三步概念图：跨 CLI 找回对话、接续原上下文、尝试切换至匹配的 Windows 标签" width="780">

概念插图，非实际应用截图；英文标签和虚构数据行仅作示意。

- **找回丢失的工作：** 一次搜索多个 CLI 的保留历史，包括未从本工具启动的对话。
- **接续原对话：** 通过已安装的原 CLI 恢复保存的上下文。
- **找到运行中的 Windows 标签：** 尝试聚焦匹配的 Windows Terminal 标签；Linux/macOS 请手动切换。

<a id="install-core"></a>

## 私密首次搜索：Core + wordmatch

须有已保存的对话；对应 CLI 已安装，账户已登录并完成认证，且可通过配置的 `command` 运行，原工作目录仍可访问。使用符合系统和 CPU 的版本，在交互式终端运行。默认启动器为 Windows `wt`（回退 `cmd`）、Linux `gnome-terminal`、macOS `osascript` 与 Terminal；缺少时调整[配置](../usage.md#configuration)。

**启动前：** 默认远程分组在发现历史后可能自动向托管服务发送**会话标题和工作目录**，没有公开的第三方 API 契约。Core + `wordmatch` 可避免这种远程分组，并不限制各 CLI 的网络行为。可选语义插件初始化时可能下载模型；首次使用请选择不含该插件的 Core。

1. 从 [按系统和 CPU 选择 Core 文件](../usage.md#core-archive-filenames) 下载对应系统、CPU 的完整 **Core** 压缩包，而非源码包。全部解压，保留程序、`providers/`、`prompts/` 和生成的 `config.toml.example`。[核对对应文件的摘要](../usage.md#verify-a-release-download)；相符不等于签名或安全保证，须遵守系统警告和组织策略。
2. 首次安装时，将生成的示例复制为程序旁的 `config.toml`，**不要覆盖已有配置，也不要直接改名使用源码的 `config.toml.template`**。在副本中找到 `# [grouping]` 和 `# engine = "remote"`，去掉**两行**开头的 `#`，将值改为：

```toml
[grouping]
engine = "wordmatch"
```

其他分组选项保持注释。若已有启用的 `[grouping]`，只修改其中的 `engine`，不要添加重复节。核对 `state_dir`、`command`，**保存后才启动**。`wordmatch` 只比较重合词，不是本地 AI。

3. 在解压目录的终端运行：

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## 第一次搜索与恢复

4. 等列表加载，按 `/` 输入自己历史中的原始标题片段或项目词语；用 `↑`/`↓` 查看详情，核对工具、标题和目录。选择 **Resumable** 再按 `Enter`。**预期：** 原 CLI 在终端打开选中的对话供你继续。搜索可能遗漏；实际测试过的原生 CLI 版本未知，见[覆盖范围与证据](../usage.md#supported-providers)。

<a id="windows-terminal"></a>

**Running/Waiting/Idle**：仅 Windows 尝试聚焦匹配标签，并非通用进程附加。短时间内对同一运行中的会话再按 `Enter`，可能改为**强制恢复**；请先确认。[Windows 路线](../usage.md#secondary-workflow-find-a-running-windows-tab)。

**没有结果？** 检查保留的历史、工具是否启用及 `state_dir`；启动失败时检查认证、原目录和启动器。[故障排查](../usage.md#troubleshooting-update-and-removal)。

**退出：** `Esc` 取消搜索，然后按 `q` 退出，或在列表或搜索中按 `Ctrl+C`，不删除历史或停止其他 CLI。终止会话功能尚未实现。[更新、还原和移除](../usage.md#troubleshooting-update-and-removal)。

[英文操作与证据指南](../usage.md)保留完整数值、负面结果和限制；[录屏](../usage.md#recorded-demo)的版本、日期未知，不是当前兼容性证明。
