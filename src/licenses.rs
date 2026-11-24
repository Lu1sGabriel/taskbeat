//! Licença do Taskbeat (MIT) e avisos das bibliotecas de terceiros, embutidos
//! no executável: vão junto com qualquer cópia do app, como a MIT pede.

#[cfg(windows)]
use std::path::Path;
#[cfg(windows)]
use windows::Win32::UI::Shell::ShellExecuteW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
#[cfg(windows)]
use windows::core::{HSTRING, PCWSTR, w};

pub const LICENSE: &str = include_str!("../LICENSE");
pub const NOTICES: &str = include_str!("../THIRD-PARTY-NOTICES.md");

#[cfg(windows)]
/// Texto para o Bloco de Notas: UTF-8 com BOM e quebras de linha do Windows.
fn windows_text(s: &str) -> String {
    format!("\u{FEFF}{}", s.replace("\r\n", "\n").replace('\n', "\r\n"))
}

#[cfg(windows)]
/// Grava `LICENSE.txt` e `THIRD-PARTY-NOTICES.txt` na pasta de instalação.
pub fn write_to(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join("LICENSE.txt"), windows_text(LICENSE))?;
    std::fs::write(dir.join("THIRD-PARTY-NOTICES.txt"), windows_text(NOTICES))
}

/// Abre a licença e os avisos de terceiros no editor de texto padrão.
#[cfg(windows)]
pub fn open() {
    let path = std::env::temp_dir().join("Taskbeat - licencas.txt");
    let text = format!("{LICENSE}\n\n{}\n\n{NOTICES}", "=".repeat(78));
    if std::fs::write(&path, windows_text(&text)).is_ok() {
        unsafe {
            ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

/// Abre a licença e os avisos de terceiros no editor de texto padrão (TextEdit).
#[cfg(target_os = "macos")]
pub fn open() {
    let path = std::env::temp_dir().join("Taskbeat - licencas.txt");
    let text = format!("{LICENSE}

{}

{NOTICES}", "=".repeat(78));
    if std::fs::write(&path, text).is_ok() {
        let _ = std::process::Command::new("/usr/bin/open").arg("-t").arg(&path).spawn();
    }
}
