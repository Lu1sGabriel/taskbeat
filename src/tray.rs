//! Ícone na área de notificação. Clique: configurações; botão direito: menu
//! rápido. Roda na thread principal (janela oculta + loop de mensagens).

use crate::i18n::tr;
use crate::menu::{self, Entry, Header};
use crate::shared::MediaCmd;
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
use windows::core::{PCWSTR, w};

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
/// Sobreposição de letra: mostrar e travar/destravar.
const ID_LYRICS: usize = 107;
const ID_LYRICS_LOCK: usize = 108;
const ID_RESTART: usize = 198;
const ID_EXIT: usize = 199;

/// "Reiniciar o Taskbeat": ao sair, o `main` abre uma cópia nova.
pub static RESTART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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

/// Itens do menu da bandeja (refeitos a cada clique em um interruptor).
fn menu_entries() -> Vec<Entry> {
    let cfg = shared().config();
    let media = shared().media.lock().unwrap().clone();
    let mut v = Vec::new();
    if !media.title.is_empty() {
        let subtitle = if media.artist.is_empty() { media.app.clone() } else { media.artist.clone() };
        v.push(Entry::Header(Header {
            art: media.art.clone(),
            title: media.title.clone(),
            subtitle,
            playing: media.playing,
            can_prev: media.can_prev,
            can_next: media.can_next,
            can_toggle: media.can_toggle,
        }));
        v.push(Entry::Separator);
    }
    v.push(Entry::action(ID_SETTINGS as u32, "\u{E713}", tr("Configurações…")));
    v.push(Entry::Separator);
    v.push(Entry::toggle(ID_MEDIA as u32, "\u{E8D6}", tr("Música na barra de tarefas"), cfg.show_media, true));
    // Sem a música na barra, os botões não têm onde aparecer: desligado e apagado.
    v.push(Entry::toggle(ID_CONTROLS as u32, "\u{E768}", tr("Botões de controle na barra"), cfg.show_media && cfg.taskbar_controls, cfg.show_media));
    v.push(Entry::toggle(ID_VIZ as u32, "\u{E9E9}", tr("Barras de áudio dinâmicas"), cfg.show_visualizer, true));
    v.push(Entry::toggle(ID_POPUP as u32, "\u{E893}", tr("Cartão \"A seguir\""), cfg.next_track_popup, true));
    v.push(Entry::toggle(ID_CAPS as u32, "\u{E765}", tr("Aviso de Caps Lock"), cfg.caps_lock_osd, true));
    v.push(Entry::toggle(ID_LYRICS as u32, "\u{E7F0}", tr("Letra na tela"), cfg.lyrics.show, true));
    if cfg.lyrics.show {
        let (icon, label) = if cfg.lyrics.locked { ("\u{E785}", "Destravar a letra") } else { ("\u{E72E}", "Travar a letra") };
        v.push(Entry::action(ID_LYRICS_LOCK as u32, icon, tr(label)));
    }
    v.push(Entry::Separator);
    v.push(Entry::toggle(ID_AUTOSTART as u32, "\u{E7E8}", tr("Iniciar com o Windows"), system::autostart_enabled(), true));
    v.push(Entry::Separator);
    v.push(Entry::action(ID_RESTART as u32, "\u{E72C}", tr("Reiniciar o Taskbeat")));
    v.push(Entry::action(ID_EXIT as u32, "\u{E8BB}", tr("Sair do Taskbeat")));
    v
}

fn show_menu(hwnd: HWND) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    // Menus da bandeja seguem o tema do sistema (o da barra de tarefas).
    let dark = !system::taskbar_is_light();
    let opts = menu::Options { dark, accent: system::app_accent(dark), min_width: 260.0, style: crate::look::Style::of(&shared().config()) };
    let mut playing_override: Option<bool> = None;
    let cmd = menu::popup(hwnd, menu_entries(), menu::Anchor::Cursor(pt), opts, &mut |id| {
        let s = shared();
        match id {
            menu::ID_PREV => s.send_media(MediaCmd::Previous),
            menu::ID_NEXT => s.send_media(MediaCmd::Next),
            menu::ID_PLAY => {
                // Mostra o novo estado na hora (o player confirma logo depois).
                let now = playing_override.unwrap_or_else(|| s.media.lock().unwrap().playing);
                playing_override = Some(!now);
                s.send_media(MediaCmd::PlayPause);
            }
            other => handle_command(hwnd, other as usize),
        }
        let mut e = menu_entries();
        if let (Some(p), Some(Entry::Header(h))) = (playing_override, e.first_mut()) {
            h.playing = p;
        }
        settings::refresh();
        Some(e)
    });
    if let Some(id) = cmd {
        handle_command(hwnd, id as usize);
    }
    settings::refresh();
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
        ID_LYRICS => s.update_config(|c| c.lyrics.show = !c.lyrics.show),
        ID_LYRICS_LOCK => s.update_config(|c| c.lyrics.locked = !c.lyrics.locked),
        ID_RESTART | ID_EXIT => {
            if cmd == ID_RESTART {
                RESTART.store(true, Ordering::Relaxed);
            }
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
            settings::media_changed();
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
