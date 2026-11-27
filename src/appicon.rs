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
        let icon = unplated_logo(aumid).or_else(|| shell_item_icon(&item));
        (name.filter(|n| !n.is_empty() && n != aumid), icon)
    }
}

/// Logo "sem placa" de um app da Store (AUMID `Família!App`), lido dos
/// arquivos do pacote. No Windows 10 o ícone que o shell entrega vem sobre a
/// placa do bloco (o quadrado na cor do app, ex.: azul, atrás do ícone); o
/// Windows 11 não desenha a placa e usa estes mesmos arquivos
/// (`*.targetsize-N_altform-unplated.png`). Sem eles: `None` (fica o do shell).
pub fn unplated_logo(aumid: &str) -> Option<Image> {
    let (family, app) = aumid.split_once('!')?;
    let dir = package_dir(family)?;
    let manifest = std::fs::read_to_string(dir.join("AppxManifest.xml")).ok()?;
    let logo = square_logo(&manifest, app)?;
    decode_png(&unplated_file(&dir.join(logo))?)
}

/// Pasta de instalação do pacote da família `family` (a primeira versão instalada).
fn package_dir(family: &str) -> Option<std::path::PathBuf> {
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily};
    unsafe {
        let fam = HSTRING::from(family);
        let (mut count, mut len) = (0u32, 0u32);
        if GetPackagesByPackageFamily(&fam, &mut count, None, &mut len, None) != ERROR_INSUFFICIENT_BUFFER || count == 0 {
            return None;
        }
        let mut names = vec![PWSTR::null(); count as usize];
        let mut buf = vec![0u16; len as usize];
        if GetPackagesByPackageFamily(&fam, &mut count, Some(names.as_mut_ptr()), &mut len, Some(PWSTR(buf.as_mut_ptr()))) != ERROR_SUCCESS {
            return None;
        }
        let full = names.first()?.to_string().ok()?;
        let mut plen = 0u32;
        let _ = GetPackagePathByFullName(&HSTRING::from(&full), &mut plen, None);
        let mut path = vec![0u16; plen.max(1) as usize];
        if GetPackagePathByFullName(&HSTRING::from(&full), &mut plen, Some(PWSTR(path.as_mut_ptr()))) != ERROR_SUCCESS {
            return None;
        }
        let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&path[..end])))
    }
}

/// O `Square44x44Logo` (ícone da lista de apps) do aplicativo `app` no manifesto.
fn square_logo(manifest: &str, app: &str) -> Option<String> {
    let id = format!("Id=\"{app}\"");
    manifest.split("<Application ").skip(1).find_map(|block| {
        let block = block.split("</Application>").next().unwrap_or(block);
        let head = block.split('>').next().unwrap_or(block);
        if !head.contains(&id) {
            return None;
        }
        let rest = &block[block.find("Square44x44Logo=\"")? + "Square44x44Logo=\"".len()..];
        Some(rest[..rest.find('"')?].to_string())
    })
}

/// Arquivo "sem placa" do logo `logo` (ex.: `Assets\AppList.png` tem
/// `AppList.targetsize-48_altform-unplated.png` ao lado): o menor de pelo
/// menos 48 px, senão o maior. Sem as versões de alto contraste nem a do tema claro.
fn unplated_file(logo: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = logo.parent()?;
    let stem = logo.file_stem()?.to_string_lossy().to_lowercase();
    let mut best: Option<(u32, std::path::PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !name.starts_with(&format!("{stem}.")) || !name.ends_with(".png") || !name.contains("altform-unplated") || name.contains("contrast") {
            continue;
        }
        let size = name.split("targetsize-").nth(1).and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).unwrap_or(0);
        let better = |b: u32| if size >= SIZE_PX as u32 { b < SIZE_PX as u32 || size < b } else { b < size };
        if best.as_ref().is_none_or(|(b, _)| better(*b)) {
            best = Some((size, e.path()));
        }
    }
    best.map(|(_, p)| p)
}

/// Lê um PNG (WIC) como imagem BGRA pré-multiplicada.
fn decode_png(path: &std::path::Path) -> Option<Image> {
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    unsafe {
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let dec = f.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).ok()?;
        if w == 0 || h == 0 || w > 512 || h > 512 {
            return None;
        }
        let mut px = vec![0u32; (w * h) as usize];
        let bytes = std::slice::from_raw_parts_mut(px.as_mut_ptr().cast::<u8>(), px.len() * 4);
        conv.CopyPixels(std::ptr::null(), w * 4, bytes).ok()?;
        Some(Image::new(w as usize, h as usize, px))
    }
}

/// Ícone de um item do shell (ex.: um app da pasta "Aplicativos").
pub fn shell_item_icon(item: &IShellItem) -> Option<Image> {
    unsafe {
        let f = item.cast::<IShellItemImageFactory>().ok()?;
        let hbm = f.GetImage(SIZE { cx: SIZE_PX, cy: SIZE_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let img = hbitmap_to_image(hbm);
        let _ = DeleteObject(hbm.into());
        img
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
    exe_icon(&find_exe_path(&exe_candidates(aumid, app))?)
}

/// Caminho do .exe de um processo.
pub(crate) fn exe_path(pid: u32) -> Option<String> {
    process_path(pid)
}

/// Ícone de um .exe.
pub(crate) fn exe_icon(path: &str) -> Option<Image> {
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

#[cfg(test)]
mod logo_probe {
    /// Logo sem placa de alguns apps da Store desta máquina, gravado em
    /// `TASKBEAT_SHOTS_DIR` (só lê). `cargo test unplated_logos_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn unplated_logos_here() {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let dir = std::path::PathBuf::from(std::env::var_os("TASKBEAT_SHOTS_DIR").expect("TASKBEAT_SHOTS_DIR"));
        for aumid in ["Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic", "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"] {
            let img = super::unplated_logo(aumid);
            println!("{aumid}: {:?}", img.as_ref().map(|i| (i.w, i.h)));
            if let Some(i) = img {
                crate::png::save(&dir.join(format!("{}.png", aumid.split('_').next().unwrap())), i.w, i.h, &i.px);
            }
        }
    }
}
