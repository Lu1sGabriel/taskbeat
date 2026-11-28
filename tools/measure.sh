#!/bin/bash
# Medição de consumo no Mac e no Linux (PR-00): a base de comparação dos PRs
# `perf`. Mede um processo por N minutos e escreve uma linha da tabela: CPU
# (% de um núcleo), RSS médio e máximo, threads e descritores abertos (não há
# equivalente de GDI). O app aberto com TASKBEAT_FRAMESTATS=1 também grava o
# `stats.log` (p50/p95/p99 por superfície e despertares por segundo) na pasta
# da config.
#
# Uso:
#   tools/measure.sh --pid <pid> --nome tocando [--minutos 5] [--cada 10] [--saida measure-out]
#   tools/measure.sh --bin target/release/taskbeat --nome parado   # abre com config limpa
#   (Linux sem tela: xvfb-run -a dbus-run-session -- tools/measure.sh …)
set -u
OS=$(uname -s)
PID=
BIN=
NOME=aberto
MINUTOS=5
CADA=10
SAIDA=measure-out
while [ $# -gt 0 ]; do
    case $1 in
        --pid) PID=$2; shift ;;
        --bin) BIN=$2; shift ;;
        --nome) NOME=$2; shift ;;
        --minutos) MINUTOS=$2; shift ;;
        --cada) CADA=$2; shift ;;
        --saida) SAIDA=$2; shift ;;
        *) echo "opção desconhecida: $1"; exit 2 ;;
    esac
    shift
done
mkdir -p "$SAIDA"
SAIDA=$(cd "$SAIDA" && pwd)
TMP=$(mktemp -d)
OWN=0
cleanup() {
    [ "$OWN" = 1 ] && [ -n "$PID" ] && kill "$PID" 2>/dev/null
    rm -rf "$TMP"
}
trap cleanup EXIT

if [ -z "$PID" ]; then
    [ -n "$BIN" ] || { echo "use --pid ou --bin"; exit 2; }
    SEED="version = 5
login_default_applied = true
update_check = false
show_visualizer = true"
    if [ "$OS" = Darwin ]; then
        CFG="$TMP/home/Library/Application Support/Taskbeat"
        mkdir -p "$CFG"; echo "$SEED" >"$CFG/config.toml"
        HOME="$TMP/home" TASKBEAT_FRAMESTATS=1 "$BIN" >"$SAIDA/$NOME.out" 2>&1 &
    else
        CFG="$TMP/config/taskbeat"
        mkdir -p "$CFG"; echo "$SEED" >"$CFG/config.toml"
        XDG_CONFIG_HOME="$TMP/config" TASKBEAT_FRAMESTATS=1 "$BIN" >"$SAIDA/$NOME.out" 2>&1 &
    fi
    PID=$!
    OWN=1
    # O primeiro minuto (caches enchendo) fica de fora.
    sleep 60
fi

sample() { # → "t cpu_s rss_kb threads fds"
    local t cpu rss thr fds
    t=$(date +%s)
    kill -0 "$PID" 2>/dev/null || return 1
    if [ "$OS" = Darwin ]; then
        rss=$(ps -o rss= -p "$PID" | tr -d ' ')
        thr=$(($(ps -M -p "$PID" | wc -l) - 1))
        fds=$(($(lsof -p "$PID" 2>/dev/null | wc -l) - 1))
    else
        rss=$(awk '/^VmRSS/{print $2}' "/proc/$PID/status")
        thr=$(awk '/^Threads/{print $2}' "/proc/$PID/status")
        fds=$(ls "/proc/$PID/fd" | wc -l)
    fi
    cpu=$(ps -o time= -p "$PID" | tr -d ' ' | awk -F'[-:]' '{s=0; for(i=1;i<=NF;i++){s=s*60+$i}; if (NF==4) s=$1*86400+$2*3600+$3*60+$4; print s}')
    echo "$t $cpu $rss $thr $fds"
}

CSV="$SAIDA/$NOME.csv"
: >"$CSV"
END=$(($(date +%s) + MINUTOS * 60))
while [ "$(date +%s)" -lt "$END" ]; do
    sample >>"$CSV" || { echo "$NOME: o processo fechou"; break; }
    sleep "$CADA"
done
# CPU pelo tempo de processo do `ps` (resolução de 1 s no Linux): em poucos
# minutos, um app parado pode aparecer como 0.
LINE=$(awk -v nome="$NOME" 'NR==1{t0=$1;c0=$2} {t=$1;c=$2; r+=$3; if($3>rm)rm=$3; if($4>tm)tm=$4; if($5>fm)fm=$5; n++}
    END{ if(n<2){print "| " nome " | (sem amostras) | | | |"; exit}
         cpu=(t>t0)?(c-c0)/(t-t0)*100:0
         printf "| %s | %.2f | %.1f / %.1f | %d | %d |\n", nome, cpu, r/n/1024, rm/1024, tm, fm }' "$CSV")
TABLE="$SAIDA/medicao.md"
if [ ! -f "$TABLE" ]; then
    {
        echo "| cenário | CPU (% de 1 núcleo) | RSS médio / máx. (MB) | threads máx. | descritores máx. |"
        echo "|---|---|---|---|---|"
    } >"$TABLE"
fi
echo "$LINE" >>"$TABLE"
echo "$LINE"
if [ "$OWN" = 1 ]; then
    kill "$PID" 2>/dev/null; wait "$PID" 2>/dev/null; OWN=0
    [ -f "$CFG/stats.log" ] && cp "$CFG/stats.log" "$SAIDA/stats-$NOME.log"
fi
