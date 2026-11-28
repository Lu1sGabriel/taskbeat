# Amostra de consumo de um processo no Windows (usada pelo soak e pela medição).
# Uso: . tools\proc-sample.ps1; Get-ProcSample -Id <pid>
#
# Três contagens separadas, porque são recursos diferentes:
# - handles do kernel: Process.HandleCount, com Refresh() antes de cada leitura
#   (o .NET guarda o valor em cache);
# - GDI e USER: GetGuiResources. Ele devolve 0 tanto em erro quanto em "zero
#   objetos", então o GetLastError é conferido.
# O processo é aberto com PROCESS_QUERY_LIMITED_INFORMATION (basta para os dois).

if (-not ('Taskbeat.Gui' -as [type])) {
    Add-Type -Namespace Taskbeat -Name Gui -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true)]
public static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool CloseHandle(IntPtr h);
[DllImport("user32.dll", SetLastError = true)]
public static extern uint GetGuiResources(IntPtr hProcess, uint flags);
public const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
public const uint GR_GDIOBJECTS = 0, GR_USEROBJECTS = 1, GR_GDIOBJECTS_PEAK = 2, GR_USEROBJECTS_PEAK = 4;
'@
}

function Get-GuiCount([IntPtr]$h, [uint32]$flag) {
    $n = [Taskbeat.Gui]::GetGuiResources($h, $flag)
    if ($n -eq 0) {
        $err = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
        if ($err -ne 0) { return -1 }
    }
    return [int]$n
}

function Get-ProcSample([int]$Id) {
    $p = Get-Process -Id $Id -ErrorAction SilentlyContinue
    if (-not $p) { return $null }
    $p.Refresh()
    $h = [Taskbeat.Gui]::OpenProcess([Taskbeat.Gui]::PROCESS_QUERY_LIMITED_INFORMATION, $false, [uint32]$Id)
    $gdi = -1; $user = -1; $gdiPeak = -1; $userPeak = -1
    if ($h -ne [IntPtr]::Zero) {
        $gdi = Get-GuiCount $h ([Taskbeat.Gui]::GR_GDIOBJECTS)
        $user = Get-GuiCount $h ([Taskbeat.Gui]::GR_USEROBJECTS)
        $gdiPeak = Get-GuiCount $h ([Taskbeat.Gui]::GR_GDIOBJECTS_PEAK)
        $userPeak = Get-GuiCount $h ([Taskbeat.Gui]::GR_USEROBJECTS_PEAK)
        [void][Taskbeat.Gui]::CloseHandle($h)
    }
    [pscustomobject]@{
        t          = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
        cpu_s      = [math]::Round($p.TotalProcessorTime.TotalSeconds, 2)
        ws_mb      = [math]::Round($p.WorkingSet64 / 1MB, 2)
        private_mb = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
        handles    = $p.HandleCount
        threads    = $p.Threads.Count
        gdi        = $gdi
        user       = $user
        gdi_peak   = $gdiPeak
        user_peak  = $userPeak
    }
}

# Inclinação (por hora) da regressão linear de y sobre t (segundos).
function Get-Slope([double[]]$t, [double[]]$y) {
    $n = $t.Count
    if ($n -lt 2) { return 0.0 }
    $mt = ($t | Measure-Object -Average).Average
    $my = ($y | Measure-Object -Average).Average
    $num = 0.0; $den = 0.0
    for ($i = 0; $i -lt $n; $i++) {
        $num += ($t[$i] - $mt) * ($y[$i] - $my)
        $den += ($t[$i] - $mt) * ($t[$i] - $mt)
    }
    if ($den -eq 0) { return 0.0 }
    return $num / $den * 3600.0
}
