//! Taskbeat — widget de música para a barra de tarefas do Windows.
//!
//! Threads:
//! - principal: ícone da bandeja + menu (tray.rs)
//! - audio-loopback: captura WASAPI de tudo que toca no PC (audio.rs)
//! - media-sessions: o que está tocando e em qual app (media.rs)
//! - overlay: análise FFT + desenho sobre a barra de tarefas (overlay.rs)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appicon;
mod audio;
mod config;
mod dsp;
mod i18n;
mod media;
mod overlay;
mod render;
mod settings;
mod setup;
mod shared;
#[cfg(test)]
mod demo_gif;
#[cfg(test)]
mod shots;
mod system;
mod taskbar;
mod text;
mod tray;
mod ui;
mod volume;

use shared::Shared;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_LBUTTONUP};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::w;

fn main() {
    unsafe {
        // Instalador (Taskbeat-Setup.exe / --install) ou desinstalador (--uninstall).
        if let Some(mode) = setup::mode_from_args() {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            setup::run(mode);
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

        let cfg = config::Config::load();
        i18n::apply(cfg.language);
        let shared = Arc::new(Shared::new(cfg));
        let audio = audio::spawn(shared.clone());
        media::spawn(shared.clone());
        let overlay = overlay::spawn(shared.clone());

        if let Err(e) = tray::run(shared.clone()) {
            audio::log::warn(&format!("bandeja falhou: {e}"));
        }

        shared.running.store(false, Ordering::Relaxed);
        let _ = overlay.join();
        let _ = audio.join();
    }
}
