# Medição de consumo no Windows (PR-00): a base de comparação dos PRs `perf`.
#
# Cada cenário roda $Minutes minutos com uma amostra a cada $EverySec segundos.
# Para cada um sai uma linha da tabela: CPU (% de um núcleo), working set,
# memória privada, handles do kernel, GDI, USER e threads. O app sobe com
# TASKBEAT_FRAMESTATS=1, e o stats.log (p50/p95/p99 por superfície e
# despertares por segundo) é copiado para a pasta de saída.
#
# Cenários automáticos (o app abre com uma config limpa; o Taskbeat instalado
# precisa estar FECHADO, porque a instância é única):
#   parado   - sem player
#   tocando  - com o player de teste (examples/ci_player.rs) no SMTC
#   card     - tocando, com o card aberto (TV_DEMO_FLYOUT)
# Cenários à mão (mixer aberto, letra em tela cheia, uso real): abra o app,
# deixe na situação e meça o processo já aberto:
#   tools\measure.ps1 -AttachPid <pid> -Label mixer
#
# Uso: tools\measure.ps1 [-Exe target\release\taskbeat.exe] [-Scenarios parado,tocando,card] [-Minutes 5]
param(
    [string]$Exe = "target\release\taskbeat.exe",
    [string[]]$Scenarios = @('parado', 'tocando', 'card'),
    [int]$Minutes = 5,
    [int]$EverySec = 10,
    [string]$Out = "measure-out",
    [int]$AttachPid = 0,
    [string]$Label = "aberto"
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'proc-sample.ps1')
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path
$table = Join-Path $Out 'medicao.md'

function Measure-Pid([int]$Id, [string]$Name) {
    $rows = @()
    $end = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() + $Minutes * 60
    while ([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() -lt $end) {
        $s = Get-ProcSample $Id
        if (-not $s) { Write-Host "${Name}: o processo fechou"; break }
        $rows += $s
        Start-Sleep -Seconds $EverySec
    }
    if ($rows.Count -lt 2) { return "| $Name | (sem amostras) | | | | | | |" }
    $dt = $rows[-1].t - $rows[0].t
    $cpu = if ($dt -gt 0) { ($rows[-1].cpu_s - $rows[0].cpu_s) / $dt * 100 } else { 0 }
    $avg = { param($k) ($rows | Measure-Object -Property $k -Average).Average }
    $max = { param($k) ($rows | Measure-Object -Property $k -Maximum).Maximum }
    $rows | Export-Csv -NoTypeInformation -Path (Join-Path $Out "$Name.csv")
    return ("| {0} | {1:N2} | {2:N1} / {3:N1} | {4:N1} | {5:N0} | {6:N0} | {7:N0} | {8:N0} |" -f $Name, $cpu, (& $avg 'ws_mb'), (& $max 'ws_mb'), (& $avg 'private_mb'), (& $max 'handles'), (& $max 'gdi'), (& $max 'user'), (& $max 'threads'))
}

$header = @(
    "Medição em $(Get-Date -Format 'yyyy-MM-dd HH:mm'), $Minutes min por cenário, amostra a cada $EverySec s.",
    "",
    "| cenário | CPU (% de 1 núcleo) | working set médio / máx. (MB) | privada média (MB) | handles máx. | GDI máx. | USER máx. | threads máx. |",
    "|---|---|---|---|---|---|---|---|"
)
Set-Content -Path $table -Value $header

if ($AttachPid -gt 0) {
    $line = Measure-Pid $AttachPid $Label
    Add-Content -Path $table -Value $line
    Write-Host $line
    exit 0
}

if (Get-Process -Name taskbeat -ErrorAction SilentlyContinue) {
    throw "já há um Taskbeat aberto: feche-o (a instância é única) ou use -AttachPid"
}
$exePath = (Resolve-Path $Exe).Path
$playerExe = Join-Path $root 'target\release\examples\ci_player.exe'
foreach ($sc in $Scenarios) {
    $player = $null
    $app = $null
    $appData = Join-Path ([IO.Path]::GetTempPath()) "taskbeat-measure-$PID-$sc"
    New-Item -ItemType Directory -Force (Join-Path $appData 'Taskbeat') | Out-Null
    Set-Content -Path (Join-Path $appData 'Taskbeat\config.toml') -Value "version = 5`nlogin_default_applied = true`nupdate_check = false`nshow_visualizer = true`n"
    try {
        if ($sc -ne 'parado') {
            if (-not (Test-Path $playerExe)) {
                Push-Location $root
                try { cargo build --release --locked --example ci_player | Out-Host } finally { Pop-Location }
            }
            $player = Start-Process -FilePath $playerExe -PassThru -WindowStyle Hidden
            Start-Sleep -Seconds 3
        }
        $psi = [System.Diagnostics.ProcessStartInfo]::new($exePath)
        $psi.UseShellExecute = $false
        $psi.Environment['APPDATA'] = $appData
        $psi.Environment['TASKBEAT_FRAMESTATS'] = '1'
        if ($sc -eq 'card') { $psi.Environment['TV_DEMO_FLYOUT'] = '1' }
        $app = [System.Diagnostics.Process]::Start($psi)
        # O primeiro minuto (caches enchendo) fica de fora.
        Start-Sleep -Seconds 60
        $line = Measure-Pid $app.Id $sc
        Add-Content -Path $table -Value $line
        Write-Host $line
    } finally {
        if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
        if ($player -and -not $player.HasExited) { Stop-Process -Id $player.Id -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Seconds 1
        $log = Join-Path $appData 'Taskbeat\stats.log'
        if (Test-Path $log) { Copy-Item $log (Join-Path $Out "stats-$sc.log") }
        Remove-Item -Recurse -Force $appData -ErrorAction SilentlyContinue
    }
}
Write-Host "tabela em $table"
