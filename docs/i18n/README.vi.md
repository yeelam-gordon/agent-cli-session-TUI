# Agent CLI Session TUI — Tìm lịch sử và tiếp tục cuộc trò chuyện

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Nhớ công việc nhưng không nhớ cuộc trò chuyện nằm trong CLI nào? Tìm trong lịch sử Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI và Kimi. Nếu biết công cụ và phiên, lịch sử của CLI đó có thể đã đủ.

<img src="../agent-session-benefits.png" alt="Minh họa: tìm kiếm từ khóa tìm token expires trong ngữ cảnh được giữ lại, không chỉ tiêu đề, rồi tiếp tục cuộc hội thoại qua CLI ban đầu. Hỗ trợ Copilot CLI, Claude Code, Codex và các công cụ khác." width="700">

Minh họa ý tưởng, không phải ảnh chụp ứng dụng thật. Nhãn tiếng Anh và các dòng giả định.

- **Tìm lại công việc:** một lần tìm trong nhiều lịch sử còn lưu, kể cả cuộc trò chuyện bắt đầu ngoài TUI này.
- **Tiếp tục cuộc trò chuyện gốc:** mở lại ngữ cảnh đã lưu bằng CLI gốc đã cài.
- **Tìm tab Windows đang chạy:** thử chọn tab Windows Terminal phù hợp; trên Linux/macOS, chuyển thủ công.

<a id="install-core"></a>

## Tìm lần đầu riêng tư: Core + wordmatch

Cần lịch sử đã lưu, CLI đã cài và xác thực, gọi được qua `command` đã cấu hình, cùng thư mục làm việc gốc còn truy cập được. Dùng terminal tương tác và phiên bản đúng hệ điều hành/CPU. Trình mở mặc định: Windows `wt` (dự phòng `cmd`), Linux `gnome-terminal`, macOS `osascript` với Terminal. Nếu thiếu, sửa [cấu hình](../usage.md#configuration).

**Trước khi chạy:** bản dựng có thay đổi mặc định quyền riêng tư tắt nhóm tự động theo mặc định và dùng `wordmatch` thủ công. Bản phát hành cũ vẫn có thể tự nhóm từ xa. Chọn rõ `remote` sẽ gửi tiêu đề và thư mục; ACP có thể gửi tóm tắt qua CLI. [PRIVACY.md](../../PRIVACY.md) mô tả nơi lưu và chỉ mục nội dung hội thoại. Plugin ngữ nghĩa có thể tải mô hình; Core không kiểm soát kết nối CLI. Ví dụ cũ: `# engine = "remote"`; mới: `# engine = "wordmatch"`.

1. Tải **gói Core đầy đủ** đúng hệ điều hành/CPU từ [Tệp Core theo hệ điều hành/CPU](../usage.md#core-archive-filenames), không lấy gói mã nguồn. Giải nén hết, giữ chương trình, `providers/`, `prompts/` và `config.toml.example` đã tạo cùng nhau. [Kiểm tra mã băm của đúng tệp](../usage.md#verify-a-release-download); khớp không phải chữ ký hay bảo đảm an toàn. Tuân thủ cảnh báo hệ điều hành và chính sách tổ chức.
2. Lần cài mới, sao chép ví dụ đã tạo thành `config.toml` cạnh chương trình. **Không ghi đè cấu hình có sẵn hoặc chỉ đổi tên `config.toml.template` trong mã nguồn.** Trong bản sao, bỏ `#` đầu **cả hai dòng** `# [grouping]` và `# engine = "remote"`, rồi đổi giá trị:

```toml
[grouping]
engine = "wordmatch"
auto_suggest = false
```

Giữ các tùy chọn khác dưới dạng chú thích. Nếu đã có bảng `[grouping]` hoạt động, sửa `engine` trong đó, không thêm bảng trùng. Kiểm tra `state_dir`, `command` và **lưu trước khi chạy**. `wordmatch` so sánh từ chung, không phải AI cục bộ.

3. Chạy từ terminal trong thư mục đã giải nén:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Tìm lần đầu và kết quả mong đợi

4. Đợi danh sách, nhấn `/` và nhập đoạn nguyên văn của tiêu đề hoặc dự án trong lịch sử của bạn. Dùng `↑`/`↓` kiểm tra công cụ, tiêu đề và thư mục; chọn **Resumable**, rồi `Enter`. **Kết quả mong đợi:** CLI gốc mở cuộc trò chuyện đã chọn để tiếp tục. Tìm kiếm có thể bỏ sót; phiên bản CLI gốc đã được kiểm thử là chưa rõ. [Phạm vi và bằng chứng](../usage.md#supported-providers).

<a id="windows-terminal"></a>

Với **Running/Waiting/Idle**, chỉ Windows thử chọn tab phù hợp; không phải kết nối vào mọi tiến trình. Nhấn lại `Enter` sớm trên cùng phiên đang chạy có thể **buộc mở lại phiên bằng CLI** thay vì chuyển tab. Hãy kiểm tra trước. [Luồng Windows](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Không có kết quả?** Kiểm tra lịch sử còn lưu, công cụ được bật và `state_dir`; nếu mở không được, kiểm tra xác thực, thư mục gốc và trình mở terminal. [Khắc phục sự cố](../usage.md#troubleshooting-update-and-removal).

**Thoát:** `Esc` hủy tìm; sau đó `q` đóng TUI, hoặc dùng `Ctrl+C` trong danh sách hoặc khi tìm kiếm mà không xóa lịch sử hoặc dừng CLI khác. Chức năng kết thúc phiên chưa được triển khai. [Cập nhật, khôi phục và gỡ bỏ](../usage.md#troubleshooting-update-and-removal).

[Hướng dẫn sử dụng và bằng chứng bằng tiếng Anh](../usage.md) giữ toàn bộ số liệu, kết quả bất lợi và giới hạn. Phiên bản, ngày [ghi hình](../usage.md#recorded-demo) chưa rõ; đây không phải bằng chứng tương thích hiện tại.
