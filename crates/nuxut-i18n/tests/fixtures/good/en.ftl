## Fixture — runtime and check tests (not real UI text)

# Nerede: test. Anlam: selamlama.
# [max: 20]
fx-hello = Hello

# Nerede: test. Değişkenler: $name (kişi adı).
fx-greet = Hello { $name }

# Nerede: test. Değişkenler: $count (dosya sayısı, tamsayı).
fx-files =
    { $count ->
        [one] { $count } file
       *[other] { $count } files
    }

# Nerede: test. Anlam: düğme ve ipucu özniteliği.
fx-button = Save
    .tooltip = Save changes

# Nerede: test. Anlam: kısmi dilde eksik; zincirde en'e düşer.
fx-partial = Only Turkish and English

# Nerede: test. Değişkenler: $n (ondalık sayı, NUMBER ile).
fx-number = Total { NUMBER($n, minimumFractionDigits: 2) }

-fx-brand = Nuxut

# Nerede: test. Anlam: terim referansı.
fx-about = About { -fx-brand }
