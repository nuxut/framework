## Fikstür — çalışma zamanı ve denetim testleri (gerçek arayüz metni değil)

# Nerede: test. Anlam: selamlama.
# [max: 20]
fx-hello = Merhaba

# Nerede: test. Değişkenler: $name (kişi adı).
fx-greet = Merhaba { $name }

# Nerede: test. Değişkenler: $count (dosya sayısı, tamsayı).
fx-files =
    { $count ->
        [one] { $count } dosya
       *[other] { $count } dosya
    }

# Nerede: test. Anlam: düğme ve ipucu özniteliği.
fx-button = Kaydet
    .tooltip = Değişiklikleri kaydet

# Nerede: test. Anlam: kısmi dilde eksik; zincirde en'e düşer.
fx-partial = Yalnızca Türkçe ve İngilizce

# Nerede: test. Değişkenler: $n (ondalık sayı, NUMBER ile).
fx-number = Toplam { NUMBER($n, minimumFractionDigits: 2) }

-fx-brand = Nuxut

# Nerede: test. Anlam: terim referansı.
fx-about = { -fx-brand } hakkında
