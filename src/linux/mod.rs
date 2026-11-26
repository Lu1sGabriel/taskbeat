//! Taskbeat no Linux: widget de música numa "ilha" no topo da tela (o mesmo
//! desenho do notch do Mac).
//!
//! - thread principal: a ilha (`island.rs`) numa superfície Wayland com
//!   layer-shell ou numa janela X11 (`window.rs`), parada no `poll` quando
//!   nada muda;
//! - media-mpris: o que está tocando, pelo MPRIS no D-Bus (`media.rs`);
//! - audio-pulse: som para as barras e volume, pelo PulseAudio/PipeWire
//!   (`audio.rs`), só gravando com as barras visíveis;
//! - tray: ícone na bandeja e instância única, no D-Bus (`tray.rs`);
//! - config-watch: relê o `config.toml` quando ele é salvo.
//!
//! O desenho é o mesmo do Windows e do Mac (`notch.rs`, `ui.rs`,
//! `render.rs`); o texto usa as bibliotecas do sistema (`ft.rs`, `text.rs`).

mod audio;
pub(crate) mod ft;
mod gfx;
mod island;
mod media;
pub(crate) mod system;
mod tray;
mod wayland;
mod window;
mod x11;

use crate::config::Config;
use crate::shared::Shared;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use system::log;

/// Pedidos para a thread principal (bandeja, outra instância).
#[derive(Clone, Copy, Debug)]
pub enum App {
    ToggleCard,
    Quit,
}

pub fn main() {
    let bus = match tray::claim() {
        tray::Instance::Other => {
            // Já está aberto: a instância aberta mostra as configurações.
            return;
        }
        tray::Instance::First(c) => Some(c),
        tray::Instance::NoBus => None,
    };
    let first_run = !Config::exists();
    let mut cfg = Config::load();
    // Abrir ao iniciar a sessão vem ligado (só o app instalado).
    if system::installed() && cfg.apply_login_default(first_run, false, system::autostart_enabled, || system::set_autostart(true)) {
        cfg.save();
    }
    crate::i18n::apply(cfg.language);
    log::info(&format!("Taskbeat {} no Linux; fonte: {:?}", env!("CARGO_PKG_VERSION"), crate::text::primary_font_file()));
    let shared = Arc::new(Shared::new(cfg));
    stop_on_signals(shared.clone());

    let (tx, rx) = channel::<App>();
    media::spawn(shared.clone());
    audio::spawn(shared.clone());
    if let Some(conn) = bus {
        tray::spawn(conn, shared.clone(), tx.clone());
    }
    watch_config(shared.clone());
    if std::env::var_os("TV_DEMO_SETTINGS").is_some() {
        tray::open_settings(&shared);
    }

    island::run(shared.clone(), rx);
    shared.running.store(false, Ordering::Relaxed);
    log::info("saindo");
}

/// Ctrl+C / `kill`: sai limpo (a thread principal acorda e vê o pedido).
fn stop_on_signals(shared: Arc<Shared>) {
    static SHARED: std::sync::OnceLock<Arc<Shared>> = std::sync::OnceLock::new();
    let _ = SHARED.set(shared);
    extern "C" fn on_signal(_: libc::c_int) {
        if let Some(s) = SHARED.get() {
            s.running.store(false, Ordering::Relaxed);
        }
        system::wake();
    }
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
    }
}

/// Relê o `config.toml` quando ele é salvo (o usuário edita no editor de
/// texto pelo menu "Configurações…"). Usa o inotify: parado, nada roda.
fn watch_config(shared: Arc<Shared>) {
    let Some(path) = Config::path() else { return };
    let Some(dir) = path.parent().map(|d| d.to_path_buf()) else { return };
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || {
            system::background_thread();
            let _ = std::fs::create_dir_all(&dir);
            let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
            if fd < 0 {
                return;
            }
            let Ok(cdir) = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) else { return };
            unsafe { libc::inotify_add_watch(fd, cdir.as_ptr(), libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO) };
            let mut buf = [0u8; 4096];
            while shared.is_running() {
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                // Editores salvam em várias etapas: espera assentar.
                std::thread::sleep(std::time::Duration::from_millis(150));
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                match toml::from_str::<Config>(&text) {
                    // Válida: lê de novo pelo caminho normal (migrações e limites).
                    Ok(_) => {
                        let new = Config::load();
                        let same = toml::to_string(&new).ok() == toml::to_string(&*shared.config.lock().unwrap()).ok();
                        if !same {
                            log::info("config: relida do arquivo");
                            shared.preview_config(|c| *c = new);
                            system::wake();
                        }
                    }
                    Err(e) => log::warn(&format!("config: arquivo com erro, ignorado: {e}")),
                }
            }
        })
        .ok();
}
