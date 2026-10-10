# Agent CLI Session TUI — 搜索并恢复 AI 编程助手的历史会话

那段调试对话是在 Claude Code、Copilot CLI，还是 Codex 里？在一个终端视图中搜索现有历史，查看匹配对话，再通过对应的已安装 CLI 继续。对话不必从本工具启动。

- 找回旧话题或项目：搜索记得的词，再核对提供程序、标题和目录。
- 继续正确的对话：`Enter` 恢复 **Resumable**；需要保留的历史、已安装并认证的 CLI 和可访问的原目录。
- 找到正在运行的 Windows 标签：仅在 Windows 上尝试聚焦匹配的 Windows Terminal 标签，不是通用进程附加；Linux/macOS 请手动切换。

若已知道提供程序和会话，原 CLI 的历史功能就够用。跨多个 CLI 或项目、只记得话题而不记得会话 ID 时，这个目录更有用。

这个终端会话管理器搜索 Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI 和 Kimi 的现有历史，包括独立启动的对话。内置支持这六个提供程序；其他 CLI 需要扩展定义，不是通用终端多路复用器。

[查看流程](#first-workflow) · [安装 Core](#install-core) · [支持范围与限制](../../README.md#supported-providers)

这是概念插图，不是应用实际截图。英文标签和模拟行仅作示意，不是真实的私人历史。启动前请按安装步骤配置隐私；默认远程分组可能自动运行。

<img src="../agent-session-benefits.png" alt="概念图：跨 CLI 历史找回并继续旧对话" width="780">

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

<a id="install-core"></a>

## 启动前：准备、隐私与安装

至少需要一个提供程序的已保存对话。恢复时，相应 CLI 必须已安装、可通过配置的命令运行并已认证，原工作目录必须可访问。使用交互式终端和匹配系统、CPU 的版本。

从 [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) 下载匹配系统、CPU 的 **Core 完整压缩包**，不是源码包。解压全部内容，保留程序、`providers/`、`prompts/` 和生成的 `config.toml.example`。检查 `state_dir`、`command`、启动器和下述隐私设置。源码的 `config.toml.template` 含占位符，不能直接改名使用。

**首次启动前注意隐私：** 默认 `remote` 分组发送会话标题和工作目录到托管服务，首次发现完成后可能自动调用，不只是按 `s` 时调用。服务没有公开的第三方接口契约。若要不联网分组，请按下面的首次安装步骤设置：

首次使用 Core 时，先把生成的 `config.toml.example` 复制为程序旁的 `config.toml`，不要覆盖已有配置。在副本中找到 `# [grouping]` 和 `# engine = "remote"`，去掉这两行开头的 `#`，把值改为 `"wordmatch"`，得到下面两行有效配置。其他分组选项保持注释。若已有启用的 `[grouping]` 配置节，只修改其中的 `engine`，不要再添加同名配置节。保存后才启动程序、扫描历史。

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` 按词语重合分组，不是本地 AI。可选语义插件初始化时可能下载模型；先选 Core 可避免加载它。此设置不控制提供程序 CLI 的网络行为。

<a id="first-workflow"></a>

## 第一次搜索与恢复

在解压目录的终端启动：Windows PowerShell 用 `.\agent-session-tui.exe`，Linux/macOS 用 `./agent-session-tui`。等待列表加载，按 `/`，输入你自己历史中的项目名或短语；用方向键查看详情，确认提供程序、标题和目录后按 `Enter`。

选择 **Resumable**：预期打开终端，由对应 CLI 加载选中的旧对话，继续交流。选择 **Running/Waiting**：仅 Windows 尝试聚焦匹配的 Windows Terminal 标签；成功时标签被选中，窗口来到前台。Linux/macOS 不支持聚焦已有标签，请手动切换。模糊或拼错的查询不保证找到目标。

Windows 默认启动器是 `wt`，回退为 `cmd`；Linux 用 `gnome-terminal`，macOS 用 `osascript` 与 Terminal。缺少时修改配置。Core 有三平台 x64/arm64 目标；语义包不覆盖每个目标，请查看发布附件。`Esc` 取消搜索；`q` / `Ctrl+C` 退出，不删除原历史，也不终止其他 CLI。终止会话功能尚未实现。

空列表、配置错误、启动或聚焦失败，以及更新、卸载、还原见 [英文故障排查](../../README.md#troubleshooting-update-and-removal)。[英文完整说明](../../README.md) 是技术依据；另见 [配置](../../README.md#configuration) 和 [分组隐私](../../README.md#what-remote-sends)。

**虚构操作示例，非实测，也不是随附会话：** 按 `/` 搜索 `build failure`；假设一行显示 Claude Code、标题 `Build failure`、项目 `demo-project`。用 `↑`/`↓` 查看提供程序、标题和原目录，确认是目标 **Resumable** 对话后按 `Enter`，由已安装且认证的 Claude CLI 恢复；原目录须可访问。这不是搜索命中或兼容性的保证。

- 列表为空：检查已保存历史、提供程序是否启用及 `state_dir`。
- 恢复失败：检查 CLI 安装、账户认证及原工作目录。
- 程序无法启动：完整解压发布包，保留 `providers/`、`prompts/` 和生成的配置示例，检查启动器。

通过列表查看已保存的历史，搜索自己的对话，再选择会话恢复。此 GIF 是录制时版本的英文界面，不是当前版本或各提供程序兼容性的验证；文档翻译不代表应用界面或已保存对话被翻译。运行前请先完成前述隐私配置。 录制版本和日期未知。 [安装 Core](#install-core).

<img src="../agentTUI.gif" alt="历史会话列表、搜索与恢复流程的 TUI 演示" width="700">
