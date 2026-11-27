//! Atalhos para abrir os apps de música instalados (linha embaixo do card).
//!
//! A lista vem da pasta "Aplicativos" do Windows (a mesma do Menu Iniciar):
//! cobre apps da Store (Spotify da Store, Apple Music, Media Player), programas
//! comuns (Spotify, foobar2000, VLC…) e apps instalados pelo navegador
//! (YouTube Music no Brave, Chrome ou Edge). Só entram os players conhecidos,
//! na ordem abaixo. Abrir é o mesmo que clicar no Menu Iniciar
//! (`shell:AppsFolder\<AUMID>`).
//!
//! A lista é refeita logo que um app é instalado ou removido (`Watch`): o
//! Windows avisa quando mudam as pastas de atalhos do Menu Iniciar ou a lista
//! de pacotes da Store. Sem aviso, a thread fica parada esperando.

use crate::appicon;
use crate::audio::log;
use crate::shared::{Launcher, Shared};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_NAME, RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::Win32::UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_Programs, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
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
/// Sem aviso do Windows, a lista ainda é refeita de tempos em tempos (por garantia).
const RESCAN: Duration = Duration::from_secs(30 * 60);
/// Depois de um aviso, espera o instalador parar de mexer nas pastas
/// (sem aviso novo por este tempo)...
const SETTLE: Duration = Duration::from_secs(2);
/// ... mas não mais que isto.
const SETTLE_MAX: Duration = Duration::from_secs(15);
/// Chave onde o Windows lista os pacotes (apps da Store) do usuário.
const PACKAGES_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// Players conhecidos: nome na pasta "Aplicativos" (sem diferenciar
/// maiúsculas) e trechos do AUMID que identificam o app em qualquer idioma.
/// A ordem é também a prioridade do atalho escolhido sozinho (`arranged`).
/// Os de `GENERAL` (players de vídeo e gerais) entram na lista, mas só
/// aparecem se o usuário ligar: o atalho é para apps de música.
const KNOWN: &[(&str, &[&str])] = &[
    ("spotify", &["spotifyab.spotifymusic", "spotify.exe"]),
    // App do YouTube Music instalado pelo Brave, Chrome ou Edge (mesmo id em todos).
    ("youtube music", &["cinhimbnkkghhklpknlkffjgod"]),
    // Shazam instalado pelo navegador (shazam.com) ou outro app com esse nome.
    ("shazam", &[]),
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

/// Players gerais e de vídeo (nomes de `KNOWN`): o Media Player do Windows
/// (o antigo Groove, que também toca vídeo) e o VLC. Escondidos por padrão.
const GENERAL: &[&str] = &["media player", "vlc media player"];

/// App de música conhecido (Spotify, YouTube Music, Shazam, Apple Music,
/// Deezer…), e não um player geral ou de vídeo nem um navegador: tem a vez
/// na escolha da música que o widget mostra (`media`).
pub fn is_music(name: &str, aumid: &str) -> bool {
    known(name, aumid).is_some() && !general(name, aumid)
}

/// Player geral ou de vídeo (não aparece sozinho como atalho).
fn general(name: &str, aumid: &str) -> bool {
    known(name, aumid).is_some_and(|k| GENERAL.contains(&KNOWN[k].0))
}

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

/// Players conhecidos instalados, na ordem de `KNOWN` (um de cada; todos: o
/// limite de `MAX` vale só para os que ficam à mostra).
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
            // App da Store: o logo sem a placa do Windows 10 (o quadrado atrás).
            let icon = appicon::unplated_logo(&id).or_else(|| appicon::shell_item_icon(&item)).map(Arc::new);
            found.push((k, Launcher { name, id, icon }));
        }
    }
    found.sort_by_key(|(k, _)| *k);
    found.into_iter().map(|(_, l)| l).collect()
}

/// Todos os apps achados na ordem escolhida pelo usuário, cada um com "à
/// mostra?". Os que estão em `order` vêm primeiro, nessa ordem; os outros
/// (achados depois, nunca mexidos) entram no fim, na ordem de sempre, à mostra
/// se forem de música (um player geral, como o VLC, entra escondido).
pub fn arrange(found: &[Launcher], order: &[String], hidden: &[String]) -> Vec<(Launcher, bool)> {
    let mut out: Vec<(Launcher, bool)> = order.iter().filter_map(|id| found.iter().find(|l| &l.id == id)).map(|l| (l.clone(), true)).collect();
    let listed = out.len();
    for l in found {
        if !out.iter().any(|(o, _)| o.id == l.id) {
            out.push((l.clone(), true));
        }
    }
    for (i, (l, on)) in out.iter_mut().enumerate() {
        *on = !hidden.contains(&l.id) && (i < listed || !general(&l.name, &l.id));
    }
    out
}

/// Os atalhos à mostra, na ordem escolhida (no máximo `MAX`).
pub fn shown(found: &[Launcher], cfg: &crate::config::Config) -> Vec<Launcher> {
    arranged(found, cfg).into_iter().filter(|(_, on)| *on).map(|(l, _)| l).take(MAX).collect()
}

/// Todos os apps achados como aparecem (ordem e "à mostra?"). Quem nunca mexeu
/// na lista (ordem e escondidos vazios) fica com UM atalho, escolhido sozinho:
/// o primeiro app de música instalado na ordem de `KNOWN` (Spotify, YouTube
/// Music, Shazam, os outros). Sem nenhum app de música, nenhum atalho (um
/// player geral, como o VLC, só se o usuário ligar). Quem já escolheu fica
/// com a própria escolha.
pub fn arranged(found: &[Launcher], cfg: &crate::config::Config) -> Vec<(Launcher, bool)> {
    if cfg.launcher_order.is_empty() && cfg.launcher_hidden.is_empty() {
        let mut all = arrange(found, &[], &[]);
        all.sort_by_key(|(l, _)| known(&l.name, &l.id).unwrap_or(usize::MAX));
        let first = all.iter().position(|(l, _)| !general(&l.name, &l.id));
        for (i, (_, on)) in all.iter_mut().enumerate() {
            *on = Some(i) == first;
        }
        return all;
    }
    arrange(found, &cfg.launcher_order, &cfg.launcher_hidden)
}

/// Ordem (todos os ids) e escondidos a gravar a partir da lista como aparece.
/// Grava os dois juntos: a primeira mudança de quem estava na escolha
/// automática guarda o que ela mostrava (senão todos apareceriam de uma vez).
pub fn lists_of(all: &[(Launcher, bool)]) -> (Vec<String>, Vec<String>) {
    (all.iter().map(|(l, _)| l.id.clone()).collect(), all.iter().filter(|(_, on)| !*on).map(|(l, _)| l.id.clone()).collect())
}

/// Ordem nova (todos os ids) com o app `id` movido para a posição `to` da lista completa.
pub fn moved(all: &[(Launcher, bool)], id: &str, to: usize) -> Vec<String> {
    let mut ids: Vec<String> = all.iter().map(|(l, _)| l.id.clone()).collect();
    if let Some(from) = ids.iter().position(|i| i == id) {
        let item = ids.remove(from);
        ids.insert(to.min(ids.len()), item);
    }
    ids
}

/// Posição na lista completa do `k`-ésimo atalho à mostra.
pub fn full_index(all: &[(Launcher, bool)], k: usize) -> Option<usize> {
    all.iter().enumerate().filter(|(_, (_, on))| *on).nth(k).map(|(i, _)| i)
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
        let watch = Watch::start_menu_and_store();
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
            // Espera um aviso (app instalado ou removido) ou o tempo de garantia.
            let t = Instant::now();
            let mut changed = false;
            while !changed && t.elapsed() < RESCAN && shared.is_running() {
                changed = watch.wait(Duration::from_secs(1));
            }
            if changed {
                // Um instalador cria várias pastas e atalhos: espera aquietar.
                let t = Instant::now();
                while t.elapsed() < SETTLE_MAX && shared.is_running() && watch.wait(SETTLE) {}
            }
        }
        drop(watch);
        unsafe { CoUninitialize() };
    });
}

/// De onde vem cada aviso de `Watch`.
enum Source {
    /// Pasta vigiada (rearma com `FindNextChangeNotification`).
    Dir,
    /// Chave do registro (rearma com outro `RegNotifyChangeKeyValue`).
    Key(HKEY),
}

/// Avisos do Windows quando algo muda nas pastas e chaves vigiadas: a thread
/// dorme no próprio Windows até um deles (custo zero parado).
struct Watch {
    handles: Vec<HANDLE>,
    sources: Vec<Source>,
}

impl Watch {
    /// As pastas de atalhos do Menu Iniciar (do usuário e de todos, com as
    /// subpastas: programas comuns e os apps instalados pelo navegador, como o
    /// YouTube Music) e a lista de pacotes da Store.
    fn start_menu_and_store() -> Self {
        let mut dirs = Vec::new();
        for id in [&FOLDERID_Programs, &FOLDERID_CommonPrograms] {
            unsafe {
                if let Ok(p) = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None) {
                    dirs.push(p.to_string().unwrap_or_default());
                    CoTaskMemFree(Some(p.0 as _));
                }
            }
        }
        let mut w = Self::dirs(&dirs);
        w.add_key(PACKAGES_KEY);
        log::info(&format!("lista de apps: vigiando {} pastas e chaves", w.handles.len()));
        w
    }

    /// Vigia as pastas `dirs` (com as subpastas): arquivos ou pastas criados,
    /// renomeados ou apagados.
    fn dirs(dirs: &[String]) -> Self {
        let mut w = Watch { handles: Vec::new(), sources: Vec::new() };
        for d in dirs.iter().filter(|d| !d.is_empty()) {
            let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME;
            if let Ok(h) = unsafe { FindFirstChangeNotificationW(&HSTRING::from(d.as_str()), true, filter) } {
                w.handles.push(h);
                w.sources.push(Source::Dir);
            }
        }
        w
    }

    /// Vigia a chave `path` de `HKEY_CURRENT_USER` (subchaves criadas ou apagadas).
    fn add_key(&mut self, path: &str) {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_NOTIFY, &mut key) != ERROR_SUCCESS {
                return;
            }
            let Ok(ev) = CreateEventW(None, false, false, None) else {
                let _ = RegCloseKey(key);
                return;
            };
            if RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_NAME, Some(ev), true) != ERROR_SUCCESS {
                let _ = CloseHandle(ev);
                let _ = RegCloseKey(key);
                return;
            }
            self.handles.push(ev);
            self.sources.push(Source::Key(key));
        }
    }

    /// Espera até `max` por um aviso. `true` se algo mudou (e já volta a vigiar).
    fn wait(&self, max: Duration) -> bool {
        if self.handles.is_empty() {
            std::thread::sleep(max);
            return false;
        }
        let r = unsafe { WaitForMultipleObjects(&self.handles, false, max.as_millis() as u32) };
        let i = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if i >= self.handles.len() {
            return false;
        }
        unsafe {
            match self.sources[i] {
                Source::Dir => {
                    let _ = FindNextChangeNotification(self.handles[i]);
                }
                Source::Key(key) => {
                    let _ = RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_NAME, Some(self.handles[i]), true);
                }
            }
        }
        true
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        for (h, src) in self.handles.iter().zip(&self.sources) {
            unsafe {
                match src {
                    Source::Dir => {
                        let _ = FindCloseChangeNotification(*h);
                    }
                    Source::Key(key) => {
                        let _ = RegCloseKey(*key);
                        let _ = CloseHandle(*h);
                    }
                }
            }
        }
    }
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
pub(crate) fn player_window(aumid: &str, title: &str) -> Option<HWND> {
    find_player_window(aumid, title, false)
}

/// Como `player_window`, mas só aceita a janela do próprio app (mesma
/// identidade) ou uma janela dos processos dele que mostre a música no
/// título: nunca "a primeira janela do navegador", que pode ser outro site.
pub(crate) fn player_window_exact(aumid: &str, title: &str) -> Option<HWND> {
    find_player_window(aumid, title, true)
}

fn find_player_window(aumid: &str, title: &str, exact: bool) -> Option<HWND> {
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
    let mine = app_pids(aumid);
    let title = title.to_lowercase();
    let candidates: Vec<&Win> = wins.iter().filter(|w| mine.contains(&w.pid)).collect();
    candidates
        .iter()
        .find(|w| !title.is_empty() && w.title.to_lowercase().contains(&title))
        .or(if exact { None } else { candidates.first() })
        .map(|w| w.hwnd)
}

/// Processos do app `aumid` (o pacote, ou os do .exe: o navegador inteiro).
fn app_pids(aumid: &str) -> Vec<u32> {
    use crate::audio::process;
    let procs = process::processes();
    if let Some((family, _)) = aumid.split_once('!') {
        let family = family.to_lowercase();
        procs.iter().filter(|p| process::package_aumid(p.pid).is_some_and(|a| a.to_lowercase().starts_with(&family))).map(|p| p.pid).collect()
    } else {
        let want = process::stems(aumid);
        procs.iter().filter(|p| want.iter().any(|w| *w == p.stem)).map(|p| p.pid).collect()
    }
}

/// Janelas dos processos do app `aumid` (numa aba: as do navegador), com o título.
pub(crate) fn app_windows(aumid: &str) -> Vec<(HWND, String)> {
    let mine = app_pids(aumid);
    top_windows().into_iter().filter(|w| mine.contains(&w.pid)).map(|w| (w.hwnd, w.title)).collect()
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
    /// Um atalho novo numa pasta vigiada (como o de um app instalado no Menu
    /// Iniciar) acorda a espera; sem mudança, ela só volta no fim do prazo.
    #[test]
    fn aviso_de_app_instalado() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("taskbeat-watch-{}", std::process::id()));
        let sub = dir.join("Brave Apps");
        std::fs::create_dir_all(&sub).unwrap();
        let w = super::Watch::dirs(&[dir.to_string_lossy().into_owned()]);
        assert!(!w.wait(Duration::from_millis(50)));
        std::fs::write(sub.join("Spotify.lnk"), b"x").unwrap();
        assert!(w.wait(Duration::from_secs(5)));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }
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
    fn ordem_e_atalhos_escondidos() {
        use super::{arrange, full_index, moved};
        use crate::shared::Launcher;
        let l = |id: &str| Launcher { name: id.into(), id: id.into(), icon: None };
        // Achados na ordem de sempre: YouTube Music, Media Player, VLC.
        let found = [l("ytm"), l("zune"), l("vlc")];
        // Nada escolhido: todos à mostra, na ordem de sempre.
        let all = arrange(&found, &[], &[]);
        assert_eq!(all.iter().map(|(l, on)| (l.id.as_str(), *on)).collect::<Vec<_>>(), [("ytm", true), ("zune", true), ("vlc", true)]);
        // Só o YouTube Music à mostra; VLC antes do Media Player; um app que
        // não está mais instalado na ordem é ignorado; um novo entra no fim.
        let order = ["vlc".to_string(), "gone".into(), "zune".into(), "ytm".into()];
        let hidden = ["vlc".to_string(), "zune".into()];
        let found2 = [l("ytm"), l("zune"), l("vlc"), l("spotify")];
        let all = arrange(&found2, &order, &hidden);
        assert_eq!(all.iter().map(|(l, on)| (l.id.as_str(), *on)).collect::<Vec<_>>(), [("vlc", false), ("zune", false), ("ytm", true), ("spotify", true)]);
        // O 2º à mostra (spotify) é o 4º da lista; arrastá-lo para o começo.
        assert_eq!(full_index(&all, 1), Some(3));
        assert_eq!(moved(&all, "spotify", 0), ["spotify", "vlc", "zune", "ytm"]);
        assert_eq!(moved(&all, "vlc", 9), ["zune", "ytm", "spotify", "vlc"]);
    }

    /// Sem escolha feita, um atalho só: o primeiro instalado na ordem de
    /// prioridade (Spotify, YouTube Music, Shazam, os outros). Quem já
    /// escolheu fica com a própria escolha; a primeira mudança grava a lista.
    #[test]
    fn atalho_escolhido_por_prioridade() {
        use super::{arranged, lists_of, moved};
        use crate::config::Config;
        use crate::shared::Launcher;
        let l = |name: &str, id: &str| Launcher { name: name.into(), id: id.into(), icon: None };
        let (spotify, ytm, shazam, vlc, zune) = (
            l("Spotify", "Spotify.exe"),
            l("YouTube Music", "Brave._crx_cinhimbnkkghhklpknlkffjgod"),
            l("Shazam", "Chrome._crx_shazam"),
            l("VLC media player", "vlc"),
            l("Media Player", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
        );
        let on = |found: &[Launcher], cfg: &Config| arranged(found, cfg).into_iter().filter(|(_, on)| *on).map(|(l, _)| l.name).collect::<Vec<_>>();
        let auto = Config::default();
        assert_eq!(on(&[vlc.clone(), ytm.clone(), spotify.clone(), zune.clone()], &auto), ["Spotify"]);
        assert_eq!(on(&[zune.clone(), vlc.clone(), ytm.clone()], &auto), ["YouTube Music"]);
        assert_eq!(on(&[vlc.clone(), shazam.clone(), zune.clone()], &auto), ["Shazam"]);
        // Só players gerais (o caso de um Windows 10 sem app de música):
        // nenhum atalho sozinho; eles ficam na lista para quem quiser ligar.
        assert!(on(&[vlc.clone(), zune.clone()], &auto).is_empty());
        assert_eq!(arranged(&[vlc.clone(), zune.clone()], &auto).len(), 2);
        assert!(on(&[], &auto).is_empty());
        // Escolha feita (como a do usuário: só o YouTube Music) continua valendo,
        // mesmo com o Spotify instalado depois.
        let found = [ytm.clone(), zune.clone(), vlc.clone()];
        let mine = Config { launcher_order: vec![ytm.id.clone(), zune.id.clone(), vlc.id.clone()], launcher_hidden: vec![zune.id.clone(), vlc.id.clone()], ..Config::default() };
        assert_eq!(on(&found, &mine), ["YouTube Music"]);
        // Um player geral instalado depois entra escondido; um de música, à mostra.
        let found3 = [ytm.clone(), zune.clone(), vlc.clone(), spotify.clone()];
        let mine2 = Config { launcher_order: vec![ytm.id.clone()], launcher_hidden: vec![], ..Config::default() };
        assert_eq!(on(&[ytm.clone(), vlc.clone()], &mine2), ["YouTube Music"]);
        assert_eq!(on(&found3, &mine2), ["YouTube Music", "Spotify"]);
        // Primeira mudança a partir da escolha automática: arrastar o VLC para o
        // começo grava a ordem e mantém escondidos os que estavam.
        let all = arranged(&found, &auto);
        let (order, hidden) = lists_of(&all);
        let order2 = moved(&all, &vlc.id, 0);
        assert_eq!(order, [ytm.id.clone(), zune.id.clone(), vlc.id.clone()]);
        assert_eq!(hidden, [zune.id.clone(), vlc.id.clone()]);
        let after = Config { launcher_order: order2, launcher_hidden: hidden, ..Config::default() };
        assert_eq!(on(&found, &after), ["YouTube Music"]);
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
