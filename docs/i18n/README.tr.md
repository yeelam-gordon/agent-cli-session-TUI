# Agent CLI Session TUI — Oturum geçmişini arayın, konuşmaya devam edin

<a id="languages"></a>
<details>
<summary>Languages / 语言 / 言語 / اللغات (16)</summary>

[English](../../README.md) · [简体中文](README.zh-CN.md) · [繁體中文](README.zh-TW.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Español](README.es.md) · [Português (Brasil)](README.pt-BR.md) · [Français](README.fr.md)

[Deutsch](README.de.md) · [Italiano](README.it.md) · [Русский](README.ru.md) · [Türkçe](README.tr.md) · [Tiếng Việt](README.vi.md) · [Bahasa Indonesia](README.id.md) · [हिन्दी](README.hi.md) · [العربية](README.ar.md)

</details>

CLI geçmişlerindeki konuşmayı tek aramada bulun. Konuşma ve bağlamı kurulu özgün CLI ile sürdürün. Yalnızca Windows’ta, başka pencere açmak yerine eşleşen çalışan Windows Terminal sekmesine odaklanmayı deneyin.

[İş akışını görün](#first-workflow) · [Core kurulumu](#install-core) · [Sağlayıcılar ve sınırlar](../../README.md#supported-providers)

Varsayılan uzak gruplama başlıkları ve çalışma yollarını otomatik gönderebilir; başlatmadan önce Core + wordmatch seçin.

<img src="../agent-session-benefits.png" alt="Kavram: CLI geçmişlerinde eski konuşmayı bulup devam etme" width="780">

Bu bir kavram çizimidir, gerçek uygulama ekran görüntüsü değildir. İngilizce etiketler ve örnek satırlar temsili olup gerçek özel geçmişler değildir.

- Eski konuyu veya projeyi bulun: hatırladığınız ifadeyi arayıp sağlayıcıyı, başlığı ve dizini kontrol edin.
- Doğru konuşmaya devam edin: **Resumable** üzerinde `Enter`; saklanan geçmiş, kurulu ve kimlik doğrulaması tamamlanmış CLI ve erişilebilir özgün dizin gerekir.
- Çalışan Windows sekmesini bulun: yalnızca Windows’ta eşleşen Windows Terminal sekmesine odaklanmayı dener; her sürece bağlanmaz. Linux/macOS’ta elle geçiş yapın.

Konuşmayı bu TUI’de başlatmış olmanız gerekmez.


<a id="install-core"></a>

## Gizlilik odaklı ilk arama: Core + wordmatch

En az bir sağlayıcıda kayıtlı konuşma gerekir. Devam etmek için ilgili CLI kurulu, kimlik doğrulaması yapılmış ve yapılandırılan `command` üzerinden erişilebilir olmalıdır; özgün çalışma dizinine de erişilebilmelidir. İşletim sistemi ve işlemcinize uygun sürümü etkileşimli terminalde kullanın.

Varsayılan başlatıcılar: Windows `wt` (yedek `cmd`), Linux `gnome-terminal`, macOS `osascript` ve Terminal. Yoksa ayarları değiştirin. Core üç sistemde x64/arm64 hedeflerine sahiptir; anlamsal paketler her hedef için sunulmaz, sürüm dosyalarına bakın. Yerel sekme odağı Windows Terminal gerektirir ve yalnızca Windows’ta kullanılabilir.

[Releases](https://github.com/yeelam-gordon/agent-cli-session-TUI/releases) sayfasından **tam Core arşivini** indirin; kaynak kodu arşivini değil. Tamamını çıkarın; çalıştırılabilir dosya, `providers/`, `prompts/` ve üretilmiş `config.toml.example` birlikte kalmalıdır. Örneği programın yanına `config.toml` olarak kopyalayın, mevcut ayarların üzerine yazmayın. `state_dir`, `command` ve terminal başlatıcısını kontrol edin. Kaynaktaki `config.toml.template` yer tutucular içerir; yalnızca adını değiştirmek yeterli değildir.

Başlatmadan önce tam dosyanın özetini karşılaştırın; eşleşme imza veya güvenlik garantisi değildir. İşletim sistemi uyarılarını ve kurum kurallarını izleyin. [SHA-256](../../README.md#verify-a-release-download).

**İlk başlatmadan önce gizlilik uyarısı:** varsayılan `remote` gruplama oturum başlıklarını ve çalışma dizinlerini barındırılan bir hizmete gönderir; ilk keşiften sonra otomatik çalışabilir. Üçüncü taraflar için yayımlanmış bir API sözleşmesi yoktur. Ağ kullanmadan gruplamak için kopyadaki `# [grouping]` ve `# engine = "remote"` satırlarının ikisinden de baştaki `#` işaretini kaldırıp değeri `"wordmatch"` yapın:

```toml
[grouping]
engine = "wordmatch"
```

Diğer seçenekleri yorum satırı olarak bırakın. Etkin bir `[grouping]` tablosu varsa oradaki `engine` değerini değiştirin, ikinci tablo eklemeyin. Programı başlatıp geçmişi taramadan önce kaydedin. `wordmatch` yalnızca sözcükleri karşılaştırır, yerel yapay zekâ değildir; sağlayıcı CLI’ların ağ kullanımını yönetmez. İsteğe bağlı anlamsal arama eklentisi ilk yüklemede model indirebilir. Bu eklentiyi yüklememek için önce Core kullanın.

<a id="first-workflow"></a>

## İlk arama ve beklenen sonuç

Önce başlık veya projeden birebir bir ifade arayın. Core sözcük ve yaklaşık eşleşme sunar; anlamsal ifadeler isteğe bağlı eklenti gerektirir.

Arşivi çıkardığınız klasördeki terminalden Windows PowerShell ile:

```powershell
.\agent-session-tui.exe
```

Linux/macOS:

```sh
./agent-session-tui
```

Altı geçmiş tanımı ve yapılandırılmış devam yolları sunulur; burada test edilmiş yerel CLI sürümleri **bilinmiyor**. Bu, gerçek kullanımda uyumluluk sertifikası değildir. [Kapsam ve kanıtlar](../../README.md#supported-providers).

| UI | `Enter` |
|---|---|
| Resumable (devam edilebilir konuşma) | Kurulu ve doğrulanmış CLI ile devam; özgün dizin erişilebilir olmalı |
| Running / Waiting / Idle | Yalnızca Windows’ta eşleşen çalışan Windows Terminal sekmesine odaklanmayı dene; diğer sistemlerde elle geçiş |
| Archived / Stopped | Kurulu ve doğrulanmış CLI ile devam; özgün dizin erişilebilir olmalı; [sınırlar](../../README.md#supported-providers) |



Liste dolunca `/` tuşuna basıp kendi geçmişinizdeki proje adını veya ifadeyi yazın. `↑`/`↓` ile ayrıntılara bakın; sağlayıcı, başlık ve dizini doğruladıktan sonra `Enter`. **Resumable** için beklenen sonuç, ilgili CLI’ın seçili konuşmayı bir terminalde açarak devam etmenizi sağlamasıdır.

`Esc` aramayı iptal eder; `q` / `Ctrl+C` TUI’dan çıkar, geçmişi silmez ve diğer CLI’ları durdurmaz. Oturum sonlandırma uygulanmamıştır.

- Boş liste: saklanan geçmişi, sağlayıcının etkinliğini ve `state_dir` değerini kontrol edin.
- Devam edilemiyor: CLI kurulumu, hesap doğrulaması ve özgün dizini kontrol edin.
- Başlatılamıyor: `providers/`, `prompts/` ve üretilmiş örnekle tüm arşivi çıkarın; başlatıcıyı kontrol edin.

Boş liste, yapılandırma, başlatma veya sekme geçişi hataları, güncelleme, kaldırma ve geri yükleme için [İngilizce sorun giderme](../../README.md#troubleshooting-update-and-removal). Teknik başvuru [İngilizce belgedir](../../README.md); [yapılandırma](../../README.md#configuration) ve [gönderilen verileri](../../README.md#what-remote-sends) inceleyin.




Sağlayıcıyı ve oturumu biliyorsanız CLI’nin kendi geçmişi yeterlidir. Birden çok CLI veya projede çalışırken, oturum ID’sini değil konuyu hatırlıyorsanız bu katalog yardımcı olur.

Copilot CLI, Claude Code, Codex CLI, Qwen CLI, Gemini CLI ve Kimi’nin saklanan konuşmalarını arayan bir terminal oturumu yöneticisi. Ayrı başlatılan konuşmaları da kapsar. Bu altı sağlayıcı yerleşiktir; bulunan kayıtlar biçimlere ve keşif filtrelerine bağlıdır. Her CLI için destek vaat edilmez.

Başlatmadan önce kurulumdaki gizlilik adımlarını izleyin; varsayılan uzak gruplama otomatik çalışabilir.





### Windows Terminal

**Running/Waiting/Idle** için yalnızca Windows’ta eşleşen Windows Terminal sekmesine geçiş denenir; başarılı olursa sekme seçilir ve pencere öne gelir. Linux/macOS’ta çalışan terminale elle geçin. Belirsiz veya yanlış yazılmış aramaların sonuçları garanti edilmez.

<details>
<summary>↳ Gizlilik odaklı ilk arama: Core + wordmatch</summary>

**Kurgusal örnek; gözlemlenmiş çalışma veya pakete eklenmiş oturum değildir:** `/` ile `build failure` arayın; Claude Code · başlık `Build failure` · proje `demo-project` satırını düşünün. `↑`/`↓` ile sağlayıcıyı, başlığı ve özgün dizini kontrol edin. Doğru **Resumable** konuşmaysa `Enter`, kurulu ve kimlik doğrulaması tamamlanmış Claude CLI ile açar; dizin erişilebilir olmalıdır. Sonuç veya uyumluluk garantisi değildir.

</details>



Kayıtlı konuşmaları listede inceleyin, kendi geçmişinizde arayın ve devam etmek istediğinizi seçin. GIF, kaydedilen sürümün İngilizce arayüzünü gösterir; güncel sürümün veya her CLI’ın uyumluluğunun doğrulaması değildir. Belgelerin çevrilmesi arayüzün veya kayıtlı konuşmaların çevrildiği anlamına gelmez. Başlatmadan önce kurulumdaki gizlilik ayarlarını yapın. Kaydın sürümü ve tarihi bilinmiyor. [Core kurulumu](#install-core).

<img src="../agentTUI.gif" alt="TUI demosu: geçmiş listesi, arama ve konuşmaya devam etme" width="700">
