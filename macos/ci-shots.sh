#!/bin/bash
# CI: abre o app de verdade (música de exemplo) em várias situações e tira
# prints da tela, para conferir o visual sem um Mac à mão.
# Uso: macos/ci-shots.sh <pasta-de-saída>
set -u
OUT=${1:-shots}
mkdir -p "$OUT"
BIN=dist/Taskbeat.app/Contents/MacOS/Taskbeat

run() {
    local name=$1
    shift
    env "$@" TV_DEMO_MEDIA=1 TASKBEAT_LANG=pt TASKBEAT_LOG="$OUT/$name.log" "$BIN" >"$OUT/$name.out" 2>&1 &
    local pid=$!
    sleep 6
    screencapture -x "$OUT/$name.png" || echo "screencapture falhou ($name)"
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    sleep 1
}

system_profiler SPDisplaysDataType | grep -E 'Resolution|Retina' > "$OUT/display.txt" || true
run live-closed
run live-closed-notch TASKBEAT_FAKE_NOTCH=196x32
run live-open TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_FLYOUT=1
run live-peek TASKBEAT_FAKE_NOTCH=196x32 TV_DEMO_POPUP=1
run live-settings TV_DEMO_SETTINGS=1
