# Gera as imagens da Chrome Web Store nesta pasta:
#   screenshot-pt.png / screenshot-en.png  (1280x800, capturas)
#   tile-pt.png / tile-en.png              (440x280, bloco promocional pequeno)
#   icon-store-128.png                     (128x128, ícone com a margem que a loja pede)
# O card e o widget são desenhados pelo próprio app (teste `store_parts`, com a
# música de exemplo do README), e a montagem é o template.html, fotografado
# pelo Edge sem janela, com um perfil temporário (o seu não é usado).
# Uso (na raiz do projeto): .\extension\store\images\make-images.ps1
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$root = Resolve-Path (Join-Path $here "..\..\..")
$parts = Join-Path $here "parts"
New-Item -ItemType Directory -Force $parts | Out-Null

Push-Location $root
try {
  $env:TASKBEAT_SHOTS_DIR = $parts
  $env:TASKBEAT_LANG = "pt"
  cargo test --release store_parts -- --ignored --quiet
  if ($LASTEXITCODE -ne 0) { throw "falha ao desenhar o card" }
} finally {
  Remove-Item Env:TASKBEAT_SHOTS_DIR, Env:TASKBEAT_LANG -ErrorAction SilentlyContinue
  Pop-Location
}

$edge = @(
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe",
  "$env:ProgramFiles\Google\Chrome\Application\chrome.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $edge) { throw "Edge ou Chrome não encontrado" }
$profile = Join-Path ([IO.Path]::GetTempPath()) "taskbeat-store-shots"
$template = (Resolve-Path (Join-Path $here "template.html")).Path -replace '\\', '/'

Add-Type -AssemblyName System.Drawing
function Save-Opaque($src, $dst) {
  # A loja quer PNG sem transparência.
  $img = [System.Drawing.Image]::FromFile($src)
  $bmp = New-Object System.Drawing.Bitmap $img.Width, $img.Height, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.DrawImage($img, 0, 0, $img.Width, $img.Height); $g.Dispose(); $img.Dispose()
  $bmp.Save($dst, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  Remove-Item $src
}

foreach ($shot in @(
  @{ kind = "shot"; w = 1280; h = 800; name = "screenshot" },
  @{ kind = "tile"; w = 440; h = 280; name = "tile" }
)) {
  foreach ($lang in "pt", "en") {
    $raw = Join-Path $here "$($shot.name)-$lang.raw.png"
    & $edge --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --allow-file-access-from-files `
      "--user-data-dir=$profile" "--window-size=$($shot.w),$($shot.h)" "--screenshot=$raw" `
      "file:///$template`?kind=$($shot.kind)&lang=$lang" 2>$null | Out-Null
    for ($i = 0; $i -lt 50 -and -not (Test-Path $raw); $i++) { Start-Sleep -Milliseconds 100 }
    Save-Opaque $raw (Join-Path $here "$($shot.name)-$lang.png")
  }
}

# Ícone da loja: 96x96 de arte com 16 px de margem transparente.
$icon = [System.Drawing.Image]::FromFile((Resolve-Path (Join-Path $here "..\..\icons\icon-128.png")))
$bmp = New-Object System.Drawing.Bitmap 128, 128, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
$g.DrawImage($icon, 16, 16, 96, 96); $g.Dispose(); $icon.Dispose()
$bmp.Save((Join-Path $here "icon-store-128.png"), [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()

Remove-Item -Recurse -Force $profile -ErrorAction SilentlyContinue
Get-ChildItem $here -Filter *.png | ForEach-Object {
  $i = [System.Drawing.Image]::FromFile($_.FullName); "{0}: {1}x{2}" -f $_.Name, $i.Width, $i.Height; $i.Dispose()
}
