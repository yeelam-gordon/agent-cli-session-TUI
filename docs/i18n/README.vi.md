# Agent CLI Session TUI — Tìm lịch sử và tiếp tục cuộc trò chuyện

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

Công cụ quản lý phiên trong terminal để tìm các cuộc trò chuyện còn lưu của Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI và Kimi, kể cả những phiên khởi chạy riêng. Sáu công cụ này có định nghĩa tích hợp; phạm vi phát hiện phụ thuộc định dạng và bộ lọc, không phải hỗ trợ mọi CLI.

## Trước lần chạy đầu: cài đặt và quyền riêng tư

Cần có cuộc trò chuyện đã lưu từ ít nhất một công cụ. Muốn tiếp tục, CLI tương ứng phải được cài đặt, xác thực và gọi được qua `command` đã cấu hình; thư mục làm việc ban đầu vẫn phải truy cập được. Dùng terminal tương tác và bản phù hợp hệ điều hành, CPU.

Tải **toàn bộ gói Core** từ [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), không dùng gói mã nguồn. Giải nén hết, giữ tệp thực thi, `providers/`, `prompts/` và `config.toml.example` đã tạo cùng nhau. Sao chép ví dụ thành `config.toml` cạnh tệp thực thi, không ghi đè cấu hình có sẵn. Kiểm tra `state_dir`, `command` và trình mở terminal. `config.toml.template` trong mã nguồn vẫn chứa các giá trị giữ chỗ chưa được thay thế, không thể chỉ đổi tên rồi dùng.

**Cảnh báo trước khi chạy:** chế độ nhóm mặc định `remote` gửi tiêu đề phiên và thư mục làm việc tới dịch vụ phân nhóm được vận hành bên ngoài máy của bạn, có thể tự chạy sau lần phát hiện lịch sử đầu tiên. Không có cam kết API công khai cho bên thứ ba. Để nhóm không dùng mạng, tìm `# [grouping]` và `# engine = "remote"` trong bản sao; bỏ `#` đầu cả hai dòng rồi đổi giá trị thành `"wordmatch"`:

```toml
[grouping]
engine = "wordmatch"
```

Giữ các tùy chọn nhóm khác ở dạng chú thích. Nếu đã có bảng `[grouping]` hoạt động, sửa `engine` ở đó, không tạo bảng trùng. Lưu trước khi khởi chạy hoặc quét lịch sử. `wordmatch` chỉ so sánh từ, không phải AI cục bộ và không điều khiển mạng của các CLI. Plugin tìm kiếm ngữ nghĩa tùy chọn có thể tải mô hình khi khởi tạo; bắt đầu với Core để không nạp plugin này.

## Tìm lần đầu và kết quả mong đợi

Trong terminal tại thư mục đã giải nén, với Windows PowerShell:

```powershell
.\agent-session-tui.exe
```

Linux/macOS:

```sh
./agent-session-tui
```

Đợi danh sách xuất hiện, nhấn `/` và nhập tên dự án hoặc cụm từ trong lịch sử của chính bạn. Dùng `↑`/`↓` xem chi tiết, kiểm tra công cụ, tiêu đề, thư mục rồi nhấn `Enter`. Với **Resumable**, kết quả mong đợi là terminal mở CLI tương ứng với cuộc trò chuyện đã chọn để tiếp tục. Với **Running/Waiting**, chỉ trên Windows chương trình thử chuyển sang tab Windows Terminal khớp; nếu thành công, tab được chọn và cửa sổ được đưa lên phía trước. Trên Linux/macOS hãy tự chuyển sang terminal đang chạy. Truy vấn mơ hồ hoặc sai chính tả không bảo đảm tìm đúng.

Mặc định dùng Windows `wt` (dự phòng `cmd`), Linux `gnome-terminal`, macOS `osascript` với Terminal; sửa cấu hình nếu thiếu. Core có bản cho kiến trúc x64 và arm64 trên cả ba hệ điều hành; gói tìm kiếm ngữ nghĩa không có sẵn cho mọi tổ hợp hệ điều hành và kiến trúc, nên hãy kiểm tra các tệp phát hành. `Esc` hủy tìm kiếm; `q` / `Ctrl+C` thoát TUI mà không xóa lịch sử hay dừng các CLI khác. Chức năng kết thúc phiên chưa được triển khai.

Danh sách trống, lỗi cấu hình, mở terminal hoặc chuyển tab, cập nhật, gỡ bỏ và khôi phục: [khắc phục sự cố bằng tiếng Anh](../../README.md#troubleshooting-update-and-removal). [Tài liệu tiếng Anh](../../README.md) là nguồn kỹ thuật chuẩn; xem [cấu hình](../../README.md#configuration) và [dữ liệu gửi đi](../../README.md#what-remote-sends).
