#!/bin/bash
# CI do Linux: abre o app de verdade e tira prints, sem uma máquina Linux com
# tela.
# - Wayland: um Sway sem tela (layer-shell, o caminho do KDE, Hyprland,
#   COSMIC…), nas escalas 1, 2 e 1,5.
# - X11: Xvfb com o compositor picom (o caminho do GNOME pelo XWayland, do
#   XFCE, Cinnamon…).
# Depois testa de ponta a ponta com um player de teste no MPRIS
# (linux/ci-mpris.py): o app mostra a música, a capa e as barras, e os
# cliques (xdotool no X11) chegam ao player. Mede memória e CPU.
# Uso: linux/ci-shots.sh <pasta-de-saída> [binário]
set -u
OUT=$(realpath -m "${1:-shots}")
BIN=$(realpath "${2:-target/release/taskbeat}")
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
TMP=$(mktemp -d)
W=1600
H=900
FAILS=0
fail() {
    echo "FALHOU: $*"
    FAILS=$((FAILS + 1))
}
PIDS=()
cleanup() {
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
    pkill -f ci-mpris.py 2>/dev/null
    pkill -x paplay 2>/dev/null
}
trap cleanup EXIT

export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-$TMP/run}
mkdir -p -m 700 "$XDG_RUNTIME_DIR"
# Configuração limpa (os padrões), fora da do usuário.
export XDG_CONFIG_HOME=$TMP/config
export TASKBEAT_LANG=pt
export TASKBEAT_PERF=1

# "Papel de parede" e capas de teste.
convert -size ${W}x${H} -define gradient:angle=135 gradient:'#2f56b0'-'#b0577f' "$TMP/bg.png"
convert -size 512x512 radial-gradient:'#ffd27a'-'#e0445f' "$TMP/art1.png"
convert -size 512x512 -define gradient:angle=45 gradient:'#38d6c4'-'#3a3ad6' "$TMP/art2.png"

# Som: sem servidor (CI), sobe um PulseAudio com saída nula.
if ! pactl info >/dev/null 2>&1; then
    pulseaudio --start --exit-idle-time=-1 >/dev/null 2>&1 || echo "pulseaudio não subiu"
    sleep 1
fi
pactl info 2>&1 | grep -E "Server Name|Default Sink"

# ---------------------------------------------------------------- app e player

APP=
start_app() { # nome, variáveis…
    local name=$1
    shift
    env "$@" TASKBEAT_LOG="$OUT/$name.log" "$BIN" >"$OUT/$name.out" 2>&1 &
    APP=$!
}
stop_app() {
    [ -n "$APP" ] && kill "$APP" 2>/dev/null && wait "$APP" 2>/dev/null
    APP=
    sleep 0.5
}
PLAYER=
start_player() {
    python3 "$HERE/ci-mpris.py" "$TMP/art1.png" "$TMP/art2.png" >"$OUT/player.out" 2>&1 &
    PLAYER=$!
    PIDS+=("$PLAYER")
    for _ in $(seq 1 30); do
        grep -q tocando "$OUT/player.out" 2>/dev/null && break
        sleep 0.3
    done
    sleep 1
}
stop_player() {
    [ -n "$PLAYER" ] && kill "$PLAYER" 2>/dev/null
    pkill -x paplay 2>/dev/null
    PLAYER=
    sleep 0.5
}

# Memória (MB) e CPU (% de um núcleo) do app em $1 segundos.
measure() { # rótulo segundos
    local t=$2 hz
    hz=$(getconf CLK_TCK)
    local a b
    a=$(awk '{print $14+$15}' /proc/"$APP"/stat 2>/dev/null) || return
    sleep "$t"
    b=$(awk '{print $14+$15}' /proc/"$APP"/stat 2>/dev/null) || return
    local rss
    rss=$(awk '/VmRSS/{print $2}' /proc/"$APP"/status)
    echo "medida [$1]: RAM $((rss / 1024)) MB, CPU $(awk -v d=$((b - a)) -v hz="$hz" -v t="$t" 'BEGIN{printf "%.1f", 100*d/hz/t}')%" | tee -a "$OUT/medidas.txt"
}

# Centro do botão $2 ("próxima", "play"…) no log $1, como "x,y" (px da superfície).
spot() {
    grep -o "$2 [0-9]*,[0-9]*" "$1" | tail -1 | awk '{print $2}'
}

# ---------------------------------------------------------------- Wayland (Sway sem tela)

cat >"$TMP/sway.cfg" <<EOF
output HEADLESS-1 resolution ${W}x${H} position 0 0 bg $TMP/bg.png fill
default_border none
xwayland disable
EOF
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c "$TMP/sway.cfg" >"$OUT/sway.log" 2>&1 &
SWAY=$!
PIDS+=("$SWAY")
SOCK=
for _ in $(seq 1 50); do
    SOCK=$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*."$SWAY".sock 2>/dev/null | head -1)
    [ -n "$SOCK" ] && break
    sleep 0.2
done
export SWAYSOCK=$SOCK
WL=$(ls -t "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | head -1)
echo "sway: socket ${SOCK:-nenhum}, display ${WL:-nenhum}"

wl_shot() { # nome
    WAYLAND_DISPLAY=$WL grim "$TMP/full.png" || {
        fail "grim ($1)"
        return
    }
    local pw
    pw=$(identify -format %w "$TMP/full.png")
    local cw=$((pw * 1100 / W))
    convert "$TMP/full.png" -crop "${cw}x$((cw * 3 / 10))+$(((pw - cw) / 2))+0" +repage "$OUT/$1.png"
}
wl_run() { # nome segundos variáveis…
    local name=$1 t=$2
    shift 2
    start_app "$name" WAYLAND_DISPLAY="$WL" DISPLAY= "$@"
    sleep "$t"
    wl_shot "$name"
    grep -q "janela: wayland" "$OUT/$name.log" || fail "$name não usou Wayland/layer-shell"
    stop_app
}

if [ -n "$SOCK" ] && [ -n "$WL" ]; then
    wl_run wl-closed 3 TV_DEMO_MEDIA=1
    wl_run wl-open 3 TV_DEMO_MEDIA=1 TV_DEMO_FLYOUT=1
    wl_run wl-peek 3 TV_DEMO_MEDIA=1 TV_DEMO_POPUP=1
    wl_run wl-idle 3
    swaymsg output HEADLESS-1 scale 2 >/dev/null
    wl_run wl-open-2x 3 TV_DEMO_MEDIA=1 TV_DEMO_FLYOUT=1
    swaymsg output HEADLESS-1 scale 1.5 >/dev/null
    wl_run wl-open-1.5x 3 TV_DEMO_MEDIA=1 TV_DEMO_FLYOUT=1
    swaymsg output HEADLESS-1 scale 1 >/dev/null

    # Player de teste: música, capa e barras de verdade.
    start_player
    start_app wl-real WAYLAND_DISPLAY="$WL" DISPLAY=
    sleep 4
    wl_shot wl-real-closed
    measure "Wayland, tocando, barras" 10
    # Mouse virtual (wlrctl): passa por cima da ilha e o card abre.
    timeout 3 swaymsg seat seat0 cursor set $((W / 2)) 12 >/dev/null 2>&1
    WAYLAND_DISPLAY=$WL timeout 3 wlrctl pointer move 1 1 >/dev/null 2>&1
    sleep 1.2
    wl_shot wl-real-hover
    grep -q "card aberto" "$OUT/wl-real.log" && echo "wayland: o card abriu com o mouse" || echo "wayland: o mouse virtual não abriu o card (sem ponteiro no Sway sem tela?)"
    stop_app
    stop_player
else
    fail "sway sem tela não subiu"
    tail -20 "$OUT/sway.log"
fi
kill "$SWAY" 2>/dev/null

# ---------------------------------------------------------------- X11 (Xvfb + picom)

Xvfb :99 -screen 0 ${W}x${H}x24 -nolisten tcp >/dev/null 2>&1 &
PIDS+=($!)
sleep 1
DISPLAY=:99 picom --backend xrender >"$OUT/picom.log" 2>&1 &
PIDS+=($!)
# "Papel de parede": uma janela comum atrás de tudo (não há gerenciador de janelas).
DISPLAY=:99 display -geometry +0+0 -borderwidth 0 -title fundo "$TMP/bg.png" >/dev/null 2>&1 &
PIDS+=($!)
sleep 1

x_shot() { # nome
    DISPLAY=:99 import -window root "$TMP/full.png" 2>/dev/null || {
        fail "import ($1)"
        return
    }
    convert "$TMP/full.png" -crop "1100x330+$(((W - 1100) / 2))+0" +repage "$OUT/$1.png"
}
x_app() { # nome variáveis…
    local name=$1
    shift
    start_app "$name" WAYLAND_DISPLAY= DISPLAY=:99 TASKBEAT_BACKEND=x11 "$@"
}

x_app x11-open TV_DEMO_MEDIA=1 TV_DEMO_FLYOUT=1
sleep 3
x_shot x11-open
grep -q "janela: x11" "$OUT/x11-open.log" || fail "x11-open não usou X11"
stop_app

x_app x11-idle
sleep 3
measure "X11, nada tocando" 10
x_shot x11-idle
stop_app

# Ponta a ponta: o app lê o player e os cliques chegam a ele.
start_player
x_app x11-real
sleep 4
LOG=$OUT/x11-real.log
grep -q 'mídia: "Midnight City" — M83 \[CI Player\] tocando=true' "$LOG" || fail "o app não leu a música do player"
grep -q "capa=512x512\|capa=384x384" "$LOG" || fail "a capa não carregou"
grep -q "áudio: gravando Stream" "$LOG" && echo "áudio: só o som do player" || echo "áudio: saída inteira (fluxo do player não achado)"
x_shot x11-real-closed
SIZE=$(grep -o "tamanho [0-9]*x[0-9]*" "$LOG" | tail -1 | awk '{print $2}')
SW=${SIZE%x*}
X0=$(((W - SW) / 2))
click() { # botão
    local p
    p=$(spot "$LOG" "$1")
    [ -z "$p" ] && {
        fail "posição de $1 não está no log"
        return
    }
    DISPLAY=:99 xdotool mousemove $((X0 + ${p%,*})) "${p#*,}" sleep 0.25 click 1
    sleep 1
}
# Mouse em cima da ilha: o card abre.
P=$(spot "$LOG" fechada)
DISPLAY=:99 xdotool mousemove $((X0 + ${P%,*})) "${P#*,}"
sleep 1.2
grep -q "card aberto" "$LOG" || fail "o card não abriu com o mouse"
x_shot x11-real-open
click próxima
grep -q "comando: Next" "$OUT/player.out" || fail "Próxima não chegou ao player"
click aleatório
grep -q "comando: Set Shuffle=1" "$OUT/player.out" || fail "Aleatório não chegou ao player"
click repetir
grep -q "comando: Set LoopStatus=Track" "$OUT/player.out" || fail "Repetir não chegou ao player"
click play
grep -q "comando: PlayPause" "$OUT/player.out" || fail "Tocar/pausar não chegou ao player"
sleep 1
x_shot x11-real-after
grep -q 'mídia: "Wait"' "$LOG" || fail "o app não mostrou a próxima música"
# Mouse longe: o card fecha.
DISPLAY=:99 xdotool mousemove 40 600
sleep 1.5
grep -q "card fechado" "$LOG" || fail "o card não fechou com o mouse longe"
# Volta a tocar (pelo próprio D-Bus) e mede com as barras.
dbus-send --session --type=method_call --print-reply --dest=org.mpris.MediaPlayer2.ciplayer /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Play
sleep 3
measure "X11, tocando, barras" 10
# Card aberto tocando: só as barras e o tempo são redesenhados.
DISPLAY=:99 xdotool mousemove $((X0 + ${P%,*})) "${P#*,}"
sleep 2
measure "X11, card aberto, tocando" 10
x_shot x11-real-open-playing
stop_app
stop_player

echo "player:"
cat "$OUT/player.out"
for f in "$OUT"/*.log; do
    echo "== $f"
    head -30 "$f"
done
echo "falhas: $FAILS"
[ "$FAILS" -eq 0 ]
