#!/bin/sh
# ADR-0003 kontrolü: nuxut-wayland dışındaki framework crate'leri Windows ve macOS hedeflerinde
# derlenebilir kalmalı. Bağlama yapılmaz (`cargo check`), yalnızca derleme.
#
# Varsayılan özelliklerle denetlenir. C kütüphanesi isteyen opsiyonel özellikler (nuxut-image
# `avif` → dav1d) çapraz derlenmez; DEPENDENCIES.md "platform" sütununda listelidir.
#
# Kullanım: tools/cross-check.sh [crate ...]    (varsayılan: tüm cross-platform crate'ler)
# Gerekli hedefler: rustup target add x86_64-pc-windows-msvc aarch64-apple-darwin
# Çıkış kodu: herhangi bir crate/hedef başarısızsa 1.
set -u

cd "$(dirname "$0")/.." || exit 1
mkdir -p target/cross-check

TARGETS="x86_64-pc-windows-msvc aarch64-apple-darwin"
# Linux'a özgü (ADR-0003): yalnızca nuxut-wayland. nuxut-video ve nuxut-portal Linux dışında
# Unavailable döner (portal: zbus yalnız Linux'ta).
CRATES="${*:-nuxut-core nuxut-locale nuxut-i18n nuxut-motion nuxut-config nuxut-theme nuxut-render nuxut-access nuxut-ui nuxut-image nuxut-image-sandbox nuxut-image-worker nuxut-palette nuxut-net nuxut-video nuxut-apply nuxut-portal nuxut}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}"

fail=0
for target in $TARGETS; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "UYARI: $target hedefi kurulu değil (rustup target add $target); atlandı"
        continue
    fi
    for crate in $CRATES; do
        log="target/cross-check/$target-$crate.log"
        mkdir -p target/cross-check
        # nuxut-net: `tls` (rustls → ring) C kodu derler; Linux'ta Windows/macOS C araç zinciri
        # yok. Kendi kodumuz TLS'siz denetlenir (ring/rustls iki platformu da resmen destekler).
        flags=""
        [ "$crate" = nuxut-net ] && flags="--no-default-features"
        # Cephe `portal` özelliğiyle: nuxut::portal Linux dışında da var (Unavailable yedeği).
        [ "$crate" = nuxut ] && flags="--features portal"
        # shellcheck disable=SC2086
        if cargo check -q -p "$crate" $flags --target "$target" >"$log" 2>&1; then
            echo "TAMAM   $target  $crate"
        else
            echo "HATA    $target  $crate  ($log)"
            fail=1
        fi
    done
done

# Özellik birleşmesi bekçisi (yerel hedef): shell nuxut-palette'i `default-features = false` ile
# çeker; görsel özelliği (nuxut-image + JXL) olmadan da derlenmeli ve ağaca JXL girmemeli.
if cargo check -q -p nuxut-palette --no-default-features --all-targets >target/cross-check/palette-nodefault.log 2>&1; then
    echo "TAMAM   yerel  nuxut-palette --no-default-features"
else
    echo "HATA    yerel  nuxut-palette --no-default-features  (target/cross-check/palette-nodefault.log)"
    fail=1
fi
# JPEG XL yalnızca yalıtılmış işçide (ADR-0007 takip, ARCHITECTURE "Duvar kağıdı yalıtımı"): kullanıcı
# görseli çözen tüketici ağaçlarında (palet her özellik kümesinde, nuxut-image ve nuxut-video
# varsayılanı, ana süreç tarafı sandbox) jxl-oxide olmamalı; işçi ağacında olmalı.
for spec in "nuxut-palette --no-default-features" "nuxut-palette --no-default-features --features image" "nuxut-palette" \
    "nuxut-image" "nuxut-video" "nuxut-image-sandbox"; do
    # shellcheck disable=SC2086
    if cargo tree -q -p $spec -e normal | grep -qi jxl; then
        echo "HATA    yerel  $spec ağacında jxl var"
        fail=1
    else
        echo "TAMAM   yerel  $spec ağacında jxl yok"
    fi
done
if cargo tree -q -p nuxut-image-worker -e normal | grep -qi jxl-oxide; then
    echo "TAMAM   yerel  nuxut-image-worker ağacında jxl-oxide var"
else
    echo "HATA    yerel  nuxut-image-worker ağacında jxl-oxide yok (işçi JPEG XL çözmeli)"
    fail=1
fi
exit $fail
