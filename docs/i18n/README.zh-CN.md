# Agent CLI Session TUI — 搜索并恢复 AI 编程助手的历史会话

[English](../../README.md) · [简体中文](README.zh-CN.md) · [日本語](README.ja.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md) · [Deutsch](README.de.md)

这个终端会话管理器搜索 Copilot CLI、Claude Code、Codex CLI、Qwen CLI、Gemini CLI 和 Kimi 的现有历史，包括独立启动的对话。内置支持这六个提供程序；其他 CLI 需要扩展定义，不是通用终端多路复用器。

## 启动前：准备、隐私与安装

至少需要一个提供程序的已保存对话。恢复时，相应 CLI 必须已安装、可通过配置的命令运行并已认证，原工作目录必须可访问。使用交互式终端和匹配系统、CPU 的版本。

**首次启动前注意隐私：** 默认 `remote` 分组发送会话标题和工作目录到托管服务，首次发现完成后可能自动调用，不只是按 `s` 时调用。服务没有公开的第三方接口契约。若要不联网分组，请在启动前编辑 `config.toml` 已有的表，不要重复添加：

```toml
[grouping]
engine = "wordmatch"
```

`wordmatch` 按词语重合分组，不是本地 AI。可选语义插件初始化时可能下载模型；先选 Core 可避免加载它。此设置不控制提供程序 CLI 的网络行为。

从 [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) 下载匹配系统、CPU 的 **Core 完整压缩包**，不是源码包。解压全部内容，保留程序、`providers/`、`prompts/` 和生成的 `config.toml.example`。把示例复制为程序旁的 `config.toml`，不要覆盖旧配置；检查 `state_dir`、`command`、启动器和上述隐私设置。源码的 `config.toml.template` 含占位符，不能直接改名使用。

## 第一次搜索与恢复

在解压目录的终端启动：Windows PowerShell 用 `.\agent-session-tui.exe`，Linux/macOS 用 `./agent-session-tui`。等待列表加载，按 `/`，输入你自己历史中的项目名或短语；用方向键查看详情，确认提供程序、标题和目录后按 `Enter`。

选择 **Resumable**：预期打开终端，由对应 CLI 加载选中的旧对话，继续交流。选择 **Running/Waiting**：仅 Windows 尝试聚焦匹配的 Windows Terminal 标签；成功时标签被选中，窗口来到前台。Linux/macOS 不支持聚焦已有标签，请手动切换。模糊或拼错的查询不保证找到目标。

Windows 默认启动器是 `wt`，回退为 `cmd`；Linux 用 `gnome-terminal`，macOS 用 `osascript` 与 Terminal。缺少时修改配置。Core 有三平台 x64/arm64 目标；语义包不覆盖每个目标，请查看发布附件。`Esc` 取消搜索；`q` / `Ctrl+C` 退出，不删除原历史，也不终止其他 CLI。终止会话功能尚未实现。

空列表、配置错误、启动或聚焦失败，以及更新、卸载、还原见 [英文故障排查](../../README.md#troubleshooting-update-and-removal)。[英文完整说明](../../README.md) 是技术依据；另见 [配置](../../README.md#configuration) 和 [分组隐私](../../README.md#what-remote-sends)。
