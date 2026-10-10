# Agent CLI Session TUI — 이전 대화를 찾아 코딩 세션 이어가기

그 디버깅 대화가 Claude Code, Copilot CLI, Codex 중 어디에 있었나요? 기존 기록을 한 화면에서 검색하고, 맞는 대화인지 확인한 뒤 해당 CLI로 이어가세요. 이 TUI에서 시작한 대화일 필요는 없습니다.

- 이전 주제나 프로젝트 찾기: 기억나는 표현으로 검색하고 제공자, 제목, 작업 디렉터리를 확인하세요.
- 올바른 대화 이어가기: **Resumable**에서 `Enter`. 보존된 기록, 설치 및 인증이 끝난 CLI, 접근 가능한 원래 디렉터리가 필요합니다.
- 실행 중인 Windows 탭 찾기: Windows에서만 일치하는 Windows Terminal 탭으로 이동을 시도합니다. 모든 프로세스에 연결하는 기능이 아니며 Linux/macOS에서는 직접 전환하세요.

제공자와 대화를 이미 안다면 해당 CLI의 기록 기능으로 충분합니다. 여러 CLI나 프로젝트에 기록이 흩어져 있거나 세션 ID 대신 주제만 기억할 때 이 목록이 유용합니다.

Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI, Kimi의 보존된 대화 기록을 검색하는 터미널 세션 관리 도구입니다. 각각 따로 시작한 대화도 대상입니다. 기본 지원은 이 여섯 가지이며, 기록 형식과 검색 대상 필터에 따라 표시 범위가 달라집니다. 모든 CLI를 지원한다는 뜻은 아닙니다.

[사용 흐름 보기](#first-workflow) · [Core 설치](#install-core) · [지원 범위와 제한](../../README.md#supported-providers)

실제 앱 스크린샷이 아닌 개념 그림입니다. 영어 표기와 예시 행은 설명용이며 실제 개인 기록이 아닙니다. 실행 전에 설치 단계의 개인정보 설정을 완료하세요. 기본 원격 그룹화는 자동으로 실행될 수 있습니다.

<img src="../agent-session-benefits.png" alt="개념도: 여러 CLI 기록에서 이전 대화를 찾아 이어가기" width="780">

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

<a id="install-core"></a>

## 실행 전: 설치와 개인정보 보호

적어도 한 도구에 저장된 대화가 있어야 합니다. 이어갈 CLI는 설치 및 인증이 완료되어 설정의 `command`로 실행할 수 있어야 하며, 원래 작업 디렉터리에도 접근할 수 있어야 합니다. OS와 CPU에 맞는 버전을 대화형 터미널에서 사용하세요.

[Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases)에서 **Core 전체 압축 파일**을 받으세요. 소스 코드 압축 파일이 아닙니다. 모두 풀고 실행 파일, `providers/`, `prompts/`, 생성된 `config.toml.example`을 함께 둡니다. 예제를 실행 파일 옆의 `config.toml`로 복사하되 기존 설정을 덮어쓰지 마세요. `state_dir`, `command`, 터미널 실행 설정을 확인하세요. 소스의 `config.toml.template`에는 치환되지 않은 값이 있으므로 이름만 바꿔 쓰면 안 됩니다.

**첫 실행 전 주의:** 기본 `remote` 그룹화는 대화 제목과 작업 디렉터리를 외부 서비스로 보내며, 최초 기록 탐색 뒤 자동으로 실행될 수 있습니다. 제삼자용 공개 API 계약은 없습니다. 네트워크 없이 그룹화하려면 복사한 설정의 `# [grouping]`과 `# engine = "remote"` 두 줄에서 앞의 `#`를 제거하고 값을 `"wordmatch"`로 바꾸세요.

```toml
[grouping]
engine = "wordmatch"
```

다른 그룹화 옵션은 주석으로 남깁니다. 이미 활성화된 `[grouping]` 설정 섹션이 있으면 그 안의 `engine`만 바꾸고 같은 섹션을 다시 추가하지 마세요. 저장한 뒤에 실행하고 기록을 읽으세요. `wordmatch`는 단어 중복을 비교할 뿐 로컬 AI가 아니며, 각 CLI의 네트워크 사용을 제어하지 않습니다. 선택 사항인 의미 검색 플러그인은 초기화 때 모델을 내려받을 수 있습니다. 우선 Core를 쓰면 이 플러그인을 로드하지 않습니다.

<a id="first-workflow"></a>

## 첫 검색과 예상 결과

압축을 푼 폴더의 터미널에서 Windows PowerShell로 실행합니다.

```powershell
.\agent-session-tui.exe
```

Linux/macOS:

```sh
./agent-session-tui
```

목록이 채워지면 `/`를 누르고 자신의 기록에 있는 프로젝트 이름이나 문구를 입력하세요. `↑`/`↓`로 상세 정보를 보고 도구, 제목, 디렉터리를 확인한 후 `Enter`를 누릅니다. **Resumable**이면 터미널에서 해당 CLI가 선택한 대화를 열어 이어갈 수 있는 것이 예상 결과입니다. **Running/Waiting**이면 Windows에서만 일치하는 Windows Terminal 탭으로 전환을 시도합니다. 성공하면 탭이 선택되고 창이 앞으로 나옵니다. Linux/macOS에서는 실행 중인 터미널로 직접 전환하세요. 모호한 검색어나 오타가 항상 원하는 결과를 찾는 것은 아닙니다.

기본 실행기는 Windows의 `wt`(대체: `cmd`), Linux의 `gnome-terminal`, macOS의 `osascript`와 Terminal입니다. 없으면 설정을 바꾸세요. Core는 세 OS의 x64/arm64를 대상으로 하며 의미 검색 패키지의 제공 대상은 다릅니다. 릴리스 파일을 확인하세요. `Esc`는 검색 취소, `q` / `Ctrl+C`는 TUI 종료입니다. 기록을 삭제하거나 다른 CLI를 중지하지 않습니다. 세션 종료 기능은 구현되지 않았습니다.

빈 목록, 설정 오류, 실행·탭 전환 실패 및 업데이트·제거·복원은 [영문 문제 해결](../../README.md#troubleshooting-update-and-removal)을 참조하세요. [영문 문서](../../README.md)가 기술 기준입니다. [설정](../../README.md#configuration)과 [전송되는 정보](../../README.md#what-remote-sends)도 확인하세요.

**가상의 사용 예이며 실행 관찰이나 포함된 세션이 아닙니다:** `/`로 `build failure`를 검색해 Claude Code · 제목 `Build failure` · 프로젝트 `demo-project` 행을 찾았다고 가정합니다. `↑`/`↓`로 제공자, 제목, 원래 디렉터리를 확인하고 원하는 **Resumable** 대화일 때 `Enter`를 누릅니다. 설치 및 인증이 끝난 Claude CLI와 접근 가능한 원래 디렉터리가 필요합니다. 검색이나 호환성을 보장하지 않습니다.

- 목록이 비어 있음: 저장된 기록, 제공자 활성화, `state_dir`를 확인하세요.
- 재개 실패: CLI 설치, 계정 인증, 원래 작업 디렉터리를 확인하세요.
- 실행 실패: 전체 배포 파일을 풀고 `providers/`, `prompts/`, 생성된 설정 예제를 보존하며 실행기를 확인하세요.

저장된 기록을 목록에서 확인하고 자신의 대화를 검색해 이어가는 흐름입니다. GIF는 녹화 당시 버전의 영어 UI이며, 현재 버전이나 각 CLI의 호환성을 검증한 자료가 아닙니다. 문서 번역이 UI나 저장된 대화의 번역을 뜻하지는 않습니다. 실행 전에 설치 단계에 설명된 개인정보 보호 설정을 완료하세요. 녹화 버전과 날짜는 알려져 있지 않습니다. [Core 설치](#install-core).

<img src="../agentTUI.gif" alt="기록 목록에서 대화를 검색하고 이어가는 TUI 데모" width="700">
