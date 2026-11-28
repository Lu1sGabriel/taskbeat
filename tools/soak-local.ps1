# Soak no Windows: o Taskbeat aberto por horas, tocando, com uma amostra de
# consumo a cada 5 min. Falha se o app fechar sozinho ou se a memória, os
# handles ou as threads crescerem de forma contínua.
#
# Roda o BINÁRIO DE RELEASE (`cargo build --release`): os testes ignoram o
# `panic = "abort"`, então só o binário de verdade mostra um abort.
#
# Uso (8 h contínuas numa máquina sua, com o Taskbeat instalado FECHADO, porque
# a instância é única):
#   cargo build --release --locked
#   tools\soak-local.ps1 -Minutes 480
# Só medir um Taskbeat que já está aberto (não abre nada, não fecha nada):
#   tools\soak-local.ps1 -AttachPid <pid> -Minutes 60 -NoPlayer
# No CI (soak.yml) o mesmo script roda com -Minutes 330 e -Deadline.
param(
    [string]$Exe = "target\release\taskbeat.exe",
    [int]$Minutes = 480,
    [int]$EverySec = 300,
    [string]$Out = "soak-out",
    # Ignora os primeiros minutos na regressão (caches enchendo).
    [int]$WarmupMin = 30,
    [switch]$NoPlayer,
    [int]$AttachPid = 0,
    # Hora (Unix, s) em que tem de parar, mesmo antes de -Minutes (limite do job).
    [long]$Deadline = 0,
    # Limites por hora.
    [double]$MaxMemMbPerHour = 1.0,
    [double]$MaxHandlesPerHour = 10.0,
    [double]$MaxThreadsPerHour = 1.0
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'proc-sample.ps1')
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$csv = Join-Path $Out 'soak.csv'
$summary = Join-Path $Out 'soak-resumo.txt'
function Say($m) { $l = "$(Get-Date -Format s) $m"; Write-Host $l; Add-Content -Path $summary -Value $l }
Set-Content -Path $summary -Value "soak do Taskbeat no Windows"

$player = $null
$app = $null
$appData = $null
try {
    if (-not $NoPlayer) {
        $playerExe = Join-Path $root 'target\release\examples\ci_player.exe'
        if (-not (Test-Path $playerExe)) {
            Say "compilando o player de teste (examples/ci_player.rs)"
            Push-Location $root
            try { cargo build --release --locked --example ci_player | Out-Host } finally { Pop-Location }
        }
        $player = Start-Process -FilePath $playerExe -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Out 'player.out') -RedirectStandardError (Join-Path $Out 'player.err')
        Start-Sleep -Seconds 3
        if ($player.HasExited) { Say "player de teste saiu (código $($player.ExitCode)): o soak mede o app parado" ; $player = $null }
        else { Say "player de teste no SMTC (pid $($player.Id))" }
    }

    if ($AttachPid -gt 0) {
        $app = Get-Process -Id $AttachPid
        Say "medindo o processo já aberto $AttachPid ($($app.Path))"
    } else {
        if (Get-Process -Name taskbeat -ErrorAction SilentlyContinue) {
            throw "já há um Taskbeat aberto: feche-o (a instância é única) ou use -AttachPid"
        }
        $exePath = (Resolve-Path $Exe).Path
        # Configuração limpa e sem efeitos na máquina: sem procurar atualização
        # e sem ligar o "abrir ao iniciar a sessão".
        $appData = Join-Path ([IO.Path]::GetTempPath()) "taskbeat-soak-$PID"
        New-Item -ItemType Directory -Force (Join-Path $appData 'Taskbeat') | Out-Null
        Set-Content -Path (Join-Path $appData 'Taskbeat\config.toml') -Value "version = 5`nlogin_default_applied = true`nupdate_check = false`nshow_visualizer = true`n"
        $psi = [System.Diagnostics.ProcessStartInfo]::new($exePath)
        $psi.UseShellExecute = $false
        $psi.Environment['APPDATA'] = $appData
        $psi.Environment['TASKBEAT_LOG'] = (Join-Path $Out 'taskbeat.log')
        $app = [System.Diagnostics.Process]::Start($psi)
        Say "app aberto: $exePath (pid $($app.Id)), config em $appData"
    }

    "t,cpu_s,ws_mb,private_mb,handles,threads,gdi,user,gdi_peak,user_peak" | Set-Content -Path $csv
    $start = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    $end = $start + $Minutes * 60
    if ($Deadline -gt 0 -and $Deadline -lt $end) { $end = $Deadline }
    $died = $false
    while ($true) {
        $s = Get-ProcSample $app.Id
        if (-not $s) { $died = $true; break }
        "$($s.t),$($s.cpu_s),$($s.ws_mb),$($s.private_mb),$($s.handles),$($s.threads),$($s.gdi),$($s.user),$($s.gdi_peak),$($s.user_peak)" | Add-Content -Path $csv
        $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        if ($now -ge $end) { break }
        $wait = [math]::Min($EverySec, $end - $now)
        # Espera acordando a cada 5 s para notar logo se o app fechou.
        $until = $now + $wait
        while ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() -lt $until) {
            if ($app.HasExited) { $died = $true; break }
            Start-Sleep -Seconds 5
        }
        if ($died) { break }
    }

    $rows = Import-Csv $csv
    $fail = @()
    if ($died) { $fail += "o app fechou sozinho durante o soak" }
    $t0 = [double]$rows[0].t
    $use = @($rows | Where-Object { [double]$_.t - $t0 -ge $WarmupMin * 60 })
    Say "amostras: $($rows.Count) (depois do aquecimento de $WarmupMin min: $($use.Count))"
    if ($use.Count -ge 6) {
        $t = [double[]]($use | ForEach-Object { [double]$_.t })
        $checks = @(
            @('private_mb', $MaxMemMbPerHour, 'MB/h de memória privada'),
            @('ws_mb', $MaxMemMbPerHour, 'MB/h de working set'),
            @('handles', $MaxHandlesPerHour, 'handles do kernel/h'),
            @('gdi', $MaxHandlesPerHour, 'objetos GDI/h'),
            @('user', $MaxHandlesPerHour, 'objetos USER/h'),
            @('threads', $MaxThreadsPerHour, 'threads/h')
        )
        foreach ($c in $checks) {
            $y = [double[]]($use | ForEach-Object { [double]$_.($c[0]) })
            $k = Get-Slope $t $y
            $line = "{0}: inclinação {1:N3} {2} (limite {3})" -f $c[0], $k, $c[2], $c[1]
            Say $line
            if ($k -gt $c[1]) { $fail += $line }
        }
    } else {
        Say "inconclusivo: menos de 6 amostras depois do aquecimento (rode mais tempo)"
    }
    $last = $rows[-1]
    Say ("última amostra: working set {0} MB, privada {1} MB, handles {2}, threads {3}, GDI {4}, USER {5}, CPU {6} s" -f $last.ws_mb, $last.private_mb, $last.handles, $last.threads, $last.gdi, $last.user, $last.cpu_s)
    if ($fail.Count -gt 0) {
        Say "FALHOU:"
        $fail | ForEach-Object { Say "  $_" }
        exit 1
    }
    Say "ok"
} finally {
    if ($app -and $AttachPid -eq 0 -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
    if ($player -and -not $player.HasExited) { Stop-Process -Id $player.Id -Force -ErrorAction SilentlyContinue }
    if ($appData) { Remove-Item -Recurse -Force $appData -ErrorAction SilentlyContinue }
}
