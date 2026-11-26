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
mod http;
mod i18n;
mod licenses;
mod notch;
#[cfg(test)]
mod png;
mod render;
mod shared;
mod ui;
mod update;
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
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
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
    unsafe {
        // Instalador (Taskbeat-Setup.exe / --install) ou desinstalador (--uninstall).
        if let Some(mode) = setup::mode_from_args() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            setup::run(mode);
            return;
        }
        // Uma instância só.
        let _mutex = CreateMutexW(None, true, &windows::core::HSTRING::from(instance_mutex()));
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        let after_update = std::env::args().any(|a| a.starts_with("--after-update="));
        if existed && after_update {
            // Versão nova recém-instalada: a antiga está fechando; espera ela
            // soltar a instância (até 15 s) em vez de chamar por ela.
            let Ok(m) = &_mutex else { return };
            let r = WaitForSingleObject(*m, 15_000);
            if r != windows::Win32::Foundation::WAIT_OBJECT_0 && r != windows::Win32::Foundation::WAIT_ABANDONED {
                return;
            }
        } else if existed {
            // Já está aberto (ex.: clicou no atalho do Menu Iniciar de novo):
            // abre as configurações da instância que está rodando. (A cópia de
            // teste do atualizador não chama ninguém: poderia achar a de verdade.)
            if instance_mutex().ends_with("SingleInstance")
                && let Ok(tray) = FindWindowW(w!("TaskbeatTray"), windows::core::PCWSTR::null())
            {
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
        webplayer::spawn(shared.clone());
        mixer::spawn(shared.clone());
        update::spawn(shared.clone());
        let overlay = overlay::spawn(shared.clone());

        if let Err(e) = tray::run(shared.clone()) {
            audio::log::warn(&format!("bandeja falhou: {e}"));
        }

        shared.running.store(false, Ordering::Relaxed);
        let _ = overlay.join();
        let _ = audio.join();
    }
}

/// Nome da instância única. No teste de ponta a ponta do atualizador
/// (`TASKBEAT_INSTANCE`) a cópia de teste roda ao lado do Taskbeat de verdade.
#[cfg(windows)]
fn instance_mutex() -> String {
    #[cfg(any(debug_assertions, feature = "e2e"))]
    if let Some(i) = update::test_env("TASKBEAT_INSTANCE") {
        return format!("Local\\Taskbeat.SingleInstance.{i}");
    }
    "Local\\Taskbeat.SingleInstance".to_string()
}
