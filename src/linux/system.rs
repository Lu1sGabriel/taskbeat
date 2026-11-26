//! Utilidades do sistema no Linux: cor de destaque e tema (portal do
//! freedesktop), abrir ao iniciar a sessão (`~/.config/autostart`), abrir
//! arquivos, "acordar" a thread principal e registro de diagnóstico.

use crate::render::{Colors, Rgb};
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

pub const APP_ID: &str = "io.github.lu1sgabriel.taskbeat";

// ---------------------------------------------------------------- acordar a thread principal

/// Pipe que tira a thread principal do `poll` (mídia mudou, menu da bandeja…).
struct Wake {
    read: RawFd,
    write: RawFd,
}

fn wake_pipe() -> &'static Wake {
    static W: OnceLock<Wake> = OnceLock::new();
    W.get_or_init(|| {
        let mut fds = [0; 2];
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
        Wake { read: fds[0], write: fds[1] }
    })
}

/// Acorda a thread principal (de qualquer thread).
pub fn wake() {
    let b = 1u8;
    unsafe { libc::write(wake_pipe().write, (&b as *const u8).cast(), 1) };
}

/// Lado de leitura (para o `poll` da thread principal).
pub fn wake_fd() -> RawFd {
    wake_pipe().read
}

/// Esvazia o pipe depois de acordar.
pub fn drain_wake() {
    let mut buf = [0u8; 64];
    while unsafe { libc::read(wake_pipe().read, buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
}

/// Threads de segundo plano: prioridade baixa (não disputa com o que o usuário faz).
pub fn background_thread() {
    unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 5) };
}

// ---------------------------------------------------------------- cores

/// Cor de destaque e tema do sistema pelo portal (GNOME 47+, KDE e outros);
/// sem portal, o azul do GNOME e tema escuro.
pub fn colors(palette: crate::config::Palette) -> Colors {
    let (accent, dark) = portal_appearance();
    let accent = accent.unwrap_or([0.21, 0.52, 0.89]);
    let light = crate::render::lerp3(accent, [1.0; 3], 0.25);
    Colors { palette, accent, accent_light: light, light_taskbar: !dark.unwrap_or(true), album: None }
}

fn portal_appearance() -> (Option<Rgb>, Option<bool>) {
    static CACHE: OnceLock<(Option<Rgb>, Option<bool>)> = OnceLock::new();
    *CACHE.get_or_init(|| {
        use dbus::arg::{RefArg, Variant};
        use dbus::blocking::Connection;
        let Ok(conn) = Connection::new_session() else { return (None, None) };
        let proxy = conn.with_proxy("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", Duration::from_millis(600));
        let read = |key: &str| -> Option<Variant<Box<dyn RefArg>>> {
            let (v,): (Variant<Box<dyn RefArg>>,) = proxy.method_call("org.freedesktop.portal.Settings", "ReadOne", ("org.freedesktop.appearance", key)).ok()?;
            Some(v)
        };
        // O valor pode vir dentro de outro variant (versões antigas do portal).
        fn inner(v: &dyn RefArg) -> &dyn RefArg {
            match v.as_iter() {
                Some(mut it) if v.signature().starts_with('v') => it.next().map(inner).unwrap_or(v),
                _ => v,
            }
        }
        let accent = read("accent-color").and_then(|v| {
            let v = inner(&*v.0);
            let c: Vec<f64> = v.as_iter()?.filter_map(|x| x.as_f64()).collect();
            (c.len() == 3 && c.iter().all(|x| (0.0..=1.0).contains(x))).then(|| [c[0] as f32, c[1] as f32, c[2] as f32])
        });
        let dark = read("color-scheme").and_then(|v| inner(&*v.0).as_u64()).map(|s| s != 2);
        (accent, dark)
    })
}

// ---------------------------------------------------------------- abrir ao iniciar a sessão

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

fn autostart_path() -> Option<PathBuf> {
    Some(config_home()?.join("autostart").join(format!("{APP_ID}.desktop")))
}

/// Como abrir o app: o AppImage (se for um) ou o próprio executável.
fn launch_command() -> Option<String> {
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())?;
    let s = exe.display().to_string();
    Some(if s.contains([' ', '"', '\\', '$', '`']) { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`")) } else { s })
}

/// Instalado (pacote, AppImage ou ~/.local)? Builds de desenvolvimento não
/// mexem no início com a sessão.
pub fn installed() -> bool {
    if std::env::var_os("APPIMAGE").is_some() {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let s = exe.display().to_string();
    !s.contains("/target/") && (s.starts_with("/usr/") || s.starts_with("/opt/") || s.contains("/.local/"))
}

pub fn autostart_enabled() -> bool {
    let Some(p) = autostart_path() else { return false };
    std::fs::read_to_string(p).is_ok_and(|s| !s.lines().any(|l| l.trim() == "Hidden=true" || l.trim() == "X-GNOME-Autostart-enabled=false"))
}

pub fn set_autostart(on: bool) {
    let Some(path) = autostart_path() else { return };
    if !on {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let Some(cmd) = launch_command() else { return };
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Taskbeat\nComment={}\nExec={cmd}\nIcon=taskbeat\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\n",
        crate::i18n::tr("Widget de música")
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, text);
}

/// Abre um arquivo no app padrão (editor de texto para o config.toml).
pub fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open").arg(p).spawn();
}

// ---------------------------------------------------------------- registro

/// Diagnóstico: com `TASKBEAT_LOG=/caminho/arquivo.log` grava cada evento.
pub mod log {
    use std::io::Write;
    use std::sync::OnceLock;

    fn file() -> Option<&'static std::path::PathBuf> {
        static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
        PATH.get_or_init(|| std::env::var_os("TASKBEAT_LOG").map(Into::into)).as_ref()
    }

    fn write(level: &str, msg: &str) {
        #[cfg(debug_assertions)]
        eprintln!("[{level}] {msg}");
        if let Some(p) = file() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                // Uma escrita só por linha (várias threads gravam no mesmo arquivo).
                let _ = f.write_all(format!("{t:.3} [{level}] {msg}\n").as_bytes());
            }
        }
    }

    pub fn info(msg: &str) {
        write("info", msg);
    }

    pub fn warn(msg: &str) {
        write("warn", msg);
    }
}
