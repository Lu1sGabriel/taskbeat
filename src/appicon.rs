//! Nome e ícone do aplicativo que está tocando.
//! 1) Pasta "Aplicativos" do shell (FOLDERID_AppsFolder + AppUserModelId): funciona
//!    para apps da Microsoft Store (Spotify da Store, WhatsApp, Media Player...) e
//!    para programas que registram AUMID (Chrome, Edge, Brave...).
//! 2) Plano B: procura o processo pelo nome do .exe e extrai o ícone do executável.

use crate::render::Image;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW};
use windows::Win32::Foundation::SIZE;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_AppsFolder, IShellItem, IShellItemImageFactory, KF_FLAG_DEFAULT, SHCreateItemInKnownFolder,
    SIGDN_NORMALDISPLAY, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::core::{HSTRING, Interface, PWSTR};

/// (nome de exibição, ícone) a partir da pasta Aplicativos do shell.
pub fn from_apps_folder(aumid: &str) -> (Option<String>, Option<Image>) {
    if aumid.is_empty() {
        return (None, None);
    }
    unsafe {
        let Ok(item) = SHCreateItemInKnownFolder::<_, IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, &HSTRING::from(aumid))
        else {
            return (None, None);
        };
        let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            s
        });
        let icon = item.cast::<IShellItemImageFactory>().ok().and_then(|f| {
            let hbm = f.GetImage(SIZE { cx: SIZE_PX, cy: SIZE_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
            let img = hbitmap_to_image(hbm);
            let _ = DeleteObject(hbm.into());
            img
        });
        (name.filter(|n| !n.is_empty() && n != aumid), icon)
    }
}

/// Converte um HBITMAP 32 bpp (com ou sem alfa pré-multiplicado) em `Image`.
unsafe fn hbitmap_to_image(hbm: HBITMAP) -> Option<Image> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(hbm.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 512 || h > 512 {
            return None;
        }
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u32; (w * h) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbm, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        normalize_alpha(&mut px);
        Some(Image::new(w as usize, h as usize, px))
    }
}

/// Garante BGRA pré-multiplicado: sem alfa -> opaco; alfa "reto" -> pré-multiplica.
fn normalize_alpha(px: &mut [u32]) {
    let has_alpha = px.iter().any(|p| p >> 24 != 0);
    let straight = px.iter().any(|p| {
        let a = p >> 24;
        ((p >> 16) & 0xff) > a || ((p >> 8) & 0xff) > a || (p & 0xff) > a
    });
    for p in px.iter_mut() {
        let a = if has_alpha { (*p >> 24) & 0xff } else { 255 };
        let (mut r, mut g, mut b) = ((*p >> 16) & 0xff, (*p >> 8) & 0xff, *p & 0xff);
        if straight || !has_alpha {
            r = r * a / 255;
            g = g * a / 255;
            b = b * a / 255;
        }
        *p = (a << 24) | (r << 16) | (g << 8) | b;
    }
}

const SIZE_PX: i32 = 48;

/// Nomes de executável prováveis para a sessão.
fn exe_candidates(aumid: &str, app: &str) -> Vec<String> {
    let mut out = Vec::new();
    let last = aumid.rsplit(['\\', '/']).next().unwrap_or(aumid);
    if last.to_lowercase().ends_with(".exe") {
        out.push(last.to_lowercase());
    }
    let known: &[(&str, &str)] = &[
        ("Spotify", "spotify.exe"),
        ("Google Chrome", "chrome.exe"),
        ("Microsoft Edge", "msedge.exe"),
        ("Firefox", "firefox.exe"),
        ("Opera", "opera.exe"),
        ("Brave", "brave.exe"),
        ("Vivaldi", "vivaldi.exe"),
        ("Arc", "arc.exe"),
        ("Apple Music", "applemusic.exe"),
        ("iTunes", "itunes.exe"),
        ("Deezer", "deezer.exe"),
        ("TIDAL", "tidal.exe"),
        ("Amazon Music", "amazon music.exe"),
        ("YouTube Music", "youtube music.exe"),
        ("VLC", "vlc.exe"),
        ("foobar2000", "foobar2000.exe"),
        ("AIMP", "aimp.exe"),
        ("MusicBee", "musicbee.exe"),
        ("Winamp", "winamp.exe"),
        ("Media Player", "microsoft.media.player.exe"),
        ("SoundCloud", "soundcloud.exe"),
        ("WhatsApp", "whatsapp.exe"),
    ];
    for (name, exe) in known {
        if *name == app {
            out.push((*exe).to_string());
        }
    }
    out.push(format!("{}.exe", app.to_lowercase()));
    out.push(format!("{}.exe", app.to_lowercase().replace(' ', "")));
    out
}

fn find_exe_path(candidates: &[String]) -> Option<String> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
            if candidates.iter().any(|c| *c == name) {
                if let Some(path) = process_path(entry.th32ProcessID) {
                    found = Some(path);
                    break;
                }
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

pub fn app_icon(aumid: &str, app: &str) -> Option<Image> {
    let path = find_exe_path(&exe_candidates(aumid, app))?;
    let mut name = [0u16; 260];
    for (d, s) in name.iter_mut().zip(path.encode_utf16().take(259)) {
        *d = s;
    }
    unsafe {
        let mut icons = [HICON::default()];
        let n = PrivateExtractIconsW(&name, 0, SIZE_PX, SIZE_PX, Some(&mut icons), None, 0);
        if n == 0 || n == u32::MAX || icons[0].is_invalid() {
            return None;
        }
        let img = hicon_to_image(icons[0]);
        let _ = DestroyIcon(icons[0]);
        img
    }
}

unsafe fn hicon_to_image(icon: HICON) -> Option<Image> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let img = if info.hbmColor.is_invalid() { None } else { hbitmap_to_image(info.hbmColor) };
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        img
    }
}
