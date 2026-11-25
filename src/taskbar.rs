//! Geometria da barra de tarefas (Windows 10/11) e detecção de tela cheia.
//!
//! - `Shell_TrayWnd` é a barra de tarefas principal.
//! - `TrayNotifyWnd` (filho dela) continua expondo o retângulo da área de
//!   notificação no Windows 11, mesmo com a barra em XAML.
//! - Barra com ocultação automática: só mostramos o visualizador quando a barra
//!   está realmente na tela.

use crate::config::{Anchor, Config};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation, IUIAutomationElement, TreeScope_Descendants, UIA_AutomationIdPropertyId};
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
    /// Por que está escondido (diagnóstico).
    pub why_hidden: Option<&'static str>,
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
    let mut why_hidden = (!visible).then_some("barra de tarefas recolhida (ocultar automaticamente)");

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
        let mut base = match cfg.anchor {
            Anchor::BeforeTray => match notify {
                Some(n) => n.left - margin - w,
                None => tb.right - (400.0 * scale) as i32 - w,
            },
            Anchor::Left => tb.left + margin,
        };
        // Botão de Widgets do Windows (o do tempo): com a barra alinhada à
        // esquerda ele fica ao lado da bandeja; centralizada, no canto
        // esquerdo. Se ficaria por baixo do nosso, passa para o lado dele.
        if let Some(wr) = widgets_rect().filter(|r| r.top < tb.bottom && r.bottom > tb.top) {
            if wr.left < base + w + margin && wr.right + margin > base {
                base = match cfg.anchor {
                    Anchor::BeforeTray => wr.left - margin - w,
                    Anchor::Left => wr.right + margin,
                };
            }
        }
        x = base + offset;
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

    if cfg.hide_on_fullscreen {
        if let Some(why) = fullscreen_active(&screen) {
            visible = false;
            why_hidden = Some(why);
        }
    }

    Some(Placement {
        x,
        y,
        w,
        h,
        scale,
        visible,
        why_hidden,
        horizontal,
        taskbar_top: on.top,
        taskbar_bottom_edge: tb.top > screen.top + (screen.bottom - screen.top) / 2,
        mon_left: screen.left,
        mon_right: screen.right,
        mon_top: screen.top,
        mon_bottom: screen.bottom,
    })
}

// ---------------------------------------------------------------- botão de Widgets

/// Retângulo atual do botão de Widgets (o do tempo) do Windows 11, se visível.
static WIDGETS: Mutex<Option<RECT>> = Mutex::new(None);

/// Botão de Widgets na barra (lido em segundo plano; começa na primeira consulta).
fn widgets_rect() -> Option<RECT> {
    static START: Once = Once::new();
    START.call_once(|| {
        let _ = std::thread::Builder::new().name("taskbar-widgets".into()).spawn(watch_widgets);
    });
    *WIDGETS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Acompanha o botão de Widgets: o registro diz se ele está ligado
/// (`TaskbarDa`) e a UI Automation da barra diz onde está (ele muda de lugar
/// com o alinhamento da barra e de largura com o texto do tempo). Com o botão
/// desligado só o registro é lido; ligado, uma leitura do retângulo a cada 2 s.
fn watch_widgets() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    crate::system::background_thread();
    let Ok(uia) = (unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }) else {
        crate::audio::log::warn("UI Automation indisponível: posição do botão de Widgets desconhecida");
        return;
    };
    let id = VARIANT::from("WidgetsButton");
    let mut el: Option<IUIAutomationElement> = None;
    let mut last_find: Option<Instant> = None;
    let mut last: Option<RECT> = None;
    loop {
        let enabled = crate::system::reg_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"), w!("TaskbarDa")) != Some(0);
        let rect = if enabled {
            let read = |e: &IUIAutomationElement| unsafe { e.CurrentBoundingRectangle() }.ok().filter(|r| r.right > r.left && r.bottom > r.top);
            let mut r = el.as_ref().and_then(read);
            // Sumiu (Explorer reiniciou, botão escondido): procura de novo, sem
            // pressa (a busca custa ~40 ms; ler o retângulo, menos de 1 ms).
            if r.is_none() && last_find.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                last_find = Some(Instant::now());
                el = find_on_taskbar(&uia, &id);
                r = el.as_ref().and_then(read);
            }
            r
        } else {
            // Ao religar, procura na hora.
            el = None;
            last_find = None;
            None
        };
        if rect != last {
            last = rect;
            crate::audio::log::info(&format!("botão de Widgets do Windows: {:?}", rect.map(|r| (r.left, r.top, r.right, r.bottom))));
            *WIDGETS.lock().unwrap_or_else(|e| e.into_inner()) = rect;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Elemento da barra com este AutomationId (XAML: o `x:Name`).
fn find_on_taskbar(uia: &IUIAutomation, id: &VARIANT) -> Option<IUIAutomationElement> {
    let tray = taskbar_hwnd()?;
    unsafe {
        let root = uia.ElementFromHandle(tray).ok()?;
        let cond = uia.CreatePropertyCondition(UIA_AutomationIdPropertyId, id).ok()?;
        root.FindFirst(TreeScope_Descendants, &cond).ok()
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Jogo/vídeo em tela cheia, apresentação, ou janela em primeiro plano cobrindo
/// o monitor da barra. Devolve o motivo (para o registro de diagnóstico).
fn fullscreen_active(screen: &RECT) -> Option<&'static str> {
    if let Ok(state) = unsafe { SHQueryUserNotificationState() } {
        if state == QUNS_RUNNING_D3D_FULL_SCREEN {
            return Some("jogo/vídeo em tela cheia (D3D)");
        }
        if state == QUNS_PRESENTATION_MODE {
            return Some("modo de apresentação");
        }
        if state == QUNS_BUSY {
            // "Ocupado" vale para tela cheia em qualquer monitor: só conta se a
            // janela em primeiro plano estiver no monitor da barra.
            if foreground_covers(screen) {
                return Some("app em tela cheia");
            }
            return None;
        }
    }
    foreground_covers(screen).then_some("janela em primeiro plano cobrindo o monitor")
}

/// A janela em primeiro plano cobre o monitor inteiro (inclusive a barra)?
fn foreground_covers(screen: &RECT) -> bool {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return false;
    }
    let cls = class_name(fg);
    // Área de trabalho, barra e janelas do próprio shell (menus, painéis) não contam.
    if matches!(
        cls.as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow" | "XamlExplorerHostIslandWindow" | "TopLevelWindowForOverflowXamlIsland"
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Mede a busca e a leitura do retângulo pela UI Automation (com o botão
    /// Iniciar, que sempre existe). `cargo test --release uia_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn uia_cost() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let uia: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.unwrap();
        for id in ["StartButton", "WidgetsButton"] {
            let t = Instant::now();
            let el = find_on_taskbar(&uia, &VARIANT::from(id));
            let found = t.elapsed();
            let t = Instant::now();
            let r = el.as_ref().map(|e| unsafe { e.CurrentBoundingRectangle() });
            println!("{id}: achou={} em {found:?}; retângulo {:?} em {:?}", el.is_some(), r.map(|r| r.map(|r| (r.left, r.top, r.right, r.bottom))), t.elapsed());
        }
    }
}
