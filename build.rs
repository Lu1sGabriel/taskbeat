//! - Embute o manifesto (Common Controls v6, DPI por monitor, UTF-8).
//! - Compila `app.rc` (ícone + versão) com o `rc.exe` do Windows SDK e liga o
//!   `.res` ao executável. Sem SDK, o app compila igual (só sem ícone no .exe;
//!   a bandeja usa um ícone desenhado em tempo de execução).

use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RC") {
        return Some(PathBuf::from(p));
    }
    let base = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(base).ok()?.filter_map(|e| e.ok().map(|e| e.path())).collect();
    versions.sort();
    versions.into_iter().rev().map(|v| v.join("x64").join("rc.exe")).find(|p| p.exists())
}

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/taskbeat.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", dir.join("app.manifest").display());

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.res");
    match find_rc() {
        Some(rc) => {
            let ok = Command::new(&rc)
                .current_dir(&dir)
                .args(["/nologo", "/fo"])
                .arg(&out)
                .arg("app.rc")
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                println!("cargo:rustc-link-arg-bins={}", out.display());
            } else {
                println!("cargo:warning=rc.exe falhou; o executável fica sem ícone");
            }
        }
        None => println!("cargo:warning=rc.exe (Windows SDK) não encontrado; o executável fica sem ícone"),
    }
}
