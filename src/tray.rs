//! Ícone na área de notificação. Clique: configurações; botão direito: menu
//! rápido. Roda na thread principal (janela oculta + loop de mensagens).

use crate::i18n::tr;
use crate::render::{Canvas, RectF};
use crate::settings;
use crate::shared::Shared;
use crate::system;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_MEDIA: u32 = WM_APP + 2;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static mut ICON_SMALL: HICON = HICON(std::ptr::null_mut());
static mut ICON_LARGE: HICON = HICON(std::ptr::null_mut());

const ID_SETTINGS: usize = 100;
const ID_MEDIA: usize = 101;
const ID_CONTROLS: usize = 102;
const ID_VIZ: usize = 103;
const ID_POPUP: usize = 104;
const ID_CAPS: usize = 105;
const ID_AUTOSTART: usize = 106;
const ID_EXIT: usize = 199;

fn shared() -> &'static Arc<Shared> {
    SHARED.get().expect("estado compartilhado")
}

pub fn run(shared_state: Arc<Shared>) -> windows::core::Result<()> {
    let _ = SHARED.set(shared_state);
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("TaskbeatTray");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("Taskbeat"), WS_POPUP, 0, 0, 0, 0, None, None, Some(hinst), None)?;
        shared().tray_hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
        let dpi = GetDpiForSystem().max(96);
        ICON_SMALL = load_icon(hinst, GetSystemMetricsForDpi(SM_CXSMICON, dpi));
        ICON_LARGE = load_icon(hinst, GetSystemMetricsForDpi(SM_CXICON, dpi));
        add_icon(hwnd);
        // Primeira abertura pelo instalador: mostra as configurações para a pessoa
        // saber que o app está rodando (o widget só aparece com música tocando).
        if std::env::args().any(|a| a == "--settings") {
            settings::open(shared().clone(), ICON_LARGE);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        remove_icon(hwnd);
        let _ = DestroyIcon(ICON_SMALL);
        let _ = DestroyIcon(ICON_LARGE);
    }
    Ok(())
}

/// Ícone embutido no .exe (recurso 1) no tamanho exato; sem ele, desenha o logo.
unsafe fn load_icon(hinst: HINSTANCE, size: i32) -> HICON {
    unsafe {
        match LoadImageW(Some(hinst), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
            Ok(h) if !h.is_invalid() => HICON(h.0),
            _ => draw_icon(size),
        }
    }
}

unsafe fn draw_icon(size: i32) -> HICON {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        let mut cv = Canvas::new(size as usize, size as usize, px);
        cv.clear();
        settings::draw_logo(&mut cv, RectF::new(0.0, 0.0, size as f32, size as f32));
        let mask = CreateBitmap(size, size, 1, 1, None);
        let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        icon
    }
}

fn fill_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()) {
        *d = c;
        n += 1;
    }
    let n = n.min(dst.len() - 1);
    dst[n] = 0;
}

fn tooltip() -> String {
    let m = shared().media.lock().unwrap().clone();
    let mut s = String::from("Taskbeat");
    if !m.title.is_empty() {
        let state = if m.playing { "▶" } else { "❚❚" };
        let who = if m.artist.is_empty() { m.title.clone() } else { format!("{} – {}", m.artist, m.title) };
        s.push_str(&format!("\n{state} {who}"));
        if !m.app.is_empty() {
            s.push_str(&format!("\n{}", m.app));
        }
    }
    // szTip tem 128 posições: corta com reticências se precisar.
    if s.encode_utf16().count() > 127 {
        let mut out = String::new();
        for ch in s.chars() {
            if out.encode_utf16().count() + ch.len_utf16() > 125 {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        s = out;
    }
    s
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: unsafe { ICON_SMALL },
        ..Default::default()
    };
    fill_wide(&mut nid.szTip, &tooltip());
    nid
}

fn add_icon(hwnd: HWND) {
    let nid = icon_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn update_icon(hwnd: HWND) {
    let nid = icon_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

fn remove_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

fn append(menu: HMENU, id: usize, text: &str, checked: bool) {
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        let _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    }
}

fn show_menu(hwnd: HWND) {
    let cfg = shared().config();
    let media = shared().media.lock().unwrap().clone();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        if !media.title.is_empty() {
            let state = if media.playing { "▶" } else { "❚❚" };
            let mut line = format!("{state}  {}", media.title);
            if !media.artist.is_empty() {
                line.push_str(&format!(" — {}", media.artist));
            }
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(line));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        }
        let _ = AppendMenuW(menu, MF_STRING | MF_DEFAULT, ID_SETTINGS, &HSTRING::from(tr("Configurações…")));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append(menu, ID_MEDIA, tr("Música na barra de tarefas"), cfg.show_media);
        append(menu, ID_CONTROLS, tr("Botões de controle na barra"), cfg.taskbar_controls);
        append(menu, ID_VIZ, tr("Barras de áudio dinâmicas"), cfg.show_visualizer);
        append(menu, ID_POPUP, tr("Cartão \"A seguir\""), cfg.next_track_popup);
        append(menu, ID_CAPS, tr("Aviso de Caps Lock"), cfg.caps_lock_osd);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append(menu, ID_AUTOSTART, tr("Iniciar com o Windows"), system::autostart_enabled());
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        append(menu, ID_EXIT, tr("Sair do Taskbeat"), false);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        handle_command(hwnd, cmd.0 as usize);
        settings::refresh();
    }
}

fn handle_command(hwnd: HWND, cmd: usize) {
    let s = shared();
    match cmd {
        ID_SETTINGS => settings::open(s.clone(), unsafe { ICON_LARGE }),
        ID_MEDIA => s.update_config(|c| c.show_media = !c.show_media),
        ID_CONTROLS => s.update_config(|c| c.taskbar_controls = !c.taskbar_controls),
        ID_VIZ => s.update_config(|c| c.show_visualizer = !c.show_visualizer),
        ID_POPUP => s.update_config(|c| c.next_track_popup = !c.next_track_popup),
        ID_CAPS => s.update_config(|c| c.caps_lock_osd = !c.caps_lock_osd),
        ID_AUTOSTART => system::set_autostart(!system::autostart_enabled()),
        ID_EXIT => {
            settings::close();
            s.running.store(false, Ordering::Relaxed);
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let tc = TASKBAR_CREATED.load(Ordering::Relaxed);
    match msg {
        WM_APP_TRAY => {
            let ev = (lp.0 & 0xffff) as u32;
            if ev == WM_LBUTTONUP {
                settings::open(shared().clone(), unsafe { ICON_LARGE });
            } else if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_APP_MEDIA => {
            update_icon(hwnd);
            LRESULT(0)
        }
        // Comandos vindos de fora (ex.: o instalador pede para fechar com ID_EXIT).
        WM_COMMAND => {
            handle_command(hwnd, wp.0 & 0xffff);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // Explorer reiniciou: recoloca o ícone.
        m if m == tc && tc != 0 => {
            add_icon(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}
