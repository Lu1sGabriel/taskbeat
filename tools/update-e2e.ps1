# Teste de ponta a ponta do atualizador (Windows), num diretório de teste e
# com um "GitHub" falso em 127.0.0.1. Nada é publicado; o Taskbeat instalado,
# a config e "Apps instalados" de verdade não são tocados.
#
# 1. Compila duas cópias com `--features e2e` (aceita o servidor local e uma
#    chave de teste): a "instalada", na versão do Cargo.toml, e a "nova".
# 2. Gera uma chave descartável e assina a nova com o mesmo código do
#    build-installer.ps1.
# 3. Abre a cópia instalada isolada: APPDATA, LOCALAPPDATA, instância única e
#    chave de "Apps instalados" próprias; sem widget nem aviso de teclas.
# 4. Espera o pop-up, clica em "Atualizar agora" e confere: exe trocado,
#    processo novo na versão nova, "Apps instalados", Taskbeat.old apagado.
# 5. Releases com o exe alterado depois de assinado, sem .sig, assinada por
#    outra chave e com o .sig de outra versão: nada aparece nem é instalado, e
#    o registro diz por quê.
#
# Uso (PowerShell 7): .\tools\update-e2e.ps1 [-Root pasta] [-Port 8765] [-SkipBuild]
# Capturas do pop-up e registros ficam em <Root>.
param(
    [string]$Root = (Join-Path ([System.IO.Path]::GetTempPath()) 'taskbeat-update-e2e'),
    [int]$Port = 8765,
    [switch]$SkipBuild
)
#Requires -Version 7
$ErrorActionPreference = 'Stop'
$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'update-signing.ps1')

Add-Type -AssemblyName System.Drawing
if (-not ('E2E' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class E2E {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    // 5 = aceita avisos; 2/3/4/7 = tela cheia ou apresentação (o pop-up espera).
    [DllImport("shell32.dll")] public static extern int SHQueryUserNotificationState(out int state);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static IntPtr Find(string cls, int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            var sb = new StringBuilder(64);
            GetClassName(h, sb, 64);
            uint p; GetWindowThreadProcessId(h, out p);
            if (sb.ToString() == cls && (pid == 0 || p == (uint)pid) && IsWindowVisible(h)) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
}
# Coordenadas físicas (a janela do pop-up é DPI-aware).
[void][E2E]::SetProcessDpiAwarenessContext([IntPtr]-4)

$results = [System.Collections.Generic.List[object]]::new()
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Ok = $ok; Teste = $name; Detalhe = $detail })
    $mark = if ($ok) { 'ok  ' } else { 'FALHOU' }
    Write-Host "[$mark] $name $(if ($detail) { "($detail)" })"
}

function Wait-Until([scriptblock]$cond, [double]$seconds) {
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ($t.Elapsed.TotalSeconds -lt $seconds) {
        $v = & $cond
        if ($v) { return $v }
        Start-Sleep -Milliseconds 250
    }
    $null
}

function Shot([IntPtr]$hwnd, [string]$name) {
    $r = New-Object E2E+RECT
    [void][E2E]::GetWindowRect($hwnd, [ref]$r)
    $w, $h = ($r.Right - $r.Left), ($r.Bottom - $r.Top)
    # 24 bits: com alfa, os pixels das janelas "layered" enganam (riscos brancos).
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
    $path = Join-Path $Root "shots\$name.png"
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    $path
}

# Clique de verdade na janela (mensagens do mouse, sem mexer no cursor).
function Click([IntPtr]$hwnd, [int]$x, [int]$y) {
    $lp = [IntPtr](($y -shl 16) -bor ($x -band 0xffff))
    [void][E2E]::PostMessage($hwnd, 0x0200, [IntPtr]0, $lp)   # WM_MOUSEMOVE
    Start-Sleep -Milliseconds 150
    [void][E2E]::PostMessage($hwnd, 0x0201, [IntPtr]1, $lp)   # WM_LBUTTONDOWN
    Start-Sleep -Milliseconds 80
    [void][E2E]::PostMessage($hwnd, 0x0202, [IntPtr]0, $lp)   # WM_LBUTTONUP
}

function Stop-TestApps {
    $ps = @(Get-Process taskbeat -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($Root, [StringComparison]::OrdinalIgnoreCase) })
    $ps | Stop-Process -Force
    $ps | ForEach-Object { $_.WaitForExit(5000) | Out-Null }
    Start-Sleep -Milliseconds 300
}

# ---------------------------------------------------------------- preparo

$version = (Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)$') { throw "versão inesperada no Cargo.toml: $version" }
$old = $version
$new = "$($Matches[1]).$($Matches[2]).$([int]$Matches[3] + 1)"
$bad = "$($Matches[1]).$($Matches[2]).$([int]$Matches[3] + 2)"
"Versão instalada: $old; nova: $new; inválidas: $bad"

Stop-TestApps
# Com -SkipBuild, reaproveita os executáveis da última vez (pasta bin).
$clean = Wait-Until {
    if (Test-Path $Root) { Get-ChildItem $Root -Force | Where-Object { -not ($SkipBuild -and $_.Name -eq 'bin') } | Remove-Item -Recurse -Force -ErrorAction SilentlyContinue }
    -not (Test-Path $Root) -or -not (Get-ChildItem $Root -Force | Where-Object { $_.Name -ne 'bin' })
} 10
if (-not $clean) { throw "não deu para limpar $Root (algum arquivo em uso?)" }
foreach ($d in 'bin', 'keys', 'install', 'appdata\Taskbeat', 'localappdata', 'www', 'shots') { New-Item -ItemType Directory -Force (Join-Path $Root $d) | Out-Null }

$target = Join-Path $repo 'target\e2e'
if (-not $SkipBuild) {
    foreach ($v in $old, $new) {
        "Compilando $v (release, --features e2e)…"
        $env:TASKBEAT_E2E_VERSION = $v
        & cargo build --release --features e2e --target-dir $target --manifest-path (Join-Path $repo 'Cargo.toml') 2>&1 | Select-String -Pattern '^(error|warning: unused)' | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'falha ao compilar' }
        Copy-Item (Join-Path $target 'release\taskbeat.exe') (Join-Path $Root "bin\$v.exe")
    }
    Remove-Item Env:TASKBEAT_E2E_VERSION
} elseif (-not ((Test-Path (Join-Path $Root "bin\$old.exe")) -and (Test-Path (Join-Path $Root "bin\$new.exe")))) {
    throw "sem executáveis de uma execução anterior em $Root (pasta bin): rode sem -SkipBuild"
}

& (Join-Path $PSScriptRoot 'new-update-key.ps1') -KeyFile (Join-Path $Root 'keys\e2e.pem') -PublicFile (Join-Path $Root 'keys\e2e-pub.txt') -NoPassword | Out-Null
& (Join-Path $PSScriptRoot 'new-update-key.ps1') -KeyFile (Join-Path $Root 'keys\outra.pem') -PublicFile (Join-Path $Root 'keys\outra-pub.txt') -NoPassword | Out-Null
$pub = Get-EmbeddedPublicKey (Join-Path $Root 'keys\e2e-pub.txt')
$otherPub = Get-EmbeddedPublicKey (Join-Path $Root 'keys\outra-pub.txt')

# Config isolada: sem widget, sem aviso de teclas, sem barras; em português.
@"
show_media = false
always_show_widget = false
caps_lock_osd = false
num_lock_osd = false
next_track_popup = false
show_visualizer = false
card_viz = false
login_default_applied = true
language = "pt"
update_check = true
update_auto = false
version = 5
"@ | Set-Content (Join-Path $Root 'appdata\Taskbeat\config.toml') -Encoding utf8NoBOM

$regKey = 'HKCU:\Software\Taskbeat-Test\e2e\Uninstall'
$api = "http://127.0.0.1:$Port/repos/Lu1sGabriel/taskbeat"
$appEnv = @{
    APPDATA                = (Join-Path $Root 'appdata')
    LOCALAPPDATA           = (Join-Path $Root 'localappdata')
    TASKBEAT_INSTANCE      = 'e2e'
    TASKBEAT_UPDATE_API    = $api
    TASKBEAT_UPDATE_PUBKEY = $pub
    TASKBEAT_UPDATE_DELAY  = '2'
    TASKBEAT_LOG           = (Join-Path $Root 'taskbeat.log')
}
$exe = Join-Path $Root 'install\Taskbeat.exe'
$log = Join-Path $Root 'appdata\Taskbeat\update.log'

# Release falsa: o JSON que a API do GitHub devolveria e os arquivos.
function Publish([string]$v, [string]$exePath, [string]$sigPath) {
    $dl = Join-Path $Root "www\dl\v$v"
    New-Item -ItemType Directory -Force $dl | Out-Null
    $name = "Taskbeat-Setup-$v.exe"
    Copy-Item $exePath (Join-Path $dl $name) -Force
    $assets = @(@{ name = $name; size = (Get-Item (Join-Path $dl $name)).Length; browser_download_url = "http://127.0.0.1:$Port/dl/v$v/$name" })
    if ($sigPath) {
        Copy-Item $sigPath (Join-Path $dl "$name.sig") -Force
        $assets += @{ name = "$name.sig"; size = (Get-Item (Join-Path $dl "$name.sig")).Length; browser_download_url = "http://127.0.0.1:$Port/dl/v$v/$name.sig" }
    }
    $rel = [ordered]@{
        tag_name = "v$v"; name = "Taskbeat $v"; draft = $false; prerelease = $false
        html_url = "https://github.com/Lu1sGabriel/taskbeat/releases/tag/v$v"
        body = "Versão de teste do **atualizador**: aviso, assinatura conferida e troca do exe com o app aberto.`r`n`r`n## Novidades`r`n- nada de verdade"
        assets = $assets
    }
    $dir = Join-Path $Root 'www\repos\Lu1sGabriel\taskbeat\releases'
    New-Item -ItemType Directory -Force $dir | Out-Null
    $rel | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $dir 'latest') -Encoding utf8NoBOM
}

function Reset-Install {
    Stop-TestApps
    # O Windows pode segurar o .exe um instante depois de o processo fechar.
    $empty = Wait-Until { Get-ChildItem (Join-Path $Root 'install') -Force | Remove-Item -Force -ErrorAction SilentlyContinue; -not (Get-ChildItem (Join-Path $Root 'install') -Force) } 10
    if (-not $empty) { throw 'não deu para limpar a pasta de instalação de teste' }
    Copy-Item (Join-Path $Root "bin\$old.exe") $exe
    New-Item -Force $regKey | Out-Null
    Set-ItemProperty $regKey -Name InstallLocation -Value (Join-Path $Root 'install')
    Set-ItemProperty $regKey -Name DisplayVersion -Value $old
    Remove-Item -Recurse -Force (Join-Path $Root 'localappdata\Taskbeat') -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $Root 'appdata\Taskbeat\update.toml'), $log -ErrorAction SilentlyContinue
}

$server = Start-Process python -ArgumentList '-I', '-m', 'http.server', $Port, '--bind', '127.0.0.1', '--directory', (Join-Path $Root 'www') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $Root 'server.log')
Start-Sleep -Seconds 1
try {
    # ------------------------------------------------------------ 1. atualização de verdade
    $newExe = Join-Path $Root "bin\Taskbeat-Setup-$new.exe"
    Copy-Item (Join-Path $Root "bin\$new.exe") $newExe -Force
    $sig = New-UpdateSignature -ExePath $newExe -Version $new -KeyFile (Join-Path $Root 'keys\e2e.pem') -PublicKey $pub
    Publish $new $newExe $sig
    Reset-Install
    $hashOld = (Get-FileHash $exe).Hash
    $hashNew = (Get-FileHash $newExe).Hash

    $p = Start-Process $exe -Environment $appEnv -PassThru
    $pop = Wait-Until { $h = [E2E]::Find('TaskbeatUpdate', $p.Id); if ($h -ne [IntPtr]::Zero) { $h } } 40
    Check 'pop-up aparece com a versão nova' ($null -ne $pop)
    if ($pop) {
        Start-Sleep -Milliseconds 900
        Check 'captura do pop-up' $true (Shot $pop '1-oferta')
        $r = New-Object E2E+RECT
        [void][E2E]::GetWindowRect($pop, [ref]$r)
        $w, $h = ($r.Right - $r.Left), ($r.Bottom - $r.Top)
        $s = $w / (404 + 2 * 26)
        # "Atualizar agora": primeiro botão, embaixo à esquerda do cartão.
        Click $pop ([int](70 * $s)) ([int]($h - 56 * $s))
        Start-Sleep -Milliseconds 400
        if ([E2E]::IsWindowVisible($pop)) { Check 'captura instalando' $true (Shot $pop '2-instalando') }
    }
    $gone = Wait-Until { $p.HasExited } 25
    Check 'versão antiga fecha' ([bool]$gone)
    # O processo antigo ainda aparece na lista enquanto este script segura o handle dele.
    $p2 = Wait-Until { Get-Process taskbeat -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $p.Id -and $_.Path -eq $exe -and -not $_.HasExited } | Select-Object -First 1 } 25
    Check 'versão nova abre sozinha' ($null -ne $p2)
    Check 'exe instalado é o novo' ((Get-FileHash $exe).Hash -eq $hashNew)
    Check '"Apps instalados" com a versão nova' ((Get-ItemProperty $regKey).DisplayVersion -eq $new) (Get-ItemProperty $regKey).DisplayVersion
    if ($p2) {
        $toast = Wait-Until { $h = [E2E]::Find('TaskbeatUpdate', $p2.Id); if ($h -ne [IntPtr]::Zero) { $h } } 15
        $quns = 0
        [void][E2E]::SHQueryUserNotificationState([ref]$quns)
        Check 'aviso "atualizado" na versão nova' ($null -ne $toast) $(if (-not $toast) { "estado de avisos do Windows: $quns; processo vivo: $(-not $p2.HasExited)" })
        if ($toast) { Start-Sleep -Milliseconds 900; Check 'captura atualizado' $true (Shot $toast '3-atualizado') }
    }
    $oldGone = Wait-Until { -not (Test-Path (Join-Path $Root 'install\Taskbeat.old')) } 15
    Check 'Taskbeat.old apagado na abertura' ([bool]$oldGone)
    Check 'licenças gravadas pela versão nova' (Test-Path (Join-Path $Root 'install\LICENSE.txt'))
    Check 'download apagado depois de instalar' (-not (Test-Path (Join-Path $Root 'localappdata\Taskbeat\Updates')))
    $text = Get-Content $log -Raw -ErrorAction SilentlyContinue
    Check 'registro: instalado e atualizado' ($text -match "instalado: $([regex]::Escape($new))" -and $text -match "atualizado de $([regex]::Escape($old)) para $([regex]::Escape($new))")
    Stop-TestApps

    # ------------------------------------------------------------ 2. releases recusadas
    $badExe = Join-Path $Root "bin\Taskbeat-Setup-$bad.exe"
    $cases = @(
        @{ Name = 'exe alterado depois de assinado'; Expect = 'assinatura inválida'; Make = {
                Copy-Item (Join-Path $Root "bin\$new.exe") $badExe -Force
                $s = New-UpdateSignature -ExePath $badExe -Version $bad -KeyFile (Join-Path $Root 'keys\e2e.pem') -PublicKey $pub
                [System.IO.File]::AppendAllText($badExe, 'x')
                $s } },
        @{ Name = 'sem .sig'; Expect = '\.sig: não instala'; Make = { Copy-Item (Join-Path $Root "bin\$new.exe") $badExe -Force; $null } },
        @{ Name = 'assinado por outra chave'; Expect = 'assinatura inválida'; Make = {
                Copy-Item (Join-Path $Root "bin\$new.exe") $badExe -Force
                New-UpdateSignature -ExePath $badExe -Version $bad -KeyFile (Join-Path $Root 'keys\outra.pem') -PublicKey $otherPub } },
        @{ Name = 'exe antigo com o .sig de outra versão'; Expect = 'assinatura inválida'; Make = {
                Copy-Item $newExe $badExe -Force
                $s = "$badExe.sig"
                Copy-Item "$newExe.sig" $s -Force
                $s } }
    )
    foreach ($c in $cases) {
        Remove-Item "$badExe.sig" -ErrorAction SilentlyContinue
        $sigPath = & $c.Make
        Publish $bad $badExe $sigPath
        Reset-Install
        $p = Start-Process $exe -Environment $appEnv -PassThru
        $refused = Wait-Until { (Get-Content $log -Raw -ErrorAction SilentlyContinue) -match $c.Expect } 30
        $shown = [E2E]::Find('TaskbeatUpdate', $p.Id) -ne [IntPtr]::Zero
        Check "recusa: $($c.Name)" ([bool]$refused -and -not $shown) ((Get-Content $log -ErrorAction SilentlyContinue | Select-Object -Last 1) -replace '^\d+ \S+ ', '')
        Check "  nada instalado ($($c.Name))" ((Get-FileHash $exe).Hash -eq $hashOld -and -not (Test-Path (Join-Path $Root 'localappdata\Taskbeat\Updates')))
        Stop-TestApps
    }
}
finally {
    Stop-TestApps
    if ($server -and -not $server.HasExited) { Stop-Process $server -Force }
    Remove-Item -Recurse -Force 'HKCU:\Software\Taskbeat-Test' -ErrorAction SilentlyContinue
}

$failed = @($results | Where-Object { -not $_.Ok })
''
"$($results.Count - $failed.Count) de $($results.Count) conferências ok. Capturas e registros: $Root"
if ($failed.Count) { exit 1 }
