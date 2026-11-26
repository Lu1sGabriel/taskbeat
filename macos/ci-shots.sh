#!/bin/bash
# CI: abre o app de verdade em várias situações e tira prints da tela, para
# conferir o visual sem um Mac à mão. Depois testa de ponta a ponta com um
# player de teste (macos/ci-nowplaying.swift) publicando no Tocando Agora.
# Uso: macos/ci-shots.sh <pasta-de-saída>
set -u
OUT=${1:-shots}
mkdir -p "$OUT"
APP=dist/Taskbeat.app
BIN=$APP/Contents/MacOS/Taskbeat
ADAPTER=("$APP/Contents/Resources/mediaremote-adapter.pl" "$PWD/$APP/Contents/Frameworks/MediaRemoteAdapter.framework")

run() {
    local name=$1
    shift
    env "$@" TASKBEAT_LANG=pt TASKBEAT_LOG="$PWD/$OUT/$name.log" "$BIN" >"$OUT/$name.out" 2>&1 &
    local pid=$!
    sleep 6
    screencapture -x "$OUT/$name.png" || echo "screencapture falhou ($name)"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    sleep 1
}

# ---- 1. Música de exemplo (sem player)
run live-closed TV_DEMO_MEDIA=1
run live-closed-notch TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32
run live-open TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_FLYOUT=1
run live-peek TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_POPUP=1
run live-settings TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1
run live-settings-viz TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1 TV_DEMO_SETTINGS_PAGE=2
run live-settings-caps TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1 TV_DEMO_SETTINGS_PAGE=3
run live-settings-general TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1 TV_DEMO_SETTINGS_PAGE=4
run live-caps TV_DEMO_MEDIA=1 TV_DEMO_CAPS=1
# Caminhos dos macOS 12/13 (sem CADisplayLink, sem taps de áudio).
run legacy-open TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_FLYOUT=1 TASKBEAT_FORCE_LEGACY=1
run legacy-caps TV_DEMO_MEDIA=1 TV_DEMO_CAPS=1 TASKBEAT_FORCE_LEGACY=1
run legacy-settings TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1 TASKBEAT_FORCE_LEGACY=1

# ---- 1b. Tela em 2x (HiDPI), se o monitor virtual tiver o modo: os mesmos
# prints como num Mac com tela Retina. Daqui em diante tudo roda nesse modo.
swift macos/ci-hidpi.swift >"$OUT/hidpi.txt" 2>&1 || echo "hidpi falhou: $?" >>"$OUT/hidpi.txt"
grep -v '^modo:' "$OUT/hidpi.txt"
if grep -q '^HiDPI:' "$OUT/hidpi.txt"; then
    run retina-closed-notch TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32
    run retina-open TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_FLYOUT=1
    run retina-peek TV_DEMO_MEDIA=1 TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_POPUP=1
    run retina-settings TV_DEMO_MEDIA=1 TV_DEMO_SETTINGS=1
    run retina-caps TV_DEMO_MEDIA=1 TV_DEMO_CAPS=1
fi

# ---- 2. Player de teste no Tocando Agora
say -o /tmp/taskbeat-ci.aiff "Taskbeat. Testing the now playing island on the notch of the MacBook." || true
swift macos/ci-nowplaying.swift /tmp/taskbeat-ci.aiff 240 >"$OUT/player.out" 2>&1 &
PLAYER=$!
for _ in $(seq 1 60); do
    grep -q tocando "$OUT/player.out" 2>/dev/null && break
    sleep 2
done
sleep 3
/usr/bin/perl "${ADAPTER[@]}" get --no-artwork >"$OUT/adapter-get.json" 2>&1 || echo "adaptador falhou: $?" >>"$OUT/adapter-get.json"
echo "adaptador:"; cat "$OUT/adapter-get.json"; echo

run real-closed TASKBEAT_FAKE_NOTCH=196x32
run real-open TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_FLYOUT=1

# Comandos pelo adaptador chegam ao player?
/usr/bin/perl "${ADAPTER[@]}" send 2 || true
sleep 2
/usr/bin/perl "${ADAPTER[@]}" send 4 || true
sleep 2
echo "player:"; cat "$OUT/player.out"
kill "$PLAYER" 2>/dev/null
for f in "$OUT"/*.log; do
    echo "== $f"
    head -40 "$f"
done
true
