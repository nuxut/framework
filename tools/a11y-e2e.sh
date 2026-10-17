#!/bin/sh
# Yalıtılmış ekran okuyucu ortamı (ADR-0011): komutu headless sway test compositor'ı + ayrı
# oturum D-Bus'ı (dbus-run-session) içinde çalıştırır. a11y bus'ı (at-spi-bus-launcher) ve
# registry (at-spi2-registryd) bu yalıtılmış oturum bus'ında D-Bus etkinleştirmesiyle kalkar;
# kullanıcının oturum bus'ına, a11y bus'ına, dconf'una ve ekranına dokunulmaz.
#
#   tools/a11y-e2e.sh KOMUT [ARG...]
#
# Komut şu ortamla çalışır: WAYLAND_DISPLAY = test compositor'ı, HYPRLAND_INSTANCE_SIGNATURE yok,
# geçici XDG_CONFIG_HOME/XDG_DATA_HOME/XDG_STATE_HOME/XDG_CACHE_HOME, DCONF_PROFILE yok sayılan,
# GSETTINGS_BACKEND=memory, GDK_DEBUG=no-portals, NUXUT_A11Y_E2E=1. Erişilebilirliği açmak
# (org.a11y.Status.IsEnabled) komutun işi (test bunu bir AT gibi yapar; atspi_dump örneği de).
#
# Örnekler:
#   cargo test -p nuxut --features a11y --test a11y_atspi --no-run   # önce derle (gate ile)
#   tools/a11y-e2e.sh target/debug/deps/a11y_atspi-<karma> --nocapture
#   tools/a11y-e2e.sh sh -c '…'  YOK — çok adımlı akış için betik dosyası ver.
#
# Ad: NUXUT_TEST_NAME (vars. a11y-e2e) — başka test oturumlarının compositor'ıyla çakışmaz.
set -eu
here=$(cd "$(dirname "$0")" && pwd)

if [ "${1:-}" = --inner ]; then
    shift
    # dbus-run-session içindeyiz: oturum bus'ı yalıtılmış (DBUS_SESSION_BUS_ADDRESS yeni).
    case ${DBUS_SESSION_BUS_ADDRESS:-} in
        '') echo "a11y-e2e: oturum bus'ı yok" >&2; exit 1 ;;
    esac
    [ "${DBUS_SESSION_BUS_ADDRESS}" != "${NUXUT_A11Y_USER_BUS:-}" ] || { echo "a11y-e2e: kullanıcının bus'ı!" >&2; exit 1; }
    unset AT_SPI_BUS_ADDRESS
    export NUXUT_A11Y_E2E=1
    # a11y bus'ı (dbus-broker, --scope user) registry'yi systemd ile etkinleştirmek ister; yalıtılmış
    # oturumda systemd yok. registryd'yi kendimiz başlatırız (a11y bus'ını oturum bus'ından
    # org.a11y.Bus.GetAddress ile bulur; launcher D-Bus etkinleştirmesiyle kalkar).
    /usr/lib/at-spi2-registryd 2> /dev/null &
    registry=$!
    trap 'set +e; kill $registry 2> /dev/null; wait $registry 2> /dev/null' EXIT
    trap 'exit 130' INT TERM
    "$@"
    exit $?
fi

[ $# -gt 0 ] || { echo "kullanım: $0 KOMUT [ARG...]" >&2; exit 2; }
command -v dbus-run-session > /dev/null || { echo "a11y-e2e: dbus-run-session yok (dbus paketi)" >&2; exit 3; }
[ -x /usr/lib/at-spi-bus-launcher ] || { echo "a11y-e2e: at-spi2-core kurulu değil; atlandı" >&2; exit 3; }

export NUXUT_TEST_NAME=${NUXUT_TEST_NAME:-a11y-e2e}
# Entegrasyon checkout'unun aracı (test_pointer orada derli; worktree'de soğuk release derlemesi olmasın).
TC=${NUXUT_TC:-$here/../../framework/tools/test-compositor.sh}
[ -x "$TC" ] || TC=$here/test-compositor.sh
user_display=${WAYLAND_DISPLAY:-}
NUXUT_A11Y_USER_BUS=${DBUS_SESSION_BUS_ADDRESS:-}
export NUXUT_A11Y_USER_BUS
tmp=$(mktemp -d "${XDG_RUNTIME_DIR:-/tmp}/nuxut-a11y-e2e.XXXXXX")

"$TC" start --owner $$ > /dev/null
trap 'set +e; "$TC" stop > /dev/null; rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM
eval "$("$TC" env)"
if [ -z "${WAYLAND_DISPLAY:-}" ] || [ "$WAYLAND_DISPLAY" = "$user_display" ]; then
    echo "a11y-e2e: WAYLAND_DISPLAY test compositor'ı değil" >&2
    exit 1
fi
unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY
mkdir -p "$tmp/config" "$tmp/data" "$tmp/state" "$tmp/cache"
export XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
export DCONF_PROFILE=$tmp/no-dconf GSETTINGS_BACKEND=memory GDK_DEBUG=no-portals
# Ortam kuruldu; ancak şimdi yalıtılmış bus (kural: dbus-run-session en son).
dbus-run-session -- "$0" --inner "$@"
