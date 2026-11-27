# Gera dist\taskbeat-extension-<versão>.zip para enviar à Chrome Web Store (e,
# se quiser, à loja do Edge). Vai só o que a extensão usa; o manifest sai sem o
# campo "key" (a chave de desenvolvimento: a loja dá a própria, e o ID muda).
# Uso (na raiz do projeto, PowerShell 7): .\extension\store\pack.ps1
$ErrorActionPreference = "Stop"
$ext = Resolve-Path (Join-Path $PSScriptRoot "..")
$root = Resolve-Path (Join-Path $ext "..")

$manifest = Get-Content (Join-Path $ext "manifest.json") -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
$manifest.Remove("key")
$version = $manifest.version
$files = @("content.js", "sw.js") + (Get-ChildItem (Join-Path $ext "icons"), (Join-Path $ext "_locales") -Recurse -File | ForEach-Object {
  [IO.Path]::GetRelativePath($ext, $_.FullName)
})

New-Item -ItemType Directory -Force (Join-Path $root "dist") | Out-Null
$out = Join-Path $root "dist\taskbeat-extension-$version.zip"
if (Test-Path $out) { Remove-Item $out }

Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::Open($out, [IO.Compression.ZipArchiveMode]::Create)
try {
  # Nomes com "/" (o formato do zip), não "\".
  $entry = $zip.CreateEntry("manifest.json")
  $w = New-Object IO.StreamWriter($entry.Open(), (New-Object Text.UTF8Encoding $false))
  $w.Write(($manifest | ConvertTo-Json -Depth 10)); $w.Dispose()
  foreach ($f in $files) {
    [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, (Join-Path $ext $f), ($f -replace '\\', '/'))
  }
} finally {
  $zip.Dispose()
}
"Pronto: $out ({0:N1} KB, versão $version)" -f ((Get-Item $out).Length / 1KB)
"Arquivos: manifest.json, " + ($files -replace '\\', '/' -join ", ")
