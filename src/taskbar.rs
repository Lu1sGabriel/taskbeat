//! Geometria da barra de tarefas (Windows 10/11) e detecção de tela cheia.
//!
//! - `Shell_TrayWnd` é a barra de tarefas principal.
//! - `TrayNotifyWnd` (filho dela) continua expondo o retângulo da área de
//!   notificação no Windows 11, mesmo com a barra em XAML.
//! - Barra com ocultação automática: só mostramos o visualizador quando a barra
//!   está realmente na tela.

use crate::config::{Anchor, Config};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GW_HWNDPREV, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowRect,
};
use windows::core::{PCWSTR, w};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub scale: f32,
    pub visible: bool,
    /// Barra na horizontal (painel de mídia só aparece nesse caso).
    pub horizontal: bool,
    /// Borda superior da barra (para o cartão de "próxima música" subir a partir dela).
    pub taskbar_top: i32,
    pub taskbar_bottom_edge: bool,
    pub mon_left: i32,
    pub mon_right: i32,
    pub mon_top: i32,
    pub mon_bottom: i32,
}

pub fn taskbar_hwnd() -> Option<HWND> {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok() }
}

fn rect_of(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok()? };
    Some(r)
}

fn intersect(a: &RECT, b: &RECT) -> RECT {
    RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) }
}

/// `length` = largura desejada em pixels lógicos (96 DPI).
pub fn compute(cfg: &Config, length: i32) -> Option<Placement> {
    let tray = taskbar_hwnd()?;
    let tb = rect_of(tray)?;
    let dpi = unsafe { GetDpiForWindow(tray) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };

    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let mon = unsafe { MonitorFromWindow(tray, MONITOR_DEFAULTTOPRIMARY) };
    let _ = unsafe { GetMonitorInfoW(mon, &mut mi) };
    let screen = mi.rcMonitor;

    let horizontal = (tb.right - tb.left) >= (tb.bottom - tb.top);
    let on = intersect(&tb, &screen);
    let thickness = if horizontal { tb.bottom - tb.top } else { tb.right - tb.left };
    let on_thickness = if horizontal { on.bottom - on.top } else { on.right - on.left };
    // Barra com ocultação automática recolhida: só uma faixa de ~2px fica visível.
    let mut visible = thickness > 0 && on_thickness * 2 >= thickness;

    let notify = unsafe { FindWindowExW(Some(tray), None, w!("TrayNotifyWnd"), PCWSTR::null()).ok() }
        .and_then(rect_of)
        .filter(|r| r.right > r.left && r.bottom > r.top);

    let pad = (4.0 * scale).round() as i32;
    let margin = (8.0 * scale).round() as i32;
    let length = (length as f32 * scale).round() as i32;
    let offset = (cfg.offset as f32 * scale).round() as i32;

    let (x, y, w, h);
    if horizontal {
        h = (on.bottom - on.top - 2 * pad).max(8);
        y = on.top + pad;
        w = length;
        x = match cfg.anchor {
            Anchor::BeforeTray => match notify {
                Some(n) => n.left - margin - w,
                None => tb.right - (400.0 * scale) as i32 - w,
            },
            Anchor::Left => tb.left + margin,
        } + offset;
    } else {
        w = (on.right - on.left - 2 * pad).max(8);
        x = on.left + pad;
        h = (length / 3).max(24);
        y = match cfg.anchor {
            Anchor::BeforeTray => match notify {
                Some(n) => n.top - margin - h,
                None => tb.bottom - (300.0 * scale) as i32 - h,
            },
            Anchor::Left => tb.top + margin,
        } + offset;
    }

    if cfg.hide_on_fullscreen && fullscreen_active(&screen) {
        visible = false;
    }

    Some(Placement {
        x,
        y,
        w,
        h,
        scale,
        visible,
        horizontal,
        taskbar_top: on.top,
        taskbar_bottom_edge: tb.top > screen.top + (screen.bottom - screen.top) / 2,
        mon_left: screen.left,
        mon_right: screen.right,
        mon_top: screen.top,
        mon_bottom: screen.bottom,
    })
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Jogo/vídeo em tela cheia, apresentação, ou janela em primeiro plano cobrindo o monitor.
fn fullscreen_active(screen: &RECT) -> bool {
    if let Ok(state) = unsafe { SHQueryUserNotificationState() } {
        if state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE {
            return true;
        }
    }
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return false;
    }
    let cls = class_name(fg);
    if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
        return false;
    }
    match rect_of(fg) {
        Some(r) => r.left <= screen.left && r.top <= screen.top && r.right >= screen.right && r.bottom >= screen.bottom,
        None => false,
    }
}

/// `true` se a barra de tarefas estiver acima da nossa janela na ordem Z
/// (acontece quando o usuário clica nela, pois ela também é "topmost").
pub fn taskbar_is_above(ours: HWND, tray: HWND) -> bool {
    let mut cur = ours;
    for _ in 0..256 {
        match unsafe { GetWindow(cur, GW_HWNDPREV) } {
            Ok(prev) if !prev.is_invalid() => {
                if prev == tray {
                    return true;
                }
                cur = prev;
            }
            _ => return false,
        }
    }
    false
}
