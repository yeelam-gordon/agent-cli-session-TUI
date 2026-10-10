# Agent CLI Session TUI — Cari riwayat sesi dan lanjutkan percakapan

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Temukan percakapan lintas riwayat CLI dalam satu pencarian. Lanjutkan konteks lewat CLI asal terpasang. Hanya di Windows, coba fokuskan tab Windows Terminal berjalan yang cocok, bukan membuka jendela lain.

[Lihat alur](#first-workflow) · [Pasang Core](#install-core) · [Penyedia dan batasan](../../README.md#supported-providers)

Pengelompokan jarak jauh bawaan dapat mengirim judul dan jalur kerja otomatis; pilih Core + wordmatch sebelum menjalankan.

<img src="../agent-session-benefits.png" alt="Konsep: menemukan dan melanjutkan percakapan lama di riwayat berbagai CLI" width="780">

Ilustrasi konsep, bukan tangkapan layar aplikasi sebenarnya. Label bahasa Inggris dan baris rekaan hanya contoh, bukan riwayat pribadi nyata.

- Temukan topik atau proyek lama: cari frasa yang diingat, lalu periksa penyedia, judul, dan direktori.
- Lanjutkan percakapan yang tepat: `Enter` pada **Resumable** memerlukan riwayat tersimpan, CLI terpasang dan terautentikasi, serta direktori asal yang dapat diakses.
- Temukan tab Windows yang berjalan: hanya di Windows, mencoba memfokuskan tab Windows Terminal yang cocok; bukan koneksi universal ke proses. Di Linux/macOS, beralih secara manual.

Percakapan tidak harus dimulai dari TUI ini.


<a id="install-core"></a>

## Pencarian pertama privat: Core + wordmatch

Anda perlu percakapan tersimpan dari setidaknya satu penyedia. Untuk melanjutkannya, CLI terkait harus terpasang, sudah diautentikasi, dan dapat dijalankan lewat `command` dalam konfigurasi; direktori kerja asal harus dapat diakses. Gunakan terminal interaktif dan versi yang cocok dengan sistem operasi serta CPU.

Peluncur bawaan: Windows `wt` (cadangan `cmd`), Linux `gnome-terminal`, macOS `osascript` dengan Terminal. Sesuaikan pengaturan bila tidak tersedia. Core memiliki target x64/arm64 pada tiga sistem; paket semantik tidak tersedia untuk semua target, periksa berkas rilis. Fokus tab native memerlukan Windows Terminal dan hanya tersedia di Windows.

Unduh **arsip Core lengkap** dari [Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases), bukan arsip kode sumber. Ekstrak seluruhnya; simpan program, `providers/`, `prompts/`, dan `config.toml.example` yang telah dihasilkan bersama-sama. Salin contoh menjadi `config.toml` di samping program tanpa menimpa pengaturan lama. Periksa `state_dir`, `command`, dan peluncur terminal. `config.toml.template` pada sumber masih memuat penanda yang harus diganti; jangan sekadar mengganti namanya.

Bandingkan digest berkas yang tepat sebelum menjalankan; kecocokan bukan tanda tangan atau jaminan keamanan. Patuhi peringatan OS dan kebijakan organisasi. [SHA-256](../../README.md#verify-a-release-download).

**Peringatan sebelum peluncuran pertama:** pengelompokan bawaan `remote` mengirim judul sesi dan direktori kerja ke layanan yang dihosting, dan dapat berjalan otomatis setelah penemuan awal. Tidak ada kontrak API publik untuk pihak ketiga. Untuk mengelompokkan tanpa jaringan, temukan `# [grouping]` dan `# engine = "remote"` dalam salinan konfigurasi; hapus `#` awal di kedua baris, lalu ubah nilainya menjadi `"wordmatch"`:

```toml
[grouping]
engine = "wordmatch"
```

Biarkan opsi lain tetap dikomentari. Jika sudah ada tabel `[grouping]` aktif, ubah `engine` di situ tanpa membuat tabel kedua. Simpan sebelum menjalankan program atau memindai riwayat. `wordmatch` hanya membandingkan kata, bukan AI lokal, dan tidak mengatur koneksi CLI penyedia. Plugin pencarian semantik opsional dapat mengunduh model saat inisialisasi; mulai dengan Core agar plugin itu tidak dimuat.

<a id="first-workflow"></a>

## Pencarian pertama dan hasil yang diharapkan

Mulai dengan potongan judul atau proyek yang sama persis. Core menawarkan pencarian kata dan perkiraan; parafrasa semantik memerlukan plugin opsional.

Dari terminal di folder hasil ekstraksi, dengan Windows PowerShell:

```powershell
.\agent-session-tui.exe
```

Linux/macOS:

```sh
./agent-session-tui
```

Tersedia enam definisi riwayat dan jalur lanjut yang dikonfigurasi; versi CLI asli yang diuji di sini **tidak diketahui**. Ini bukan sertifikasi kompatibilitas penggunaan nyata. Lihat [cakupan dan bukti](../../README.md#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable | Lanjutkan melalui CLI terpasang dan terautentikasi, dengan direktori asal yang dapat diakses |
| Running / Waiting / Idle | Hanya di Windows, coba fokuskan tab Windows Terminal berjalan yang cocok; lainnya beralih manual |
| Archived / Stopped | Lanjutkan melalui CLI terpasang dan terautentikasi, dengan direktori asal yang dapat diakses; [limits](../../README.md#supported-providers) |



Tunggu daftar muncul, tekan `/`, lalu masukkan nama proyek atau frasa dari riwayat Anda sendiri. Gunakan `↑`/`↓` untuk memeriksa detail, pastikan penyedia, judul, dan direktori sebelum `Enter`. Untuk **Resumable**, hasil yang diharapkan adalah terminal yang membuka CLI dengan percakapan terpilih untuk dilanjutkan.

`Esc` membatalkan pencarian; `q` / `Ctrl+C` keluar dari TUI tanpa menghapus riwayat atau menghentikan CLI lain. Penghentian sesi belum diimplementasikan.

- Daftar kosong: periksa riwayat tersimpan, penyedia aktif, dan `state_dir`.
- Gagal melanjutkan: periksa pemasangan CLI, autentikasi akun, dan direktori asal.
- Gagal mulai: ekstrak seluruh arsip dengan `providers/`, `prompts/`, dan contoh konfigurasi yang dihasilkan; periksa peluncur.

Daftar kosong, kesalahan konfigurasi, peluncuran atau perpindahan tab, pembaruan, penghapusan dan pemulihan: [pemecahan masalah dalam bahasa Inggris](../../README.md#troubleshooting-update-and-removal). [Dokumentasi Inggris](../../README.md) menjadi acuan teknis; lihat [konfigurasi](../../README.md#configuration) dan [data yang dikirim](../../README.md#what-remote-sends).




Jika sudah tahu penyedia dan sesinya, riwayat CLI itu cukup. Katalog ini berguna untuk beberapa CLI atau proyek saat Anda ingat topiknya, bukan ID sesi.

Pengelola sesi terminal untuk percakapan yang masih tersimpan dari Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI, dan Kimi, termasuk yang dimulai secara terpisah. Keenam penyedia ini didukung secara bawaan; cakupan bergantung pada format dan filter penemuan riwayat, bukan dukungan untuk semua CLI.

Ikuti langkah privasi sebelum menjalankan; pengelompokan jarak jauh bawaan dapat berjalan otomatis.





### Windows Terminal

Untuk **Running/Waiting/Idle**, hanya di Windows program mencoba memilih tab Windows Terminal yang cocok; jika berhasil, tab dipilih dan jendela tampil di depan. Di Linux/macOS, pindah ke terminal yang sedang berjalan secara manual. Hasil untuk kueri samar atau salah ketik tidak dijamin.

<details>
<summary>↳ Pencarian pertama privat: Core + wordmatch</summary>

**Contoh fiktif, bukan hasil pengamatan runtime atau sesi bawaan:** tekan `/`, cari `build failure`, dan bayangkan baris Claude Code · judul `Build failure` · proyek `demo-project`. Dengan `↑`/`↓`, periksa penyedia, judul, dan direktori asal. Jika ini percakapan **Resumable** yang benar, `Enter` membukanya melalui Claude CLI terpasang dan terautentikasi; direktori harus dapat diakses. Bukan jaminan hasil atau kompatibilitas.

</details>



Lihat percakapan tersimpan, cari dalam riwayat Anda, lalu pilih percakapan untuk dilanjutkan. GIF menampilkan antarmuka berbahasa Inggris dari versi yang direkam, bukan verifikasi versi saat ini atau kompatibilitas setiap CLI. Terjemahan dokumentasi tidak menerjemahkan antarmuka maupun percakapan tersimpan. Atur privasi seperti dalam petunjuk pemasangan sebelum menjalankan program. Versi dan tanggal rekaman tidak diketahui. [Pasang Core](#install-core).

<img src="../agentTUI.gif" alt="Demo TUI: daftar riwayat, pencarian, dan melanjutkan percakapan" width="700">
