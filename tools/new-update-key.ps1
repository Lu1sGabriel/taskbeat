# Gera o par de chaves que assina as atualizações do Taskbeat (ECDSA P-256).
#
# - A chave PRIVADA vai, cifrada com uma senha, para um arquivo FORA do
#   repositório (padrão: %USERPROFILE%\.taskbeat-keys\update-signing-key.pem),
#   só com acesso do seu usuário. Ela nunca entra no repositório nem no GitHub
#   (nem nos segredos do Actions): se a conta do GitHub for invadida, ninguém
#   consegue assinar uma atualização.
# - A chave PÚBLICA vai para assets\update-public-key.txt; ela entra no
#   repositório e no app, que só instala atualizações assinadas pela privada.
#
# Guarde uma cópia da chave privada e da senha (pen drive, gerenciador de
# senhas). Se ela se perder, as próximas versões não podem ser assinadas com
# ela, e trocar a chave exige que cada pessoa instale uma versão nova à mão.
#
# Uso (PowerShell 7, no seu terminal):
#   .\tools\new-update-key.ps1
#   .\tools\new-update-key.ps1 -KeyFile D:\chaves\taskbeat.pem
param(
    [string]$KeyFile = (Join-Path $env:USERPROFILE '.taskbeat-keys\update-signing-key.pem'),
    [string]$PublicFile = (Join-Path $PSScriptRoot '..\assets\update-public-key.txt'),
    # Só para testes (chave descartável sem senha).
    [switch]$NoPassword
)
#Requires -Version 7
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'update-signing.ps1')

$repo = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$KeyFile = [System.IO.Path]::GetFullPath($KeyFile)
$PublicFile = [System.IO.Path]::GetFullPath($PublicFile)
if ($KeyFile.StartsWith($repo.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw "a chave privada não pode ficar dentro do repositório ($repo)"
}
if (Test-Path -LiteralPath $KeyFile) {
    throw "já existe uma chave em $KeyFile; não sobrescrevo (perder a chave impede assinar as próximas versões)"
}

$password = $null
if (-not $NoPassword) {
    $a = [System.Net.NetworkCredential]::new('', (Read-Host -AsSecureString 'Senha para a chave (12+ caracteres)')).Password
    $b = [System.Net.NetworkCredential]::new('', (Read-Host -AsSecureString 'Repita a senha')).Password
    if ($a -ne $b) { throw 'as senhas não conferem' }
    if ($a.Length -lt 12) { throw 'senha curta demais (use 12 caracteres ou mais)' }
    $password = $a
}

$pem, $pub = [TaskbeatSigning]::Create($password)
New-Item -ItemType Directory -Force (Split-Path -Parent $KeyFile) | Out-Null
Set-Content -LiteralPath $KeyFile -Value $pem -Encoding ascii -NoNewline
# Só o seu usuário lê o arquivo (sem herdar permissões da pasta).
$acl = New-Object System.Security.AccessControl.FileSecurity
$acl.SetAccessRuleProtection($true, $false)
$me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule($me, 'FullControl', 'Allow')))
Set-Acl -LiteralPath $KeyFile -AclObject $acl

# Confere o arquivo gravado antes de publicar a chave pública.
$check = 'taskbeat-update-v1 teste'
if (-not [TaskbeatSigning]::Verify($pub, $check, [TaskbeatSigning]::Sign((Get-Content -LiteralPath $KeyFile -Raw), $password, $check))) {
    throw 'a chave gravada não confere'
}

$header = @'
# Chave pública que confere as atualizações do Taskbeat: ECDSA P-256, X‖Y em
# hexadecimal (128 dígitos) na primeira linha sem "#".
# Gerada por tools/new-update-key.ps1. A chave privada fica só com o mantenedor,
# fora do repositório. Sem chave aqui, o app procura versões novas mas não
# avisa nem instala nenhuma (nada pode ser conferido).
'@
Set-Content -LiteralPath $PublicFile -Value ($header.TrimEnd() + "`n" + $pub + "`n") -Encoding utf8NoBOM -NoNewline

$fp = (Get-FileHash -InputStream ([System.IO.MemoryStream]::new([System.Text.Encoding]::ASCII.GetBytes($pub))) -Algorithm SHA256).Hash.Substring(0, 16).ToLowerInvariant()
"Chave privada: $KeyFile"
"Chave pública: $PublicFile (impressão digital $fp)"
''
'Agora:'
'  1. Guarde uma cópia da chave privada e da senha fora deste PC.'
'  2. Faça o commit de assets\update-public-key.txt.'
'  3. O build-installer.ps1 assina cada release com ela (pede a senha).'
