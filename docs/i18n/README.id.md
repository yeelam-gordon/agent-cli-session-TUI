# Agent CLI Session TUI — Cari riwayat sesi dan lanjutkan percakapan

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

Pengelola sesi terminal untuk percakapan yang masih tersimpan dari Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI, dan Kimi, termasuk yang dimulai secara terpisah. Keenam penyedia ini didukung secara bawaan; cakupan bergantung pada format dan filter penemuan riwayat, bukan dukungan untuk semua CLI.

## Sebelum mulai: pemasangan dan privasi

Anda perlu percakapan tersimpan dari setidaknya satu penyedia. Untuk melanjutkannya, CLI terkait harus terpasang, sudah diautentikasi, dan dapat dijalankan lewat `command` dalam konfigurasi; direktori kerja asal harus dapat diakses. Gunakan terminal interaktif dan versi yang cocok dengan sistem operasi serta CPU.

Unduh **arsip Core lengkap** dari [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), bukan arsip kode sumber. Ekstrak seluruhnya; simpan program, `providers/`, `prompts/`, dan `config.toml.example` yang telah dihasilkan bersama-sama. Salin contoh menjadi `config.toml` di samping program tanpa menimpa pengaturan lama. Periksa `state_dir`, `command`, dan peluncur terminal. `config.toml.template` pada sumber masih memuat penanda yang harus diganti; jangan sekadar mengganti namanya.

**Peringatan sebelum peluncuran pertama:** pengelompokan bawaan `remote` mengirim judul sesi dan direktori kerja ke layanan yang dihosting, dan dapat berjalan otomatis setelah penemuan awal. Tidak ada kontrak API publik untuk pihak ketiga. Untuk mengelompokkan tanpa jaringan, temukan `# [grouping]` dan `# engine = "remote"` dalam salinan konfigurasi; hapus `#` awal di kedua baris, lalu ubah nilainya menjadi `"wordmatch"`:

```toml
[grouping]
engine = "wordmatch"
```

Biarkan opsi lain tetap dikomentari. Jika sudah ada tabel `[grouping]` aktif, ubah `engine` di situ tanpa membuat tabel kedua. Simpan sebelum menjalankan program atau memindai riwayat. `wordmatch` hanya membandingkan kata, bukan AI lokal, dan tidak mengatur koneksi CLI penyedia. Plugin pencarian semantik opsional dapat mengunduh model saat inisialisasi; mulai dengan Core agar plugin itu tidak dimuat.

## Pencarian pertama dan hasil yang diharapkan

Dari terminal di folder hasil ekstraksi, dengan Windows PowerShell:

```powershell
.\agent-session-tui.exe
```

Linux/macOS:

```sh
./agent-session-tui
```

Tunggu daftar muncul, tekan `/`, lalu masukkan nama proyek atau frasa dari riwayat Anda sendiri. Gunakan `↑`/`↓` untuk memeriksa detail, pastikan penyedia, judul, dan direktori sebelum `Enter`. Untuk **Resumable**, hasil yang diharapkan adalah terminal yang membuka CLI dengan percakapan terpilih untuk dilanjutkan. Untuk **Running/Waiting**, hanya di Windows program mencoba memilih tab Windows Terminal yang cocok; jika berhasil, tab dipilih dan jendela tampil di depan. Di Linux/macOS, pindah ke terminal yang sedang berjalan secara manual. Hasil untuk kueri samar atau salah ketik tidak dijamin.

Peluncur bawaan: Windows `wt` (cadangan `cmd`), Linux `gnome-terminal`, macOS `osascript` dengan Terminal. Sesuaikan pengaturan bila tidak tersedia. Core memiliki target x64/arm64 pada tiga sistem; paket semantik tidak tersedia untuk semua target, periksa berkas rilis. `Esc` membatalkan pencarian; `q` / `Ctrl+C` keluar dari TUI tanpa menghapus riwayat atau menghentikan CLI lain. Penghentian sesi belum diimplementasikan.

Daftar kosong, kesalahan konfigurasi, peluncuran atau perpindahan tab, pembaruan, penghapusan dan pemulihan: [pemecahan masalah dalam bahasa Inggris](../../README.md#troubleshooting-update-and-removal). [Dokumentasi Inggris](../../README.md) menjadi acuan teknis; lihat [konfigurasi](../../README.md#configuration) dan [data yang dikirim](../../README.md#what-remote-sends).
