//! Sobreposição de letra (Windows): janela fixa na tela, sempre por cima, no
//! estilo das sobreposições do Discord e da Steam. Mostra a letra
//! sincronizada, o player ou os dois (layouts em `lyrics_view.rs`), num dos
//! nove lugares prontos de um monitor ou onde o usuário soltar.
//!
//! - Destravada: arrasta pelo fundo; enquanto arrasta, os nove lugares
//!   aparecem tracejados no monitor e a janela gruda no mais perto (ímã).
//!   Travada: a posição fica presa e os botões continuam valendo. Opcional:
//!   travada, o clique atravessa a janela (WS_EX_TRANSPARENT), para jogos.
//! - Barrinha com o mouse em cima: travar, tela cheia e esconder.
//! - Ctrl+roda do mouse muda o tamanho; atalhos globais (padrão Win+Alt+L
//!   mostra/esconde; com Shift, trava/destrava).
//! - Thread própria, parada quando não há o que fazer: acorda na troca de
//!   linha (timer até a próxima), quando a música ou as opções mudam e com o
//!   mouse. Escondida, solta a superfície, o texto e a busca.
//!
//! Jogos em tela cheia exclusiva não deixam nada por cima (o Windows não
//! compõe outras janelas); em janela sem borda funciona.

use crate::audio::log;
use crate::config::Config;
use crate::lyrics::config::{Hotkey, Layout, LyricsConfig, Position, ZOOM_MAX, ZOOM_MIN};
use crate::lyrics::fetch::Fetcher;
use crate::lyrics::{self, Line, Lyrics, Track};
use crate::lyrics_view::{self as view, Geo, LHit, LyricFrame, View};
use crate::overlay::Surface;
use crate::render::{Colors, Image, RectF};
use crate::shared::{MediaCmd, MediaInfo, MixCmd, MixerState, Shared};
use crate::text::TextRenderer;
use crate::ui::{self, Hit, Interact, Playback};
use crate::volume::Volume;
use crate::vsync::{Pacer, Target};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const WM_APP_CONFIG: u32 = WM_APP + 40;
const WM_APP_MEDIA: u32 = WM_APP + 41;
const WM_APP_LYRICS: u32 = WM_APP + 42;
const WM_APP_FRAME: u32 = WM_APP + 43;
const WM_APP_FG: u32 = WM_APP + 44;
const WM_APP_REFETCH: u32 = WM_APP + 45;
const WM_APP_FULL: u32 = WM_APP + 46;
const WM_APP_HIRES: u32 = WM_APP + 47;
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Timers: a próxima linha, pedir a letra (a música parou de mudar), tentar
/// de novo depois de uma falha, o segundo do progresso, a dica do botão, o
/// fim do aviso e gravar o tamanho (Ctrl+roda).
const T_LINE: usize = 1;
const T_FETCH: usize = 2;
const T_RETRY: usize = 3;
const T_TICK: usize = 4;
const T_TIP: usize = 5;
const T_TOAST: usize = 6;
const T_SAVE: usize = 7;

const HK_SHOW: i32 = 0x4C01;
const HK_LOCK: i32 = 0x4C02;

/// Troca de linha (a letra sobe e a ênfase passa para a próxima).
const LINE_ANIM: f32 = 0.30;
/// Troca de música (texto desliza, capa cruza).
const TRACK_ANIM: f32 = 0.45;
/// A letra que chega aparece em…
const LYRIC_FADE: f32 = 0.30;
/// A barrinha aparece/some.
const TOOLS_ANIM: f32 = 0.14;
/// Abre/fecha o mixer.
const MIXER_ANIM: f32 = 0.22;
/// Quanto o mouse fica parado no botão até a dica aparecer, e quanto o aviso dura.
const TIP_DELAY: f32 = 0.45;
const TOAST_SECS: f32 = 3.2;
/// Fundo animado da tela cheia: um quadro a cada… (≈ 12 fps: são manchas
/// grandes e desfocadas andando devagar; cada quadro reescreve a tela toda).
const BG_FRAME: Duration = Duration::from_millis(83);
/// Espera a música parar de mudar antes de pedir a letra.
const FETCH_DELAY_MS: u32 = 450;
/// Depois de uma falha (sem internet), tenta de novo.
const RETRY_MS: u32 = 60_000;
/// Quanto o valor mudado no mixer vale por cima do que a thread publica.
const MIX_HOLD: Duration = Duration::from_millis(1200);

// ---------------------------------------------------------------- controle de fora

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
/// Janela da thread (0 = não está rodando).
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// A thread está viva (mexido só com o lock: ninguém sobe duas).
static RUNNING: Mutex<bool> = Mutex::new(false);
/// Atalho que o Windows recusou (já é de outro programa): 0 = nenhum, senão
/// `hotkey_code` dele. Guarda qual foi para não valer para o escolhido depois.
static HOTKEY_REFUSED: AtomicU8 = AtomicU8::new(0);
/// Letra que a busca entregou: (chave da música, letra).
static FETCHED: Mutex<Option<(String, Lyrics)>> = Mutex::new(None);
/// Capa em alta resolução que chegou: (título, imagem).
static HIRES: Mutex<Option<(String, Image)>> = Mutex::new(None);

/// Guarda o estado compartilhado e sobe a thread se a sobreposição ou o atalho estiverem ligados.
pub fn init(shared: Arc<Shared>) {
    let lc = shared.config().lyrics;
    let _ = SHARED.set(shared);
    config_changed(&lc);
}

fn wanted(lc: &LyricsConfig) -> bool {
    lc.show || lc.hotkey != Hotkey::Off
}

/// As opções mudaram (chamado por `Shared::update_config` e `preview_config`):
/// sobe a thread se agora precisa, ou avisa a que está rodando.
pub fn config_changed(lc: &LyricsConfig) {
    let Some(shared) = SHARED.get() else { return };
    let mut running = RUNNING.lock().unwrap();
    if *running {
        post(WM_APP_CONFIG);
    } else if wanted(lc) {
        let shared = shared.clone();
        log::info(&format!("letra: subindo a thread (na tela: {}, atalho: {:?})", lc.show, lc.hotkey));
        match std::thread::Builder::new().name("lyrics".into()).spawn(move || run(shared)) {
            Ok(_) => *running = true,
            Err(e) => log::warn(&format!("letra: não deu para subir a thread: {e}")),
        }
    }
}

/// A música (ou a posição) mudou.
pub fn media_changed() {
    post(WM_APP_MEDIA);
}

/// Procura a letra de novo (cache limpo, pasta mudou).
pub fn refetch() {
    post(WM_APP_REFETCH);
}

/// Pedido de tela cheia que ainda não foi atendido (a thread pode estar subindo).
static FULL_REQ: AtomicBool = AtomicBool::new(false);

/// Abre a tela cheia (botão das configurações); liga a sobreposição se precisar.
pub fn open_fullscreen() {
    FULL_REQ.store(true, Ordering::Relaxed);
    if let Some(shared) = SHARED.get()
        && !shared.config().lyrics.show
    {
        shared.update_config(|c| c.lyrics.show = true);
    }
    post(WM_APP_FULL);
}

fn post(msg: u32) {
    let h = WINDOW.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as _)), msg, WPARAM(0), LPARAM(0));
        }
    }
}

/// Nome do atalho escolhido (`lock`: o de travar/destravar).
pub fn hotkey_label(hk: Hotkey, lock: bool) -> Option<&'static str> {
    Some(match (hk, lock) {
        (Hotkey::WinAltL, false) => "Win+Alt+L",
        (Hotkey::WinAltL, true) => "Win+Alt+Shift+L",
        (Hotkey::CtrlAltL, false) => "Ctrl+Alt+L",
        (Hotkey::CtrlAltL, true) => "Ctrl+Alt+Shift+L",
        (Hotkey::CtrlShiftL, false) => "Ctrl+Shift+L",
        (Hotkey::CtrlShiftL, true) => "Ctrl+Shift+Alt+L",
        (Hotkey::Off, _) => return None,
    })
}

fn hotkey_code(hk: Hotkey) -> u8 {
    match hk {
        Hotkey::Off => 0,
        Hotkey::WinAltL => 1,
        Hotkey::CtrlAltL => 2,
        Hotkey::CtrlShiftL => 3,
    }
}

/// O Windows recusou o atalho `hk` (outro programa já usa a combinação)?
pub fn hotkey_refused(hk: Hotkey) -> bool {
    hk != Hotkey::Off && HOTKEY_REFUSED.load(Ordering::Relaxed) == hotkey_code(hk)
}

/// O atalho de mostrar/esconder que funciona agora (`None`: desligado ou recusado).
pub fn working_hotkey(hk: Hotkey) -> Option<&'static str> {
    hotkey_label(hk, false).filter(|_| !hotkey_refused(hk))
}

/// Nome de cada lugar pronto (guias do arrasto, configurações).
pub fn position_name(p: Position) -> &'static str {
    match p {
        Position::TopLeft => "Canto superior esquerdo",
        Position::Top => "Topo, no meio",
        Position::TopRight => "Canto superior direito",
        Position::Left => "Esquerda, no meio",
        Position::Center => "Centro",
        Position::Right => "Direita, no meio",
        Position::BottomLeft => "Canto inferior esquerdo",
        Position::Bottom => "Embaixo, no meio",
        Position::BottomRight => "Canto inferior direito",
        Position::Free => "Livre",
    }
}

// ---------------------------------------------------------------- janela

#[derive(Clone, Copy, Debug)]
enum Ev {
    App(u32),
    Hotkey(i32),
    Timer(usize),
    Move(f32, f32),
    Leave,
    Down(f32, f32),
    Up(f32, f32),
    Wheel(i16),
    Key(u32),
    CaptureLost,
    Display,
}

thread_local! {
    static EVENTS: RefCell<Vec<Ev>> = const { RefCell::new(Vec::new()) };
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    /// Na tela cheia a janela pode ser ativada (para o Esc chegar).
    static ACTIVATE: Cell<bool> = const { Cell::new(false) };
    /// Cursor sob o mouse (mover, mão nos botões).
    static CURSOR: Cell<u8> = const { Cell::new(0) };
}

fn push(ev: Ev) {
    EVENTS.with(|e| e.borrow_mut().push(ev));
}

fn xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT((if ACTIVATE.with(|a| a.get()) { MA_ACTIVATE } else { MA_NOACTIVATE }) as isize),
            WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let id = match CURSOR.with(|c| c.get()) {
                    1 => IDC_SIZEALL,
                    2 => IDC_HAND,
                    3 => IDC_SIZEWE,
                    4 => IDC_SIZENS,
                    5 => IDC_SIZENWSE,
                    6 => IDC_SIZENESW,
                    _ => IDC_ARROW,
                };
                if let Ok(c) = LoadCursorW(None, id) {
                    SetCursor(Some(c));
                }
                return LRESULT(1);
            }
            WM_MOUSEMOVE => {
                if !TRACKING.with(|t| t.get()) {
                    let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    TRACKING.with(|t| t.set(true));
                }
                let (x, y) = xy(lp);
                push(Ev::Move(x, y));
            }
            WM_MOUSELEAVE => {
                TRACKING.with(|t| t.set(false));
                push(Ev::Leave);
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                SetCapture(hwnd);
                let (x, y) = xy(lp);
                push(Ev::Down(x, y));
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                push(Ev::Up(x, y));
                let _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => push(Ev::CaptureLost),
            WM_MOUSEWHEEL => push(Ev::Wheel(((wp.0 >> 16) & 0xffff) as i16)),
            WM_KEYDOWN => push(Ev::Key(wp.0 as u32)),
            WM_HOTKEY => push(Ev::Hotkey(wp.0 as i32)),
            WM_TIMER => push(Ev::Timer(wp.0)),
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => {
                push(Ev::Display);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            m if (WM_APP_CONFIG..=WM_APP_HIRES).contains(&m) => push(Ev::App(m)),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// Janela das guias do arrasto: só desenho, o mouse passa direto.
unsafe extern "system" fn guides_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// Janela em primeiro plano mudou: reafirma "sempre por cima" e vê se um app em tela cheia cobriu o monitor.
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    // Vem de dentro do GetMessage (sem mensagem própria): posta uma para o laço acordar.
    post(WM_APP_FG);
}

#[derive(Clone, Copy, Debug)]
struct Mon {
    handle: HMONITOR,
    rc: RECT,
    work: RECT,
}

/// Monitores: o principal primeiro, depois os outros da esquerda para a direita.
fn monitors() -> Vec<Mon> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> windows::core::BOOL {
        unsafe {
            let v = &mut *(lp.0 as *mut Vec<(Mon, bool)>);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(h, &mut mi).as_bool() {
                v.push((Mon { handle: h, rc: mi.rcMonitor, work: mi.rcWork }, mi.dwFlags & MONITORINFOF_PRIMARY != 0));
            }
        }
        true.into()
    }
    let mut v: Vec<(Mon, bool)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut v as *mut _ as isize));
    }
    v.sort_by_key(|(m, primary)| (!*primary, m.rc.left, m.rc.top));
    v.into_iter().map(|(m, _)| m).collect()
}

fn dpi_scale(m: HMONITOR) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    unsafe {
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    (x as f32 / 96.0).clamp(1.0, 4.0)
}

fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

/// Número de um alvo do mouse (para a assinatura do quadro).
fn hcode(h: Option<LHit>) -> u32 {
    match h {
        None => 0,
        Some(LHit::Body) => 1,
        Some(LHit::Lock) => 2,
        Some(LHit::Full) => 3,
        Some(LHit::Close) => 4,
        Some(LHit::Compact) => 5,
        Some(LHit::Card(h)) => {
            10 + match h {
                Hit::Shuffle => 1,
                Hit::Prev => 2,
                Hit::Play => 3,
                Hit::Next => 4,
                Hit::Repeat => 5,
                Hit::Seek => 6,
                Hit::VolBtn => 7,
                Hit::Vol => 8,
                Hit::MixToggle => 9,
                Hit::MixDevice => 10,
                Hit::MixVol(i) => 20 + i as u32,
                Hit::MixMute(i) => 60 + i as u32,
                _ => 0,
            }
        }
    }
}

/// Arrasto da janela pelo fundo.
#[derive(Clone, Copy, Debug)]
struct Drag {
    cursor0: POINT,
    pos0: (i32, i32),
    moved: bool,
    /// Grudou num lugar pronto: (índice do monitor, lugar).
    snap: Option<(usize, Position)>,
    /// Monitor (índice) sob o mouse.
    mon: usize,
}

/// Arrasto de uma borda (muda a largura e/ou a altura).
#[derive(Clone, Copy, Debug)]
struct Resize {
    /// Bordas presas ao mouse (`view::EDGE_*`); as opostas ficam paradas.
    edge: u8,
    cursor0: POINT,
    /// O painel na tela quando começou (px).
    card0: RectF,
    moved: bool,
}

/// Cursor de cada borda (códigos do `CURSOR`).
fn edge_cursor(e: u8) -> u8 {
    let (l, r, t, b) = (e & view::EDGE_L != 0, e & view::EDGE_R != 0, e & view::EDGE_T != 0, e & view::EDGE_B != 0);
    match (l || r, t || b) {
        (true, true) if (l && t) || (r && b) => 5,
        (true, true) => 6,
        (true, false) => 3,
        (false, true) => 4,
        _ => 0,
    }
}

/// Guias do arrasto (os nove lugares tracejados no monitor).
struct Guides {
    hwnd: HWND,
    surface: Option<Surface>,
    shown: bool,
    /// O que está desenhado: monitor, lugar destacado e tamanho do painel.
    key: Option<(usize, Option<Position>, i32, i32)>,
}

/// Volume ou mudo que o usuário mudou num app do mixer, e quando.
struct MixHold {
    key: String,
    volume: Option<f32>,
    muted: Option<bool>,
    at: Instant,
}

struct St {
    shared: Arc<Shared>,
    hwnd: HWND,
    cfg: Config,
    cfg_version: u64,
    card_cfg: Config,
    colors: Colors,
    media: MediaInfo,
    media_version: u64,
    // letra
    track: Track,
    track_key: String,
    lyrics: Lyrics,
    lines: Vec<Line>,
    fetcher: Option<Fetcher>,
    /// A música mudou e a letra ainda não foi pedida (a janela estava escondida).
    fetch_due: bool,
    /// Linha atual (-1: antes da primeira / sem letra sincronizada).
    cur: i32,
    /// Troca de linha: de onde o foco vem e quando começou.
    anim: Option<(f32, Instant)>,
    scroll: f32,
    /// A letra chegou (aparece suave) e a música trocou (música anterior).
    lyr_at: Option<Instant>,
    change: Option<(Instant, MediaInfo)>,
    // janela
    visible: bool,
    surface: Option<Surface>,
    tr: Option<TextRenderer>,
    pacer: Option<Pacer>,
    geo: Geo,
    pos: (i32, i32),
    /// Escala do monitor e a usada (com o tamanho escolhido; na tela cheia, a do monitor).
    dpi: f32,
    scale: f32,
    mon: Option<Mon>,
    fullscreen: bool,
    /// Tela cheia: relógio do fundo animado (só anda tocando) e o último quadro dele.
    bg_clock: f32,
    bg_frame: Instant,
    /// Capa em alta resolução (tela cheia), da música com este título.
    hires: Option<(String, Arc<Image>)>,
    hires_asked: Option<String>,
    /// Um app em tela cheia está na frente, no mesmo monitor (opção "esconder").
    covered: bool,
    hook: Option<HWINEVENTHOOK>,
    hotkey: Option<Hotkey>,
    ex_style: u32,
    guides: Guides,
    // volume e mixer (layouts com player)
    volume: Option<Volume>,
    vol_drag: bool,
    mixer_open: bool,
    mixer_anim: f32,
    mix_scroll: f32,
    mix_hold: Vec<MixHold>,
    mix_drag: Option<(usize, String)>,
    mix_order: Vec<String>,
    // entrada
    hover: Option<LHit>,
    pressed: Option<LHit>,
    drag: Option<Drag>,
    /// Arrastando uma borda, e a borda sob o mouse agora (`view::EDGE_*`).
    resize: Option<Resize>,
    edge_hover: u8,
    seek: Option<f32>,
    seek_sent: Option<(f64, Instant)>,
    play_override: Option<(bool, Instant)>,
    tools: f32,
    last_step: Instant,
    icons: ui::IconAnims,
    /// Assinatura do último quadro (pula quadros iguais do relógio).
    last_sig: Option<(i64, u64)>,
    /// Botão da barrinha sob o mouse e desde quando (a dica aparece depois).
    tip: Option<(LHit, Instant)>,
    /// Aviso rápido (travou/destravou, tamanho) e quando apareceu.
    toast: Option<(String, Instant)>,
}

fn register_class(name: windows::core::PCWSTR, proc_: WNDPROC) -> Option<HINSTANCE> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: proc_,
            hInstance: hinst,
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        Some(hinst)
    }
}

fn run(shared: Arc<Shared>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let Some(hinst) = register_class(w!("TaskbeatLyrics"), Some(wndproc)) else { return finish() };
    let _ = register_class(w!("TaskbeatLyricsGuides"), Some(guides_proc));
    let made = unsafe {
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
        CreateWindowExW(ex, w!("TaskbeatLyrics"), w!("Taskbeat — Letra"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst), None)
    };
    let hwnd = match made {
        Ok(h) => h,
        Err(e) => {
            log::warn(&format!("letra: janela não criada: {e}"));
            return finish();
        }
    };
    let guides = unsafe {
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        CreateWindowExW(ex, w!("TaskbeatLyricsGuides"), w!("Taskbeat — Guias"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst), None).unwrap_or_default()
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    let cfg = shared.config();
    let now = Instant::now();
    let mut st = St {
        card_cfg: view::card_config(&cfg, &cfg.lyrics),
        colors: crate::system::colors(cfg.palette),
        cfg_version: u64::MAX,
        cfg,
        media: MediaInfo::default(),
        media_version: u64::MAX,
        track: Track::default(),
        track_key: String::new(),
        lyrics: Lyrics::Loading,
        lines: Vec::new(),
        fetcher: None,
        fetch_due: false,
        cur: -1,
        anim: None,
        scroll: 0.0,
        lyr_at: None,
        change: None,
        visible: false,
        surface: None,
        tr: None,
        pacer: Pacer::new(Target::Post(hwnd, WM_APP_FRAME)),
        geo: Geo::default(),
        pos: (0, 0),
        dpi: 1.0,
        scale: 1.0,
        mon: None,
        fullscreen: false,
        bg_clock: 0.0,
        bg_frame: now,
        hires: None,
        hires_asked: None,
        covered: false,
        hook: None,
        hotkey: None,
        ex_style: 0,
        guides: Guides { hwnd: guides, surface: None, shown: false, key: None },
        volume: None,
        vol_drag: false,
        mixer_open: false,
        mixer_anim: 0.0,
        mix_scroll: 0.0,
        mix_hold: Vec::new(),
        mix_drag: None,
        mix_order: Vec::new(),
        hover: None,
        pressed: None,
        drag: None,
        resize: None,
        edge_hover: 0,
        seek: None,
        seek_sent: None,
        play_override: None,
        tools: 0.0,
        last_step: now,
        icons: ui::IconAnims::default(),
        last_sig: None,
        tip: None,
        toast: None,
        shared,
        hwnd,
    };
    st.reload_config();
    st.reload_media();
    if FULL_REQ.swap(false, Ordering::Relaxed) {
        st.set_fullscreen(true);
    }
    let mut msg = MSG::default();
    'outer: loop {
        unsafe {
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !st.pump() {
            break;
        }
    }
    st.close();
}

/// A thread vai sair: marca que não está mais rodando.
fn finish() {
    WINDOW.store(0, Ordering::Relaxed);
    *RUNNING.lock().unwrap() = false;
}

impl St {
    fn lc(&self) -> &LyricsConfig {
        &self.cfg.lyrics
    }

    /// Volume na janela (compacto ou player, com a opção ligada).
    fn has_volume(&self) -> bool {
        !self.fullscreen && self.lc().volume && matches!(self.lc().layout, Layout::Compact | Layout::Player | Layout::PlayerLyrics)
    }

    fn has_player(&self) -> bool {
        !self.fullscreen && matches!(self.lc().layout, Layout::Player | Layout::PlayerLyrics)
    }

    /// Trata o que chegou; `false` = a thread deve sair.
    fn pump(&mut self) -> bool {
        let events = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
        for ev in events {
            match ev {
                Ev::App(WM_APP_CONFIG) => {
                    if !self.reload_config() {
                        return false;
                    }
                }
                Ev::App(WM_APP_MEDIA) => self.reload_media(),
                Ev::App(WM_APP_LYRICS) => self.lyrics_arrived(),
                Ev::App(WM_APP_FRAME) => {
                    if let Some(p) = &self.pacer {
                        p.ack();
                    }
                }
                Ev::App(WM_APP_FG) => self.foreground_changed(),
                Ev::App(WM_APP_REFETCH) => {
                    self.track_key.clear();
                    self.reload_media_force();
                }
                Ev::App(WM_APP_FULL) => {
                    if FULL_REQ.swap(false, Ordering::Relaxed) {
                        self.set_fullscreen(true);
                    }
                }
                Ev::App(WM_APP_HIRES) => self.hires_arrived(),
                Ev::App(_) => {}
                Ev::Hotkey(id) => self.hotkey(id),
                Ev::Timer(T_LINE) => self.update_line(true),
                Ev::Timer(T_FETCH) => self.fetch_now(),
                Ev::Timer(T_RETRY) => {
                    self.kill(T_RETRY);
                    if self.lyrics == Lyrics::Failed {
                        self.fetch_due = true;
                        self.fetch_now();
                    }
                }
                Ev::Timer(T_SAVE) => {
                    self.kill(T_SAVE);
                    self.shared.save_config();
                }
                Ev::Timer(id @ (T_TIP | T_TOAST)) => {
                    self.kill(id);
                    self.last_sig = None;
                }
                Ev::Timer(_) => {}
                Ev::Display => {
                    self.relayout();
                    self.last_sig = None;
                }
                ev => self.input(ev),
            }
        }
        self.render();
        true
    }

    // ------------------------------------------------------------ opções e música

    /// Relê as opções. `false`: não precisa mais da thread (ela sai).
    fn reload_config(&mut self) -> bool {
        let v = self.shared.config_version.load(Ordering::Relaxed);
        if v == self.cfg_version {
            return true;
        }
        self.cfg_version = v;
        let old = self.cfg.lyrics.clone();
        self.cfg = self.shared.config();
        if !wanted(self.lc()) {
            // Sai com o lock: quem quiser subir a thread de novo espera e vê que ela saiu.
            let mut running = RUNNING.lock().unwrap();
            if !wanted(&self.shared.config().lyrics) {
                WINDOW.store(0, Ordering::Relaxed);
                *running = false;
                return false;
            }
        }
        self.card_cfg = view::card_config(&self.cfg, self.lc());
        self.colors = crate::system::colors(self.cfg.palette);
        self.register_hotkeys();
        // Monitor, tamanho e trava podem ter mudado (soltou em outro monitor,
        // Ctrl+roda, as configurações): sem isto a janela já à vista
        // continuava calculando o lugar no monitor antigo.
        if self.visible {
            self.relayout();
        }
        if old.online != self.lc().online && matches!(self.lyrics, Lyrics::Offline | Lyrics::Failed) {
            self.track_key.clear();
        }
        if old.layout != self.lc().layout {
            self.scroll = 0.0;
        }
        if !self.lc().mixer || !matches!(self.lc().layout, Layout::Player | Layout::PlayerLyrics) {
            self.set_mixer(false);
        }
        // Travou ou destravou: um aviso curto do que mudou e de como voltar.
        if old.locked != self.lc().locked && old.show && self.lc().show {
            let msg = self.lock_toast();
            self.show_toast(msg);
        }
        self.last_sig = None;
        self.reload_media_force();
        true
    }

    fn reload_media(&mut self) {
        let v = self.shared.media_version.load(Ordering::Relaxed);
        if v == self.media_version {
            return;
        }
        self.reload_media_force();
    }

    fn reload_media_force(&mut self) {
        self.media_version = self.shared.media_version.load(Ordering::Relaxed);
        let mut new = self.shared.media.lock().unwrap().clone();
        // Aleatório e repetir que só a extensão do navegador alcança (como no widget).
        self.shared.web.lock().unwrap().merge_into(&mut new);
        let old = std::mem::replace(&mut self.media, new);
        let track = Track { artist: self.media.artist.clone(), title: self.media.title.clone(), album: self.media.album.clone(), dur: self.media.dur };
        let key = track.key();
        if key != self.track_key {
            // Troca de música animada (com a anterior à vista e a opção do widget ligada).
            let had = !self.track_key.is_empty() && old.present();
            self.change = (had && self.visible && self.cfg.track_anim).then(|| (Instant::now(), old));
            self.track_key = key;
            self.track = track;
            self.lyrics = Lyrics::Loading;
            self.lines.clear();
            self.cur = -1;
            self.anim = None;
            self.scroll = 0.0;
            self.lyr_at = None;
            self.hires = None;
            self.kill(T_RETRY);
            if self.media.title.trim().is_empty() {
                self.lyrics = Lyrics::NotFound;
                self.kill(T_FETCH);
            } else {
                self.fetch_due = true;
                self.set_timer(T_FETCH, FETCH_DELAY_MS);
            }
            self.last_sig = None;
        } else {
            // A duração costuma chegar depois do título.
            self.track.dur = self.media.dur;
        }
        if let Some((p, t)) = self.play_override
            && (p == self.media.playing || t.elapsed() > Duration::from_secs(2))
        {
            self.play_override = None;
        }
        if self.fullscreen {
            self.ask_hires();
        }
        self.update_visibility();
        self.update_line(true);
        self.schedule_tick();
    }

    /// Pede a letra da música atual (só com a janela à vista e um layout com letra).
    fn fetch_now(&mut self) {
        self.kill(T_FETCH);
        if !self.fetch_due {
            return;
        }
        if !self.visible || !self.lc().layout.has_lyrics() && !self.fullscreen {
            return;
        }
        if self.track.title.trim().is_empty() {
            return;
        }
        if self.fetcher.is_none() {
            let hwnd = self.hwnd.0 as isize;
            self.fetcher = Fetcher::spawn(lyrics::http_win::get, move |key, l| {
                *FETCHED.lock().unwrap() = Some((key, l));
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as _)), WM_APP_LYRICS, WPARAM(0), LPARAM(0));
                }
            });
        }
        if let Some(f) = &self.fetcher {
            self.fetch_due = false;
            f.request(self.track.clone(), self.lc().online);
        }
    }

    fn lyrics_arrived(&mut self) {
        let Some((key, l)) = FETCHED.lock().unwrap().take() else { return };
        if key != self.track_key {
            return;
        }
        if l == Lyrics::Failed {
            self.set_timer(T_RETRY, RETRY_MS);
        }
        self.lines = match &l {
            Lyrics::Synced(lines) => lyrics::with_gaps(lines),
            _ => Vec::new(),
        };
        self.lyrics = l;
        self.lyr_at = Some(Instant::now());
        self.cur = -2;
        self.anim = None;
        self.last_sig = None;
        self.update_line(false);
    }

    /// Tela cheia: pede a capa da música em alta resolução (numa thread à parte).
    fn ask_hires(&mut self) {
        let title = self.media.title.clone();
        if title.is_empty() || self.hires.as_ref().is_some_and(|(t, _)| *t == title) || self.hires_asked.as_ref() == Some(&title) {
            return;
        }
        self.hires_asked = Some(title);
        let hwnd = self.hwnd.0 as isize;
        let _ = std::thread::Builder::new().name("lyrics-cover".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Some(c) = crate::media::current_cover(1024) {
                *HIRES.lock().unwrap() = Some(c);
                unsafe {
                    let _ = PostMessageW(Some(HWND(hwnd as _)), WM_APP_HIRES, WPARAM(0), LPARAM(0));
                }
            }
        });
    }

    fn hires_arrived(&mut self) {
        let Some((title, img)) = HIRES.lock().unwrap().take() else { return };
        self.hires_asked = None;
        // Só vale se for maior que a do card e da música que está tocando.
        let small = self.media.art.as_ref().map(|a| a.w).unwrap_or(0);
        if title == self.media.title && img.w > small {
            self.hires = Some((title, Arc::new(img)));
            self.last_sig = None;
        }
    }

    /// Posição da música agora, com o ajuste de tempo.
    fn position(&self) -> f64 {
        let mut pos = self.media.position_now();
        if let Some((p, t)) = self.seek_sent
            && t.elapsed() < Duration::from_millis(1500)
        {
            pos = p + if self.media.playing { t.elapsed().as_secs_f64() } else { 0.0 };
        }
        pos + self.lc().offset_ms as f64 / 1000.0
    }

    /// Acha a linha atual e marca o timer até a próxima troca.
    fn update_line(&mut self, animate: bool) {
        self.kill(T_LINE);
        if !matches!(self.lyrics, Lyrics::Synced(_)) || self.lines.is_empty() {
            self.cur = -1;
            return;
        }
        let t = self.position();
        let new = lyrics::line_at(&self.lines, t).map(|i| i as i32).unwrap_or(-1);
        if new != self.cur {
            let from = self.focus();
            // Anima só a troca de uma linha para a seguinte (pulo = busca na música).
            self.anim = (animate && self.visible && self.cur >= -1 && new == self.cur + 1).then(|| (from, Instant::now()));
            self.cur = new;
            self.last_sig = None;
        }
        if self.media.playing
            && self.visible
            && let Some(next) = lyrics::next_change(&self.lines, t)
        {
            // Acorda um pouquinho antes (o timer do Windows atrasa até ~16 ms).
            let ms = ((next - t) * 1000.0 - 8.0).ceil().clamp(5.0, 3_600_000.0) as u32;
            self.set_timer(T_LINE, ms);
        }
    }

    /// Foco agora (contínuo durante a troca de linha).
    fn focus(&self) -> f32 {
        match self.anim {
            Some((from, t0)) => {
                let u = (t0.elapsed().as_secs_f32() / LINE_ANIM).min(1.0);
                from + (self.cur as f32 - from) * ui::ease_out_cubic(u)
            }
            None => self.cur as f32,
        }
    }

    /// Relógio do progresso (compacto, player, tela cheia): um quadro por segundo tocando.
    fn schedule_tick(&mut self) {
        let shows_time = self.fullscreen || matches!(self.lc().layout, Layout::Compact | Layout::Player | Layout::PlayerLyrics);
        if self.visible && self.media.playing && shows_time {
            self.set_timer(T_TICK, 1000);
        } else {
            self.kill(T_TICK);
        }
    }

    fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.hwnd), id, ms, None);
        }
    }

    fn kill(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
    }

    // ------------------------------------------------------------ atalhos e avisos

    fn register_hotkeys(&mut self) {
        let hk = self.lc().hotkey;
        if self.hotkey == Some(hk) {
            return;
        }
        self.hotkey = Some(hk);
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HK_SHOW);
            let _ = UnregisterHotKey(Some(self.hwnd), HK_LOCK);
        }
        let mods = match hk {
            Hotkey::WinAltL => MOD_WIN | MOD_ALT,
            Hotkey::CtrlAltL => MOD_CONTROL | MOD_ALT,
            Hotkey::CtrlShiftL => MOD_CONTROL | MOD_SHIFT,
            Hotkey::Off => {
                HOTKEY_REFUSED.store(0, Ordering::Relaxed);
                log::info("letra: atalho desligado");
                return;
            }
        };
        let lock = if mods.0 & MOD_SHIFT.0 != 0 { mods | MOD_ALT } else { mods | MOD_SHIFT };
        let key = 'L' as u32;
        // Resultado no log (com `TASKBEAT_LOG`): "registrado" ou o erro do Windows
        // (1409 = a combinação já é de outro programa).
        let reg = |id: i32, m: HOT_KEY_MODIFIERS, lock: bool| {
            let name = hotkey_label(hk, lock).unwrap_or("?");
            match unsafe { RegisterHotKey(Some(self.hwnd), id, m | MOD_NOREPEAT, key) } {
                Ok(()) => {
                    log::info(&format!("letra: atalho {name} registrado"));
                    true
                }
                Err(e) => {
                    log::warn(&format!("letra: atalho {name} recusado pelo Windows (erro {}: {})", e.code().0 & 0xffff, e.message()));
                    false
                }
            }
        };
        let ok = reg(HK_SHOW, mods, false);
        reg(HK_LOCK, lock, true);
        HOTKEY_REFUSED.store(if ok { 0 } else { hotkey_code(hk) }, Ordering::Relaxed);
    }

    fn hotkey(&mut self, id: i32) {
        log::info(&format!("letra: atalho apertado ({})", if id == HK_SHOW { "mostrar/esconder" } else { "travar/destravar" }));
        match id {
            HK_SHOW => {
                if self.fullscreen {
                    self.set_fullscreen(false);
                }
                self.shared.update_config(|c| c.lyrics.show = !c.lyrics.show);
            }
            HK_LOCK => self.shared.update_config(|c| {
                if c.lyrics.show {
                    c.lyrics.locked = !c.lyrics.locked;
                } else {
                    c.lyrics.show = true;
                    c.lyrics.locked = false;
                }
            }),
            _ => {}
        }
    }

    fn lock_toast(&self) -> String {
        use crate::i18n::tr;
        let lc = self.lc();
        let key = hotkey_label(lc.hotkey, true).filter(|_| !hotkey_refused(lc.hotkey));
        match (lc.locked, lc.click_through, key) {
            (true, true, Some(k)) => tr("Travada · o clique passa direto · {} destrava").replace("{}", k),
            (true, true, None) => tr("Travada · o clique passa direto · destrave pelo menu da bandeja").to_string(),
            (true, false, _) => tr("Posição travada").to_string(),
            (false, _, _) => tr("Destravada · arraste para mover").to_string(),
        }
    }

    fn show_toast(&mut self, msg: String) {
        self.toast = Some((msg, Instant::now()));
        // Acorda para o fim (some sozinho).
        self.set_timer(T_TOAST, ((TOAST_SECS - 0.4) * 1000.0) as u32);
        self.last_sig = None;
    }

    /// Texto da dica do botão da barrinha.
    fn tip_text(&self, h: LHit) -> String {
        use crate::i18n::tr;
        let lc = self.lc();
        match h {
            LHit::Lock => {
                let what = if lc.locked { tr("Destravar a posição") } else { tr("Travar a posição") };
                match hotkey_label(lc.hotkey, true).filter(|_| !hotkey_refused(lc.hotkey)) {
                    Some(k) => format!("{what} ({k})"),
                    None => what.to_string(),
                }
            }
            LHit::Compact if lc.layout == Layout::Compact => tr("Expandir").to_string(),
            LHit::Compact => tr("Compactar").to_string(),
            LHit::Full => tr("Tela cheia (Esc volta)").to_string(),
            LHit::Close => match working_hotkey(lc.hotkey) {
                Some(k) => format!("{} · {}", tr("Esconder"), tr("{} mostra de novo").replace("{}", k)),
                None => tr("Esconder").to_string(),
            },
            _ => String::new(),
        }
    }

    // ------------------------------------------------------------ visibilidade e lugar

    fn update_visibility(&mut self) {
        let lc = self.lc();
        let want = lc.show && (self.fullscreen || !(lc.hide_idle && !self.media.present())) && !(self.covered && !self.fullscreen);
        if want == self.visible {
            return;
        }
        self.visible = want;
        if want {
            self.relayout();
            unsafe {
                self.hook = Some(SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS));
            }
            if self.fetch_due {
                self.set_timer(T_FETCH, 10);
            }
            self.last_sig = None;
        } else {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
                if let Some(h) = self.hook.take() {
                    let _ = UnhookWinEvent(h);
                }
            }
            for t in [T_LINE, T_TICK] {
                self.kill(t);
            }
            if let Some(p) = &self.pacer {
                p.set_active(false);
            }
            self.set_mixer(false);
            self.hide_guides();
            self.surface = None;
            self.tr = None;
            self.fetcher = None;
            self.volume = None;
            self.hover = None;
            self.drag = None;
            self.change = None;
            self.hires = None;
            view::forget();
            crate::system::trim_memory();
        }
    }

    /// Monitor escolhido (o principal se ele não existir mais).
    fn monitor(&self) -> (usize, Mon) {
        let mons = monitors();
        let i = (self.lc().monitor.max(0) as usize).min(mons.len().saturating_sub(1));
        match mons.get(i) {
            Some(m) => (i, *m),
            None => (0, Mon { handle: HMONITOR::default(), rc: RECT { left: 0, top: 0, right: 1920, bottom: 1080 }, work: RECT { left: 0, top: 0, right: 1920, bottom: 1040 } }),
        }
    }

    /// Recalcula escala, monitor e estilo da janela (o tamanho vem no desenho).
    fn relayout(&mut self) {
        let (_, mon) = self.monitor();
        self.mon = Some(mon);
        self.dpi = dpi_scale(mon.handle);
        self.scale = if self.fullscreen { self.dpi } else { self.dpi * self.lc().zoom as f32 / 100.0 };
        if let Some(p) = &self.pacer {
            p.set_monitor(mon.handle);
        }
        self.apply_style();
        self.last_sig = None;
    }

    /// Travada com "o clique atravessa": a janela não pega o mouse. Na tela
    /// cheia, sempre clicável e ativável (para o Esc).
    fn apply_style(&mut self) {
        let lc = self.lc();
        let click_through = lc.locked && lc.click_through && !self.fullscreen;
        let mut ex = (WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST).0;
        if click_through {
            ex |= WS_EX_TRANSPARENT.0;
        }
        if !self.fullscreen {
            ex |= WS_EX_NOACTIVATE.0;
        }
        ACTIVATE.with(|a| a.set(self.fullscreen));
        if ex != self.ex_style {
            self.ex_style = ex;
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, ex as isize);
            }
            if click_through {
                self.hover = None;
                self.pressed = None;
                self.drag = None;
            }
        }
    }

    /// Onde o painel fica (x, y na tela) num lugar pronto do monitor `m`.
    fn grid_spot(&self, m: &Mon, p: Position, cw: f32, ch: f32) -> Option<(f32, f32)> {
        let (fx, fy) = p.frac()?;
        let a = self.grid_area(m);
        Some((a.x + (a.w - cw) * fx, a.y + (a.h - ch) * fy))
    }

    /// Onde os lugares prontos cabem: a área de trabalho de `m` menos a margem
    /// (em coordenadas da tela).
    fn grid_area(&self, m: &Mon) -> RectF {
        let work = m.work;
        let margin = 24.0 * self.dpi;
        RectF::new(work.left as f32 + margin, work.top as f32 + margin, (work.right - work.left) as f32 - 2.0 * margin, (work.bottom - work.top) as f32 - 2.0 * margin)
    }

    /// Onde a superfície fica na tela para o painel ir ao lugar escolhido.
    fn place(&self, g: &Geo, mon: &Mon) -> (i32, i32) {
        if g.fullscreen {
            return (mon.rc.left, mon.rc.top);
        }
        let lc = self.lc();
        let work = mon.work;
        let (ww, wh) = ((work.right - work.left) as f32, (work.bottom - work.top) as f32);
        let (cw, ch) = (g.card.w, g.card.h);
        let (cx, cy) = match self.grid_spot(mon, lc.position, cw, ch) {
            Some(p) => p,
            None => {
                let (mx, my) = (work.left as f32 + lc.free_x * ww, work.top as f32 + lc.free_y * wh);
                ((mx - cw / 2.0).clamp(work.left as f32, (work.right as f32 - cw).max(work.left as f32)), (my - ch / 2.0).clamp(work.top as f32, (work.bottom as f32 - ch).max(work.top as f32)))
            }
        };
        ((cx - g.card.x).round() as i32, (cy - g.card.y).round() as i32)
    }

    fn foreground_changed(&mut self) {
        if self.visible {
            unsafe {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
            }
        }
        let covered = self.lc().hide_fullscreen && self.fullscreen_app();
        if covered != self.covered {
            self.covered = covered;
            self.update_visibility();
        }
    }

    /// Um app (não o shell) cobre o monitor da sobreposição inteiro?
    fn fullscreen_app(&self) -> bool {
        let Some(mon) = self.mon else { return false };
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == self.hwnd {
                return false;
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(fg, &mut buf) as usize;
            let cls = String::from_utf16_lossy(&buf[..n]);
            if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Windows.UI.Core.CoreWindow" | "XamlExplorerHostIslandWindow") {
                return false;
            }
            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_err() {
                return false;
            }
            r.left <= mon.rc.left && r.top <= mon.rc.top && r.right >= mon.rc.right && r.bottom >= mon.rc.bottom
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        if on == self.fullscreen {
            return;
        }
        if on && !self.lc().show {
            self.shared.update_config(|c| c.lyrics.show = true);
        }
        self.fullscreen = on;
        // As máscaras de texto da tela cheia são enormes: não ficam guardadas
        // depois (nem as da janela durante ela).
        self.tr = None;
        self.anim = None;
        self.hover = None;
        self.pressed = None;
        self.drag = None;
        self.surface = None;
        self.change = None;
        self.last_sig = None;
        self.set_mixer(false);
        if !on {
            self.hires = None;
            view::forget();
        }
        self.update_visibility();
        self.relayout();
        if on {
            self.fetch_due |= self.lyrics == Lyrics::Loading;
            self.fetch_now();
            self.ask_hires();
        }
        self.render();
        if on {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
        }
        self.schedule_tick();
    }

    // ------------------------------------------------------------ volume e mixer

    fn set_mixer(&mut self, open: bool) {
        if open != self.mixer_open {
            self.mixer_open = open;
            self.mix_scroll = 0.0;
            self.mix_drag = None;
            self.shared.send_mixer(if open { MixCmd::Open } else { MixCmd::Close });
            if !open {
                self.mix_order.clear();
            }
            self.last_sig = None;
        }
    }

    /// O mixer como aparece: o publicado pela thread, com o que o usuário
    /// acabou de mudar por cima, na ordem da abertura (os sons do sistema em cima).
    fn mixer_view(&mut self) -> MixerState {
        let mut st = self.shared.mixer.lock().unwrap().clone();
        self.mix_hold.retain(|h| h.at.elapsed() < MIX_HOLD);
        for a in &mut st.apps {
            if let Some(h) = self.mix_hold.iter().find(|h| h.key == a.key) {
                a.volume = h.volume.unwrap_or(a.volume);
                a.muted = h.muted.unwrap_or(a.muted);
            }
        }
        if self.mixer_open {
            self.mix_order.retain(|k| st.apps.iter().any(|a| a.key == *k));
            for a in &st.apps {
                if !self.mix_order.contains(&a.key) {
                    self.mix_order.push(a.key.clone());
                }
            }
        }
        let order = &self.mix_order;
        st.apps.sort_by_key(|a| (!a.system, order.iter().position(|k| *k == a.key).unwrap_or(usize::MAX)));
        st
    }

    fn set_mix(&mut self, key: &str, volume: Option<f32>, mute: Option<bool>) {
        let muted_now = self.shared.mixer.lock().unwrap().apps.iter().find(|a| a.key == key).map(|a| a.muted).unwrap_or(false);
        let i = match self.mix_hold.iter().position(|h| h.key == key) {
            Some(i) => i,
            None => {
                self.mix_hold.push(MixHold { key: key.to_string(), volume: None, muted: None, at: Instant::now() });
                self.mix_hold.len() - 1
            }
        };
        let h = &mut self.mix_hold[i];
        h.at = Instant::now();
        if let Some(v) = volume {
            let v = v.clamp(0.0, 1.0);
            h.volume = Some(v);
            self.shared.send_mixer(MixCmd::Volume(key.to_string(), v));
            // Mexer no volume tira o mudo, como no Mixer de Volume do Windows.
            if muted_now && v > 0.0 && mute.is_none() {
                h.muted = Some(false);
                self.shared.send_mixer(MixCmd::Mute(key.to_string(), false));
            }
        }
        if let Some(m) = mute {
            h.muted = Some(m);
            self.shared.send_mixer(MixCmd::Mute(key.to_string(), m));
        }
        self.last_sig = None;
    }

    /// Lista das saídas de áudio (embaixo do nome da saída no mixer); a escolhida vira a padrão.
    fn pick_output(&mut self) {
        let Some(l) = self.geo.cl.clone() else { return };
        let mv = self.mixer_view();
        if mv.devices.is_empty() {
            return;
        }
        let d = l.mixer_dev;
        let at = RECT { left: self.pos.0 + d.x as i32, top: self.pos.1 + d.y as i32, right: self.pos.0 + d.right() as i32, bottom: self.pos.1 + d.bottom() as i32 };
        let th = view::theme(&self.card_cfg, self.lc(), &self.colors, &self.media);
        let entries: Vec<crate::menu::Entry> = mv.devices.iter().enumerate().map(|(k, o)| crate::menu::Entry::radio(k as u32 + 1, &o.name, o.id == mv.device_id)).collect();
        let opts = crate::menu::Options { dark: th.dark, accent: th.accent, min_width: 380.0, style: th.style };
        let pick = crate::menu::popup(self.hwnd, entries, crate::menu::Anchor::Below(at), opts, &mut |_| None);
        self.pressed = None;
        if let Some(o) = pick.and_then(|k| mv.devices.get(k as usize - 1)).filter(|o| o.id != mv.device_id) {
            log::info(&format!("letra: trocando a saída de áudio para {}", o.name));
            self.shared.send_mixer(MixCmd::SetDevice(o.id.clone()));
        }
    }

    fn mix_key(&mut self, i: usize) -> Option<(String, bool)> {
        self.mixer_view().apps.get(i).map(|a| (a.key.clone(), a.muted))
    }

    // ------------------------------------------------------------ entrada

    fn playback(&self) -> Playback {
        let playing = self.play_override.map(|(p, _)| p).unwrap_or(self.media.playing);
        let (volume, muted) = self.volume.as_ref().map(|v| (v.level, v.muted)).unwrap_or((0.0, false));
        Playback { playing, shuffle: self.media.shuffle, repeat: self.media.repeat, pos: self.position() - self.lc().offset_ms as f64 / 1000.0, seek_preview: self.seek, volume, muted }
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<LHit> {
        view::hit(&self.geo, self.lc(), x, y, !self.fullscreen)
    }

    fn seek_frac(&self, x: f32) -> Option<f32> {
        let (x0, x1) = view::seek_track(&self.geo, self.scale)?;
        Some(((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0))
    }

    fn vol_frac(&self, x: f32) -> Option<f32> {
        let (x0, x1) = view::vol_track(&self.geo, self.scale)?;
        Some(((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0))
    }

    fn mix_frac(&self, i: usize, x: f32) -> Option<f32> {
        let (x0, x1) = self.geo.cl.as_ref()?.mix_track(i, self.scale)?;
        Some(((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0))
    }

    fn input(&mut self, ev: Ev) {
        if !self.visible {
            return;
        }
        match ev {
            Ev::Move(x, y) => {
                if let Some(r) = self.resize {
                    self.resize_to(r);
                    return;
                }
                if let Some(d) = self.drag {
                    self.drag_to(d);
                    return;
                }
                if self.seek.is_some() {
                    self.seek = self.seek_frac(x);
                    self.last_sig = None;
                    return;
                }
                if self.vol_drag {
                    if let (Some(f), Some(v)) = (self.vol_frac(x), self.volume.as_mut()) {
                        v.set(f);
                    }
                    self.last_sig = None;
                    return;
                }
                if let Some((i, key)) = self.mix_drag.clone() {
                    if let Some(f) = self.mix_frac(i, x) {
                        self.set_mix(&key, Some(f), None);
                    }
                    return;
                }
                let h = self.hit_at(x, y);
                if h != self.hover {
                    self.hover = h;
                    // Dica: só nos botões da barrinha, depois de um instante.
                    self.tip = h.filter(|h| matches!(h, LHit::Lock | LHit::Compact | LHit::Full | LHit::Close)).map(|h| (h, Instant::now()));
                    if self.tip.is_some() {
                        self.set_timer(T_TIP, (TIP_DELAY * 1000.0) as u32 + 10);
                    }
                }
                // Bordas (destravada): fora dos botões da barrinha e da alça dela.
                self.edge_hover = if self.lc().locked || self.fullscreen || matches!(h, Some(LHit::Lock | LHit::Compact | LHit::Full | LHit::Close)) || (self.tools > 0.0 && self.geo.toolbar.contains(x, y)) {
                    0
                } else {
                    view::resize_edge(&self.geo, self.lc(), x, y, self.scale)
                };
                let cursor = match h {
                    _ if self.edge_hover != 0 => edge_cursor(self.edge_hover),
                    Some(LHit::Body) if !self.lc().locked && !self.fullscreen => 1,
                    Some(LHit::Card(Hit::Seek)) if !self.media.can_seek => 0,
                    Some(LHit::Card(_)) | Some(LHit::Lock) | Some(LHit::Compact) | Some(LHit::Full) | Some(LHit::Close) => 2,
                    _ => 0,
                };
                CURSOR.with(|c| c.set(cursor));
            }
            Ev::Leave => {
                if self.drag.is_none() && self.resize.is_none() && self.seek.is_none() && !self.vol_drag && self.mix_drag.is_none() {
                    self.edge_hover = 0;
                    self.hover = None;
                    self.pressed = None;
                    self.tip = None;
                }
            }
            Ev::Down(x, y) => {
                if self.edge_hover != 0 && !self.lc().locked && !self.fullscreen {
                    let c = self.geo.card;
                    let card0 = RectF::new(self.pos.0 as f32 + c.x, self.pos.1 as f32 + c.y, c.w, c.h);
                    self.resize = Some(Resize { edge: self.edge_hover, cursor0: cursor_pos(), card0, moved: false });
                    self.pressed = None;
                    return;
                }
                let h = self.hit_at(x, y);
                self.pressed = h;
                match h {
                    Some(LHit::Body) if !self.lc().locked && !self.fullscreen => {
                        let (i, _) = self.monitor();
                        self.drag = Some(Drag { cursor0: cursor_pos(), pos0: self.pos, moved: false, snap: None, mon: i });
                    }
                    Some(LHit::Card(Hit::Seek)) if self.media.can_seek => self.seek = self.seek_frac(x),
                    Some(LHit::Card(Hit::Vol)) => {
                        self.vol_drag = true;
                        if let (Some(f), Some(v)) = (self.vol_frac(x), self.volume.as_mut()) {
                            v.set(f);
                        }
                    }
                    Some(LHit::Card(Hit::MixVol(i))) => {
                        if let Some((key, _)) = self.mix_key(i as usize) {
                            if let Some(f) = self.mix_frac(i as usize, x) {
                                self.set_mix(&key, Some(f), None);
                            }
                            self.mix_drag = Some((i as usize, key));
                        }
                    }
                    _ => {}
                }
                self.last_sig = None;
            }
            Ev::Up(x, y) => {
                if self.resize.is_some() {
                    self.finish_resize();
                    return;
                }
                if let Some(d) = self.drag.take() {
                    self.hide_guides();
                    if d.moved {
                        self.drop_at(d);
                    }
                    self.pressed = None;
                    return;
                }
                if let Some(f) = self.seek.take() {
                    let p = f as f64 * self.media.dur;
                    self.shared.send_media(MediaCmd::Seek(p));
                    self.seek_sent = Some((p, Instant::now()));
                    self.pressed = None;
                    self.update_line(false);
                    self.last_sig = None;
                    return;
                }
                if self.vol_drag || self.mix_drag.is_some() {
                    self.vol_drag = false;
                    self.mix_drag = None;
                    self.pressed = None;
                    self.last_sig = None;
                    return;
                }
                let h = self.hit_at(x, y);
                if let Some(h) = h.filter(|h| Some(*h) == self.pressed) {
                    self.click(h);
                }
                self.pressed = None;
            }
            Ev::CaptureLost => {
                self.finish_resize();
                if let Some(d) = self.drag.take() {
                    self.hide_guides();
                    if d.moved {
                        self.drop_at(d);
                    }
                }
                self.seek = None;
                self.vol_drag = false;
                self.mix_drag = None;
                self.pressed = None;
            }
            Ev::Wheel(delta) => self.wheel(delta),
            Ev::Key(k) if self.fullscreen => {
                if k == VK_ESCAPE.0 as u32 {
                    self.set_fullscreen(false);
                } else if k == VK_SPACE.0 as u32 {
                    self.click(LHit::Card(Hit::Play));
                }
            }
            _ => {}
        }
    }

    /// Roda: Ctrl muda o tamanho; sobre o mixer rola a lista; sobre o
    /// player muda o volume; na letra simples, rola a letra.
    fn wheel(&mut self, delta: i16) {
        let notches = delta as f32 / 120.0;
        self.last_sig = None;
        if ctrl_down() && !self.fullscreen {
            let z = (self.lc().zoom + (notches.signum() as i32) * 10).clamp(ZOOM_MIN, ZOOM_MAX);
            if z != self.lc().zoom {
                self.shared.preview_config(|c| c.lyrics.zoom = z);
                self.set_timer(T_SAVE, 700);
                self.show_toast(crate::i18n::tr("Tamanho {}%").replace("{}", &z.to_string()));
            }
            return;
        }
        let mouse = {
            let p = cursor_pos();
            ((p.x - self.pos.0) as f32, (p.y - self.pos.1) as f32)
        };
        if let Some(l) = self.geo.cl.as_ref().filter(|_| self.has_player()) {
            if self.mixer_open && l.mixer_list.contains(mouse.0, mouse.1) {
                self.mix_scroll = (self.mix_scroll - notches * 36.0).clamp(0.0, l.mixer_scroll_max);
                return;
            }
            if self.lc().volume && l.card.contains(mouse.0, mouse.1) && !self.geo.lyr.contains(mouse.0, mouse.1) {
                if let Some(v) = self.volume.as_mut() {
                    v.step(notches * 0.04);
                }
                return;
            }
        }
        // Compacto: a roda em cima do mini player (fora da letra) muda o volume.
        if self.has_volume() && self.lc().layout == Layout::Compact && self.geo.card.contains(mouse.0, mouse.1) && !self.geo.lyr.contains(mouse.0, mouse.1) {
            if let Some(v) = self.volume.as_mut() {
                v.step(notches * 0.04);
            }
            return;
        }
        if let (Lyrics::Plain(lines), Some(tr)) = (&self.lyrics, self.tr.as_mut()) {
            let s = self.scale;
            let total = view::plain_height(tr, lines, self.geo.dim, self.geo.lyr.w, s) / s;
            let max = (total - self.geo.lyr.h / s + 12.0).max(0.0);
            self.scroll = (self.scroll - notches * 48.0).clamp(0.0, max);
        }
    }

    fn click(&mut self, h: LHit) {
        match h {
            LHit::Lock => self.shared.update_config(|c| c.lyrics.locked = !c.lyrics.locked),
            LHit::Compact => self.shared.update_config(|c| c.lyrics.toggle_compact()),
            LHit::Full => self.set_fullscreen(!self.fullscreen),
            LHit::Close => {
                if self.fullscreen {
                    self.set_fullscreen(false);
                }
                self.shared.update_config(|c| c.lyrics.show = false);
            }
            LHit::Card(Hit::Play) if self.media.can_toggle => {
                let p = !self.playback().playing;
                self.play_override = Some((p, Instant::now()));
                self.icons.kick(Hit::Play);
                self.shared.send_media(MediaCmd::PlayPause);
            }
            LHit::Card(Hit::Prev) => {
                self.icons.kick(Hit::Prev);
                self.shared.send_media(MediaCmd::Previous);
            }
            LHit::Card(Hit::Next) => {
                self.icons.kick(Hit::Next);
                self.shared.send_media(MediaCmd::Next);
            }
            LHit::Card(Hit::Shuffle) if self.media.can_shuffle => {
                self.icons.kick(Hit::Shuffle);
                if self.media.web_shuffle {
                    self.shared.send_web(crate::shared::WebCmd::Shuffle { aumid: self.media.aumid.clone(), title: self.media.title.clone() });
                } else {
                    self.shared.send_media(MediaCmd::Shuffle(!self.media.shuffle));
                }
            }
            LHit::Card(Hit::Repeat) if self.media.can_repeat => {
                self.icons.kick(Hit::Repeat);
                if self.media.web_repeat {
                    self.shared.send_web(crate::shared::WebCmd::Repeat { aumid: self.media.aumid.clone(), title: self.media.title.clone() });
                } else {
                    self.shared.send_media(MediaCmd::Repeat((self.media.repeat + 1) % 3));
                }
            }
            LHit::Card(Hit::VolBtn) => {
                self.icons.kick(Hit::VolBtn);
                if let Some(v) = self.volume.as_mut() {
                    v.toggle_mute();
                }
            }
            LHit::Card(Hit::MixToggle) => self.set_mixer(!self.mixer_open),
            LHit::Card(Hit::MixMute(i)) => {
                if let Some((key, muted)) = self.mix_key(i as usize) {
                    self.set_mix(&key, None, Some(!muted));
                }
            }
            LHit::Card(Hit::MixDevice) => self.pick_output(),
            _ => {}
        }
        self.last_sig = None;
    }

    // ------------------------------------------------------------ tamanho pelas bordas

    /// Arrastando uma borda: o tamanho novo vale na hora (só aqui, sem gravar);
    /// as bordas opostas ficam paradas na tela.
    fn resize_to(&mut self, mut r: Resize) {
        let c = cursor_pos();
        let (dx, dy) = ((c.x - r.cursor0.x) as f32, (c.y - r.cursor0.y) as f32);
        if !r.moved && dx.abs() < 2.0 && dy.abs() < 2.0 {
            return;
        }
        r.moved = true;
        let mut w = r.card0.w;
        let mut h = r.card0.h;
        if r.edge & view::EDGE_R != 0 {
            w += dx;
        }
        if r.edge & view::EDGE_L != 0 {
            w -= dx;
        }
        if r.edge & view::EDGE_B != 0 {
            h += dy;
        }
        if r.edge & view::EDGE_T != 0 {
            h -= dy;
        }
        // Nunca maior que a área de trabalho do monitor.
        if let Some(m) = self.mon {
            w = w.min((m.work.right - m.work.left) as f32);
            h = h.min((m.work.bottom - m.work.top) as f32);
        }
        let (width, height) = view::size_for(&self.geo, &self.cfg.lyrics, w, h, self.scale);
        self.cfg.lyrics.width = width;
        self.cfg.lyrics.height = height;
        self.resize = Some(r);
        self.last_sig = None;
    }

    /// Soltou a borda: grava o tamanho. O lugar pronto continua se o painel
    /// ainda está nele; senão a janela fica onde está ("Livre").
    fn finish_resize(&mut self) {
        let Some(r) = self.resize.take() else { return };
        self.pressed = None;
        if !r.moved {
            return;
        }
        let (width, height) = (self.cfg.lyrics.width, self.cfg.lyrics.height);
        let c = self.geo.card;
        let card = RectF::new(self.pos.0 as f32 + c.x, self.pos.1 as f32 + c.y, c.w, c.h);
        let (_, mon) = self.monitor();
        let work = mon.work;
        let (ww, wh) = ((work.right - work.left) as f32, (work.bottom - work.top) as f32);
        let keep = self.grid_spot(&mon, self.lc().position, card.w, card.h).is_some_and(|(x, y)| (x - card.x).abs() < 3.0 && (y - card.y).abs() < 3.0);
        let free = (!keep).then(|| (((card.cx() - work.left as f32) / ww).clamp(0.0, 1.0), ((card.cy() - work.top as f32) / wh).clamp(0.0, 1.0)));
        log::info(&format!("letra: tamanho {width}x{height} (lugar {})", if keep { "pronto" } else { "livre" }));
        self.shared.update_config(|c| {
            c.lyrics.width = width;
            c.lyrics.height = height;
            if let Some((fx, fy)) = free {
                c.lyrics.position = Position::Free;
                c.lyrics.free_x = fx;
                c.lyrics.free_y = fy;
            }
        });
        self.last_sig = None;
    }

    // ------------------------------------------------------------ arrasto e guias

    fn drag_to(&mut self, mut d: Drag) {
        let c = cursor_pos();
        let (dx, dy) = (c.x - d.cursor0.x, c.y - d.cursor0.y);
        if !d.moved && dx.abs() < 4 && dy.abs() < 4 {
            return;
        }
        d.moved = true;
        let card = self.geo.card;
        let (mut px, mut py) = ((d.pos0.0 + dx) as f32 + card.x, (d.pos0.1 + dy) as f32 + card.y);
        let mons = monitors();
        let mi = mons.iter().position(|m| c.x >= m.rc.left && c.x < m.rc.right && c.y >= m.rc.top && c.y < m.rc.bottom).unwrap_or(d.mon);
        d.mon = mi;
        d.snap = None;
        if let Some(m) = mons.get(mi) {
            // Ímã: perto de um lugar pronto, gruda nele.
            let reach = 40.0 * self.dpi;
            let best = Position::GRID
                .iter()
                .filter_map(|p| self.grid_spot(m, *p, card.w, card.h).map(|(x, y)| (*p, x, y, ((x - px).powi(2) + (y - py).powi(2)).sqrt())))
                .min_by(|a, b| a.3.total_cmp(&b.3));
            if let Some((p, x, y, dist)) = best
                && dist < reach
            {
                (px, py) = (x, y);
                d.snap = Some((mi, p));
            }
            self.show_guides(mi, m, d.snap.map(|(_, p)| p));
        }
        let (x, y) = ((px - card.x).round() as i32, (py - card.y).round() as i32);
        self.pos = (x, y);
        unsafe {
            let _ = SetWindowPos(self.hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.drag = Some(d);
    }

    /// Mostra (ou atualiza) os nove lugares tracejados no monitor `m`, logo
    /// abaixo da janela; redesenha só quando muda o monitor ou o destaque.
    fn show_guides(&mut self, mi: usize, m: &Mon, snap: Option<Position>) {
        let card = self.geo.card;
        let key = (mi, snap, card.w as i32, card.h as i32);
        if self.guides.key == Some(key) && self.guides.shown {
            return;
        }
        self.guides.key = Some(key);
        let (w, h) = (m.rc.right - m.rc.left, m.rc.bottom - m.rc.top);
        Surface::ensure(&mut self.guides.surface, w, h);
        let accent = view::theme(&self.card_cfg, self.lc(), &self.colors, &self.media).accent;
        // Cada lugar na sua terça parte da tela, sem cruzar com os vizinhos.
        let a = self.grid_area(m);
        let area = RectF::new(a.x - m.rc.left as f32, a.y - m.rc.top as f32, a.w, a.h);
        let mut slots: Vec<(RectF, &'static str, bool)> = Vec::new();
        for p in Position::GRID {
            if let Some(frac) = p.frac() {
                let g = view::guide_rect(card.w, card.h, frac, area, 16.0 * self.dpi);
                let r = RectF::new(g.x.round(), g.y.round(), g.w.round(), g.h.round());
                slots.push((r, crate::i18n::tr(position_name(p)), snap == Some(p)));
            }
        }
        let radius = self.geo.radius.max(10.0 * self.dpi);
        let dpi = self.dpi;
        let mut tr = self.tr.take().unwrap_or_else(TextRenderer::new);
        tr.set_scale(dpi);
        if let Some(surf) = self.guides.surface.as_mut() {
            let mut cv = surf.canvas();
            cv.clear();
            view::draw_guides(&mut cv, &mut tr, &slots, accent, radius, dpi);
            surf.present_at(self.guides.hwnd, m.rc.left, m.rc.top as f32, 1.0);
        }
        // O texto da janela volta à escala dela no próximo quadro.
        self.tr = Some(tr);
        self.last_sig = None;
        if !self.guides.shown {
            self.guides.shown = true;
            unsafe {
                let _ = SetWindowPos(self.guides.hwnd, Some(self.hwnd), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let _ = ShowWindow(self.guides.hwnd, SW_SHOWNOACTIVATE);
                // A janela da letra continua por cima das guias.
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn hide_guides(&mut self) {
        if self.guides.shown {
            unsafe {
                let _ = ShowWindow(self.guides.hwnd, SW_HIDE);
            }
            self.guides.shown = false;
        }
        self.guides.surface = None;
        self.guides.key = None;
    }

    /// Soltou: grava o lugar pronto (se grudou) ou o lugar livre, e o monitor.
    fn drop_at(&mut self, d: Drag) {
        let mons = monitors();
        let Some(m) = mons.get(d.mon) else { return };
        let g = &self.geo;
        let (cx, cy) = (self.pos.0 as f32 + g.card.cx(), self.pos.1 as f32 + g.card.cy());
        let work = m.work;
        let fx = ((cx - work.left as f32) / (work.right - work.left).max(1) as f32).clamp(0.0, 1.0);
        let fy = ((cy - work.top as f32) / (work.bottom - work.top).max(1) as f32).clamp(0.0, 1.0);
        let mon = d.mon as i32;
        let snap = d.snap.map(|(_, p)| p);
        self.shared.update_config(|c| {
            c.lyrics.monitor = mon;
            match snap {
                Some(p) => c.lyrics.position = p,
                None => {
                    c.lyrics.position = Position::Free;
                    c.lyrics.free_x = fx;
                    c.lyrics.free_y = fy;
                }
            }
        });
    }

    // ------------------------------------------------------------ desenho

    fn render(&mut self) {
        if !self.visible {
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_step).as_secs_f32().min(0.1);
        self.last_step = now;
        // Barrinha: aparece com o mouse em cima (com o clique atravessando, nunca há mouse).
        let tools_target = if self.hover.is_some() && !self.fullscreen { 1.0 } else { 0.0 };
        if self.tools != tools_target {
            let step = dt / TOOLS_ANIM;
            self.tools = if tools_target > self.tools { (self.tools + step).min(1.0) } else { (self.tools - step).max(0.0) };
        }
        let mixer_target = if self.mixer_open { 1.0 } else { 0.0 };
        if self.mixer_anim != mixer_target {
            let step = dt / MIXER_ANIM;
            self.mixer_anim = if mixer_target > self.mixer_anim { (self.mixer_anim + step).min(1.0) } else { (self.mixer_anim - step).max(0.0) };
        }
        if self.anim.is_some_and(|(_, t0)| t0.elapsed().as_secs_f32() >= LINE_ANIM) {
            self.anim = None;
        }
        if self.change.as_ref().is_some_and(|(t0, _)| t0.elapsed().as_secs_f32() >= TRACK_ANIM) {
            self.change = None;
        }
        let lyr_alpha = self.lyr_at.map(|t| (t.elapsed().as_secs_f32() / LYRIC_FADE).min(1.0)).unwrap_or(1.0);
        if self.has_volume() {
            self.volume.get_or_insert_with(Volume::new).poll();
        }
        let pb = self.playback();
        self.icons.observe(&pb);
        // Dica (aparece depois de um instante parado) e aviso (some sozinho).
        let tip_a = self.tip.map(|(_, t0)| ((t0.elapsed().as_secs_f32() - TIP_DELAY) / 0.12).clamp(0.0, 1.0)).unwrap_or(0.0);
        if self.toast.as_ref().is_some_and(|(_, t0)| t0.elapsed().as_secs_f32() >= TOAST_SECS) {
            self.toast = None;
        }
        let toast_a = self.toast.as_ref().map(|(_, t0)| {
            let e = t0.elapsed().as_secs_f32();
            (e / 0.15).min(1.0) * ((TOAST_SECS - e) / 0.4).clamp(0.0, 1.0)
        });
        let toast_moving = self.toast.as_ref().is_some_and(|(_, t0)| {
            let e = t0.elapsed().as_secs_f32();
            e < 0.15 || e > TOAST_SECS - 0.4
        });
        // Fundo animado da tela cheia: o relógio só anda tocando.
        let bg_anim = self.fullscreen && self.lc().animated_bg && pb.playing;
        if bg_anim {
            self.bg_clock += dt;
        }
        let busy = self.anim.is_some()
            || self.tools != tools_target
            || self.mixer_anim != mixer_target
            || self.icons.running()
            || self.seek.is_some()
            || self.vol_drag
            || self.mix_drag.is_some()
            || (tip_a > 0.0 && tip_a < 1.0)
            || toast_moving
            || self.change.is_some()
            || lyr_alpha < 1.0;
        if let Some(p) = &self.pacer {
            p.set_active(busy || bg_anim);
        }
        let Some(mon) = self.mon else { return };
        // Só o fundo mexendo: ~24 quadros por segundo bastam (movimento lento).
        if bg_anim && !busy && self.surface.is_some() && now - self.bg_frame < BG_FRAME {
            return;
        }
        // Pula quadros iguais (ex.: o relógio sem mudar de segundo).
        let sig = (
            pb.pos.floor() as i64,
            ui::layer_key((
                (hcode(self.hover), hcode(self.pressed), self.cur, pb.playing, (pb.volume * 200.0) as i32, pb.muted),
                (self.tools.to_bits(), self.scroll.to_bits(), tip_a.to_bits(), toast_a.map(f32::to_bits), self.mixer_anim.to_bits(), self.mix_scroll.to_bits()),
                self.shared.mixer_version.load(Ordering::Relaxed),
            )),
        );
        if !busy && !bg_anim && self.last_sig == Some(sig) && self.surface.is_some() {
            return;
        }
        self.last_sig = Some(sig);
        self.bg_frame = now;
        let s = self.scale;
        let mut tr = self.tr.take().unwrap_or_else(TextRenderer::new);
        tr.set_scale(s);
        tr.set_capacity(260);
        let focus = self.focus();
        let mixer = (self.has_player() && self.lc().mixer).then(|| self.mixer_view());
        let mix_geo = mixer.as_ref().map(|m| (m.apps.len(), ui::ease_move(self.mixer_anim), self.mix_scroll));
        let geo = if self.fullscreen {
            view::fullscreen_geometry((mon.rc.right - mon.rc.left) as f32, (mon.rc.bottom - mon.rc.top) as f32, s)
        } else {
            let frame = LyricFrame { lyrics: &self.lyrics, lines: &self.lines, focus, scroll: self.scroll };
            let caption = if self.cfg.lyrics.layout == Layout::Caption { view::caption_content(&mut tr, &frame, &self.cfg.lyrics, s) } else { (0.0, 0.0) };
            view::geometry(&self.cfg.lyrics, &self.card_cfg, s, caption, mix_geo)
        };
        let pos = if self.drag.is_some_and(|d| d.moved) {
            self.pos
        } else if let Some(r) = self.resize.filter(|r| r.moved) {
            // As bordas que não estão sendo arrastadas ficam onde estavam.
            let x = if r.edge & view::EDGE_L != 0 { r.card0.right() - geo.card.w } else { r.card0.x };
            let y = if r.edge & view::EDGE_T != 0 { r.card0.bottom() - geo.card.h } else { r.card0.y };
            ((x - geo.card.x).round() as i32, (y - geo.card.y).round() as i32)
        } else {
            self.place(&geo, &mon)
        };
        let (w, h) = (geo.w as i32, geo.h as i32);
        let tip_text = self.tip.map(|(h, _)| self.tip_text(h)).filter(|_| tip_a > 0.0);
        let toast_text = self.toast.as_ref().map(|(m, _)| m.clone());
        let change = self.change.as_ref().map(|(t0, old)| (old, (t0.elapsed().as_secs_f32() / TRACK_ANIM).min(1.0)));
        let hires = self.hires.as_ref().filter(|(t, _)| *t == self.media.title).map(|(_, i)| i.clone());
        Surface::ensure(&mut self.surface, w, h);
        let Some(surface) = self.surface.as_mut() else {
            self.tr = Some(tr);
            return;
        };
        let card_hit = |h: Option<LHit>| match h {
            Some(LHit::Card(h)) => Some(h),
            _ => None,
        };
        let it = Interact {
            hover: card_hit(self.hover),
            pressed: card_hit(self.pressed),
            drag: if self.seek.is_some() {
                Some(Hit::Seek)
            } else if self.vol_drag {
                Some(Hit::Vol)
            } else {
                self.mix_drag.as_ref().map(|(i, _)| Hit::MixVol(*i as u8))
            },
            fx: self.icons.fx(self.cfg.icon_anim),
            ..Default::default()
        };
        {
            let mut cv = surface.canvas();
            cv.clear();
            let v = View {
                card_cfg: &self.card_cfg,
                lc: &self.cfg.lyrics,
                colors: &self.colors,
                media: &self.media,
                pb,
                it,
                hover: self.hover,
                tools: ui::ease_out_cubic(self.tools),
                bg_time: Some(self.bg_clock),
                hires: hires.as_deref(),
                change,
                mixer: mixer.as_ref(),
                lyr_alpha,
                tip: tip_text.as_deref().map(|t| (t, tip_a)),
                toast: toast_text.as_deref().zip(toast_a),
                frame: LyricFrame { lyrics: &self.lyrics, lines: &self.lines, focus, scroll: self.scroll },
                s,
            };
            view::draw(&mut cv, &mut tr, &geo, &v);
        }
        surface.present_at(self.hwnd, pos.0, pos.1 as f32, 1.0);
        if self.resize.is_none() && (pos != self.pos || self.geo.w != geo.w || self.geo.h != geo.h) {
            log::info(&format!("letra: janela em {pos:?}, {w}x{h}, escala {s}"));
        }
        self.tr = Some(tr);
        self.geo = geo;
        self.pos = pos;
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn close(&mut self) {
        self.set_mixer(false);
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HK_SHOW);
            let _ = UnregisterHotKey(Some(self.hwnd), HK_LOCK);
            if let Some(h) = self.hook.take() {
                let _ = UnhookWinEvent(h);
            }
            let _ = DestroyWindow(self.guides.hwnd);
            let _ = DestroyWindow(self.hwnd);
        }
        self.surface = None;
        self.guides.surface = None;
        self.fetcher = None;
        self.pacer = None;
        HOTKEY_REFUSED.store(0, Ordering::Relaxed);
        // `reload_config` já marcou a saída (com o lock); se foi outro motivo (WM_QUIT), marca aqui.
        if WINDOW.load(Ordering::Relaxed) == self.hwnd.0 as isize {
            finish();
        }
    }
}
