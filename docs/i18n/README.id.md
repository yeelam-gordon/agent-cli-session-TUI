# Agent CLI Session TUI — Cari riwayat sesi dan lanjutkan percakapan

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Ingat pekerjaannya, tetapi lupa percakapan itu ada di CLI mana? Cari dalam riwayat Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI, dan Kimi. Jika penyedia dan sesi sudah diketahui, riwayat CLI itu sendiri mungkin cukup.

<img src="../agent-session-benefits.png" alt="Konsep: pencarian kata menemukan token expires dalam konteks yang tersimpan, bukan hanya judul, lalu melanjutkan percakapan tersimpan lewat CLI asalnya. Mendukung Copilot CLI, Claude Code, Codex dan lainnya." width="700">

Ilustrasi konsep, bukan tangkapan layar aplikasi sebenarnya. Label bahasa Inggris dan baris rekaan.

- **Temukan kembali pekerjaan:** satu pencarian dalam beberapa riwayat tersimpan, termasuk percakapan yang dimulai di luar TUI ini.
- **Lanjutkan percakapan asal:** buka kembali konteks tersimpan melalui CLI asal yang terpasang.
- **Temukan tab Windows yang berjalan:** coba pilih tab Windows Terminal yang cocok; di Linux/macOS, beralih manual.

<a id="install-core"></a>

## Pencarian pertama privat: Core + wordmatch

Perlu riwayat tersimpan, CLI terpasang dengan autentikasi selesai, dapat dijalankan lewat `command` yang dikonfigurasi, dan direktori kerja asal yang dapat diakses. Gunakan terminal interaktif dan versi untuk OS/CPU Anda. Peluncur bawaan: Windows `wt` (cadangan `cmd`), Linux `gnome-terminal`, macOS `osascript` dengan Terminal. Jika tidak tersedia, ubah [konfigurasi](../usage.md#configuration).

**Sebelum menjalankan:** build dengan perubahan standar privasi menonaktifkan pengelompokan otomatis secara bawaan dan memakai `wordmatch` manual. Rilis lama masih dapat mengelompokkan otomatis dari jarak jauh. Pilihan eksplisit `remote` mengirim judul dan direktori; ACP dapat mengirim ringkasan melalui CLI. [PRIVACY.md](../../PRIVACY.md) menjelaskan lokasi penyimpanan dan indeks isi percakapan. Plugin semantik dapat mengunduh model; Core tidak mengontrol koneksi CLI.

1. Unduh **arsip Core lengkap** untuk OS/CPU Anda dari [Berkas Core untuk OS/CPU](../usage.md#core-archive-filenames), bukan arsip kode sumber. Ekstrak semuanya. Simpan program, `providers/`, `prompts/`, dan `config.toml.example` yang dihasilkan dalam folder yang sama. [Periksa digest berkas yang tepat](../usage.md#verify-a-release-download); cocok bukan berarti bertanda tangan atau dijamin aman. Patuhi peringatan OS dan kebijakan organisasi.
2. Pada pemasangan baru, salin contoh yang dihasilkan menjadi `config.toml` di samping program. **Jangan timpa pengaturan lama atau sekadar mengganti nama `config.toml.template` dari sumber.** Dalam salinan, aktifkan `# [grouping]` dan `# engine = "wordmatch"` (contoh lama: `# engine = "remote"`), lalu atur bagian itu seperti di bawah, termasuk menonaktifkan pengelompokan otomatis:

```toml
[grouping]
engine = "wordmatch"
auto_suggest = false
```

Biarkan opsi lainnya dikomentari. Jika tabel `[grouping]` aktif sudah ada, ubah `engine` di situ, jangan membuat duplikat. Periksa `state_dir` dan `command`; **simpan sebelum menjalankan**. `wordmatch` membandingkan kata yang sama, bukan AI lokal.

3. Jalankan dari terminal dalam folder hasil ekstraksi:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## Pencarian pertama dan hasil yang diharapkan

4. Tunggu daftar, tekan `/`, lalu ketik potongan judul atau proyek persis dari riwayat Anda. Dengan `↑`/`↓`, periksa penyedia, judul, dan direktori; pilih **Resumable**, lalu `Enter`. **Hasil yang diharapkan:** CLI asal membuka percakapan terpilih untuk dilanjutkan. Pencarian bisa melewatkan hasil; versi CLI asli yang telah diuji tidak diketahui. [Cakupan dan bukti](../usage.md#supported-providers).

<a id="windows-terminal"></a>

Untuk **Running/Waiting/Idle**, hanya Windows mencoba memilih tab yang cocok; bukan koneksi universal ke proses. Menekan `Enter` lagi segera pada sesi berjalan yang sama dapat **memaksa melanjutkan sesi** alih-alih berpindah tab. Periksa dahulu. [Alur Windows](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Tidak ada hasil?** Periksa riwayat tersimpan, penyedia aktif, dan `state_dir`; jika gagal diluncurkan, periksa autentikasi, direktori asal, dan peluncur. [Pemecahan masalah](../usage.md#troubleshooting-update-and-removal).

**Keluar:** `Esc` membatalkan pencarian; setelah itu `q` menutup TUI, atau gunakan `Ctrl+C` pada daftar atau pencarian tanpa menghapus riwayat atau menghentikan CLI lain. Penghentian sesi belum diimplementasikan. [Pembaruan, pemulihan, dan penghapusan](../usage.md#troubleshooting-update-and-removal).

[Panduan penggunaan dan bukti dalam bahasa Inggris](../usage.md) menyimpan angka lengkap, hasil merugikan, dan batasan. Versi dan tanggal [rekaman](../usage.md#recorded-demo) tidak diketahui; bukan bukti kompatibilitas saat ini.
