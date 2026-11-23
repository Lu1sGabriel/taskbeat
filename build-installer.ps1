# Gera dist\Taskbeat-Setup-<versão>.exe.
# O próprio taskbeat.exe é o instalador: com "setup" no nome do arquivo ele abre
# a tela de instalação (veja src/setup.rs).
# Uso: .\build-installer.ps1
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "falha ao compilar" }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\Taskbeat-Setup-$version.exe"
Copy-Item target\release\taskbeat.exe $out -Force
"Pronto: $out ({0:N1} MB)" -f ((Get-Item $out).Length / 1MB)
