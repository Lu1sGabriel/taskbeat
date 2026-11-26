//! Instalador e desinstalador, no mesmo visual do app (Mica, Fluent, DirectWrite).
//!
//! O próprio `taskbeat.exe` é o instalador: distribuído como
//! `Taskbeat-Setup-<versão>.exe` (nome com "setup") ou rodando com `--install`,
//! ele abre esta janela, copia a si mesmo para a pasta escolhida como
//! `Taskbeat.exe`, cria atalhos e se registra em "Apps instalados". A cópia
//! instalada roda com `--uninstall` para remover tudo.
//!
//! Tudo é por usuário (`HKEY_CURRENT_USER`, pasta do usuário): não pede
//! administrador.

use crate::config::Config;
use crate::i18n::tr;
use crate::overlay::Surface;
use crate::render::{Canvas, RectF, Rgb, lerp3};
use crate::system;
use crate::text::{Align, Font, TextRenderer};
use crate::ui;
use std::cell::RefCell;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, w};

const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Taskbeat";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const ID_EXIT: usize = 199;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_APP_DONE: u32 = WM_APP + 20;
const TIMER: usize = 1;
const W: f32 = 600.0;
const H: f32 = 452.0;
const STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Install,
    Uninstall,
}

/// Como este processo foi aberto: instalador, desinstalador ou o app normal.
pub fn mode_from_args() -> Option<Mode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a.eq_ignore_ascii_case("--uninstall") || a.eq_ignore_ascii_case("/uninstall")) {
        return Some(Mode::Uninstall);
    }
    if args.iter().any(|a| a.eq_ignore_ascii_case("--install")) {
        return Some(Mode::Install);
    }
    let name = std::env::current_exe().ok()?.file_stem()?.to_string_lossy().to_ascii_lowercase();
    name.contains("setup").then_some(Mode::Install)
}

// ---------------------------------------------------------------- estado

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Change,
    Licenses,
    Opt(usize),
    Cancel,
    Primary,
}

enum Phase {
    Ready,
    Working(Instant),
    Done,
    Failed(String),
}

/// O que o instalador encontrou na máquina.
#[derive(Clone, Debug, PartialEq)]
enum Found {
    /// Nada instalado: instalação normal (escolhe pasta e opções).
    Fresh,
    /// Versão mais antiga instalada: atualiza no lugar, sem desinstalar.
    Older(String),
    /// A mesma versão: reinstala (repara) por cima.
    Same,
    /// Uma versão mais nova já está instalada.
    Newer(String),
}

/// "1.10.2" → (1, 10, 2). Partes que faltam ou não são números valem 0.
fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Compara a versão instalada (se houver) com a deste instalador.
fn classify(installed: Option<&str>, mine: &str) -> Found {
    let Some(old) = installed.map(str::trim).filter(|v| !v.is_empty()) else { return Found::Fresh };
    match parse_version(old).cmp(&parse_version(mine)) {
        std::cmp::Ordering::Less => Found::Older(old.to_string()),
        std::cmp::Ordering::Equal => Found::Same,
        std::cmp::Ordering::Greater => Found::Newer(old.to_string()),
    }
}

/// Instalação existente: versão registrada, se o executável ainda estiver lá.
fn installed_version() -> Option<String> {
    // Depuração: `$env:TASKBEAT_FAKE_INSTALLED="1.0.1"` simula uma instalação.
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("TASKBEAT_FAKE_INSTALLED") {
        return Some(v);
    }
    let dir = reg_get_sz(UNINSTALL_KEY, "InstallLocation").filter(|d| !d.is_empty())?;
    PathBuf::from(dir).join("Taskbeat.exe").exists().then(|| reg_get_sz(UNINSTALL_KEY, "DisplayVersion").unwrap_or_else(|| "0".into()))
}

struct St {
    hwnd: HWND,
    mode: Mode,
    /// Já existe uma instalação (atualiza no lugar).
    update: bool,
    found: Found,
    scale: f32,
    dark: bool,
    mica: bool,
    accent: Rgb,
    dir: PathBuf,
    /// Instalar: [iniciar com o Windows, atalho na área de trabalho, abrir ao terminar].
    /// Remover: [apagar configurações].
    opts: Vec<bool>,
    phase: Phase,
    /// Resultado do trabalho em segundo plano (chegou, mas a barra ainda está terminando).
    result: Option<Result<(), String>>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    tracking: bool,
    surface: Option<Surface>,
    text: TextRenderer,
    t0: Instant,
}

thread_local! {
    static ST: RefCell<Option<St>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut St) -> R) -> Option<R> {
    ST.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn opt_labels(mode: Mode) -> &'static [&'static str] {
    match mode {
        Mode::Install => &["Iniciar com o Windows", "Atalho na área de trabalho", "Abrir o Taskbeat ao terminar"],
        Mode::Uninstall => &["Apagar também minhas configurações"],
    }
}

// ---------------------------------------------------------------- caminhos

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

fn start_menu_link() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|d| d.join("Taskbeat.lnk"))
}

fn desktop_link() -> Option<PathBuf> {
    known_folder(&FOLDERID_Desktop).map(|d| d.join("Taskbeat.lnk"))
}

fn default_dir() -> PathBuf {
    if let Some(d) = reg_get_sz(UNINSTALL_KEY, "InstallLocation").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
    local.join("Programs").join("Taskbeat")
}

fn config_dir() -> Option<PathBuf> {
    Config::path().and_then(|p| p.parent().map(Path::to_path_buf))
}

// ---------------------------------------------------------------- registro

pub(crate) fn reg_set_sz(key: &str, name: &str, value: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_SZ.0, Some(wide.as_ptr() as _), (wide.len() * 2) as u32).ok() }
}

pub(crate) fn reg_set_dword(key: &str, name: &str, value: u32) -> windows::core::Result<()> {
    unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), REG_DWORD.0, Some(&value as *const u32 as _), 4).ok() }
}

pub(crate) fn reg_get_sz(key: &str, name: &str) -> Option<String> {
    unsafe {
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let r = RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut size));
        if r != ERROR_SUCCESS {
            return None;
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

fn reg_delete_value(key: &str, name: &str) {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name));
    }
}

// ---------------------------------------------------------------- app aberto

fn other_taskbeat_pids() -> Vec<u32> {
    let me = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            if e.th32ProcessID != me && String::from_utf16_lossy(&e.szExeFile[..n]).eq_ignore_ascii_case("taskbeat.exe") {
                out.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Fecha o Taskbeat aberto: pede pelo item "Sair" da bandeja e, se não fechar
/// em 3 s, encerra o processo.
fn stop_running_app() {
    unsafe {
        if let Ok(tray) = FindWindowW(w!("TaskbeatTray"), PCWSTR::null()) {
            let _ = PostMessageW(Some(tray), WM_COMMAND, WPARAM(ID_EXIT), LPARAM(0));
        }
    }
    for _ in 0..30 {
        if other_taskbeat_pids().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for pid in other_taskbeat_pids() {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) {
                let _ = TerminateProcess(h, 0);
                let _ = WaitForSingleObject(h, 2000);
                let _ = CloseHandle(h);
            }
        }
    }
}

// ---------------------------------------------------------------- trabalho

fn create_shortcut(link_path: &Path, exe: &Path, dir: &Path) -> windows::core::Result<()> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        link.SetDescription(&HSTRING::from(tr("Widget de música para a barra de tarefas")))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        if let Some(parent) = link_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(link_path.as_os_str()), true)
    }
}

fn write_retry(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match std::fs::write(path, data) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(last.unwrap())
}

fn install(dir: &Path, autostart: bool, desktop: bool) -> Result<(), String> {
    let no_access = || tr("Não foi possível gravar nessa pasta. Escolha outra, por exemplo dentro da sua pasta de usuário.").to_string();
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&me).map_err(|e| e.to_string())?;
    stop_running_app();
    std::fs::create_dir_all(dir).map_err(|_| no_access())?;
    let exe = dir.join("Taskbeat.exe");
    if me.canonicalize().ok() != exe.canonicalize().ok() {
        // O navegador pode abrir o Taskbeat.exe de novo (host da extensão)
        // logo depois de ele ser fechado, travando o arquivo: o antigo sai do
        // caminho com outro nome (renomear um .exe em uso é permitido).
        let old = dir.join("Taskbeat.old");
        let _ = std::fs::remove_file(&old);
        if exe.exists() {
            let _ = std::fs::rename(&exe, &old);
        }
        write_retry(&exe, &bytes).map_err(|_| no_access())?;
        let _ = std::fs::remove_file(&old);
    }
    // A licença MIT pede que os avisos acompanhem as cópias do programa.
    crate::licenses::write_to(dir).map_err(|_| no_access())?;
    // Instalação anterior em outra pasta: some o executável velho.
    if let Some(old) = reg_get_sz(UNINSTALL_KEY, "InstallLocation").map(PathBuf::from) {
        if old.canonicalize().ok() != dir.canonicalize().ok() {
            let _ = std::fs::remove_file(old.join("Taskbeat.exe"));
            let _ = std::fs::remove_dir(&old);
        }
    }
    if let Some(l) = start_menu_link() {
        create_shortcut(&l, &exe, dir).map_err(|e| e.to_string())?;
    }
    if let Some(l) = desktop_link() {
        if desktop {
            let _ = create_shortcut(&l, &exe, dir);
        } else {
            let _ = std::fs::remove_file(l);
        }
    }
    let exe_s = exe.display().to_string();
    let r = (|| -> windows::core::Result<()> {
        reg_set_sz(UNINSTALL_KEY, "DisplayName", "Taskbeat")?;
        reg_set_sz(UNINSTALL_KEY, "DisplayVersion", VERSION)?;
        reg_set_sz(UNINSTALL_KEY, "Publisher", "Taskbeat")?;
        reg_set_sz(UNINSTALL_KEY, "DisplayIcon", &format!("\"{exe_s}\",0"))?;
        reg_set_sz(UNINSTALL_KEY, "InstallLocation", &dir.display().to_string())?;
        reg_set_sz(UNINSTALL_KEY, "UninstallString", &format!("\"{exe_s}\" --uninstall"))?;
        reg_set_dword(UNINSTALL_KEY, "NoModify", 1)?;
        reg_set_dword(UNINSTALL_KEY, "NoRepair", 1)?;
        reg_set_dword(UNINSTALL_KEY, "EstimatedSize", (bytes.len() / 1024) as u32)?;
        // Mesmo formato que a opção "Iniciar com o Windows" do app usa.
        if autostart {
            reg_set_sz(RUN_KEY, "Taskbeat", &format!("\"{exe_s}\""))?;
        } else {
            reg_delete_value(RUN_KEY, "Taskbeat");
        }
        Ok(())
    })();
    r.map_err(|e| e.to_string())?;
    // A extensão do navegador é opcional: sem isto o resto funciona. A pasta
    // dela vai junto (para carregar sem compactação até ela estar na loja).
    if let Err(e) = register_browser_host(dir, &exe) {
        crate::audio::log::warn(&format!("extensão do navegador: registro falhou ({e})"));
    }
    if let Err(e) = crate::webext::write_extension(&dir.join("extension")) {
        crate::audio::log::warn(&format!("extensão do navegador: pasta não gravada ({e})"));
    }
    Ok(())
}

/// Arquivo com o manifesto do host da extensão (na pasta do programa).
const HOST_MANIFEST: &str = "browser-host.json";

/// Registra o Taskbeat como host de Native Messaging (a extensão do
/// navegador abre o `Taskbeat.exe` para falar com o app): o manifesto na
/// pasta e a chave de cada navegador apontando para ele.
fn register_browser_host(dir: &Path, exe: &Path) -> Result<(), String> {
    let manifest = dir.join(HOST_MANIFEST);
    write_retry(&manifest, crate::webext::host_manifest(exe).as_bytes()).map_err(|e| e.to_string())?;
    let m = manifest.display().to_string();
    for key in crate::webext::registry_keys() {
        reg_set_sz(&key, "", &m).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn unregister_browser_host() {
    for key in crate::webext::registry_keys() {
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key));
        }
    }
}

fn uninstall(dir: &Path, wipe: bool) -> Result<(), String> {
    // Antes de fechar o app: o navegador não abre mais o host da extensão.
    unregister_browser_host();
    stop_running_app();
    reg_delete_value(RUN_KEY, "Taskbeat");
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY));
    }
    for l in [start_menu_link(), desktop_link()].into_iter().flatten() {
        let _ = std::fs::remove_file(l);
    }
    if wipe {
        if let Some(c) = config_dir() {
            let _ = std::fs::remove_dir_all(c);
        }
    }
    let _ = dir;
    Ok(())
}

/// Um programa não apaga o próprio executável enquanto roda: o cmd apaga a
/// pasta logo depois que este processo fechar.
fn schedule_self_delete(dir: &Path) {
    let dir = dir.display().to_string();
    // Tenta por até ~15 s (espera o processo fechar e soltar o .exe).
    let _ = Command::new("cmd.exe")
        .raw_arg(format!("/C for /L %i in (1,1,15) do @(ping 127.0.0.1 -n 2 >nul & rmdir /s /q \"{dir}\" 2>nul & if not exist \"{dir}\" exit)"))
        .creation_flags(CREATE_NO_WINDOW)
        .current_dir(std::env::temp_dir())
        .spawn();
}

fn pick_folder(hwnd: HWND, current: &Path) -> Option<PathBuf> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().unwrap_or_default();
        let _ = dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
        let _ = dlg.SetTitle(&HSTRING::from(tr("Escolha onde instalar o Taskbeat")));
        let start = current.parent().filter(|p| p.exists()).unwrap_or(current);
        if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None) {
            let _ = dlg.SetFolder(&item);
        }
        dlg.Show(Some(hwnd)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        let picked = PathBuf::from(s?);
        // Escolheu a pasta "mãe": instala numa subpasta Taskbeat.
        let named = picked.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("taskbeat"));
        Some(if named { picked } else { picked.join("Taskbeat") })
    }
}

// ---------------------------------------------------------------- janela

pub fn run(mode: Mode) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // Se já houver configurações (atualização), respeita o idioma escolhido.
        if let Some(p) = Config::path() {
            if let Some(cfg) = std::fs::read_to_string(p).ok().and_then(|s| toml::from_str::<Config>(&s).ok()) {
                crate::i18n::apply(cfg.language);
            }
        }
        let Ok(module) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = module.into();
        let dpi = GetDpiForSystem().max(96);
        let icon = LoadImageW(Some(hinst), PCWSTR(1 as _), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE).map(|h| HICON(h.0)).unwrap_or_default();
        let class = w!("TaskbeatSetup");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hIcon: icon,
            hIconSm: icon,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let s = dpi as f32 / 96.0;
        let mut rc = RECT { left: 0, top: 0, right: (W * s) as i32, bottom: (H * s) as i32 };
        let _ = AdjustWindowRectExForDpi(&mut rc, STYLE, false, WINDOW_EX_STYLE(0), dpi);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let x = (GetSystemMetrics(SM_CXSCREEN) - ww) / 2;
        let y = ((GetSystemMetrics(SM_CYSCREEN) - wh) / 2).max(0);
        let title = tr(match mode {
            Mode::Install if installed_version().is_some() => "Atualizar o Taskbeat",
            Mode::Install => "Instalar o Taskbeat",
            Mode::Uninstall => "Remover o Taskbeat",
        });
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), class, &HSTRING::from(title), STYLE, x, y, ww, wh, None, None, Some(hinst), None) else {
            return;
        };
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let dark = system::apps_use_dark();
        let mica = crate::settings::apply_theme(hwnd, dark);
        let mut text = TextRenderer::new();
        text.set_capacity(200);
        text.set_scale(scale);
        let found = classify(installed_version().as_deref(), VERSION);
        let update = found != Found::Fresh;
        let dir = if mode == Mode::Uninstall {
            std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(default_dir)
        } else {
            default_dir()
        };
        let autostart_now = reg_get_sz(RUN_KEY, "Taskbeat").is_some() || !update;
        let desktop_now = desktop_link().is_some_and(|l| l.exists());
        let opts = match mode {
            Mode::Install => vec![autostart_now, desktop_now, true],
            Mode::Uninstall => vec![false],
        };
        ST.with(|st| {
            *st.borrow_mut() = Some(St {
                hwnd,
                mode,
                update,
                found,
                scale,
                dark,
                mica,
                accent: system::app_accent(dark),
                dir,
                opts,
                phase: Phase::Ready,
                result: None,
                hover: None,
                pressed: None,
                tracking: false,
                surface: None,
                text,
                t0: Instant::now(),
            })
        });
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
        // Animação leve das barras da faixa.
        SetTimer(Some(hwnd), TIMER, 50, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyIcon(icon);
    }
}

// ---------------------------------------------------------------- layout

struct Lay {
    banner: RectF,
    path: RectF,
    change: RectF,
    /// (índice da opção, retângulo) das opções visíveis.
    opts: Vec<(usize, RectF)>,
    cancel: Option<RectF>,
    primary: RectF,
    /// Link "Licença MIT e avisos de terceiros" no rodapé.
    licenses: RectF,
}

impl St {
    fn layout(&mut self) -> Lay {
        let s = self.scale;
        let banner = RectF::new(16.0 * s, 16.0 * s, (W - 32.0) * s, 150.0 * s);
        let path = RectF::new(24.0 * s, 214.0 * s, (W - 48.0 - 128.0) * s, 38.0 * s);
        let change = RectF::new(path.right() + 8.0 * s, path.y, 120.0 * s, 38.0 * s);
        // Atualização: pasta, inicialização e atalhos continuam como estão; só
        // sobra "abrir ao terminar".
        let (first, visible): (f32, Vec<usize>) = match (self.mode, self.update) {
            (Mode::Install, false) => (272.0, (0..self.opts.len()).collect()),
            (Mode::Install, true) => (326.0, vec![2]),
            (Mode::Uninstall, _) => (268.0, (0..self.opts.len()).collect()),
        };
        let opts = visible.into_iter().enumerate().map(|(row, i)| (i, RectF::new(16.0 * s, (first + row as f32 * 40.0) * s, (W - 32.0) * s, 36.0 * s))).collect();
        let by = (H - 58.0) * s;
        let primary_label = self.primary_label();
        let pw = (self.text.mask(tr(primary_label), Font::BodyStrong, (300.0 * s) as i32, (20.0 * s) as i32, Align::Left).used_width() + 48.0 * s).max(120.0 * s);
        let primary = RectF::new((W - 24.0) * s - pw, by, pw, 38.0 * s);
        let cancel = matches!(self.phase, Phase::Ready).then(|| RectF::new(primary.x - 118.0 * s, by, 110.0 * s, 38.0 * s));
        let lw = self.text.mask(tr("Licença MIT e avisos de terceiros"), Font::Caption, (400.0 * s) as i32, (18.0 * s) as i32, Align::Left).used_width();
        let licenses = RectF::new(24.0 * s, by + 20.0 * s, lw + 2.0 * s, 18.0 * s);
        Lay { banner, path, change, opts, cancel, primary, licenses }
    }

    fn primary_label(&self) -> &'static str {
        match (&self.phase, self.mode) {
            (Phase::Ready, Mode::Install) => match self.found {
                Found::Fresh => "Instalar",
                Found::Older(_) => "Atualizar",
                Found::Same => "Reinstalar",
                Found::Newer(_) => "Instalar mesmo assim",
            },
            (Phase::Ready, Mode::Uninstall) => "Remover",
            (Phase::Working(_), Mode::Install) if self.update => "Atualizando…",
            (Phase::Working(_), Mode::Install) => "Instalando…",
            (Phase::Working(_), Mode::Uninstall) => "Removendo…",
            (Phase::Done, Mode::Install) => "Concluir",
            (Phase::Done, Mode::Uninstall) | (Phase::Failed(_), _) => "Fechar",
        }
    }

    fn hit(&mut self, x: f32, y: f32) -> Option<Hit> {
        let l = self.layout();
        if l.primary.contains(x, y) && !matches!(self.phase, Phase::Working(_)) {
            return Some(Hit::Primary);
        }
        if !matches!(self.phase, Phase::Ready) {
            return None;
        }
        if l.cancel.is_some_and(|c| c.contains(x, y)) {
            return Some(Hit::Cancel);
        }
        if self.mode == Mode::Install && !self.update && l.change.contains(x, y) {
            return Some(Hit::Change);
        }
        if self.mode == Mode::Install && l.licenses.contains(x, y) {
            return Some(Hit::Licenses);
        }
        l.opts.iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| Hit::Opt(*i))
    }

    fn fg(&self) -> Rgb {
        if self.dark { [1.0; 3] } else { [0.0; 3] }
    }

    /// Texto quebrado em linhas que cabem em `w` (medindo com a própria fonte).
    fn wrap(&mut self, text: &str, font: Font, w: f32) -> Vec<String> {
        let s = self.scale;
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                let wd = self.text.mask(&cand, font, (2000.0 * s) as i32, (24.0 * s) as i32, Align::Left).used_width();
                if wd > w && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    line = word.to_string();
                } else {
                    line = cand;
                }
            }
            lines.push(line);
        }
        lines
    }

    fn paragraph(&mut self, cv: &mut Canvas, text: &str, font: Font, x: f32, y: f32, w: f32, line_h: f32, c: Rgb, a: f32) -> f32 {
        let mut yy = y;
        for line in self.wrap(text, font, w) {
            ui::text(cv, &mut self.text, &line, font, x, yy, w + 40.0 * self.scale, line_h, c, a);
            yy += line_h;
        }
        yy
    }

    fn paint(&mut self, cv: &mut Canvas, w: f32, h: f32) {
        let s = self.scale;
        let fg = self.fg();
        if self.mica {
            cv.clear();
        } else {
            cv.fill_raw(if self.dark { 0xFF20_2020 } else { 0xFFF3_F3F3 });
        }
        let _ = (w, h);
        let l = self.layout();
        let t = self.t0.elapsed().as_secs_f32();

        // ---- faixa com gradiente da marca
        let b = l.banner;
        let (bx, by) = (b.x, b.y);
        cv.drop_shadow(b, 12.0 * s, 16.0 * s, 4.0 * s, if self.dark { 0.45 } else { 0.18 });
        cv.fill_rrect_ca(b.x, b.y, b.right(), b.bottom(), 12.0 * s, |x, y| {
            let u = ((x - bx) / b.w).clamp(0.0, 1.0);
            let v = ((y - by) / b.h).clamp(0.0, 1.0);
            let c = if u < 0.55 { lerp3([0.26, 0.14, 0.62], [0.62, 0.17, 0.48], u / 0.55) } else { lerp3([0.62, 0.17, 0.48], [0.86, 0.42, 0.24], (u - 0.55) / 0.45) };
            let glow = (-(((u - 0.18) * 2.4).powi(2) + ((v - 0.2) * 2.2).powi(2))).exp();
            (lerp3(lerp3(c, [0.05, 0.03, 0.10], 0.25 * v), [1.0; 3], glow * 0.12), 1.0)
        });
        // Barras de equalizador decorativas (animadas de leve).
        let bars = 9;
        for i in 0..bars {
            let x = b.right() - 36.0 * s - (bars - i) as f32 * 14.0 * s;
            let ph = t * (1.3 + i as f32 * 0.17) + i as f32 * 1.7;
            let hgt = (0.35 + 0.35 * (0.5 + 0.5 * ph.sin())) * 70.0 * s;
            cv.fill_rrect(x, b.bottom() - 28.0 * s - hgt, x + 7.0 * s, b.bottom() - 28.0 * s, 3.5 * s, 0.16, [1.0; 3]);
        }
        crate::settings::draw_logo(cv, RectF::new(b.x + 28.0 * s, b.cy() - 34.0 * s, 68.0 * s, 68.0 * s));
        ui::text(cv, &mut self.text, "Taskbeat", Font::Heading, b.x + 114.0 * s, b.cy() - 34.0 * s, 300.0 * s, 40.0 * s, [1.0; 3], 1.0);
        ui::text(cv, &mut self.text, tr("Widget de música para a barra de tarefas"), Font::Body, b.x + 115.0 * s, b.cy() + 6.0 * s, 330.0 * s, 22.0 * s, [1.0; 3], 0.85);
        let chip = RectF::new(b.x + 115.0 * s, b.cy() + 34.0 * s, 58.0 * s, 20.0 * s);
        cv.fill_round(chip, 10.0 * s, [1.0; 3], 0.18);
        ui::text_center(cv, &mut self.text, &format!("v{VERSION}"), Font::Caption, chip, [1.0; 3], 0.95);

        let x0 = 24.0 * s;
        let body_w = (W - 48.0) * s;
        match (&self.phase, self.mode) {
            (Phase::Ready, Mode::Install) if self.update => {
                self.paint_update(cv, &l);
            }
            (Phase::Ready, Mode::Install) => {
                ui::text(cv, &mut self.text, tr("Pasta de instalação"), Font::BodyStrong, x0, 184.0 * s, body_w, 24.0 * s, fg, 1.0);
                self.draw_box(cv, l.path, false);
                let path = self.dir.display().to_string();
                ui::glyph(cv, &mut self.text, RectF::new(l.path.x + 6.0 * s, l.path.y, 28.0 * s, l.path.h), "\u{E8B7}", Font::IconMd, fg, 0.8);
                ui::text(cv, &mut self.text, &path, Font::Body, l.path.x + 38.0 * s, l.path.y, l.path.w - 48.0 * s, l.path.h, fg, 0.95);
                let hot = self.hover == Some(Hit::Change);
                self.draw_box(cv, l.change, hot);
                ui::text_center(cv, &mut self.text, tr("Alterar…"), Font::Body, l.change, fg, 1.0);
                self.draw_options(cv, &l);
                let note_w = (l.cancel.map(|c| c.x).unwrap_or(l.primary.x) - x0 - 12.0 * s).max(10.0);
                ui::text(cv, &mut self.text, tr("Instala só para você; não precisa de administrador."), Font::Caption, x0, l.primary.y + 1.0 * s, note_w, 18.0 * s, fg, 0.55);
                // Link para a licença e os avisos de terceiros.
                let hot = self.hover == Some(Hit::Licenses);
                let lr = l.licenses;
                ui::text(cv, &mut self.text, tr("Licença MIT e avisos de terceiros"), Font::Caption, lr.x, lr.y, note_w, lr.h, self.accent, if hot { 1.0 } else { 0.9 });
                if hot {
                    cv.fill_round(RectF::new(lr.x, lr.bottom() - 2.0 * s, lr.w - 2.0 * s, 1.0 * s), 0.0, self.accent, 0.9);
                }
            }
            (Phase::Ready, Mode::Uninstall) => {
                ui::text(cv, &mut self.text, tr("Remover o Taskbeat?"), Font::Display, x0, 184.0 * s, body_w, 28.0 * s, fg, 1.0);
                self.paragraph(cv, tr("O app, os atalhos e a inicialização com o Windows serão removidos."), Font::Body, x0, 216.0 * s, body_w, 22.0 * s, fg, 0.75);
                self.draw_options(cv, &l);
            }
            (Phase::Working(start), _) => {
                let label = tr(self.primary_label());
                ui::text(cv, &mut self.text, label, Font::Display, x0, 196.0 * s, body_w, 28.0 * s, fg, 1.0);
                let p = (start.elapsed().as_secs_f32() / 0.9).min(if self.result.is_some() { 1.0 } else { 0.85 });
                let p = ui::ease_out_cubic(p);
                let (tx0, tx1, ty) = (x0, x0 + body_w, 244.0 * s);
                cv.fill_rrect(tx0, ty, tx1, ty + 6.0 * s, 3.0 * s, if self.dark { 0.18 } else { 0.12 }, fg);
                cv.fill_rrect(tx0, ty, tx0 + (tx1 - tx0) * p.max(0.03), ty + 6.0 * s, 3.0 * s, 1.0, self.accent);
            }
            (Phase::Done, mode) => {
                let c = RectF::new(x0, 190.0 * s, 44.0 * s, 44.0 * s);
                cv.fill_circle(c.cx(), c.cy(), 22.0 * s, self.accent, 1.0);
                ui::glyph(cv, &mut self.text, c, "\u{E73E}", Font::IconLg, crate::render::on_color(self.accent), 1.0);
                let tx = x0 + 60.0 * s;
                let tw = body_w - 60.0 * s;
                if mode == Mode::Install && self.update {
                    ui::text(cv, &mut self.text, tr("Pronto! O Taskbeat foi atualizado."), Font::Display, tx, 192.0 * s, tw, 28.0 * s, fg, 1.0);
                    let line = format!("{} v{VERSION}. {}", tr("Agora na versão"), tr("Suas configurações foram mantidas."));
                    self.paragraph(cv, &line, Font::Body, tx, 226.0 * s, tw, 22.0 * s, fg, 0.78);
                } else if mode == Mode::Install {
                    ui::text(cv, &mut self.text, tr("Pronto! O Taskbeat foi instalado."), Font::Display, tx, 192.0 * s, tw, 28.0 * s, fg, 1.0);
                    let y = self.paragraph(cv, tr("A música aparece na barra de tarefas, ao lado do relógio, sempre que algo estiver tocando."), Font::Body, tx, 226.0 * s, tw, 22.0 * s, fg, 0.78);
                    self.paragraph(cv, tr("As configurações ficam no ícone do Taskbeat perto do relógio e no Menu Iniciar."), Font::Body, tx, y + 8.0 * s, tw, 22.0 * s, fg, 0.78);
                } else {
                    ui::text(cv, &mut self.text, tr("O Taskbeat foi removido."), Font::Display, tx, 192.0 * s, tw, 28.0 * s, fg, 1.0);
                    ui::text(cv, &mut self.text, tr("Obrigado por usar!"), Font::Body, tx, 226.0 * s, tw, 22.0 * s, fg, 0.78);
                }
            }
            (Phase::Failed(msg), _) => {
                let msg = msg.clone();
                let c = RectF::new(x0, 190.0 * s, 44.0 * s, 44.0 * s);
                cv.fill_circle(c.cx(), c.cy(), 22.0 * s, [0.86, 0.22, 0.27], 1.0);
                ui::glyph(cv, &mut self.text, c, "\u{E711}", Font::IconLg, [1.0; 3], 1.0);
                let tx = x0 + 60.0 * s;
                let tw = body_w - 60.0 * s;
                ui::text(cv, &mut self.text, tr("Não foi possível instalar"), Font::Display, tx, 192.0 * s, tw, 28.0 * s, fg, 1.0);
                self.paragraph(cv, &msg, Font::Body, tx, 226.0 * s, tw, 22.0 * s, fg, 0.78);
            }
        }

        // ---- botões
        if let Some(c) = l.cancel {
            let hot = self.hover == Some(Hit::Cancel);
            self.draw_box(cv, c, hot);
            ui::text_center(cv, &mut self.text, tr("Cancelar"), Font::Body, c, fg, 1.0);
        }
        let busy = matches!(self.phase, Phase::Working(_));
        let hot = self.hover == Some(Hit::Primary) && !busy;
        let pressed = self.pressed == Some(Hit::Primary) && hot;
        let danger = self.mode == Mode::Uninstall && matches!(self.phase, Phase::Ready);
        let base = if danger { [0.80, 0.20, 0.25] } else { self.accent };
        let col = if pressed { lerp3(base, [0.0; 3], 0.12) } else if hot { lerp3(base, [1.0; 3], 0.10) } else { base };
        if !busy {
            cv.drop_shadow(l.primary, 6.0 * s, 8.0 * s, 2.0 * s, if self.dark { 0.35 } else { 0.15 });
        }
        cv.fill_round(l.primary, 6.0 * s, col, if busy { 0.55 } else { 1.0 });
        let on = crate::render::on_color(base);
        let label = tr(self.primary_label());
        ui::text_center(cv, &mut self.text, label, Font::BodyStrong, l.primary.offset(0.0, -1.0 * s), on, 1.0);
    }

    /// Já instalado: compara as versões e atualiza no lugar (sem desinstalar).
    fn paint_update(&mut self, cv: &mut Canvas, l: &Lay) {
        let s = self.scale;
        let fg = self.fg();
        let x0 = 24.0 * s;
        let body_w = (W - 48.0) * s;
        let (title, from, desc) = match self.found.clone() {
            Found::Older(v) => (
                "Atualização disponível",
                Some(v),
                "O Taskbeat instalado será atualizado no mesmo lugar. Configurações, atalhos e inicialização com o Windows continuam como estão; não precisa desinstalar.",
            ),
            Found::Same => ("Esta versão já está instalada", None, "Dá para reinstalar por cima, se algo não estiver funcionando. Suas configurações são mantidas."),
            Found::Newer(v) => ("Uma versão mais nova já está instalada", Some(v), "Este instalador é mais antigo que o Taskbeat instalado. Instalar volta para a versão dele."),
            Found::Fresh => ("", None, ""),
        };
        ui::text(cv, &mut self.text, tr(title), Font::Display, x0, 182.0 * s, body_w, 28.0 * s, fg, 1.0);

        // Versões: instalada → deste instalador.
        let y = 220.0 * s;
        let h = 30.0 * s;
        let pill = |st: &mut St, cv: &mut Canvas, x: f32, label: &str, caption: &str, strong: bool| -> f32 {
            let tw = st.text.mask(label, Font::BodyStrong, (200.0 * s) as i32, h as i32, Align::Left).used_width();
            let r = RectF::new(x, y, tw + 28.0 * s, h);
            if strong {
                cv.fill_round(r, h / 2.0, st.accent, 1.0);
                ui::text_center(cv, &mut st.text, label, Font::BodyStrong, r, crate::render::on_color(st.accent), 1.0);
            } else {
                st.draw_box(cv, r, false);
                ui::text_center(cv, &mut st.text, label, Font::BodyStrong, r, fg, 0.9);
            }
            ui::text(cv, &mut st.text, caption, Font::Small, r.x + 2.0 * s, r.bottom() + 2.0 * s, 160.0 * s, 16.0 * s, fg, 0.55);
            r.right()
        };
        let mine = format!("v{VERSION}");
        match from {
            Some(old) => {
                let old = format!("v{}", old.trim_start_matches(['v', 'V']));
                let newer = matches!(self.found, Found::Older(_));
                let x = pill(self, cv, x0, &old, tr("instalada"), !newer);
                let arrow = RectF::new(x + 6.0 * s, y, 28.0 * s, h);
                ui::glyph(cv, &mut self.text, arrow, "\u{E72A}", Font::IconSmall, fg, 0.7);
                pill(self, cv, arrow.right() + 6.0 * s, &mine, tr("este instalador"), newer);
            }
            None => {
                pill(self, cv, x0, &mine, tr("instalada"), true);
            }
        }
        self.paragraph(cv, tr(desc), Font::Body, x0, 274.0 * s, body_w, 21.0 * s, fg, 0.72);
        self.draw_options(cv, l);
        let note_w = (l.cancel.map(|c| c.x).unwrap_or(l.primary.x) - x0 - 12.0 * s).max(10.0);
        let place = format!("{} {}", tr("Pasta:"), self.dir.display());
        ui::text(cv, &mut self.text, &place, Font::Caption, x0, l.primary.y + 1.0 * s, note_w, 18.0 * s, fg, 0.55);
        let hot = self.hover == Some(Hit::Licenses);
        let lr = l.licenses;
        ui::text(cv, &mut self.text, tr("Licença MIT e avisos de terceiros"), Font::Caption, lr.x, lr.y, note_w, lr.h, self.accent, if hot { 1.0 } else { 0.9 });
    }

    fn draw_box(&self, cv: &mut Canvas, r: RectF, hot: bool) {
        let s = self.scale;
        let a = if self.dark { if hot { 0.09 } else { 0.06 } } else if hot { 0.5 } else { 0.75 };
        cv.fill_round(r, 6.0 * s, [1.0; 3], a);
        cv.stroke_round(r, 6.0 * s, s, if self.dark { [1.0; 3] } else { [0.0; 3] }, if self.dark { 0.08 } else { 0.10 });
    }

    /// Opções com o mesmo toggle da tela de configurações.
    fn draw_options(&mut self, cv: &mut Canvas, l: &Lay) {
        let s = self.scale;
        let fg = self.fg();
        let labels = opt_labels(self.mode);
        for &(i, r) in &l.opts {
            let r = &r;
            let on = self.opts[i];
            let hot = self.hover == Some(Hit::Opt(i));
            if hot {
                cv.fill_round(*r, 6.0 * s, fg, 0.04);
            }
            let sw = RectF::new(r.x + 8.0 * s, r.cy() - 10.0 * s, 40.0 * s, 20.0 * s);
            let knob = if hot { 7.0 * s } else { 6.0 * s };
            if on {
                cv.fill_round(sw, 10.0 * s, self.accent, 1.0);
                cv.fill_circle(sw.right() - 10.0 * s, sw.cy(), knob, crate::render::on_color(self.accent), 1.0);
            } else {
                cv.stroke_round(sw, 10.0 * s, s, fg, 0.6);
                cv.fill_circle(sw.x + 10.0 * s, sw.cy(), knob, fg, 0.8);
            }
            ui::text(cv, &mut self.text, tr(labels[i]), Font::Body, sw.right() + 14.0 * s, r.y, r.w - 70.0 * s, r.h, fg, 1.0);
        }
    }
}

// ---------------------------------------------------------------- eventos

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn start_work(st: &mut St) {
    st.phase = Phase::Working(Instant::now());
    st.result = None;
    unsafe {
        SetTimer(Some(st.hwnd), TIMER, 16, None);
    }
    let hwnd = st.hwnd.0 as isize;
    let (mode, dir, opts) = (st.mode, st.dir.clone(), st.opts.clone());
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let r = match mode {
            Mode::Install => install(&dir, opts[0], opts[1]),
            Mode::Uninstall => uninstall(&dir, opts[0]),
        };
        *PENDING.lock().unwrap() = Some(r);
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as _)), WM_APP_DONE, WPARAM(0), LPARAM(0));
        }
    });
}

static PENDING: std::sync::Mutex<Option<Result<(), String>>> = std::sync::Mutex::new(None);

/// Botão principal.
fn primary(st: &mut St) -> bool {
    match st.phase {
        Phase::Ready => {
            start_work(st);
            false
        }
        Phase::Working(_) => false,
        Phase::Done => {
            if st.mode == Mode::Install && st.opts[2] {
                // Atualização: só volta a abrir o app (ele já estava configurado);
                // instalação nova: abre as configurações para mostrar que está rodando.
                let exe = st.dir.join("Taskbeat.exe");
                let mut cmd = Command::new(&exe);
                if !st.update {
                    cmd.arg("--settings");
                }
                let _ = cmd.current_dir(&st.dir).spawn();
            }
            if st.mode == Mode::Uninstall {
                schedule_self_delete(&st.dir);
            }
            true
        }
        Phase::Failed(_) => true,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                with(|st| {
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let (w, h) = (rc.right.max(1), rc.bottom.max(1));
                    Surface::ensure(&mut st.surface, w, h);
                    if let Some(mut surf) = st.surface.take() {
                        {
                            let mut cv = surf.canvas();
                            st.paint(&mut cv, w as f32, h as f32);
                        }
                        let _ = BitBlt(hdc, 0, 0, w, h, Some(surf.dc), 0, 0, SRCCOPY);
                        st.surface = Some(surf);
                    }
                });
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_TIMER => {
                // Barra de progresso; termina e mostra o resultado.
                let stop = with(|st| {
                    if let (Phase::Working(start), Some(_)) = (&st.phase, &st.result) {
                        if start.elapsed() > Duration::from_millis(950) {
                            st.phase = match st.result.take() {
                                Some(Ok(())) => Phase::Done,
                                Some(Err(e)) => Phase::Failed(e),
                                None => Phase::Done,
                            };
                            return true;
                        }
                    }
                    false
                })
                .unwrap_or(false);
                if stop {
                    // Animação da faixa continua devagar.
                    let _ = SetTimer(Some(hwnd), TIMER, 50, None);
                }
                invalidate(hwnd);
                LRESULT(0)
            }
            m if m == WM_APP_DONE => {
                let r = PENDING.lock().unwrap().take();
                with(|st| st.result = r);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as f32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
                let changed = with(|st| {
                    if !st.tracking {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        let _ = TrackMouseEvent(&mut tme);
                        st.tracking = true;
                    }
                    let h = st.hit(x, y);
                    let c = h != st.hover;
                    st.hover = h;
                    c
                })
                .unwrap_or(false);
                if changed {
                    let cursor = with(|st| st.hover.is_some()).unwrap_or(false);
                    let _ = SetCursor(LoadCursorW(None, if cursor { IDC_HAND } else { IDC_ARROW }).ok());
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let hand = with(|st| st.hover.is_some()).unwrap_or(false);
                let _ = SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                LRESULT(1)
            }
            WM_MOUSELEAVE => {
                with(|st| {
                    st.tracking = false;
                    st.hover = None;
                });
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let x = (lp.0 & 0xffff) as i16 as f32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
                SetCapture(hwnd);
                with(|st| st.pressed = st.hit(x, y));
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let x = (lp.0 & 0xffff) as i16 as f32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
                let action = with(|st| {
                    let h = st.hit(x, y);
                    let p = st.pressed.take();
                    if h.is_none() || h != p {
                        return None;
                    }
                    h
                })
                .flatten();
                let mut close = false;
                match action {
                    Some(Hit::Opt(i)) => {
                        with(|st| st.opts[i] = !st.opts[i]);
                    }
                    Some(Hit::Cancel) => close = true,
                    Some(Hit::Licenses) => crate::licenses::open(),
                    Some(Hit::Primary) => close = with(primary).unwrap_or(false),
                    Some(Hit::Change) => {
                        // O seletor é modal: nada de `borrow` aberto aqui.
                        let cur = with(|st| st.dir.clone()).unwrap_or_default();
                        if let Some(d) = pick_folder(hwnd, &cur) {
                            with(|st| st.dir = d);
                        }
                    }
                    None => {}
                }
                if close {
                    let _ = DestroyWindow(hwnd);
                } else {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 == 0x0D => {
                // Enter = botão principal.
                if with(primary).unwrap_or(false) {
                    let _ = DestroyWindow(hwnd);
                } else {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wp.0 == 0x1B => {
                let busy = with(|st| matches!(st.phase, Phase::Working(_))).unwrap_or(false);
                if !busy {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                // Não fecha no meio da instalação.
                let busy = with(|st| matches!(st.phase, Phase::Working(_))).unwrap_or(false);
                if !busy {
                    // Fechou pelo X depois de remover: ainda apaga a pasta.
                    with(|st| {
                        if st.mode == Mode::Uninstall && matches!(st.phase, Phase::Done) {
                            schedule_self_delete(&st.dir);
                        }
                    });
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED => {
                with(|st| {
                    st.dark = system::apps_use_dark();
                    st.accent = system::app_accent(st.dark);
                    st.mica = crate::settings::apply_theme(hwnd, st.dark);
                });
                invalidate(hwnd);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER);
                // Libera texto (DirectWrite/Direct2D) e a superfície aqui, com a
                // janela e o COM ainda vivos: soltá-los na saída do processo
                // derrubava o programa (e segurava a pasta durante a remoção).
                ST.with(|s| {
                    if let Ok(mut g) = s.try_borrow_mut() {
                        g.take();
                    }
                });
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compara_versoes() {
        assert_eq!(classify(None, "1.1.0"), Found::Fresh);
        assert_eq!(classify(Some(""), "1.1.0"), Found::Fresh);
        assert_eq!(classify(Some("1.0.1"), "1.1.0"), Found::Older("1.0.1".into()));
        assert_eq!(classify(Some("1.1.0"), "1.1.0"), Found::Same);
        assert_eq!(classify(Some("1.10.0"), "1.9.3"), Found::Newer("1.10.0".into()));
        assert_eq!(classify(Some("v1.0"), "1.0.0"), Found::Same);
        assert_eq!(parse_version("2.3.4-beta"), (2, 3, 4));
    }
}
