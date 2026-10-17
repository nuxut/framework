#!/bin/sh
# ADR-0008 blob sahnelerinin görüntüleri, YALNIZCA izole test compositor'ında.
#
#   tools/test-compositor.sh start
#   tools/blob-shots.sh ÇIKIŞ_DİZİNİ [SAHNE...]   (eşzamanlı canlı testler varsa bir flock kilidi içinde)
#   tools/test-compositor.sh stop
#
# Her sahne (varsayılan: hepsi, `blob_scenes --list`) için:
#   ÇIKIŞ/<sahne>.png             açık panel (son durum)
#   ÇIKIŞ/<sahne>-anim-NN.png     açılış animasyonu, 10 kare, BLOB_SHOTS_STEP_MS aralıklı
#                                 (varsayılan 40 ms; yay o anda dondurulup çizilir: deterministik)
# Görüntü test compositor'ının HEADLESS-1 çıkışından alınır (yüzey tam ekran; kullanıcının
# ekranına dokunulmaz). BLOB_SHOTS_CROP="x,y wxh" verilirse yalnız o bölge kırpılır.
set -eu
unset HYPRLAND_INSTANCE_SIGNATURE

out=${1:?çıkış dizini gerekli}
shift
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
user_display=${WAYLAND_DISPLAY:-}
eval "$("$root/tools/test-compositor.sh" env)"
if [ -z "${WAYLAND_DISPLAY:-}" ] || [ "$WAYLAND_DISPLAY" = "$user_display" ]; then
    echo "WAYLAND_DISPLAY test compositor'ınınki değil; çıkılıyor" >&2
    exit 1
fi

bin=$root/target/debug/examples/blob_scenes
src=$root/crates/nuxut-wayland/examples/blob_scenes.rs
scenes_src=$root/crates/nuxut-render/tests/it/support/scenes.rs
if [ ! -x "$bin" ] || [ "$src" -nt "$bin" ] || [ "$scenes_src" -nt "$bin" ]; then
    (cd "$root" && $(cargo_cmd) build -q -p nuxut-wayland --example blob_scenes) >&2
fi
mkdir -p "$out"
work=$(mktemp -d)
step=${BLOB_SHOTS_STEP_MS:-40}
pid=""
cleanup() {
    exec 3>&- 2> /dev/null || true
    [ -n "$pid" ] && kill "$pid" 2> /dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

shot() {
    if [ -n "${BLOB_SHOTS_CROP:-}" ]; then
        grim -g "$BLOB_SHOTS_CROP" "$1"
    else
        grim -o HEADLESS-1 "$1"
    fi
}

# `drawn` satır sayısı `n`'e ulaşana dek bekler (en çok 5 sn).
wait_drawn() {
    i=0
    while [ "$(grep -c '^drawn$' "$work/log" 2> /dev/null || true)" -lt "$1" ]; do
        i=$((i + 1))
        if [ $i -gt 100 ] || ! kill -0 "$pid" 2> /dev/null; then
            echo "sahne çizilmedi; günlük:" >&2
            tail -5 "$work/log" >&2
            exit 1
        fi
        sleep 0.05
    done
    # Kare sunulsun.
    sleep 0.15
}

# run SAHNE [--animate]: uygulamayı stdin FIFO'suyla başlatır (fd 3 yazma ucu).
run() {
    rm -f "$work/fifo" "$work/log"
    mkfifo "$work/fifo"
    (exec "$bin" "$@" < "$work/fifo" > "$work/log" 2>&1) &
    pid=$!
    exec 3> "$work/fifo"
}

stop_app() {
    echo q >&3 || true
    exec 3>&-
    n=0
    while kill -0 "$pid" 2> /dev/null && [ $n -lt 50 ]; do sleep 0.1; n=$((n + 1)); done
    kill "$pid" 2> /dev/null || true
    pid=""
}

if [ $# -gt 0 ]; then
    names=$*
else
    names=$("$bin" --list)
fi
for scene in $names; do
    run "$scene"
    wait_drawn 1
    shot "$out/$scene.png"
    stop_app

    run "$scene" --animate
    k=0
    while [ $k -lt 10 ]; do
        echo "t $((k * step))" >&3
        wait_drawn $((k + 1))
        shot "$out/$scene-anim-$(printf %02d $k).png"
        k=$((k + 1))
    done
    stop_app
    echo "$scene"
done
