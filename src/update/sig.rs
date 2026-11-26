//! Conferência da assinatura das atualizações com a CNG do Windows (BCrypt):
//! SHA-256 e ECDSA P-256. Nenhuma criptografia feita à mão.

use super::{Version, hex, signed_message, unhex};
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::Security::Cryptography::*;

fn ok(s: NTSTATUS) -> bool {
    s.0 >= 0
}

/// SHA-256 dos bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let st = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    assert!(ok(st), "BCryptHash falhou: {:#x}", st.0);
    out
}

/// Chave pública no formato da CNG (cabeçalho + X‖Y).
fn public_blob(xy: &[u8; 64]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(8 + 64);
    blob.extend_from_slice(&BCRYPT_ECDSA_PUBLIC_P256_MAGIC.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(xy);
    blob
}

/// A assinatura `sig` (r‖s, 64 bytes) de `msg` confere com a chave pública `xy`?
pub fn verify_p256(xy: &[u8; 64], msg: &[u8], sig: &[u8]) -> bool {
    if sig.len() != 64 {
        return false;
    }
    let hash = sha256(msg);
    unsafe {
        let mut key = BCRYPT_KEY_HANDLE::default();
        if !ok(BCryptImportKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, None, BCRYPT_ECCPUBLIC_BLOB, &mut key, &public_blob(xy), 0)) {
            return false;
        }
        let good = ok(BCryptVerifySignature(key, None, &hash, sig, BCRYPT_FLAGS(0)));
        let _ = BCryptDestroyKey(key);
        good
    }
}

/// Confere o instalador `exe` (nome `file`, versão `version`) contra o
/// conteúdo do `.sig` (r‖s em hexadecimal) e a chave pública embutida.
pub fn check(key: &[u8; 64], file: &str, version: &Version, exe: &[u8], sig_text: &[u8]) -> Result<(), String> {
    let sig = std::str::from_utf8(sig_text).ok().and_then(unhex).filter(|s| s.len() == 64).ok_or("assinatura mal formada (.sig)")?;
    let msg = signed_message(file, version, &hex(&sha256(exe)));
    if verify_p256(key, msg.as_bytes(), &sig) { Ok(()) } else { Err("assinatura inválida".into()) }
}

/// Par de chaves de teste, gerado na hora pela CNG (só nos testes).
#[cfg(test)]
pub struct TestKey(BCRYPT_KEY_HANDLE);

#[cfg(test)]
impl TestKey {
    pub fn new() -> Self {
        unsafe {
            let mut key = BCRYPT_KEY_HANDLE::default();
            assert!(ok(BCryptGenerateKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, &mut key, 256, 0)));
            assert!(ok(BCryptFinalizeKeyPair(key, 0)));
            TestKey(key)
        }
    }

    pub fn public(&self) -> [u8; 64] {
        let mut blob = [0u8; 72];
        let mut len = 0u32;
        let st = unsafe { BCryptExportKey(self.0, None, BCRYPT_ECCPUBLIC_BLOB, Some(&mut blob), &mut len, 0) };
        assert!(ok(st) && len == 72);
        blob[8..].try_into().unwrap()
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        let hash = sha256(msg);
        let mut sig = [0u8; 64];
        let mut len = 0u32;
        let st = unsafe { BCryptSignHash(self.0, None, &hash, Some(&mut sig), &mut len, BCRYPT_FLAGS(0)) };
        assert!(ok(st) && len == 64);
        sig
    }

    /// O `.sig` de um instalador, como o `build-installer.ps1` gera.
    pub fn sig_file(&self, file: &str, version: &Version, exe: &[u8]) -> String {
        hex(&self.sign(signed_message(file, version, &hex(&sha256(exe))).as_bytes())) + "\n"
    }
}

#[cfg(test)]
impl Drop for TestKey {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyKey(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_conhecido() {
        // Vetores do FIPS 180-2.
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn assinatura_valida_passa() {
        let k = TestKey::new();
        let v = Version::parse("1.4.0").unwrap();
        let exe = b"MZ conteudo do instalador".to_vec();
        let sig = k.sig_file("Taskbeat-Setup-1.4.0.exe", &v, &exe);
        assert_eq!(check(&k.public(), "Taskbeat-Setup-1.4.0.exe", &v, &exe, sig.as_bytes()), Ok(()));
        // Maiúsculas e sem a quebra de linha final também valem.
        assert_eq!(check(&k.public(), "Taskbeat-Setup-1.4.0.exe", &v, &exe, sig.trim().to_uppercase().as_bytes()), Ok(()));
    }

    /// Assinatura feita pelo `tools/update-signing.ps1` (.NET) com uma chave
    /// descartável: a CNG aceita exatamente o formato que o script de release gera.
    #[test]
    fn assinatura_do_script_de_release_confere() {
        let key: [u8; 64] = unhex("6b9c16b6b99b6b951414c721aabc6fbb465e7a1dae8cb872647d63d126232be12bed20c29da972074f9461a90b5bee595df3e242a862bf563bfeff700d0bf964").unwrap().try_into().unwrap();
        let sig = "b2c56b57cf0cab7fd45c0ab837398fb7ad80bee6a246b2193f627b9b3a7bc522269de760421b3eeed9c9529dabd38d819a96ed247602a1205dcfd63decf92c2e\n";
        let v = Version::parse("1.4.0").unwrap();
        let exe = b"MZ fake installer bytes";
        assert_eq!(hex(&sha256(exe)), "706ac79a1c87727e3f48df89fc37b494f1172bba1278ef1b00c852fdacfc8245");
        assert_eq!(check(&key, "Taskbeat-Setup-1.4.0.exe", &v, exe, sig.as_bytes()), Ok(()));
        assert!(check(&key, "Taskbeat-Setup-1.4.0.exe", &v, b"MZ fake installer byteZ", sig.as_bytes()).is_err());
    }

    #[test]
    fn qualquer_mudanca_e_recusada() {
        let k = TestKey::new();
        let other = TestKey::new();
        let v = Version::parse("1.4.0").unwrap();
        let file = "Taskbeat-Setup-1.4.0.exe";
        let exe = vec![7u8; 4096];
        let sig = k.sig_file(file, &v, &exe);
        let key = k.public();
        // Um byte do exe trocado.
        let mut bad = exe.clone();
        bad[100] ^= 1;
        assert!(check(&key, file, &v, &bad, sig.as_bytes()).is_err());
        // Exe certo, mas apresentado como outra versão (exe antigo republicado).
        let v2 = Version::parse("1.5.0").unwrap();
        assert!(check(&key, "Taskbeat-Setup-1.5.0.exe", &v2, &exe, sig.as_bytes()).is_err());
        assert!(check(&key, file, &v2, &exe, sig.as_bytes()).is_err());
        // Assinado por outra chave.
        assert!(check(&other.public(), file, &v, &exe, sig.as_bytes()).is_err());
        // Assinatura alterada, cortada, vazia ou que não é hexadecimal.
        let mut s = unhex(&sig).unwrap();
        s[10] ^= 0x80;
        assert!(check(&key, file, &v, &exe, hex(&s).as_bytes()).is_err());
        assert!(check(&key, file, &v, &exe, &sig.as_bytes()[..64]).is_err());
        assert!(check(&key, file, &v, &exe, b"").is_err());
        assert!(check(&key, file, &v, &exe, "zz".repeat(64).as_bytes()).is_err());
        // Chave pública que não é um ponto da curva.
        assert!(!verify_p256(&[0u8; 64], b"x", &unhex(&sig).unwrap()));
    }
}
