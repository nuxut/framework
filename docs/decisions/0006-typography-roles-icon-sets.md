# ADR-0006 — Tipografi rolleri, varsayılan fontlar ve semantik ikon setleri

Tarih: 2026-10-05 · Durum: kabul edildi

## Bağlam
Tema tek bir `family` + `mono` + `icon` dizesi taşıyordu. Saat, bildirim gövdesi, takvim
açıklaması ve kod aynı aileden geliyordu; GNOME/KDE'deki gibi rol bazlı seçim yoktu.
İkonlar Material Symbols ligatür adıyla (`Icon::symbol("volume_up")`) isteniyordu: ikon
fontu değişirse her çağrı yeri kırılır. Kullanıcı Nerd Font desteği de istedi.

## Karar

### 1. Tipografi rolleri
`[typography]` beş rol taşır; her rol **aile + ağırlık + boyut ölçeği**:

| Rol        | Kullanım                                               | Varsayılan aile             |
|------------|--------------------------------------------------------|-----------------------------|
| `ui`       | arayüz metni (etiket, düğme, liste, başlık)            | Inter                       |
| `display`  | büyük metin (saat, sayfa başlığı)                      | Inter (`opsz` → Display kesimi) |
| `document` | uzun okuma (bildirim gövdesi, takvim açıklaması, içerik)| Source Serif 4              |
| `mono`     | kod, terminal benzeri, sayısal tablo                   | JetBrainsMono Nerd Font     |
| `icon`     | ikon fontu (boşsa ikon setinin fontu)                  | Material Symbols Rounded    |

- Boyutlar tek tabandan (`typography.size`) tip ölçeğiyle türetilir; rolün `scale`'i o rolün
  tüm boylarını çarpar.
- `opsz` ekseni zaten boyuta bağlıdır (renderer): Inter büyük boyda kendiliğinden Display kesimine
  geçer, ayrı aile gerekmez.
- Sayılar için **tabular rakam** (`tnum`): `TextStyle::with_tabular_numbers()`. Saat, yüzde,
  sayaç gibi değişen sayılar titremesin diye bileşenler bunu açar.
- Bileşenler fontu **yalnızca rol üzerinden** alır (`text_style(theme, TextRole)`); kodda aile adı
  yok (bekçi testi: `crates/nuxut/tests/it/no_hardcoded_fonts.rs`).
- Eski şema uyarıyla eşlenir: `family` → `ui.family`, `weight` → `ui.weight`, düz dize `mono` /
  `icon` → `mono.family` / `icon.family`.

### 2. Yedek zinciri
Rol ailesi kurulu değilse: rol ailesi → genel aile (`ui`/`display` → `sans-serif`,
`document` → `serif`, `mono` → `monospace`; genel aile fontconfig tercihine çözülür).
Panik yok, kullanıcıya uyarı yok; yalnızca `debug` log (aile adı kullanıcı içeriği değildir,
config'teki varsayılan/seçimdir). İkon fontu yoksa: `material` seçiliyken Material Symbols yoksa
ve kurulu bir Nerd Font varsa `nerd` setine düşülür; hiçbiri yoksa ikon çizilmez (tofu/ligatür
metni görünmez), yerleşim boyutu korunur.

### 3. Semantik ikon adları ve ikon setleri
İkon **semantik adla** istenir: `Icon::named(IconName::VolumeHigh)` (config/IPC için
`IconName::from_str("volume_high")`). Ad, seçili **ikon seti** tarafından glife çevrilir:

- `material` (varsayılan): Material Symbols Rounded ligatürü. Değişken eksenler: `FILL` (dolgu
  animasyonu), `wght`, `opsz`. Arayüz için daha estetik ve tutarlı.
- `nerd`: Nerd Font PUA kod noktası, **Material Design Icons bloğu** (`nf-md-*`) tercih edilir ki
  stil `material`'e yakın ve kendi içinde tutarlı kalsın. `FILL` ekseni yok: her ad bir
  (çizgi, dolu) glif çiftidir; dolgu geçişi iki glif arasında `effects` opaklık geçişidir.
  Font: `icon.family` boşsa `mono.family` (Nerd Font). Geniş kapsam, terminal ekosistemiyle uyum.

`[typography] icon_set = "material" | "nerd"`. Eşleme tablosu tek dosyadadır
(`nuxut-ui/src/icon_names.rs`); `IconName` kapalı bir enum olduğundan her adın iki sette de
karşılığı derleme zamanında zorunludur (kapsamlı `match`), ayrıca test her girişin fontta
gerçekten bulunduğunu (kuruluysa) doğrular. Yeni ad eklemek = enum + iki set girişi.
Ham ligatür/kod noktası API'si kalkar; `Icon::symbol(&str)` geçiş süresince semantik ada
çözülen (eski Material ligatür adları da), bilinmeyeni boş çizen bir geçiş kısa yolu olur;
tüketiciler `Icon::named`'e geçince kaldırılır.

## Gerekçe
- Rol ayrımı okunabilirliği artırır (serif uzun metin, tabular saat) ve Settings'te GNOME/KDE
  benzeri "Arayüz / Belge / Eş aralıklı" seçimini mümkün kılar.
- Inter 4 değişken + `opsz` + `tnum`: tek aileyle hem küçük arayüz hem büyük saat.
- Semantik adlar ikon fontunu çağrı yerlerinden ayırır; set değişimi tek ayardır.
- Material Symbols'un değişken eksenleri (dolgu animasyonu) arayüz estetiğinde üstün; Nerd Font
  ise kullanıcı tercihi ve terminal uyumu için tam destekli alternatif.

## Sonuçlar
- `Typography` yapısı değişir (sözleşme: `docs/ARCHITECTURE.md` "Tema" ve "Tipografi");
  shell ve Settings uyarlanır. Eski config'ler çalışmaya devam eder (uyarıyla).
- Önerilen paketler (`docs/DEPENDENCIES.md`): `inter-font`, `adobe-source-serif-fonts`,
  `ttf-jetbrains-mono-nerd`, `ttf-material-symbols-variable-git` (AUR). Hiçbiri zorunlu değil.
- Cross-platform (ADR-0003): aile adları platformdan bağımsızdır; yedek zinciri genel aileye
  dayanır.
