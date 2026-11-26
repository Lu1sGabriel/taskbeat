//! Taskbeat — widget de música para a barra de tarefas do Windows e o notch do Mac.
//!
//! Windows — threads:
//! - principal: ícone da bandeja + menu (tray.rs)
//! - audio-loopback: captura WASAPI de tudo que toca no PC (audio.rs)
//! - media-sessions: o que está tocando e em qual app (media.rs)
//! - overlay: análise FFT + desenho sobre a barra de tarefas (overlay.rs)
//!
//! macOS: veja `mac/mod.rs` (widget no notch, AppKit na thread principal).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// No Mac parte do código comum (opções da barra de tarefas, aviso de teclas…)
// não é usada.
#![cfg_attr(target_os = "macos", allow(dead_code))]

// Comuns aos dois sistemas: configurações, análise do áudio, traduções e desenho.
mod config;
mod dsp;
mod i18n;
mod licenses;
mod notch;
#[cfg(test)]
mod png;
mod render;
mod shared;
mod ui;
// Mesma interface nos dois sistemas; DirectWrite no Windows, CoreText no Mac.
#[cfg_attr(target_os = "macos", path = "mac/text.rs")]
mod text;

#[cfg(test)]
mod demo_gif;

// Só Windows.
#[cfg(windows)]
mod appicon;
#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod launchers;
#[cfg(windows)]
mod media;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod mixer;
#[cfg(windows)]
mod overlay;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod setup;
#[cfg(all(windows, test))]
mod shots;
#[cfg(windows)]
mod system;
#[cfg(windows)]
mod taskbar;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod volume;
#[cfg(windows)]
mod vsync;
#[cfg(windows)]
mod webext;
#[cfg(windows)]
mod webplayer;

// Só macOS.
#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "macos")]
fn main() {
    mac::main();
}

#[cfg(windows)]
use shared::Shared;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::Ordering;
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
#[cfg(windows)]
use windows::Win32::System::Threading::CreateMutexW;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_LBUTTONUP};
#[cfg(windows)]
use windows::Win32::Foundation::{LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
#[cfg(windows)]
use windows::core::w;

#[cfg(windows)]
fn main() {
    // Aberto pelo navegador (extensão do Taskbeat): só repassa mensagens ao
    // Taskbeat em execução e sai quando o navegador fecha a conexão.
    if webext::host_mode() {
        webext::run_host();
        return;
    }
    unsafe {
        // Instalador (Taskbeat-Setup.exe / --install) ou desinstalador (--uninstall).
        if let Some(mode) = setup::mode_from_args() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            setup::run(mode);
            return;
        }
        // Só a tela de configurações, numa instância à parte (capturas e testes
        // com o Taskbeat aberto): sem widget, bandeja, teclas nem captura de som.
        // Para não mexer na configuração de verdade, use com outro `APPDATA`.
        if std::env::var_os("TASKBEAT_SETTINGS_ONLY").is_some() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let cfg = config::Config::load();
            i18n::apply(cfg.language);
            let shared = Arc::new(Shared::new(cfg));
            launchers::spawn(shared.clone());
            settings::run_alone(shared);
            return;
        }
        // Uma instância só.
        let _mutex = CreateMutexW(None, true, w!("Local\\Taskbeat.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Já está aberto (ex.: clicou no atalho do Menu Iniciar de novo):
            // abre as configurações da instância que está rodando.
            if let Ok(tray) = FindWindowW(w!("TaskbeatTray"), windows::core::PCWSTR::null()) {
                let mut pid = 0u32;
                GetWindowThreadProcessId(tray, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(tray), tray::WM_APP_TRAY, WPARAM(0), LPARAM(WM_LBUTTONUP as isize));
            }
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let first_run = !config::Config::exists();
        let mut cfg = config::Config::load();
        // Iniciar com o Windows vem ligado (o instalador já marca; isto cobre
        // quem abre o .exe direto). Não vale para builds de desenvolvimento.
        if !cfg!(debug_assertions) && cfg.apply_login_default(first_run, false, system::autostart_enabled, || system::set_autostart(true)) {
            cfg.save();
        }
        i18n::apply(cfg.language);
        let shared = Arc::new(Shared::new(cfg));
        let audio = audio::spawn(shared.clone());
        media::spawn(shared.clone());
        launchers::spawn(shared.clone());
        webext::spawn(shared.clone());
        webplayer::spawn(shared.clone());
        mixer::spawn(shared.clone());
        let overlay = overlay::spawn(shared.clone());

        if let Err(e) = tray::run(shared.clone()) {
            audio::log::warn(&format!("bandeja falhou: {e}"));
        }

        shared.running.store(false, Ordering::Relaxed);
        let _ = overlay.join();
        let _ = audio.join();
    }
}
