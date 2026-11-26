//! Troca do executável no Windows, sem administrador (a instalação é por
//! usuário, em `%LOCALAPPDATA%\Programs\Taskbeat`):
//!
//! 1. o instalador novo e o `.sig` são baixados para
//!    `%LOCALAPPDATA%\Taskbeat\Updates` e só ficam lá se a assinatura conferir;
//! 2. na hora de instalar, a assinatura é conferida de novo nos bytes que
//!    serão gravados; o `Taskbeat.exe` em uso é renomeado para `Taskbeat.old`
//!    (o Windows deixa renomear um .exe aberto, não apagar) e o novo toma o
//!    lugar dele; a versão em "Apps instalados" é atualizada;
//! 3. o novo é aberto com `--after-update=<versão antiga>` e este fecha; o
//!    novo espera o antigo soltar a instância única, apaga o `Taskbeat.old`
//!    e acerta o registro.
//!
//! A configuração (`%APPDATA%\Taskbeat`) não é tocada.

use super::{MAX_EXE, Offer, Version, log, sig};
use crate::setup;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Mesmo id do item "Sair" do menu da bandeja (`tray.rs`).
const ID_EXIT: usize = 199;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const OLD: &str = "Taskbeat.old";
const NEW: &str = "Taskbeat.new";

/// Chave de "Apps instalados". No teste de ponta a ponta (`TASKBEAT_INSTANCE`)
/// fica fora da lista de apps, para não tocar na instalação de verdade.
pub fn uninstall_key() -> String {
    #[cfg(any(debug_assertions, feature = "e2e"))]
    if let Some(i) = super::test_env("TASKBEAT_INSTANCE") {
        return format!(r"Software\Taskbeat-Test\{i}\Uninstall");
    }
    setup::UNINSTALL_KEY.to_string()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Pasta da instalação, se este processo for o `Taskbeat.exe` instalado (e
/// não uma cópia solta, como a de `target\release` ou a de Downloads).
pub fn installed_dir() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = PathBuf::from(setup::reg_get_sz(&uninstall_key(), "InstallLocation").filter(|d| !d.is_empty())?);
    same_file(&me, &dir.join("Taskbeat.exe")).then_some(dir)
}

/// Onde os instaladores baixados esperam a pessoa decidir.
fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Taskbeat").join("Updates"))
}

fn http_get(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let r = crate::http::get(url, &[("Accept", "application/octet-stream")], max)?;
    if !r.ok() {
        return Err(format!("download respondeu {} ({url})", r.status));
    }
    Ok(r.body)
}

/// Instalador já conferido, guardado no cache.
pub struct Verified {
    pub version: Version,
    pub file: String,
    exe: PathBuf,
    sig: PathBuf,
}

/// Confere `exe` com o `.sig` e a chave embutida.
fn verify(file: &str, version: &Version, exe: &[u8], sig_text: &[u8]) -> Result<(), String> {
    let key = super::public_key().ok_or("o app não tem a chave pública das atualizações")?;
    sig::check(&key, file, version, exe, sig_text)
}

/// Baixa o instalador e a assinatura e confere. Só com a assinatura válida
/// os dois ficam no cache; senão, nada fica e o motivo vai para o registro.
pub fn download(o: &Offer) -> Result<Verified, String> {
    let sig_asset = o.sig.as_ref().ok_or_else(|| format!("{} sem {}.sig: não instala", o.version, o.file.name))?;
    let dir = cache_dir().ok_or("sem pasta para o download")?;
    let exe_path = dir.join(&o.file.name);
    let sig_path = dir.join(&sig_asset.name);
    let v = Verified { version: o.version.clone(), file: o.file.name.clone(), exe: exe_path.clone(), sig: sig_path.clone() };
    // Já baixado e conferido antes (o aviso voltou depois de "Depois").
    if let (Ok(e), Ok(s)) = (std::fs::read(&exe_path), std::fs::read(&sig_path))
        && verify(&v.file, &v.version, &e, &s).is_ok()
    {
        return Ok(v);
    }
    let sig_text = http_get(&sig_asset.browser_download_url, 4096)?;
    let exe = http_get(&o.file.browser_download_url, MAX_EXE)?;
    if o.file.size != 0 && exe.len() as u64 != o.file.size {
        return Err(format!("download incompleto ({} de {} bytes)", exe.len(), o.file.size));
    }
    verify(&v.file, &v.version, &exe, &sig_text).map_err(|e| format!("{}: {e}; não instala", v.file))?;
    // Só o último instalador conferido fica guardado.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(&exe_path, &exe).map_err(|e| e.to_string())?;
    std::fs::write(&sig_path, &sig_text).map_err(|e| e.to_string())?;
    log(&format!("{} baixado e assinatura conferida", v.file));
    Ok(v)
}

/// Apaga os instaladores baixados (depois de instalar ou de pular a versão).
pub fn forget_download() {
    if let Some(d) = cache_dir() {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Troca o `Taskbeat.exe` da pasta `dir` por `bytes`. O atual é renomeado
/// (pode estar em uso); se algo falhar no meio, ele volta para o lugar.
pub fn swap_exe(dir: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let exe = dir.join("Taskbeat.exe");
    let new = dir.join(NEW);
    write_synced(&new, bytes).map_err(|e| format!("gravar {NEW}: {e}"))?;
    // Sobra de uma atualização anterior que ainda não pôde ser apagada:
    // o antigo vai com outro nome.
    let mut old = dir.join(OLD);
    if old.exists() && std::fs::remove_file(&old).is_err() {
        old = dir.join(format!("Taskbeat.{}.old", std::process::id()));
    }
    if exe.exists()
        && let Err(e) = std::fs::rename(&exe, &old)
    {
        let _ = std::fs::remove_file(&new);
        return Err(format!("renomear o Taskbeat.exe: {e}"));
    }
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("pôr o novo no lugar: {e}"));
    }
    Ok(exe)
}

/// Versão e tamanho em "Apps instalados".
fn set_registry(version: &Version, size: usize) {
    let key = uninstall_key();
    let _ = setup::reg_set_sz(&key, "DisplayVersion", &version.to_string());
    let _ = setup::reg_set_dword(&key, "EstimatedSize", (size / 1024) as u32);
}

/// Instala o que `download` guardou: confere de novo os bytes que serão
/// gravados e troca o executável. Devolve o caminho do novo.
pub fn apply(v: &Verified) -> Result<PathBuf, String> {
    let dir = installed_dir().ok_or("esta cópia não é a instalada")?;
    let exe = std::fs::read(&v.exe).map_err(|e| format!("ler o download: {e}"))?;
    let sig_text = std::fs::read(&v.sig).map_err(|e| format!("ler a assinatura: {e}"))?;
    if let Err(e) = verify(&v.file, &v.version, &exe, &sig_text) {
        log(&format!("{}: {e}; não instala", v.file));
        return Err(e);
    }
    let path = swap_exe(&dir, &exe)?;
    set_registry(&v.version, exe.len());
    log(&format!("instalado: {} ({} bytes) em {}", v.version, exe.len(), dir.display()));
    forget_download();
    Ok(path)
}

/// Abre o executável novo e fecha este (o mesmo caminho do "Sair").
pub fn restart(shared: &crate::shared::Shared, exe: &Path, from: &Version) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    std::process::Command::new(exe)
        .arg(format!("--after-update={from}"))
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("abrir a versão nova: {e}"))?;
    log(&format!("reiniciando na versão nova ({})", exe.display()));
    let tray = shared.tray_hwnd.load(std::sync::atomic::Ordering::Relaxed);
    unsafe {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_COMMAND};
        let _ = PostMessageW(Some(HWND(tray as _)), WM_COMMAND, WPARAM(ID_EXIT), LPARAM(0));
    }
    Ok(())
}

/// `--after-update=1.3.0`: a versão de onde veio.
pub fn updated_from() -> Option<Version> {
    std::env::args().find_map(|a| a.strip_prefix("--after-update=").and_then(Version::parse))
}

/// Ao abrir: apaga o executável antigo da última atualização (espera um
/// pouco: o processo antigo pode estar terminando de fechar) e acerta a
/// versão em "Apps instalados" se o exe instalado foi trocado.
pub fn after_start() {
    let Some(dir) = installed_dir() else { return };
    let leftovers = |d: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(d)
            .map(|it| {
                it.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).is_some_and(|n| n == "taskbeat.new" || (n.starts_with("taskbeat.") && n.ends_with(".old"))))
                    .collect()
            })
            .unwrap_or_default()
    };
    for _ in 0..20 {
        let left: Vec<PathBuf> = leftovers(&dir).into_iter().filter(|p| std::fs::remove_file(p).is_err()).collect();
        if left.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let key = uninstall_key();
    let me = Version::current();
    let registered = setup::reg_get_sz(&key, "DisplayVersion").and_then(|v| Version::parse(&v));
    if updated_from().is_some() || registered.as_ref() != Some(&me) {
        let size = std::fs::metadata(dir.join("Taskbeat.exe")).map(|m| m.len() as usize).unwrap_or(0);
        set_registry(&me, size);
        // Os avisos de licença vão junto com o programa (os desta versão).
        let _ = crate::licenses::write_to(&dir);
        log(&format!("\"Apps instalados\" e licenças acertados para {me}"));
    }
    // Instaladores de versões que já não são novas.
    if let Some(cache) = cache_dir() {
        let stale = std::fs::read_dir(&cache).ok().into_iter().flatten().filter_map(|e| e.ok()).any(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix("Taskbeat-Setup-").and_then(|r| r.strip_suffix(".exe")).and_then(Version::parse).is_some_and(|v| v <= me)
        });
        if stale {
            forget_download();
        }
    }
}

/// Abre uma página no navegador padrão (só as do repositório).
pub fn open_page(url: &str) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    if !url.starts_with("https://github.com/Lu1sGabriel/taskbeat/") {
        return;
    }
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("taskbeat-update-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn troca_o_exe_e_guarda_o_antigo() {
        let d = scratch("swap");
        std::fs::write(d.join("Taskbeat.exe"), b"velho").unwrap();
        let exe = swap_exe(&d, b"novo").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"novo");
        assert_eq!(std::fs::read(d.join(OLD)).unwrap(), b"velho");
        assert!(!d.join(NEW).exists());
        // De novo, com o .old da vez anterior ainda lá.
        swap_exe(&d, b"mais novo").unwrap();
        assert_eq!(std::fs::read(d.join("Taskbeat.exe")).unwrap(), b"mais novo");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// O .exe aberto (em execução, com a imagem mapeada) pode ser renomeado:
    /// copia um executável do sistema, roda e troca enquanto ele roda.
    #[test]
    fn troca_com_o_exe_rodando() {
        let d = scratch("running");
        let sys = PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into())).join("System32").join("ping.exe");
        std::fs::copy(&sys, d.join("Taskbeat.exe")).unwrap();
        let mut child = std::process::Command::new(d.join("Taskbeat.exe")).args(["-n", "4", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // Apagar falha (está em uso)...
        assert!(std::fs::remove_file(d.join("Taskbeat.exe")).is_err());
        // ...mas trocar funciona.
        swap_exe(&d, b"novo").unwrap();
        assert_eq!(std::fs::read(d.join("Taskbeat.exe")).unwrap(), b"novo");
        assert!(d.join(OLD).exists());
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&d);
    }
}
