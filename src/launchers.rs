//! Atalhos para abrir os apps de música instalados (linha embaixo do card).
//!
//! A lista vem da pasta "Aplicativos" do Windows (a mesma do Menu Iniciar):
//! cobre apps da Store (Spotify da Store, Apple Music, Media Player), programas
//! comuns (Spotify, foobar2000, VLC…) e apps instalados pelo navegador
//! (YouTube Music no Brave, Chrome ou Edge). Só entram os players conhecidos,
//! na ordem abaixo. Abrir é o mesmo que clicar no Menu Iniciar
//! (`shell:AppsFolder\<AUMID>`).

use crate::appicon;
use crate::audio::log;
use crate::shared::{Launcher, Shared};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    ShellExecuteW,
};
use windows::Win32::Foundation::{HWND, LPARAM, PROPERTYKEY};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SW_SHOWNORMAL,
    SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, HSTRING, w};

/// No máximo tantos atalhos (a linha cabe no card).
pub const MAX: usize = 7;
/// A lista é refeita de tempos em tempos (apps instalados ou removidos).
const RESCAN: Duration = Duration::from_secs(30 * 60);

/// Players conhecidos: nome na pasta "Aplicativos" (sem diferenciar
/// maiúsculas) e trechos do AUMID que identificam o app em qualquer idioma.
const KNOWN: &[(&str, &[&str])] = &[
    ("spotify", &["spotifyab.spotifymusic", "spotify.exe"]),
    // App do YouTube Music instalado pelo Brave, Chrome ou Edge (mesmo id em todos).
    ("youtube music", &["cinhimbnkkghhklpknlkffjgod"]),
    ("apple music", &["appleinc.applemusicwin"]),
    ("deezer", &["deezer"]),
    ("tidal", &["tidal"]),
    ("amazon music", &["amazonmobilellc.amazonmusic"]),
    ("qobuz", &["qobuz"]),
    ("soundcloud", &[]),
    ("pandora", &[]),
    ("plexamp", &[]),
    ("itunes", &[]),
    ("media player", &["microsoft.zunemusic"]),
    ("foobar2000", &[]),
    ("musicbee", &[]),
    ("aimp", &[]),
    ("winamp", &[]),
    ("mediamonkey", &[]),
    ("dopamine", &[]),
    ("vlc media player", &[]),
];

/// Qual player conhecido é este item da pasta "Aplicativos" (posição em `KNOWN`).
fn known(name: &str, aumid: &str) -> Option<usize> {
    let (name, id) = (name.trim().to_lowercase(), aumid.to_lowercase());
    // Desinstaladores e afins também aparecem na pasta.
    if ["uninstall", "desinstalar", "deinstallieren", "désinstaller"].iter().any(|u| name.contains(u)) {
        return None;
    }
    KNOWN.iter().position(|(n, ids)| name == *n || ids.iter().any(|i| id.contains(i)))
}

fn display(item: &IShellItem, kind: SIGDN) -> String {
    unsafe {
        item.GetDisplayName(kind)
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            })
            .unwrap_or_default()
    }
}

/// Players conhecidos instalados, na ordem de `KNOWN` (um de cada).
pub fn scan() -> Vec<Launcher> {
    let mut found: Vec<(usize, Launcher)> = Vec::new();
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return Vec::new() };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return Vec::new() };
        loop {
            let mut batch = [None];
            let mut n = 0u32;
            if items.Next(&mut batch, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let (name, id) = (display(&item, SIGDN_NORMALDISPLAY), display(&item, SIGDN_PARENTRELATIVEPARSING));
            let Some(k) = known(&name, &id) else { continue };
            if found.iter().any(|(j, _)| *j == k) {
                continue;
            }
            let icon = appicon::shell_item_icon(&item).map(Arc::new);
            found.push((k, Launcher { name, id, icon }));
        }
    }
    found.sort_by_key(|(k, _)| *k);
    found.into_iter().map(|(_, l)| l).take(MAX).collect()
}

/// Procura os apps em segundo plano: logo ao abrir e de tempos em tempos.
pub fn spawn(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("launchers".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        crate::system::background_thread();
        // Deixa o app abrir primeiro.
        std::thread::sleep(Duration::from_secs(3));
        while shared.is_running() {
            let list = scan();
            let names: Vec<&str> = list.iter().map(|l| l.name.as_str()).collect();
            let changed = {
                let cur = shared.launchers.lock().unwrap();
                cur.len() != list.len() || cur.iter().zip(&list).any(|(a, b)| a.id != b.id)
            };
            if changed {
                log::info(&format!("apps de música instalados: {names:?}"));
                *shared.launchers.lock().unwrap() = Arc::new(list);
                // Redesenha o card (e as prévias) com a lista nova.
                shared.config_version.fetch_add(1, Ordering::Relaxed);
            }
            let mut slept = Duration::ZERO;
            while slept < RESCAN && shared.is_running() {
                std::thread::sleep(Duration::from_secs(1));
                slept += Duration::from_secs(1);
            }
        }
        unsafe { CoUninitialize() };
    });
}

/// Abre o app. Se ele já estiver aberto, traz a janela dele para a frente: o
/// Menu Iniciar abriria outra (os apps instalados pelo navegador, como o
/// YouTube Music, abrem uma janela nova a cada clique).
pub fn open(id: &str) {
    if bring_forward(id, "") {
        return;
    }
    let target = HSTRING::from(format!("shell:AppsFolder\\{id}"));
    let id = id.to_string();
    // O shell prefere uma thread STA; o card não fica esperando.
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r = ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL);
        if r.0 as isize <= 32 {
            log::warn(&format!("não deu para abrir {id} (código {})", r.0 as isize));
        }
        CoUninitialize();
    });
}

// ---------------------------------------------------------------- janela do player

/// Uma janela principal na tela (ou minimizada).
struct Win {
    hwnd: HWND,
    pid: u32,
    title: String,
    /// AUMID da janela (os apps instalados pelo navegador e os da Store têm).
    aumid: String,
}

/// Identificador de app (AUMID) de uma janela: `PKEY_AppUserModel_ID`.
fn window_aumid(hwnd: HWND) -> String {
    const KEY: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };
    unsafe { SHGetPropertyStoreForWindow::<IPropertyStore>(hwnd).and_then(|store| store.GetValue(&KEY)).map(|v| v.to_string()).unwrap_or_default() }
}

fn top_windows() -> Vec<Win> {
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> BOOL {
        unsafe {
            let wins = &mut *(out.0 as *mut Vec<Win>);
            let shown = IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool();
            let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid());
            let tool = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0;
            if shown && !owned && !tool {
                let mut buf = [0u16; 512];
                let n = GetWindowTextW(hwnd, &mut buf);
                if n > 0 {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut pid));
                    wins.push(Win { hwnd, pid, title: String::from_utf16_lossy(&buf[..n as usize]), aumid: String::new() });
                }
            }
            true.into()
        }
    }
    let mut wins: Vec<Win> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut wins as *mut _ as isize));
    }
    wins
}

/// Janela do player da sessão `aumid` (de preferência a que mostra `title`).
/// Ordem: mesma identidade de app na janela (apps do navegador e da Store);
/// depois janelas dos processos do app, com o título da música no nome.
fn player_window(aumid: &str, title: &str) -> Option<HWND> {
    use crate::audio::process;
    let mut wins = top_windows();
    for w in &mut wins {
        w.aumid = window_aumid(w.hwnd);
    }
    if let Some(w) = wins.iter().find(|w| w.aumid.eq_ignore_ascii_case(aumid)) {
        return Some(w.hwnd);
    }
    // App instalado pelo navegador sem janela própria aberta: as outras janelas
    // do navegador não são ele (só servem se mostrarem a música tocando).
    if aumid.contains("_crx_") && title.is_empty() {
        return None;
    }
    let procs = process::processes();
    let mine: Vec<u32> = if let Some((family, _)) = aumid.split_once('!') {
        let family = family.to_lowercase();
        procs.iter().filter(|p| process::package_aumid(p.pid).is_some_and(|a| a.to_lowercase().starts_with(&family))).map(|p| p.pid).collect()
    } else {
        let want = process::stems(aumid);
        procs.iter().filter(|p| want.iter().any(|w| *w == p.stem)).map(|p| p.pid).collect()
    };
    let title = title.to_lowercase();
    let candidates: Vec<&Win> = wins.iter().filter(|w| mine.contains(&w.pid)).collect();
    candidates
        .iter()
        .find(|w| !title.is_empty() && w.title.to_lowercase().contains(&title))
        .or(candidates.first())
        .map(|w| w.hwnd)
}

/// Traz o player para a frente (restaura se estiver minimizado). Usado quando
/// o player não deixa o Windows mudar aleatório/repetir (navegadores): a
/// pessoa usa o botão do próprio player. `false` se a janela não foi achada.
pub fn focus_player(aumid: &str, title: &str) -> bool {
    let ok = bring_forward(aumid, title);
    if !ok {
        log::info(&format!("janela do player não encontrada ({aumid})"));
    }
    ok
}

/// Traz para a frente a janela do app `id` (restaura se estiver minimizada).
fn bring_forward(id: &str, title: &str) -> bool {
    let Some(hwnd) = player_window(id, title) else { return false };
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let ok = SetForegroundWindow(hwnd).as_bool();
        log::info(&format!("trazendo para a frente ({id}): {ok}"));
        ok || IsWindowVisible(hwnd).as_bool()
    }
}

#[cfg(test)]
mod tests {
    /// Qual janela seria trazida para a frente para o player atual (sem trazer).
    /// `cargo test --release player_window_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn player_window_here() {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;
        let m = Manager::RequestAsync().and_then(|op| op.join()).unwrap();
        let s = m.GetCurrentSession().unwrap();
        let aumid = s.SourceAppUserModelId().unwrap().to_string();
        let title = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).and_then(|p| p.Title()).map(|t| t.to_string()).unwrap_or_default();
        let t = std::time::Instant::now();
        let hwnd = super::player_window(&aumid, &title);
        let name = hwnd.map(|h| {
            let mut buf = [0u16; 256];
            let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(h, &mut buf) };
            String::from_utf16_lossy(&buf[..n as usize])
        });
        println!("{aumid} \"{title}\" → {name:?} ({:?})", t.elapsed());
    }

    #[test]
    fn known_players() {
        use super::known;
        assert_eq!(known("Spotify", "Spotify.exe"), Some(0));
        assert_eq!(known("Spotify Music", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), Some(0));
        assert_eq!(known("YouTube Music", "Brave._crx_cinhimbnkkghhklpknlkffjgod"), Some(1));
        assert_eq!(known("Reprodutor Multimídia", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), known("Media Player", ""));
        assert_eq!(known("Uninstall Spotify", "x"), None);
        assert_eq!(known("Discord", "com.squirrel.Discord.Discord"), None);
    }

    /// O que a pasta "Aplicativos" desta máquina tem de players.
    /// `cargo test --release launchers_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn launchers_here() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        let t = std::time::Instant::now();
        let list = super::scan();
        println!("{:?}", t.elapsed());
        for l in &list {
            println!("{} | {} | ícone {:?}", l.name, l.id, l.icon.as_ref().map(|i| (i.w, i.h)));
        }
    }
}

#[cfg(test)]
mod open_tests {
    /// Para cada atalho desta máquina, qual janela aberta seria trazida para a
    /// frente em vez de abrir outra (sem trazer).
    /// `cargo test --release shortcut_windows_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn shortcut_windows_here() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        }
        for l in super::scan() {
            let w = super::player_window(&l.id, "").map(|h| {
                let mut buf = [0u16; 256];
                let n = unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(h, &mut buf) };
                String::from_utf16_lossy(&buf[..n as usize])
            });
            println!("{} → {:?}", l.name, w);
        }
    }
}
