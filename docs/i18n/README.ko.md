# Agent CLI Session TUI — 이전 대화를 찾아 코딩 세션 이어가기

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

하던 작업은 기억나는데 어느 CLI의 대화였는지 모르겠나요? Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI, Kimi에 보존된 대화를 검색합니다. 도구와 대화를 이미 안다면 그 CLI의 기록 기능만으로 충분할 수 있습니다.

<img src="../agent-session-benefits.png" alt="개념도: 키워드 검색이 제목뿐 아니라 보존된 대화 맥락에서 token expires를 찾고 원래 CLI로 저장된 대화를 재개합니다. Copilot CLI, Claude Code, Codex 등을 지원합니다." width="700">

실제 앱 화면이 아닌 개념 그림입니다. 영어 표기와 가상의 행은 설명용입니다.

- **이전 작업 찾기:** 여러 CLI의 보존된 기록을 한 번에 검색합니다. 이 TUI 밖에서 시작한 대화도 포함합니다.
- **원래 대화 이어가기:** 설치된 원래 CLI를 통해 저장된 맥락을 다시 엽니다.
- **실행 중인 Windows 탭 찾기:** 일치하는 Windows Terminal 탭으로 전환을 시도합니다. Linux/macOS에서는 직접 전환하세요.

<a id="install-core"></a>

## 개인정보를 고려한 첫 검색: Core + wordmatch

저장된 대화, 설치되어 있고 계정 로그인·인증을 마쳐 설정의 `command`로 실행 가능한 CLI, 접근 가능한 원래 작업 디렉터리가 필요합니다. OS와 CPU에 맞는 버전을 대화형 터미널에서 쓰세요. 기본 실행기는 Windows `wt`(대체 `cmd`), Linux `gnome-terminal`, macOS `osascript`와 Terminal입니다. 없으면 [설정](../usage.md#configuration)을 변경하세요.

**실행 전:** 기본 원격 그룹화는 기록 탐색 뒤 **대화 제목과 작업 디렉터리**를 외부 서비스로 자동 전송할 수 있습니다. 공개된 제삼자 API 계약은 없습니다. Core + `wordmatch`는 이 원격 그룹화를 피하지만 각 CLI의 통신은 제어하지 않습니다. 선택 사항인 의미 검색 플러그인은 초기화 때 모델을 내려받을 수 있으므로 우선 플러그인 없는 Core를 쓰세요.

1. [OS·CPU별 Core 파일](../usage.md#core-archive-filenames)에서 OS·CPU에 맞는 **Core 전체 압축 파일**을 받으세요. 소스 코드 압축 파일이 아닙니다. 전부 풀고 프로그램, `providers/`, `prompts/`, 생성된 `config.toml.example`을 함께 둡니다. [해당 파일의 다이제스트를 확인](../usage.md#verify-a-release-download)하세요. 일치는 서명이나 안전 보장이 아니며 OS 경고와 조직 정책을 따라야 합니다.
2. 처음 설치할 때 생성된 예제를 실행 파일 옆 `config.toml`로 복사하세요. **기존 설정을 덮어쓰거나 소스의 `config.toml.template` 이름만 바꿔 쓰지 마세요.** 복사본에서 `# [grouping]`과 `# engine = "remote"` **두 줄 모두** 앞의 `#`를 지우고 값을 바꾸세요.

```toml
[grouping]
engine = "wordmatch"
```

다른 그룹화 옵션은 주석으로 남깁니다. 활성화된 `[grouping]` 섹션이 있으면 그 안의 `engine`만 바꾸고 중복 섹션은 추가하지 마세요. `state_dir`, `command`를 확인하고 **저장한 뒤 실행**하세요. `wordmatch`는 여러 세션 사이에 겹치는 단어를 비교하며 로컬 AI가 아닙니다.

3. 압축을 푼 폴더의 터미널에서 실행하세요.

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## 첫 검색과 예상 결과

4. 목록이 나오면 `/`를 누르고 자신의 기록에 있는 제목·프로젝트 문구를 그대로 입력하세요. `↑`/`↓`로 상세 정보를 보고 도구, 제목, 디렉터리를 확인한 뒤 **Resumable**에서 `Enter`를 누릅니다. **예상 결과:** 원래 CLI가 선택한 대화를 열어 이어갈 수 있습니다. 검색 결과에서 일부 대화가 누락될 수 있고 실제 테스트한 네이티브 CLI 버전은 알려져 있지 않습니다. [지원 범위와 근거](../usage.md#supported-providers).

<a id="windows-terminal"></a>

**Running/Waiting/Idle**은 Windows의 일치하는 탭으로 전환을 시도할 뿐 모든 프로세스에 연결하지 않습니다. 같은 실행 중 대화에서 곧바로 `Enter`를 다시 누르면 대신 **강제로 재개**할 수 있으니 먼저 확인하세요. [Windows 동작](../usage.md#secondary-workflow-find-a-running-windows-tab).

**결과가 없나요?** 보존된 기록, 제공자 활성화, `state_dir`를 확인하세요. 실행 실패 시 인증, 원래 디렉터리, 실행기를 확인하세요. [문제 해결](../usage.md#troubleshooting-update-and-removal).

**종료:** `Esc`로 검색을 취소한 뒤 `q`로 종료하거나, 목록이나 검색에서 `Ctrl+C`로 종료하세요. 기록을 지우거나 다른 CLI를 중지하지 않습니다. 세션 종료 기능은 미구현입니다. [업데이트·복원·제거](../usage.md#troubleshooting-update-and-removal).

[영문 사용·근거 안내](../usage.md)에 전체 수치, 나빠진 결과, 제한이 있습니다. [녹화](../usage.md#recorded-demo)의 버전·날짜는 미상이며 현재 호환성 증명이 아닙니다.
