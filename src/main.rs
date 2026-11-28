//! Taskbeat — widget de música para a barra de tarefas do Windows e o notch do Mac.
//!
//! Windows — threads:
//! - principal: ícone da bandeja + menu (tray.rs)
//! - audio-loopback: captura WASAPI de tudo que toca no PC (audio.rs)
//! - media-sessions: o que está tocando e em qual app (media.rs)
//! - overlay: análise FFT + desenho sobre a barra de tarefas (overlay.rs)
//!
//! macOS: veja `mac/mod.rs` (widget no notch, AppKit na thread principal).
//! Linux: veja `linux/mod.rs` (ilha no topo da tela, Wayland ou X11).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// No Mac e no Linux parte do código comum (opções da barra de tarefas, aviso
// de teclas…) não é usada.
#![cfg_attr(any(target_os = "macos", target_os = "linux"), allow(dead_code))]

// Subtrair `Duration` de `Instant` pode dar panic (veja `clock.rs`).
#![warn(clippy::unchecked_time_subtraction)]

// Comuns aos dois sistemas: configurações, análise do áudio, traduções e desenho.
mod anim;
mod clock;
mod config;
mod dsp;
mod files;
mod http;
mod i18n;
mod licenses;
mod look;
mod lyrics;
mod notch;
#[cfg(test)]
mod png;
mod render;
mod shared;
mod ui;
mod update;
// Mesma interface nos três sistemas; DirectWrite no Windows, CoreText no Mac,
// FreeType + HarfBuzz no Linux.
#[cfg_attr(target_os = "macos", path = "mac/text.rs")]
#[cfg_attr(target_os = "linux", path = "linux/text.rs")]
mod text;

#[cfg(test)]
mod demo_gif;

// Só Windows.
#[cfg(windows)]
mod appicon;
#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod cover;
#[cfg(windows)]
mod launchers;
#[cfg(any(windows, target_os = "macos"))]
mod lyrics_bg;
#[cfg(windows)]
mod lyrics_overlay;
#[cfg(any(windows, target_os = "macos"))]
mod lyrics_view;
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
#[cfg(all(windows, test))]
mod settings_mock;
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

// Só Linux.
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(windows)]
use shared::Shared;
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::Ordering;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
#[cfg(windows)]
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_LBUTTONUP};
#[cfg(windows)]
use windows::Win32::Foundation::{LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
#[cfg(windows)]
use windows::core::w;

/// Quanto a versão recém-instalada espera a anterior fechar.
#[cfg(windows)]
const AFTER_UPDATE_WAIT_MS: u32 = 60_000;

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
        let mutex = CreateMutexW(None, true, &windows::core::HSTRING::from(instance_mutex()));
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        let after_update = std::env::args().any(|a| a.starts_with("--after-update="));
        if existed && after_update {
            // Versão nova recém-instalada: a antiga está fechando; espera ela
            // soltar a instância em vez de chamar por ela. 60 s (eram 15, e
            // uma antiga lenta deixava o app fechado; B13). A antiga segura o
            // mutex até morrer, e um mutex abandonado (até num abort) também
            // libera a espera.
            let Ok(m) = &mutex else { return };
            let r = WaitForSingleObject(*m, AFTER_UPDATE_WAIT_MS);
            if r != windows::Win32::Foundation::WAIT_OBJECT_0 && r != windows::Win32::Foundation::WAIT_ABANDONED {
                update::log(&format!("a versão anterior não fechou em {} s; esta sai sem abrir", AFTER_UPDATE_WAIT_MS / 1000));
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
        webext::spawn(shared.clone());
        webplayer::spawn(shared.clone());
        mixer::spawn(shared.clone());
        update::spawn(shared.clone());
        let overlay = overlay::spawn(shared.clone());
        lyrics_overlay::init(shared.clone());

        if let Err(e) = tray::run(shared.clone()) {
            audio::log::warn(&format!("bandeja falhou: {e}"));
        }

        shared.running.store(false, Ordering::Relaxed);
        let _ = overlay.join();
        let _ = audio.join();

        // "Reiniciar o Taskbeat" (bandeja): solta a instância única antes de
        // abrir a cópia nova (senão ela acharia esta e sairia).
        if tray::RESTART.load(Ordering::Relaxed) {
            if let Ok(m) = mutex {
                let _ = ReleaseMutex(m);
                let _ = CloseHandle(m);
            }
            match std::env::current_exe() {
                Ok(exe) => {
                    if let Err(e) = std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn() {
                        audio::log::warn(&format!("reiniciar: não abriu a cópia nova: {e}"));
                    }
                }
                Err(e) => audio::log::warn(&format!("reiniciar: caminho do exe: {e}")),
            }
        }
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
