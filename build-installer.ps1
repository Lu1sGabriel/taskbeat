# Gera dist\Taskbeat-Setup-<versão>.exe e a assinatura dele (.sig).
# O próprio taskbeat.exe é o instalador: com "setup" no nome do arquivo ele abre
# a tela de instalação (veja src/setup.rs).
#
# A assinatura é o que deixa o app instalar a versão sozinho (src/update): ele
# só aceita um exe assinado pela chave privada que é par da pública embutida
# (assets\update-public-key.txt). A chave privada fica fora do repositório:
# por padrão em %USERPROFILE%\.taskbeat-keys\update-signing-key.pem, ou onde
# -SigningKey / $env:TASKBEAT_SIGNING_KEY indicar. A senha é pedida (ou vem de
# $env:TASKBEAT_SIGNING_PASSWORD). Publique o .exe e o .sig na release.
#
# Uso (PowerShell 7): .\build-installer.ps1
#                     .\build-installer.ps1 -NoSign   (sem .sig: o app não instala essa versão sozinho)
param(
    [string]$SigningKey = $(if ($env:TASKBEAT_SIGNING_KEY) { $env:TASKBEAT_SIGNING_KEY } else { Join-Path $env:USERPROFILE '.taskbeat-keys\update-signing-key.pem' }),
    [switch]$NoSign
)
#Requires -Version 7
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "falha ao compilar" }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\Taskbeat-Setup-$version.exe"
Copy-Item target\release\taskbeat.exe $out -Force
Remove-Item "$out.sig" -ErrorAction SilentlyContinue
"Pronto: $out ({0:N1} MB)" -f ((Get-Item $out).Length / 1MB)

if ($NoSign) {
    Write-Warning "sem assinatura: o app não instala esta versão sozinho"
    return
}
. (Join-Path $PSScriptRoot 'tools\update-signing.ps1')
$pub = Get-EmbeddedPublicKey (Join-Path $PSScriptRoot 'assets\update-public-key.txt')
if (-not $pub) { throw "assets\update-public-key.txt sem chave pública (gere com tools\new-update-key.ps1)" }
$sig = New-UpdateSignature -ExePath (Resolve-Path $out).Path -Version $version -KeyFile $SigningKey -PublicKey $pub
"Assinado: $sig"
