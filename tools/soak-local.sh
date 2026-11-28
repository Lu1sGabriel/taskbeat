#!/bin/bash
# Soak no Mac e no Linux: o Taskbeat aberto por horas, tocando o player de
# teste do CI, com uma amostra de consumo a cada 5 min. Falha se o app fechar
# sozinho ou se a memória (RSS), os descritores abertos ou as threads crescerem
# de forma contínua.
#
# Roda o BINÁRIO DE RELEASE: os testes ignoram o `panic = "abort"`, então só o
# binário de verdade mostra um abort.
#
# Uso (8 h contínuas numa máquina sua, com o Taskbeat instalado FECHADO):
#   cargo build --release --locked            # Linux
#   ./build-macos.sh                          # Mac (o .app traz o adaptador do Tocando Agora)
#   tools/soak-local.sh --minutos 480
# Opções: --bin <app>  --minutos N  --cada S  --saida <pasta>  --aquecimento N
#         --sem-player  --pid <pid> (só mede um processo já aberto)
#         --prazo <unix> (para antes, no limite do job do CI)
# No Linux sem tela (CI): xvfb-run -a dbus-run-session -- tools/soak-local.sh …
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$HERE")
OS=$(uname -s)
if [ "$OS" = Darwin ]; then BIN=dist/Taskbeat.app/Contents/MacOS/Taskbeat; else BIN=target/release/taskbeat; fi
MINUTOS=480
CADA=300
SAIDA=soak-out
AQUECIMENTO=30
PLAYER_ON=1
ATTACH=0
PRAZO=0
MAX_MEM=1.0      # MB/h
MAX_FDS=10.0     # descritores/h
MAX_THREADS=1.0  # threads/h
while [ $# -gt 0 ]; do
    case $1 in
        --bin) BIN=$2; shift ;;
        --minutos) MINUTOS=$2; shift ;;
        --cada) CADA=$2; shift ;;
        --saida) SAIDA=$2; shift ;;
        --aquecimento) AQUECIMENTO=$2; shift ;;
        --sem-player) PLAYER_ON=0 ;;
        --pid) ATTACH=$2; shift ;;
        --prazo) PRAZO=$2; shift ;;
        *) echo "opção desconhecida: $1"; exit 2 ;;
    esac
    shift
done
mkdir -p "$SAIDA"
SAIDA=$(cd "$SAIDA" && pwd)
CSV=$SAIDA/soak.csv
RESUMO=$SAIDA/soak-resumo.txt
note() { echo "$(date -u +%FT%TZ) $*" | tee -a "$RESUMO"; }
echo "soak do Taskbeat ($OS)" >"$RESUMO"

TMP=$(mktemp -d)
APP=
PLAYER=
cleanup() {
    [ -n "$APP" ] && [ "$ATTACH" = 0 ] && kill "$APP" 2>/dev/null
    [ -n "$PLAYER" ] && kill "$PLAYER" 2>/dev/null
    pkill -x paplay 2>/dev/null
    rm -rf "$TMP"
}
trap cleanup EXIT

# ---------------------------------------------------------------- player
if [ "$PLAYER_ON" = 1 ]; then
    if [ "$OS" = Darwin ]; then
        say -o "$TMP/ci.aiff" "Taskbeat soak test. Playing in a loop for many hours." 2>/dev/null || true
        swift "$ROOT/macos/ci-nowplaying.swift" "$TMP/ci.aiff" $((MINUTOS * 60 + 600)) >"$SAIDA/player.out" 2>&1 &
    else
        if ! pactl info >/dev/null 2>&1; then
            pulseaudio --start --exit-idle-time=-1 >/dev/null 2>&1 || note "pulseaudio não subiu"
            sleep 1
        fi
        if command -v convert >/dev/null; then
            convert -size 512x512 radial-gradient:'#ffd27a'-'#e0445f' "$TMP/art1.png"
            convert -size 512x512 radial-gradient:'#38d6c4'-'#3a3ad6' "$TMP/art2.png"
        else
            cp "$ROOT/assets/taskbeat-256.png" "$TMP/art1.png"
            cp "$ROOT/assets/taskbeat-256.png" "$TMP/art2.png"
        fi
        python3 "$ROOT/linux/ci-mpris.py" "$TMP/art1.png" "$TMP/art2.png" >"$SAIDA/player.out" 2>&1 &
    fi
    PLAYER=$!
    for _ in $(seq 1 60); do
        grep -q tocando "$SAIDA/player.out" 2>/dev/null && break
        sleep 2
    done
    if kill -0 "$PLAYER" 2>/dev/null; then note "player de teste aberto (pid $PLAYER)"; else note "player de teste não abriu: o soak mede o app parado"; PLAYER=; fi
fi

# ---------------------------------------------------------------- app
if [ "$ATTACH" != 0 ]; then
    APP=$ATTACH
    note "medindo o processo já aberto $APP"
else
    if pgrep -xi taskbeat >/dev/null; then
        note "já há um Taskbeat aberto: feche-o ou use --pid"
        exit 2
    fi
    # Configuração limpa e sem efeitos na máquina: sem procurar atualização e
    # sem ligar o "abrir ao iniciar a sessão".
    SEED="version = 5
login_default_applied = true
update_check = false
show_visualizer = true"
    if [ "$OS" = Darwin ]; then
        mkdir -p "$TMP/home/Library/Application Support/Taskbeat"
        echo "$SEED" >"$TMP/home/Library/Application Support/Taskbeat/config.toml"
        HOME="$TMP/home" TASKBEAT_LOG="$SAIDA/taskbeat.log" "$BIN" >"$SAIDA/taskbeat.out" 2>&1 &
    else
        mkdir -p "$TMP/config/taskbeat"
        echo "$SEED" >"$TMP/config/taskbeat/config.toml"
        XDG_CONFIG_HOME="$TMP/config" TASKBEAT_LOG="$SAIDA/taskbeat.log" "$BIN" >"$SAIDA/taskbeat.out" 2>&1 &
    fi
    APP=$!
    note "app aberto: $BIN (pid $APP)"
fi

sample() { # pid → "t,cpu_s,rss_mb,fds,threads"
    local pid=$1 t cpu rss fds thr
    t=$(date +%s)
    kill -0 "$pid" 2>/dev/null || return 1
    if [ "$OS" = Darwin ]; then
        rss=$(ps -o rss= -p "$pid" | tr -d ' ')
        thr=$(($(ps -M -p "$pid" | wc -l) - 1))
        fds=$(($(lsof -p "$pid" 2>/dev/null | wc -l) - 1))
        cpu=$(ps -o time= -p "$pid" | tr -d ' ')
    else
        rss=$(awk '/^VmRSS/{print $2}' "/proc/$pid/status")
        thr=$(awk '/^Threads/{print $2}' "/proc/$pid/status")
        fds=$(ls "/proc/$pid/fd" | wc -l)
        cpu=$(ps -o time= -p "$pid" | tr -d ' ')
    fi
    # ps -o time: [[dd-]hh:]mm:ss(.cc) → segundos.
    cpu=$(echo "$cpu" | awk -F'[-:]' '{s=0; for(i=1;i<=NF;i++){s=s*60+$i}; if (NF==4) s=$1*86400+$2*3600+$3*60+$4; print s}')
    echo "$t,$cpu,$(awk "BEGIN{printf \"%.2f\", ${rss:-0}/1024}"),$fds,$thr"
}

echo "t,cpu_s,rss_mb,fds,threads" >"$CSV"
START=$(date +%s)
END=$((START + MINUTOS * 60))
[ "$PRAZO" -gt 0 ] && [ "$PRAZO" -lt "$END" ] && END=$PRAZO
DIED=0
while true; do
    if ! line=$(sample "$APP"); then DIED=1; break; fi
    echo "$line" >>"$CSV"
    now=$(date +%s)
    [ "$now" -ge "$END" ] && break
    until=$((now + (CADA < END - now ? CADA : END - now)))
    while [ "$(date +%s)" -lt "$until" ]; do
        kill -0 "$APP" 2>/dev/null || { DIED=1; break; }
        sleep 5
    done
    [ "$DIED" = 1 ] && break
done

# ---------------------------------------------------------------- análise
FAIL=0
[ "$DIED" = 1 ] && { note "FALHOU: o app fechou sozinho durante o soak"; FAIL=1; }
slope() { # coluna → inclinação por hora, depois do aquecimento
    awk -F, -v col="$1" -v warm=$((AQUECIMENTO * 60)) 'NR==1{next} NR==2{t0=$1} $1-t0>=warm{n++; x[n]=$1; y[n]=$col}
        END{ if(n<6){print "nan"; exit} mx=0; my=0; for(i=1;i<=n;i++){mx+=x[i]; my+=y[i]} mx/=n; my/=n;
             num=0; den=0; for(i=1;i<=n;i++){num+=(x[i]-mx)*(y[i]-my); den+=(x[i]-mx)^2}
             if(den==0){print 0} else {printf "%.3f", num/den*3600} }' "$CSV"
}
N=$(($(wc -l <"$CSV") - 1))
note "amostras: $N"
check() { # coluna, limite, rótulo
    local k
    k=$(slope "$1")
    if [ "$k" = nan ]; then note "$3: inconclusivo (menos de 6 amostras depois de $AQUECIMENTO min)"; return; fi
    note "$3: inclinação $k/h (limite $2)"
    if awk "BEGIN{exit !($k > $2)}"; then note "FALHOU: $3 cresce $k/h"; FAIL=1; fi
}
check 3 "$MAX_MEM" "memória (RSS, MB)"
check 4 "$MAX_FDS" "descritores abertos"
check 5 "$MAX_THREADS" "threads"
note "última amostra: $(tail -1 "$CSV")"
[ "$FAIL" = 0 ] && note "ok"
exit "$FAIL"
