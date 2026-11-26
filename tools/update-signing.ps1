# Assinatura das atualizações do Taskbeat (ECDSA P-256 + SHA-256), com a
# criptografia do .NET. Usado pelo build-installer.ps1, pelo new-update-key.ps1
# e pelo teste de ponta a ponta; carregue com `. tools\update-signing.ps1`.
#
# O app (src/update) confere com a CNG do Windows a assinatura desta mensagem:
#
#   taskbeat-update-v1
#   file: Taskbeat-Setup-<versão>.exe
#   version: <versão>
#   sha256: <sha-256 do exe, hexadecimal minúsculo>
#
# (quebras de linha \n, inclusive no fim). O .sig é a assinatura r‖s (64
# bytes) em hexadecimal, numa linha.

#Requires -Version 7

if (-not ('TaskbeatSigning' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Security.Cryptography;
using System.Text;

public static class TaskbeatSigning
{
    static string Hex(byte[] b) => Convert.ToHexString(b).ToLowerInvariant();

    static ECDsa Load(string pem, string password)
    {
        var k = ECDsa.Create();
        if (pem.Contains("ENCRYPTED PRIVATE KEY"))
        {
            if (string.IsNullOrEmpty(password)) throw new ArgumentException("a chave tem senha");
            k.ImportFromEncryptedPem(pem, password);
        }
        else
        {
            k.ImportFromPem(pem);
        }
        if (k.KeySize != 256) throw new ArgumentException("a chave não é P-256");
        return k;
    }

    static string PublicHex(ECDsa k)
    {
        var p = k.ExportParameters(false);
        return Hex(p.Q.X) + Hex(p.Q.Y);
    }

    /// Par novo: [PEM da chave privada (cifrada se houver senha), chave pública X‖Y em hex].
    public static string[] Create(string password)
    {
        using (var k = ECDsa.Create(ECCurve.NamedCurves.nistP256))
        {
            string pem = string.IsNullOrEmpty(password)
                ? k.ExportPkcs8PrivateKeyPem()
                : k.ExportEncryptedPkcs8PrivateKeyPem(password, new PbeParameters(PbeEncryptionAlgorithm.Aes256Cbc, HashAlgorithmName.SHA256, 600000));
            return new[] { pem, PublicHex(k) };
        }
    }

    public static string PublicOf(string pem, string password)
    {
        using (var k = Load(pem, password)) { return PublicHex(k); }
    }

    /// Assinatura r‖s (IEEE P1363, 64 bytes) de SHA-256(mensagem), em hex.
    public static string Sign(string pem, string password, string message)
    {
        using (var k = Load(pem, password))
        {
            return Hex(k.SignData(Encoding.UTF8.GetBytes(message), HashAlgorithmName.SHA256));
        }
    }

    public static bool Verify(string publicHex, string message, string sigHex)
    {
        var q = Convert.FromHexString(publicHex.Trim());
        if (q.Length != 64) return false;
        var x = new byte[32];
        var y = new byte[32];
        Array.Copy(q, 0, x, 0, 32);
        Array.Copy(q, 32, y, 0, 32);
        var p = new ECParameters { Curve = ECCurve.NamedCurves.nistP256, Q = new ECPoint { X = x, Y = y } };
        using (var k = ECDsa.Create(p))
        {
            return k.VerifyData(Encoding.UTF8.GetBytes(message), Convert.FromHexString(sigHex.Trim()), HashAlgorithmName.SHA256);
        }
    }
}
'@
}

# Mensagem assinada (igual à de src/update/mod.rs: signed_message).
function Get-UpdateMessage([string]$FileName, [string]$Version, [string]$ExePath) {
    $sha = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "taskbeat-update-v1`nfile: $FileName`nversion: $Version`nsha256: $sha`n"
}

# Chave pública embutida no app: a primeira linha sem "#" do arquivo.
function Get-EmbeddedPublicKey([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $line = Get-Content -LiteralPath $Path | ForEach-Object { $_.Trim() } | Where-Object { $_ -and -not $_.StartsWith('#') } | Select-Object -First 1
    if ($line -match '^[0-9a-fA-F]{128}$') { $line.ToLowerInvariant() } else { $null }
}

# Senha da chave: $env:TASKBEAT_SIGNING_PASSWORD ou pergunta (sem eco).
function Read-SigningPassword([string]$Pem) {
    if (-not $Pem.Contains('ENCRYPTED PRIVATE KEY')) { return $null }
    if ($env:TASKBEAT_SIGNING_PASSWORD) { return $env:TASKBEAT_SIGNING_PASSWORD }
    $s = Read-Host -AsSecureString 'Senha da chave de assinatura'
    [System.Net.NetworkCredential]::new('', $s).Password
}

# Assina $ExePath como "$FileName" na versão $Version e grava "$ExePath.sig".
# Recusa se a chave privada não for a par da pública embutida no app, e confere
# a assinatura gerada antes de gravar.
function New-UpdateSignature {
    param(
        [Parameter(Mandatory)][string]$ExePath,
        [Parameter(Mandatory)][string]$Version,
        [Parameter(Mandatory)][string]$KeyFile,
        [Parameter(Mandatory)][string]$PublicKey,
        [string]$FileName = (Split-Path -Leaf $ExePath)
    )
    if (-not (Test-Path -LiteralPath $KeyFile)) {
        throw "chave de assinatura não encontrada: $KeyFile (gere com tools\new-update-key.ps1)"
    }
    $pem = Get-Content -LiteralPath $KeyFile -Raw
    $password = Read-SigningPassword $pem
    $pub = [TaskbeatSigning]::PublicOf($pem, $password)
    if ($pub -ne $PublicKey) {
        throw "a chave privada em $KeyFile não é a par da chave pública embutida no app; o app recusaria esta assinatura"
    }
    $msg = Get-UpdateMessage $FileName $Version $ExePath
    $sig = [TaskbeatSigning]::Sign($pem, $password, $msg)
    if (-not [TaskbeatSigning]::Verify($PublicKey, $msg, $sig)) { throw 'a assinatura gerada não confere' }
    Set-Content -LiteralPath "$ExePath.sig" -Value $sig -Encoding ascii -NoNewline
    Add-Content -LiteralPath "$ExePath.sig" -Value "`n" -Encoding ascii -NoNewline
    "$ExePath.sig"
}
