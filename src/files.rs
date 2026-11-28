//! Gravação de arquivos que não perde o conteúdo antigo no meio do caminho, e
//! um registro comum aos três sistemas para o código compartilhado.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Grava `bytes` em `path` sem nunca deixar o arquivo pela metade: escreve um
/// `.tmp` no **mesmo diretório** (o nome leva o PID, para duas threads ou dois
/// processos não colidirem), força para o disco e troca com `rename`.
///
/// No Windows o `rename` do std usa `MoveFileExW(REPLACE_EXISTING)` (e tenta de
/// novo com a semântica POSIX num `ACCESS_DENIED`). Num `SHARING_VIOLATION`
/// (outro programa com o arquivo aberto) ele devolve `Err`: aí o `.tmp` fica,
/// o destino continua o antigo e quem chama registra no log.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// `<nome>.<pid>.tmp` ao lado de `path`.
pub fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}.tmp", std::process::id()));
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
        assert!(!tmp_path(&p).exists());
        // Cria a pasta que falta.
        let q = d.join("sub").join("b.toml");
        write_atomic(&q, b"x").unwrap();
        assert_eq!(std::fs::read(&q).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tmp_leva_o_pid_e_fica_na_mesma_pasta() {
        let p = Path::new("/x/y/config.toml");
        let t = tmp_path(p);
        assert_eq!(t.parent(), p.parent());
        assert_eq!(t.file_name().unwrap().to_string_lossy(), format!("config.toml.{}.tmp", std::process::id()));
    }
}
