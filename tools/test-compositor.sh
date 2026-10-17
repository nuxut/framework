#!/bin/sh
# Yalıtılmış test compositor'ı: kullanıcının oturumundan tamamen ayrı, penceresiz ve girdisiz
# headless sway. Kullanıcının ekranına, girdi aygıtlarına, monitör düzenine, X soketlerine ve
# systemd/dbus ortamına dokunmaz (kendi config'i; /etc/sway/config.d dahil edilmez, xwayland
# kapalı). Kullanıcının Hyprland örneğine hiçbir komut gitmez: HYPRLAND_INSTANCE_SIGNATURE
# bu betikte kaldırılır.
#
#   tools/test-compositor.sh prepare          # kilit DIŞINDA: sanal işaretçiyi derler (güncelse iş yok)
#   tools/test-compositor.sh start [--owner PID]  # başlatır (derlemez; ikili yoksa/eskiyse hata);
#                                             # dışa aktarılacak ortamı yazdırır.
#                                             # --owner: PID ölünce compositor kendiliğinden durur
#   tools/test-compositor.sh env              # eval "$(tools/test-compositor.sh env)"
#   tools/test-compositor.sh stop             # yalnızca kendi süreçlerini kapatır ve doğrular;
#                                             # kapatamazsa çıkış kodu 1
#   tools/test-compositor.sh status           # çalışmıyorsa çıkış kodu 1; env de 1 ve kapalı-güvenli ortam
#   tools/test-compositor.sh shot DOSYA [ÇIKIŞ]        # grim (varsayılan HEADLESS-1)
#   tools/test-compositor.sh output add WxH@ÖLÇEK     # ör. 2560x1440@1.5; yeni adı yazdırır
#   tools/test-compositor.sh output remove AD
#   tools/test-compositor.sh output scale AD ÖLÇEK
#   tools/test-compositor.sh pointer move X Y          # düzen koordinatı (mantıksal)
#   tools/test-compositor.sh pointer click|press|release [left|right|middle]
#   tools/test-compositor.sh pointer scroll DY
#   tools/test-compositor.sh pointer wheel N           # tekerlek çentiği (pozitif aşağı)
#   tools/test-compositor.sh key TUŞ                   # wtype -k (ör. Escape, Return)
#   tools/test-compositor.sh type METİN                # wtype
#   tools/test-compositor.sh keylock caps|num on|off   # kalıcı sanal klavyenin kilit durumu
#   tools/test-compositor.sh vkey KOMBİNASYON          # kalıcı sanal klavyeden: ctrl+v, Return…
#
# Birden çok bağımsız örnek: NUXUT_TEST_NAME=<ad> (varsayılan nuxut-test). start ve stop AYNI
# adla çağrılmalı (ad yoksa stop "çalışmıyor" der).
#
# Test betiği kalıbı (set -eu altında). Trap'in başında `set +e` ŞART: bash, set -e'yi trap
# gövdesinde de uygular; `kill`/`wait` sıfır dışı dönünce (wait, TERM'lenen çocuk için 143)
# trap orada biter ve `stop` hiç çalışmaz — sway arkada kalır (olay 2026-10-10). --owner $$
# ayrıca güvence: betik trap'siz/KILL ile ölse de compositor kapanır.
#   TC=<framework>/tools/test-compositor.sh
#   "$TC" prepare        # kilit (flock .live-test.lock) DIŞINDA, betikten önce; start derlemez
#   "$TC" start --owner $$ > /dev/null
#   trap 'set +e; kill $pid 2>/dev/null; wait $pid 2>/dev/null; "$TC" stop' EXIT
#   trap 'exit 130' INT TERM
#   eval "$("$TC" env)"
# Sınır: Hyprland'e özgü yetenekler (hyprctl/Hyprland IPC, Hyprland pencere kuralları) burada
# yoktur; shell'in Hyprland servisi bu compositor'da bağlanamaz, onları kullanan testler sahte
# (fake) bir IPC kaynağıyla ya da kullanıcının ekranında kısa, kırpılmış, odak almayan testle
# yapılır.
set -eu
unset HYPRLAND_INSTANCE_SIGNATURE
# Çağıranın (kullanıcının) display'i: sanal girdi aracı buna asla göndermez.
user_display=${WAYLAND_DISPLAY:-}

name=${NUXUT_TEST_NAME:-nuxut-test}
case $name in
    *[!a-z0-9_-]* | '') echo "geçersiz ad: $name" >&2; exit 2 ;;
esac
: "${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR gerekli}"
state=$XDG_RUNTIME_DIR/nuxut-test/$name
root=$(cd "$(dirname "$0")/.." && pwd)
# Derleme komutu: NUXUT_CARGO verilmişse o; yoksa repo yanındaki bir derleme kapısı
# (../tools/cargo-gate ya da worktree'de ../../tools/cargo-gate) varsa onunla, değilse cargo.
cargo_cmd() {
    if [ -n "${NUXUT_CARGO:-}" ]; then echo "$NUXUT_CARGO"; return; fi
    for g in "$root/../tools/cargo-gate" "$root/../../tools/cargo-gate"; do
        [ -x "$g" ] && { echo "$g cargo"; return; }
    done
    echo cargo
}

# Süreç kimliği: pid + /proc başlangıç zamanı (22. alan). pid yeniden kullanılsa da yanlış
# sürece dokunulmaz; fork anında belirlenir, exec'te (alt kabuk → env → sway) değişmez, bu
# yüzden `$!` ile hemen kaydedilebilir. Zombi (Z) ölü sayılır (kill -0 zombide başarılıdır).
proc_start() {
    [ -r "/proc/$1/stat" ] || return 1
    sed 's/^.*) //' "/proc/$1/stat" 2> /dev/null | cut -d' ' -f20
}

proc_running() {
    st=$(sed 's/^.*) //' "/proc/$1/stat" 2> /dev/null | cut -d' ' -f1) || return 1
    [ -n "$st" ] && [ "$st" != Z ] && [ "$st" != X ]
}

# record AD PID: $state/AD.pid + $state/AD.start
record() {
    echo "$2" > "$state/$1.pid"
    proc_start "$2" > "$state/$1.start" || : > "$state/$1.start"
}

# ours AD: kayıtlı süreç hâlâ aynı süreç ve çalışıyor mu; öyleyse pid'i yazdırır.
ours() {
    [ -s "$state/$1.pid" ] || return 1
    p=$(cat "$state/$1.pid")
    s=$(cat "$state/$1.start" 2> /dev/null) || return 1
    [ -n "$s" ] && [ "$(proc_start "$p")" = "$s" ] && proc_running "$p" && echo "$p"
}

alive() {
    ours sway > /dev/null
}

# State dizini kaybolmuş/bozulmuş olsa da bu adın süreçlerini bul: komut satırı tam olarak
# "sway -c $state/config" olan sway ya da "… serve $state/pointer.fifo" ile biten sanal
# işaretçi; yalnızca bizim kullanıcımıza ait. Başka adların (başka test oturumlarının) yolları farklıdır;
# onlara dokunulmaz.
strays() {
    sway_cmd="sway -c $state/config "
    pointer_tail=" serve $state/pointer.fifo "
    for d in /proc/[0-9]*; do
        [ -O "$d" ] || continue
        c=$(tr '\0' ' ' < "$d/cmdline" 2> /dev/null) || continue
        case $c in
            "$sway_cmd" | *"$pointer_tail") proc_running "${d#/proc/}" && echo "${d#/proc/}" ;;
        esac
    done
    return 0
}

# kill_wait PID: TERM, 5 sn bekle, gerekirse KILL, 2 sn daha bekle. Ölmezse 1.
kill_wait() {
    kill -TERM "$1" 2> /dev/null || true
    n=0
    while proc_running "$1" && [ $n -lt 50 ]; do sleep 0.1; n=$((n + 1)); done
    if proc_running "$1"; then
        echo "pid $1 TERM'e yanıt vermedi; KILL" >&2
        kill -KILL "$1" 2> /dev/null || true
        n=0
        while proc_running "$1" && [ $n -lt 20 ]; do sleep 0.1; n=$((n + 1)); done
    fi
    ! proc_running "$1"
}

need() {
    alive || { echo "test compositor'ı çalışmıyor; önce: $0 start" >&2; exit 1; }
    WAYLAND_DISPLAY=$(cat "$state/wayland_display")
    SWAYSOCK=$(cat "$state/swaysock")
    export WAYLAND_DISPLAY SWAYSOCK
    unset DISPLAY
}

# Çalışmıyorken `eval "$(… env)"` boş kalıp kullanıcının display'ine düşmesin (olay 2026-10-04):
# stdout'a var olmayan bir display yazılır ve `false` ile eval başarısız olur (set -e durdurur;
# durmasa bile uygulama hiçbir compositor'a bağlanamaz).
fail_closed_env() {
    echo "export WAYLAND_DISPLAY=nuxut-test-calismiyor"
    echo "unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY SWAYSOCK"
    echo "false"
}

print_env() {
    echo "export WAYLAND_DISPLAY=$(cat "$state/wayland_display")"
    echo "export SWAYSOCK=$(cat "$state/swaysock")"
    echo "export NUXUT_TEST_STATE=$state"
    echo "unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY"
}

start() {
    owner=
    if [ "${1:-}" = --owner ]; then
        owner=${2:?--owner PID gerekli}
        case $owner in *[!0-9]* | '') echo "geçersiz --owner: $owner" >&2; exit 2 ;; esac
        proc_running "$owner" || { echo "--owner $owner çalışmıyor" >&2; exit 2; }
    fi
    if alive; then
        echo "zaten çalışıyor (pid $(ours sway))" >&2
        [ -n "$owner" ] && ! ours watch > /dev/null && start_watch "$owner"
        print_env
        return 0
    fi
    command -v sway > /dev/null || { echo "sway kurulu değil" >&2; exit 1; }
    # Kilit içinde derleme yok: ikili hazır değilse sway açılmadan hata (bkz. prepare).
    pointer_bin > /dev/null || exit 1
    # Önceki örnekten kalan (kaydı kaybolmuş ya da yarım) süreçler: aynı config yoluyla ikinci
    # bir sway açılmasın.
    cleanup || { echo "önceki örnek kapatılamadı; başlatılmadı" >&2; exit 1; }
    rm -rf "$state"
    mkdir -p "$state"
    chmod 700 "$state"
    cat > "$state/config" << EOF
# nuxut test compositor'ı (tools/test-compositor.sh). Kullanıcı config'i dahil EDİLMEZ.
output HEADLESS-1 mode --custom 1920x1080@60Hz position 0 0 scale 1
xwayland disable
default_border none
default_floating_border none
focus_follows_mouse no
exec printenv WAYLAND_DISPLAY > $state/wayland_display
exec printenv SWAYSOCK > $state/swaysock
EOF
    # WAYLAND_DISPLAY/DISPLAY kaldırılır: aksi halde sway kullanıcının oturumunda iç içe pencere
    # açar. Yalnızca headless backend, girdi aygıtı yok. Renderer: bu VM'de (llvmpipe) gles2
    # gbm tampon ayıramıyor (çıkış değişiklikleri commit edilemiyor); pixman kararlı.
    # `exec` zinciri (alt kabuk → env → sway) aynı pid'de kalır: $! sway'in kendisidir.
    (exec env -u WAYLAND_DISPLAY -u DISPLAY -u SWAYSOCK \
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER="${WLR_RENDERER:-pixman}" \
        sway -c "$state/config" > "$state/sway.log" 2>&1 < /dev/null) &
    record sway $!
    # Geriye uyum: eski araçlar $state/pid okuyabilir.
    cp "$state/sway.pid" "$state/pid"
    n=0
    while [ ! -s "$state/wayland_display" ] || [ ! -s "$state/swaysock" ]; do
        if ! alive; then
            echo "sway başlamadı; günlük: $state/sway.log" >&2
            tail -5 "$state/sway.log" >&2
            cleanup > /dev/null || true
            exit 1
        fi
        n=$((n + 1))
        [ $n -gt 100 ] && { echo "sway 10 sn içinde hazır olmadı" >&2; stop > /dev/null || true; exit 1; }
        sleep 0.1
    done
    # Sanal işaretçi + klavye: tek, kalıcı aygıtlar (her komutta yeni aygıt istemcilerde
    # enter'ı ve wl_keyboard'u kaçırır). Klavye keymap'i: NUXUT_TEST_LAYOUT (varsayılan tr).
    bin=$(pointer_bin) || { stop > /dev/null || true; exit 1; }
    mkfifo -m 600 "$state/pointer.fifo"
    xkbcli compile-keymap --layout "${NUXUT_TEST_LAYOUT:-tr}" > "$state/keymap.xkb" 2> /dev/null || true
    (WAYLAND_DISPLAY=$(cat "$state/wayland_display") NUXUT_TEST_KEYMAP=$state/keymap.xkb \
        NUXUT_USER_WAYLAND_DISPLAY=$user_display \
        exec "$bin" serve "$state/pointer.fifo" > "$state/pointer.log" 2>&1 < /dev/null) &
    record pointer $!
    [ -n "$owner" ] && start_watch "$owner"
    print_env
}

# Sahip izleyici: ayrı süreç (bu betiğin `__watch` kipi); sahip ölünce `stop` çağırır. Böylece
# test betiğinin trap'i çalışmasa da (set -e, KILL, zaman aşımı) compositor arkada kalmaz.
start_watch() {
    (NUXUT_TEST_NAME=$name exec sh "$0" __watch "$1" "$(proc_start "$1")" \
        > "$state/watch.log" 2>&1 < /dev/null) &
    record watch $!
}

watch_owner() {
    o=$1
    os=$2
    while [ "$(proc_start "$o" 2> /dev/null || true)" = "$os" ] && proc_running "$o" && alive; do
        sleep 0.5
    done
    if alive; then
        echo "sahip süreç $o bitti; compositor durduruluyor"
        stop
    fi
}

# Sanal işaretçi ikilisi. `start` DERLEMEZ: canlı test kilidi (.live-test.lock) içinde çağrılır
# ve derleme kuyruğu doluyken kilit dakikalarca tutuluyordu (2026-10-16: 70+ dk, herkes bekledi;
# karar). Derleme `prepare` alt komutundadır; kilitten ÖNCE çağrılır.
pointer_paths() {
    bin=$root/target/release/examples/test_pointer
    src=$root/crates/nuxut-wayland/examples/test_pointer.rs
}

# İkili var ve kaynağından yeni mi.
pointer_fresh() {
    pointer_paths
    [ -x "$bin" ] && ! [ "$src" -nt "$bin" ]
}

pointer_bin() {
    if ! pointer_fresh; then
        echo "sanal işaretçi yok ya da kaynağından eski; kilit DIŞINDA önce: $0 prepare" >&2
        return 1
    fi
    echo "$bin"
}

# Kilit dışında: sanal işaretçiyi (gerekirse) derler. Güncelse iş yok.
prepare() {
    pointer_fresh && return 0
    (cd "$root" && $(cargo_cmd) build --release -q -p nuxut-wayland --example test_pointer) >&2
    # Kaynak yalnız dokunulmuşsa (checkout) cargo yeniden bağlamaz; ikili güncel sayılsın.
    [ -x "$bin" ] && touch "$bin"
    pointer_fresh || { echo "sanal işaretçi derlenemedi" >&2; exit 1; }
}

# Bu adın tüm süreçlerini kapatır: izleyici (kendisi değilse), sanal işaretçi, kayıtlı sway ve
# kaydı kaybolmuş süreçler. Hepsi kapandıysa 0; kapatılamayan varsa adını yazar, 1 döner.
# Kapatılan süreçleri $stopped'a yazar.
cleanup() {
    failed=
    stopped=
    if p=$(ours watch) && [ "$p" != "$$" ]; then
        kill_wait "$p" || failed="$failed izleyici:$p"
    fi
    for what in pointer sway; do
        if p=$(ours $what); then
            if kill_wait "$p"; then stopped="$stopped $what:$p"; else failed="$failed $what:$p"; fi
        fi
    done
    # Kayıtlılar kapandı; hâlâ eşleşen varsa kaydı kaybolmuştur.
    for p in $(strays); do
        if kill_wait "$p"; then stopped="$stopped kayıtsız:$p"; else failed="$failed kayıtsız:$p"; fi
    done
    if [ -n "$failed" ]; then
        echo "HATA: kapatılamadı:$failed" >&2
        return 1
    fi
    return 0
}

stop() {
    had_state=false
    [ -d "$state" ] && had_state=true
    if ! cleanup; then
        echo "HATA: test compositor'ı ($name) durdurulamadı; durum korunuyor: $state" >&2
        exit 1
    fi
    rm -rf "$state"
    if [ -n "$stopped" ]; then
        echo "durdu ($name:$stopped)"
    elif $had_state; then
        echo "zaten durmuştu ($name); durum temizlendi"
    else
        others=$(ls "$XDG_RUNTIME_DIR/nuxut-test" 2> /dev/null | tr '\n' ' ')
        echo "çalışmıyor ($name)${others:+; var olan adlar: $others}" >&2
    fi
}

layout_size() {
    swaymsg -t get_outputs | jq -r '[.[] | select(.active) | .rect] |
        "\(map(.x + .width) | max) \(map(.y + .height) | max)"'
}

pointer_tool() {
    ours pointer > /dev/null || { echo "sanal işaretçi çalışmıyor" >&2; exit 1; }
    echo "$*" > "$state/pointer.fifo"
}

output_cmd() {
    case ${1:-} in
        add)
            spec=${2:?WxH@ölçek gerekli}
            mode=${spec%@*}
            scale=1
            case $spec in *@*) scale=${spec#*@} ;; esac
            before=$(swaymsg -t get_outputs | jq -r '.[].name' | sort)
            swaymsg create_output > /dev/null
            new=$(swaymsg -t get_outputs | jq -r '.[].name' | sort | grep -vxF "$before" | head -1)
            [ -n "$new" ] || { echo "yeni çıkış bulunamadı" >&2; exit 1; }
            swaymsg -- output "$new" mode --custom "${mode}@60Hz" scale "$scale" > /dev/null
            echo "$new"
            ;;
        remove) swaymsg output "${2:?ad gerekli}" unplug > /dev/null ;;
        scale) swaymsg output "${2:?ad gerekli}" scale "${3:?ölçek gerekli}" > /dev/null ;;
        *) echo "output add WxH@ÖLÇEK | remove AD | scale AD ÖLÇEK" >&2; exit 2 ;;
    esac
}

case ${1:-} in
    start) shift; start "$@" ;;
    env) alive || { echo "çalışmıyor; önce: $0 start" >&2; fail_closed_env; exit 1; }; print_env ;;
    stop) stop ;;
    prepare) prepare ;;
    __watch) watch_owner "${2:?}" "${3:-}" ;;
    status) if alive; then echo "çalışıyor (pid $(ours sway))"; print_env; else echo "çalışmıyor"; exit 1; fi ;;
    shot) need; grim -o "${3:-HEADLESS-1}" "${2:?dosya gerekli}" ;;
    output) need; shift; output_cmd "$@" ;;
    pointer)
        need
        case ${2:-} in
            move) set -- $(layout_size) "${3:?x}" "${4:?y}"; pointer_tool move "$3" "$4" "$1" "$2" ;;
            click | press | release) pointer_tool "$2" "${3:-left}" ;;
            scroll) pointer_tool scroll "${3:?dy}" ;;
            wheel) pointer_tool wheel "${3:?n}" ;;
            *) echo "pointer move X Y | click [düğme] | scroll DY | wheel N" >&2; exit 2 ;;
        esac
        ;;
    key) need; wtype -k "${2:?tuş gerekli}" ;;
    keylock) need; pointer_tool lock "${2:?caps|num}" "${3:?on|off}" ;;
    vkey) need; pointer_tool key "${2:?kombinasyon gerekli}" ;;
    type) need; wtype "${2:?metin gerekli}" ;;
    *) sed -n '2,41p' "$0"; exit 2 ;;
esac
