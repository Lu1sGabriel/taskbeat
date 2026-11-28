//! Gravação de arquivos que não perde o conteúdo antigo no meio do caminho, e
//! um registro comum aos três sistemas para o código compartilhado.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Grava `bytes` em `path` sem nunca deixar o arquivo pela metade: escreve um
/// `.tmp` no **mesmo diretório** (o nome leva o PID e um contador por gravação,
/// para dois processos ou duas threads não dividirem o mesmo `.tmp`), força
/// para o disco e troca com `rename`. Se `path` é um link simbólico (dotfiles),
/// grava no arquivo para onde ele aponta, e o link continua link.
///
/// No Windows o `rename` do std usa `MoveFileExW(REPLACE_EXISTING)` (e tenta de
/// novo com a semântica POSIX num `ACCESS_DENIED`). Num `SHARING_VIOLATION`
/// (outro programa com o arquivo aberto) ele devolve `Err`: aí o `.tmp` fica,
/// o destino continua o antigo e quem chama registra no log.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let real = resolve_link(path);
    let path = real.as_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        // Num SHARING_VIOLATION o `.tmp` completo fica para diagnóstico;
        // só some quando a própria gravação falhou no meio.
        if std::fs::metadata(&tmp).is_ok_and(|m| m.len() != bytes.len() as u64) {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    written
}

/// O arquivo de verdade por trás de um link simbólico (um link quebrado ou que
/// não dá para resolver fica como está).
fn resolve_link(path: &Path) -> PathBuf {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
        && let Ok(real) = std::fs::canonicalize(path)
    {
        return real;
    }
    path.to_path_buf()
}

/// `<nome>.<pid>.<n>.tmp` ao lado de `path` (`n` sobe a cada chamada).
pub fn tmp_path(path: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// Aviso no registro do sistema em uso (o mesmo `TASKBEAT_LOG` de sempre).
pub fn warn(msg: &str) {
    #[cfg(windows)]
    crate::audio::log::warn(msg);
    #[cfg(target_os = "macos")]
    crate::mac::system::log::warn(msg);
    #[cfg(target_os = "linux")]
    crate::linux::system::log::warn(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("taskbeat-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn troca_o_arquivo_inteiro_e_nao_deixa_tmp() {
        let d = dir("troca");
        let p = d.join("a.toml");
        std::fs::write(&p, "antigo").unwrap();
        write_atomic(&p, b"novo").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "novo");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "sobrou um .tmp");
        // Cria a pasta que falta.
        let q = d.join("sub").join("b.toml");
        write_atomic(&q, b"x").unwrap();
        assert_eq!(std::fs::read(&q).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tmp_leva_o_pid_e_um_contador_e_fica_na_mesma_pasta() {
        let p = Path::new("/x/y/config.toml");
        let (a, b) = (tmp_path(p), tmp_path(p));
        assert_ne!(a, b);
        assert_eq!(a.parent(), p.parent());
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(&format!("config.toml.{}.", std::process::id())) && name.ends_with(".tmp"), "{name}");
    }

    /// Várias threads gravando o mesmo arquivo: cada uma com o seu `.tmp`, e
    /// o resultado é sempre um dos conteúdos inteiros.
    #[test]
    fn threads_gravando_juntas_nao_misturam() {
        let d = dir("threads");
        let p = d.join("c.toml");
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    let body = vec![b'a' + i as u8; 1000 + i * 977];
                    for _ in 0..20 {
                        let _ = write_atomic(&p, &body);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let got = std::fs::read(&p).unwrap();
        let i = (got[0] - b'a') as usize;
        assert_eq!(got, vec![got[0]; 1000 + i * 977]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Um `config.toml` que é link (dotfiles) continua link.
    #[cfg(unix)]
    #[test]
    fn link_simbolico_continua_link() {
        let d = dir("link");
        let real = d.join("real.toml");
        std::fs::write(&real, "antigo").unwrap();
        let link = d.join("config.toml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomic(&link, b"novo").unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "novo");
        let _ = std::fs::remove_dir_all(&d);
    }
}
