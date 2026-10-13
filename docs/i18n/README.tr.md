# Agent CLI Session TUI — Oturum geçmişini arayın, konuşmaya devam edin

<a id="languages"></a>
<details>
<summary>Languages (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

Yaptığınız işi hatırlıyor ama konuşmanın hangi CLI’da olduğunu bilmiyor musunuz? Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI ve Kimi geçmişlerini arayın. Sağlayıcı ve oturum belliyse CLI’ın kendi geçmişi yeterli olabilir.

<img src="../agent-session-benefits.png" alt="Kavram: sözcük araması token expires ifadesini yalnızca başlıkta değil, korunan konuşma bağlamında bulur ve kayıtlı konuşmayı özgün CLI ile sürdürür. Copilot CLI, Claude Code, Codex ve diğerleri desteklenir." width="700">

Kavram çizimi, gerçek uygulama ekran görüntüsü değildir. İngilizce etiketler ve kurgusal satırlar kullanılır.

- **Önceki işi bulun:** bu TUI dışında başlatılan konuşmalar dahil, saklanan CLI geçmişlerinde tek arama.
- **Özgün konuşmayı sürdürün:** kaydedilmiş bağlamı kurulu özgün CLI ile yeniden açın.
- **Çalışan Windows sekmesini bulun:** eşleşen Windows Terminal sekmesine geçmeyi deneyin; Linux/macOS’ta elle geçiş yapın.

<a id="install-core"></a>

## Gizlilik odaklı ilk arama: Core + wordmatch

Saklanan geçmiş, kurulu ve kimlik doğrulaması yapılmış, yapılandırılmış `command` ile çalıştırılabilen CLI ve erişilebilir özgün çalışma dizini gerekir. OS/CPU’nuzla eşleşen sürümü etkileşimli terminalde kullanın. Varsayılan başlatıcılar: Windows `wt` (yedek `cmd`), Linux `gnome-terminal`, macOS `osascript` ve Terminal. Yoksa [yapılandırmayı](../usage.md#configuration) değiştirin.

**Başlatmadan önce:** yeni gizlilik varsayılanlarını içeren derlemelerde otomatik gruplama varsayılan olarak kapalıdır; elle gruplama `wordmatch` kullanır. Eski sürümler hâlâ otomatik uzak gruplama yapabilir. Açıkça seçilen `remote`, başlıkları ve çalışma dizinlerini gönderir; ACP, CLI üzerinden özetler gönderebilir. [PRIVACY.md](../../PRIVACY.md) kayıt yollarını ve konuşma içeriği dizinlerini açıklar. Anlamsal eklenti model indirebilir; Core CLI bağlantılarını yönetmez. Eski örnek: `# engine = "remote"`; yeni: `# engine = "wordmatch"`.

1. [OS/CPU için Core dosyaları](../usage.md#core-archive-filenames) üzerinden OS/CPU’nuzla eşleşen **tam Core arşivini** indirin; kaynak kodu arşivini değil. Tamamını çıkarın; program, `providers/`, `prompts/` ve üretilmiş `config.toml.example` birlikte kalmalıdır. [Tam dosyanın özetini doğrulayın](../usage.md#verify-a-release-download); eşleşme imza veya güvenlik garantisi değildir. OS uyarılarına ve kurum politikasına uyun.
2. Yeni kurulumda üretilmiş örneği programın yanında `config.toml` olarak kopyalayın. **Mevcut ayarları ezmeyin; kaynak `config.toml.template` dosyasını yalnızca yeniden adlandırmayın.** Kopyadaki `# [grouping]` ve `# engine = "remote"` satırlarının **ikisinden de** baştaki `#` işaretini kaldırıp değeri değiştirin:

```toml
[grouping]
engine = "wordmatch"
auto_suggest = false
```

Diğer seçenekleri yorum satırı bırakın. Etkin `[grouping]` tablosu varsa oradaki `engine` değerini değiştirin, ikinci tablo eklemeyin. `state_dir` ve `command` değerlerini kontrol edip **başlatmadan önce kaydedin**. `wordmatch` ortak sözcükleri karşılaştırır; yerel yapay zekâ değildir.

3. Arşivi çıkardığınız klasördeki terminalden çalıştırın:

```powershell
# Windows PowerShell
.\agent-session-tui.exe
```

```sh
# Linux/macOS
./agent-session-tui
```

<a id="first-workflow"></a>

## İlk arama ve beklenen sonuç

4. Listeyi bekleyin; `/` ile kendi geçmişinizden birebir başlık veya proje parçası yazın. `↑`/`↓` ile sağlayıcı, başlık ve dizini doğrulayın; **Resumable** durumundaki konuşmayı seçip `Enter` tuşuna basın. **Beklenen sonuç:** özgün CLI, seçilen konuşmayı sürdürmeniz için açar. Arama bazı sonuçları kaçırabilir; test edilmiş yerel CLI sürümleri bilinmiyor. [Kapsam ve kanıtlar](../usage.md#supported-providers).

<a id="windows-terminal"></a>

**Running/Waiting/Idle** için yalnızca Windows eşleşen sekmeye geçmeyi dener; her sürece bağlanmaz. Aynı çalışan oturumda kısa sürede `Enter` tuşuna tekrar basılması, sekmeye geçmek yerine **konuşmayı zorla yeniden açabilir**. Önce seçimi doğrulayın. [Windows akışı](../usage.md#secondary-workflow-find-a-running-windows-tab).

**Sonuç yok mu?** Saklanan geçmişi, sağlayıcının etkinliğini ve `state_dir` değerini kontrol edin; başlatma hatasında kimlik doğrulaması, özgün dizin ve başlatıcıyı kontrol edin. [Sorun giderme](../usage.md#troubleshooting-update-and-removal).

**Çıkış:** `Esc` aramayı iptal eder; ardından `q` ile çıkın ya da listede veya aramada `Ctrl+C` kullanın; geçmişi silmez veya diğer CLI’ları durdurmaz. Oturum sonlandırma uygulanmamıştır. [Güncelleme, geri yükleme ve kaldırma](../usage.md#troubleshooting-update-and-removal).

[İngilizce kullanım ve kanıt rehberi](../usage.md) tüm sayıları, olumsuz sonuçları ve sınırları korur. [Kaydın](../usage.md#recorded-demo) sürümü ve tarihi bilinmiyor; güncel uyumluluk kanıtı değildir.
