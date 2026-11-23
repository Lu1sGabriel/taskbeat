//! Janelas sobre a barra de tarefas:
//! - widget compacto (capa + título/artista; botões opcionais),
//! - card expandido ao clicar (todos os controles, progresso e volume),
//! - cartão "A seguir" (só quando o player troca de faixa sozinho),
//! - aviso animado de Caps Lock / Num Lock (opcional).
//!
//! O desenho fica em `ui.rs`; aqui ficam estado, entrada, animação e janelas.
//! Janelas "layered" com alfa por pixel (UpdateLayeredWindow), topmost. O
//! widget não rouba foco; o card recebe foco para saber quando o usuário
//! clicou fora (WM_ACTIVATE). Thread própria com timer de alta resolução.

use crate::i18n::tr;
use crate::audio::log;
use crate::config::{Config, OsdPosition};
use crate::dsp::{Analyzer, FFT_SIZE};
use crate::render::{Canvas, Colors};
use crate::shared::{MediaCmd, MediaInfo, Shared};
use crate::system;
use crate::taskbar::{self, Placement};
use crate::text::TextRenderer;
use crate::ui::{self, CLayout, Hit, Interact, Playback, WLayout};
use crate::volume::Volume;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CAPITAL, VK_ESCAPE, VK_NUMLOCK,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const WM_MOUSELEAVE: u32 = 0x02A3;
const ACTIVE_PERIOD_MS: i32 = 16;
const ANIM_PERIOD_MS: i32 = 33;
/// Repouso: nada animando; só acompanha posição da barra e mudanças (4 fps).
const IDLE_PERIOD_MS: i32 = 250;
const SILENCE_HIDE_AFTER: Duration = Duration::from_millis(2500);

// ---------------------------------------------------------------- entrada

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Win {
    Widget,
    Flyout,
}

#[derive(Clone, Copy, Debug)]
enum Ev {
    Move(f32, f32),
    Leave,
    Down(f32, f32),
    Up(f32, f32),
    Wheel(i16),
    Deactivate,
    Escape,
}

thread_local! {
    static EVENTS: RefCell<Vec<(Win, Ev)>> = const { RefCell::new(Vec::new()) };
    static TRACKING: Cell<[bool; 2]> = const { Cell::new([false, false]) };
    /// Teclas de bloqueio pressionadas (código virtual), vindas do hook.
    static LOCK_KEYS: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    /// Estado "pressionada" para ignorar a repetição automática.
    static LOCK_DOWN: Cell<[bool; 2]> = const { Cell::new([false, false]) };
    /// O Windows avisou que tema/cor de destaque mudou (em vez de consultar sempre).
    static COLORS_DIRTY: Cell<bool> = const { Cell::new(false) };
}

fn push(win: Win, ev: Ev) {
    EVENTS.with(|e| e.borrow_mut().push((win, ev)));
}

fn xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32)
}

unsafe fn mouse(win: Win, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    unsafe {
        let idx = win as usize;
        match msg {
            WM_MOUSEMOVE => {
                let mut t = TRACKING.with(|t| t.get());
                if !t[idx] {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    t[idx] = true;
                    TRACKING.with(|c| c.set(t));
                }
                let (x, y) = xy(lp);
                push(win, Ev::Move(x, y));
            }
            WM_MOUSELEAVE => {
                let mut t = TRACKING.with(|t| t.get());
                t[idx] = false;
                TRACKING.with(|c| c.set(t));
                push(win, Ev::Leave);
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                SetCapture(hwnd);
                let (x, y) = xy(lp);
                push(win, Ev::Down(x, y));
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                let (x, y) = xy(lp);
                push(win, Ev::Up(x, y));
            }
            WM_MOUSEWHEEL => push(win, Ev::Wheel(((wp.0 >> 16) & 0xffff) as i16)),
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

unsafe extern "system" fn widget_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED | WM_SYSCOLORCHANGE) {
            COLORS_DIRTY.with(|c| c.set(true));
        }
        mouse(Win::Widget, hwnd, msg, wp, lp).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
    }
}

unsafe extern "system" fn flyout_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => push(Win::Flyout, Ev::Deactivate),
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => push(Win::Flyout, Ev::Escape),
            _ => {
                if let Some(r) = mouse(Win::Flyout, hwnd, msg, wp, lp) {
                    return r;
                }
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn passive_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// Hook de teclado: só olha Caps Lock e Num Lock (nada mais é lido ou guardado).
unsafe extern "system" fn keyboard_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
            let slot = match k.vkCode as u16 {
                v if v == VK_CAPITAL.0 => Some(0),
                v if v == VK_NUMLOCK.0 => Some(1),
                _ => None,
            };
            if let Some(i) = slot {
                let msg = wp.0 as u32;
                let mut down = LOCK_DOWN.with(|d| d.get());
                if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && !down[i] {
                    down[i] = true;
                    LOCK_KEYS.with(|q| q.borrow_mut().push(k.vkCode as u16));
                } else if msg == WM_KEYUP || msg == WM_SYSKEYUP {
                    down[i] = false;
                }
                LOCK_DOWN.with(|d| d.set(down));
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

fn create_window(class: PCWSTR, proc_: WNDPROC, ex_extra: WINDOW_EX_STYLE) -> windows::core::Result<HWND> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: proc_,
            hInstance: hinst,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | ex_extra;
        CreateWindowExW(ex, class, w!("Taskbeat"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst), None)
    }
}

pub fn spawn(shared: Arc<Shared>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("overlay".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            if let Err(e) = run(shared) {
                log::warn(&format!("overlay terminou com erro: {e}"));
            }
        })
        .expect("thread do overlay")
}

// ---------------------------------------------------------------- superfície

/// DIB 32 bpp top-down selecionado em um DC de memória.
pub(crate) struct Surface {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let bmi = BITMAPINFO {
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
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bmp.into());
            Some(Self { dc, bmp, old, bits: bits as *mut u32, w, h })
        }
    }

    pub fn ensure(slot: &mut Option<Surface>, w: i32, h: i32) {
        if slot.as_ref().map(|s| (s.w, s.h)) != Some((w, h)) {
            *slot = Surface::new(w.max(1), h.max(1));
        }
    }

    pub fn canvas(&mut self) -> Canvas<'_> {
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) };
        Canvas::new(self.w as usize, self.h as usize, px)
    }

    fn present(&self, hwnd: HWND, x: i32, y: i32, alpha: f32) {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0) as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let pos = POINT { x, y };
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT { x: 0, y: 0 };
        unsafe {
            let _ = UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&size), Some(self.dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Janela animada simples (card, "A seguir", aviso de teclas).
struct Layer {
    hwnd: HWND,
    surface: Option<Surface>,
    shown: bool,
}

impl Layer {
    fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
            self.surface = None;
        }
    }

    fn show(&mut self, activate: bool) {
        if !self.shown {
            unsafe {
                if activate {
                    let _ = ShowWindow(self.hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(self.hwnd);
                } else {
                    let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
            }
            self.shown = true;
        }
    }
}

/// Valor "otimista": mostra o novo estado na hora, até o player confirmar (ou 2 s).
struct Pending<T: Copy + PartialEq> {
    v: Option<(T, Instant)>,
}

impl<T: Copy + PartialEq> Pending<T> {
    fn new() -> Self {
        Self { v: None }
    }
    fn set(&mut self, v: T) {
        self.v = Some((v, Instant::now()));
    }
    fn resolve(&mut self, actual: T) -> T {
        match self.v {
            Some((v, t)) if v != actual && t.elapsed() < Duration::from_secs(2) => v,
            _ => {
                self.v = None;
                actual
            }
        }
    }
}

// ---------------------------------------------------------------- estado

struct Overlay {
    shared: Arc<Shared>,
    hwnd: HWND,
    timer: HANDLE,
    period: i32,
    surface: Option<Surface>,
    text: TextRenderer,
    analyzer: Analyzer,
    samples: Vec<f32>,
    cfg: Config,
    cfg_version: u64,
    colors: Colors,
    placement: Option<Placement>,
    panel: bool,
    viz: bool,
    volume: Volume,
    media: MediaInfo,
    seen_seq: u64,
    started: Instant,
    // card expandido
    flyout: Layer,
    flyout_open: bool,
    flyout_anim: f32,
    flyout_last_hover: Instant,
    // "A seguir"
    popup: Layer,
    popup_start: Option<Instant>,
    // Caps/Num Lock
    osd: Layer,
    osd_start: Option<Instant>,
    osd_key: (&'static str, &'static str, bool),
    lock_state: [bool; 2],
    hook: Option<HHOOK>,
    // entrada
    hover_w: Option<Hit>,
    hover_f: Option<Hit>,
    pressed: Option<(Win, Hit)>,
    drag: Option<Hit>,
    seek_preview: Option<f32>,
    last_input: Instant,
    play_pending: Pending<bool>,
    shuffle_pending: Pending<bool>,
    repeat_pending: Pending<u8>,
    seek_pending: Option<(f64, Instant)>,
    // áudio / visibilidade
    last_written: u64,
    last_audio: Instant,
    last_sound: Instant,
    last_layout: Instant,
    last_colors: Instant,
    last_frame: Instant,
    fade: f32,
    shown: bool,
    demo_popup: bool,
    demo_flyout: bool,
    demo_osd: bool,
    media_version: u64,
    /// Assinatura do último quadro desenhado do widget (pula se nada mudou).
    last_sig: Option<WidgetSig>,
    trimmed: bool,
}

/// Tudo que muda a aparência do widget quando nada está animando.
#[derive(Clone, Copy, PartialEq, Debug)]
struct WidgetSig {
    media: u64,
    cfg: u64,
    colors: u64,
    place: (i32, i32, i32, i32),
    hover: Option<Hit>,
    pressed: Option<(Win, Hit)>,
    fade: u16,
    flyout_open: bool,
    playing: bool,
    progress_px: i32,
}

fn run(shared: Arc<Shared>) -> windows::core::Result<()> {
    let hwnd = create_window(w!("TaskbeatWidget"), Some(widget_proc), WS_EX_NOACTIVATE)?;
    let popup = create_window(w!("TaskbeatUpNext"), Some(passive_proc), WS_EX_NOACTIVATE | WS_EX_TRANSPARENT)?;
    let osd = create_window(w!("TaskbeatLockKeys"), Some(passive_proc), WS_EX_NOACTIVATE | WS_EX_TRANSPARENT)?;
    let flyout = create_window(w!("TaskbeatFlyout"), Some(flyout_proc), WINDOW_EX_STYLE(0))?;
    let timer = unsafe {
        CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
            .or_else(|_| CreateWaitableTimerExW(None, PCWSTR::null(), 0, TIMER_ALL_ACCESS.0))?
    };
    let cfg = shared.config();
    let rate = shared.sample_rate.load(Ordering::Relaxed);
    let media = shared.media.lock().unwrap().clone();
    let now = Instant::now();
    let lock_state = unsafe { [GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0, GetKeyState(VK_NUMLOCK.0 as i32) & 1 != 0] };
    let mut ov = Overlay {
        hwnd,
        timer,
        period: 0,
        surface: None,
        text: TextRenderer::new(),
        analyzer: Analyzer::new(cfg.bars, rate),
        samples: vec![0.0; FFT_SIZE],
        colors: system::colors(cfg.palette),
        cfg_version: shared.config_version.load(Ordering::Relaxed),
        cfg,
        placement: None,
        panel: false,
        viz: false,
        volume: Volume::new(),
        seen_seq: media.auto_change_seq,
        media,
        started: now,
        flyout: Layer { hwnd: flyout, surface: None, shown: false },
        flyout_open: false,
        flyout_anim: 0.0,
        flyout_last_hover: now,
        popup: Layer { hwnd: popup, surface: None, shown: false },
        popup_start: None,
        osd: Layer { hwnd: osd, surface: None, shown: false },
        osd_start: None,
        osd_key: (tr("Caps Lock"), "A", false),
        lock_state,
        hook: None,
        hover_w: None,
        hover_f: None,
        pressed: None,
        drag: None,
        seek_preview: None,
        last_input: now - Duration::from_secs(10),
        play_pending: Pending::new(),
        shuffle_pending: Pending::new(),
        repeat_pending: Pending::new(),
        seek_pending: None,
        last_written: 0,
        last_audio: now,
        last_sound: now - SILENCE_HIDE_AFTER,
        last_layout: now - Duration::from_secs(10),
        last_colors: now,
        last_frame: now,
        fade: 0.0,
        shown: false,
        demo_popup: std::env::var_os("TV_DEMO_POPUP").is_some(),
        demo_flyout: std::env::var_os("TV_DEMO_FLYOUT").is_some(),
        demo_osd: std::env::var_os("TV_DEMO_CAPS").is_some(),
        media_version: u64::MAX,
        last_sig: None,
        trimmed: false,
        shared,
    };
    ov.set_period(ACTIVE_PERIOD_MS);
    ov.update_hook();

    let mut msg = MSG::default();
    'outer: loop {
        let r = unsafe { MsgWaitForMultipleObjects(Some(&[ov.timer]), false, 1000, QS_ALLINPUT) };
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !ov.shared.is_running() {
            break;
        }
        let has_input = EVENTS.with(|e| !e.borrow().is_empty()) || LOCK_KEYS.with(|q| !q.borrow().is_empty());
        if r == WAIT_OBJECT_0 || r == WAIT_TIMEOUT || has_input {
            ov.frame();
        }
    }

    unsafe {
        if let Some(h) = ov.hook.take() {
            let _ = UnhookWindowsHookEx(h);
        }
        let _ = CancelWaitableTimer(ov.timer);
        let _ = CloseHandle(ov.timer);
        ov.surface = None;
        for l in [&mut ov.flyout, &mut ov.popup, &mut ov.osd] {
            l.surface = None;
            let _ = DestroyWindow(l.hwnd);
        }
        let _ = DestroyWindow(ov.hwnd);
    }
    Ok(())
}

impl Overlay {
    fn set_period(&mut self, ms: i32) {
        if self.period == ms {
            return;
        }
        self.period = ms;
        let due: i64 = -(ms as i64) * 10_000; // relativo, em unidades de 100 ns
        unsafe {
            let _ = SetWaitableTimer(self.timer, &due, ms, None, None, false);
        }
    }

    /// Instala/remove o hook de teclado conforme as opções de Caps/Num Lock.
    fn update_hook(&mut self) {
        let want = self.cfg.caps_lock_osd || self.cfg.num_lock_osd;
        unsafe {
            if want && self.hook.is_none() {
                let hinst = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
                self.hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinst, 0).ok();
                self.lock_state = [GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0, GetKeyState(VK_NUMLOCK.0 as i32) & 1 != 0];
            } else if !want {
                if let Some(h) = self.hook.take() {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
        }
    }

    fn scale(&self) -> f32 {
        self.placement.map(|p| p.scale).unwrap_or(1.0)
    }

    fn playback(&mut self) -> Playback {
        let m = self.media.clone();
        let mut pos = m.position_now();
        if let Some((p, t)) = self.seek_pending {
            if t.elapsed() < Duration::from_millis(1500) {
                pos = p + if m.playing { t.elapsed().as_secs_f64() } else { 0.0 };
            } else {
                self.seek_pending = None;
            }
        }
        Playback {
            playing: self.play_pending.resolve(m.playing),
            shuffle: self.shuffle_pending.resolve(m.shuffle),
            repeat: self.repeat_pending.resolve(m.repeat),
            pos,
            seek_preview: self.seek_preview,
            volume: self.volume.level,
            muted: self.volume.muted,
        }
    }

    fn card_layout(&self) -> CLayout {
        CLayout::build(self.scale(), &self.cfg, self.media.dur > 0.0)
    }

    fn widget_layout(&self) -> Option<WLayout> {
        let p = self.placement?;
        Some(WLayout::build(p.scale, p.h as f32, &self.cfg, self.panel, self.viz))
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.001, 0.1);
        self.last_frame = now;

        let ver = self.shared.config_version.load(Ordering::Relaxed);
        if ver != self.cfg_version {
            self.cfg_version = ver;
            self.cfg = self.shared.config();
            self.colors = system::colors(self.cfg.palette);
            self.last_layout = now - Duration::from_secs(10);
            self.update_hook();
        }
        if COLORS_DIRTY.with(|c| c.replace(false)) || now - self.last_colors > Duration::from_secs(30) {
            self.last_colors = now;
            self.colors = system::colors(self.cfg.palette);
            self.last_sig = None;
        }

        // Mídia: só copia quando a thread de mídia publicou algo novo.
        let mv = self.shared.media_version.load(Ordering::Relaxed);
        if mv != self.media_version {
            self.media_version = mv;
            self.media = self.shared.media.lock().unwrap().clone();
        }
        self.colors.album = self.media.art.as_ref().map(|a| a.vibrant);
        let visible = self.placement.is_some_and(|p| p.visible);

        // Cartão "A seguir": só troca automática.
        if self.media.auto_change_seq != self.seen_seq {
            self.seen_seq = self.media.auto_change_seq;
            if self.cfg.next_track_popup && self.media.present() && visible && !self.flyout_open {
                self.popup_start = Some(now);
            }
        }
        if self.demo_popup && self.media.present() && visible {
            self.demo_popup = false;
            self.popup_start = Some(now);
        }
        if self.demo_flyout && self.media.present() && visible {
            self.demo_flyout = false;
            self.open_flyout(now);
        }
        if self.demo_osd && visible {
            self.demo_osd = false;
            self.osd_key = (tr("Caps Lock"), "A", true);
            self.osd_start = Some(now);
        }
        self.process_lock_keys(now);
        if self.shared.osd_test.swap(false, Ordering::Relaxed) {
            // Já visível (ex.: arrastando o ajuste de posição): mantém aberto sem
            // reiniciar a animação de entrada.
            let shown = self.osd_start.is_some_and(|t| (now - t).as_secs_f32() > 0.3);
            self.osd_key = (tr("Caps Lock"), "A", self.lock_state[0]);
            self.osd_start = Some(if shown { now - Duration::from_millis(350) } else { now });
        }

        // Quais partes aparecem e onde.
        let horizontal = self.placement.map(|p| p.horizontal).unwrap_or(true);
        let panel = self.cfg.show_media && self.media.present() && horizontal;
        let viz = self.cfg.show_visualizer;
        let layout_every = Duration::from_millis(if self.period == IDLE_PERIOD_MS { 500 } else { 250 });
        if panel != self.panel || viz != self.viz || now - self.last_layout > layout_every {
            self.panel = panel;
            self.viz = viz;
            self.last_layout = now;
            let logical = WLayout::build(1.0, 40.0, &self.cfg, panel, viz).width.ceil() as i32;
            self.placement = taskbar::compute(&self.cfg, logical.max(1));
        }

        // Áudio -> espectro (só com o visualizador ligado).
        if viz {
            let rate = self.shared.sample_rate.load(Ordering::Relaxed);
            self.analyzer.configure(self.cfg.bars, rate);
            {
                let ring = self.shared.audio.lock().unwrap();
                if ring.written != self.last_written {
                    self.last_written = ring.written;
                    self.last_audio = now;
                    ring.latest(&mut self.samples);
                }
            }
            let media_ok = !self.cfg.only_when_media || self.media.playing;
            if now - self.last_audio > Duration::from_millis(150) || !media_ok {
                self.samples.fill(0.0);
            }
            self.analyzer.update(&self.samples, dt, self.cfg.sensitivity_db);
            if self.analyzer.energy() > 0.03 {
                self.last_sound = now;
            }
        }
        let sound = now - self.last_sound < SILENCE_HIDE_AFTER;

        if self.flyout_open {
            self.volume.poll();
        }
        let had_input = self.process_input(now);

        let placement = self.placement.filter(|p| p.visible);
        let want = placement.is_some() && (panel || (viz && sound));
        let target = if want { 1.0 } else { 0.0 };
        let speed = if want { 6.0 } else { 2.5 };
        self.fade += (target - self.fade).clamp(-speed * dt, speed * dt);

        let idle_close = Duration::from_secs(self.cfg.card_autoclose.max(2) as u64);
        if self.flyout_open && (!want || (self.hover_f.is_none() && self.drag.is_none() && now - self.flyout_last_hover > idle_close)) {
            self.flyout_open = false;
        }
        let pb = self.playback();
        let flyout_active = self.update_flyout(dt, placement, pb);
        let popup_active = self.update_popup(now, placement);
        let osd_active = self.update_osd(now, placement);

        // Ritmo: 60 fps quando há interação/animação forte, 30 fps para o
        // letreiro e o indicador, 10 fps em repouso.
        let busy = (viz && sound)
            || self.hover_w.is_some()
            || self.drag.is_some()
            || flyout_active
            || popup_active
            || osd_active
            || had_input
            || (self.fade > 0.0 && self.fade < 1.0)
            || now - self.last_input < Duration::from_millis(500);
        let decorative = panel && pb.playing && (self.cfg.playing_indicator || self.cfg.marquee || self.cfg.taskbar_progress);
        self.set_period(if busy {
            ACTIVE_PERIOD_MS
        } else if decorative {
            ANIM_PERIOD_MS
        } else {
            IDLE_PERIOD_MS
        });

        let Some(p) = placement.filter(|_| self.fade > 0.001) else {
            if self.shown {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                self.shown = false;
                self.hover_w = None;
            }
            self.fade = 0.0;
            return;
        };

        // Nada animando e nada mudou desde o último quadro: não redesenha.
        let animating = (viz && sound) || decorative || (self.fade > 0.0 && self.fade < 1.0);
        let sig = WidgetSig {
            media: self.media_version,
            cfg: self.cfg_version,
            colors: self.last_colors.elapsed().as_secs() / 30,
            place: (p.x, p.y, p.w, p.h),
            hover: self.hover_w,
            pressed: self.pressed,
            fade: (self.fade * 255.0) as u16,
            flyout_open: self.flyout_open,
            playing: pb.playing,
            progress_px: if self.cfg.taskbar_progress && self.media.dur > 0.0 { (pb.pos / self.media.dur * 400.0) as i32 } else { 0 },
        };
        if !animating && self.shown && self.last_sig == Some(sig) {
            self.keep_on_top();
            self.maybe_trim(now);
            return;
        }
        self.last_sig = Some(sig);

        self.text.set_scale(p.scale);
        Surface::ensure(&mut self.surface, p.w, p.h);
        let Some(mut surface) = self.surface.take() else { return };
        let layout = WLayout::build(p.scale, p.h as f32, &self.cfg, panel, viz);
        {
            let mut cv = surface.canvas();
            cv.fill_raw(0x0100_0000); // alfa 1/255: invisível, mas recebe cliques
            let theme = ui::widget_theme(&self.cfg, &self.colors, &self.media);
            let pressed = self.pressed.filter(|(w, _)| *w == Win::Widget).map(|(_, h)| h);
            let f = ui::WidgetFrame {
                cfg: &self.cfg,
                media: &self.media,
                theme,
                colors: &self.colors,
                it: Interact { hover: self.hover_w, pressed, drag: None },
                pb,
                time: (now - self.started).as_secs_f32(),
                viz: Some((&self.analyzer.values, &self.analyzer.peaks)),
                flyout_open: self.flyout_open,
                s: p.scale,
            };
            ui::draw_widget(&mut cv, &mut self.text, &layout, &f);
        }
        let fade = self.fade.clamp(0.0, 1.0);
        surface.present(self.hwnd, p.x, p.y, fade * fade * (3.0 - 2.0 * fade));
        self.surface = Some(surface);

        if !self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.shown = true;
        }
        self.keep_on_top();
        self.maybe_trim(now);
    }

    /// Se o usuário clicou na barra, ela subiu na ordem Z: voltamos para cima dela.
    fn keep_on_top(&self) {
        unsafe {
            if let Some(tray) = taskbar::taskbar_hwnd() {
                if taskbar::taskbar_is_above(self.hwnd, tray) {
                    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
                    let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
                }
            }
        }
    }

    /// Uma vez, depois que tudo carregou: devolve a memória usada só na inicialização.
    fn maybe_trim(&mut self, now: Instant) {
        if !self.trimmed && now - self.started > Duration::from_secs(12) {
            self.trimmed = true;
            system::trim_memory();
        }
    }

    // ------------------------------------------------------------ teclas de bloqueio

    fn process_lock_keys(&mut self, now: Instant) {
        let keys: Vec<u16> = LOCK_KEYS.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for vk in keys {
            let (i, name, sym, enabled) = if vk == VK_CAPITAL.0 {
                (0, "Caps Lock", "A", self.cfg.caps_lock_osd)
            } else {
                (1, tr("Num Lock"), "1", self.cfg.num_lock_osd)
            };
            // A tecla alterna o estado; o hook vê a tecla antes do Windows aplicar.
            self.lock_state[i] = !self.lock_state[i];
            let show_state = self.lock_state[i] || self.cfg.osd_show_off;
            if enabled && show_state && self.placement.is_some_and(|p| p.visible) {
                self.osd_key = (name, sym, self.lock_state[i]);
                self.osd_start = Some(now);
            }
        }
    }

    fn update_osd(&mut self, now: Instant, placement: Option<Placement>) -> bool {
        let Some(start) = self.osd_start else { return false };
        let t = (now - start).as_secs_f32();
        let secs = self.cfg.osd_duration as f32 / 10.0;
        let Some(p) = placement.filter(|_| t < secs) else {
            self.osd_start = None;
            self.osd.hide();
            return false;
        };
        let s = p.scale;
        let (w, h) = ui::osd_size(s, &self.cfg);
        let e = if t < 0.25 {
            ui::ease_out_cubic(t / 0.25)
        } else if t > secs - 0.3 {
            1.0 - ui::ease_out_cubic((t - (secs - 0.3)) / 0.3)
        } else {
            1.0
        };
        let m = ui::SHADOW_MARGIN * s;
        let slide = ((1.0 - e) * 14.0 * s) as i32;
        let gap = (12.0 * s) as i32;
        let (wi, hi, mi) = (w as i32, h as i32, m as i32);
        let center_x = (p.mon_left + p.mon_right) / 2 - wi / 2;
        let (x, y) = match self.cfg.osd_position {
            OsdPosition::Bottom if p.taskbar_bottom_edge => (center_x, p.taskbar_top - hi + mi - (20.0 * s) as i32 + slide),
            OsdPosition::Bottom => (center_x, p.mon_bottom - hi + mi - (60.0 * s) as i32 + slide),
            OsdPosition::Center => (center_x, (p.mon_top + p.mon_bottom) / 2 - hi / 2 + slide),
            OsdPosition::Top => (center_x, p.mon_top + (40.0 * s) as i32 - mi - slide),
            OsdPosition::AbovePlayer => {
                let x = p.x + p.w / 2 - wi / 2;
                let y = if p.taskbar_bottom_edge { p.taskbar_top - hi + mi - gap + slide } else { p.y + p.h + gap - mi - slide };
                (x, y)
            }
        };
        // Ajuste fino do usuário; nunca sai do monitor.
        let x = (x + (self.cfg.osd_offset_x as f32 * s) as i32).clamp(p.mon_left - mi, p.mon_right - wi + mi);
        let y = (y + (self.cfg.osd_offset_y as f32 * s) as i32).clamp(p.mon_top - mi, p.mon_bottom - hi + mi);

        self.text.set_scale(s);
        Surface::ensure(&mut self.osd.surface, w as i32, h as i32);
        let Some(mut surf) = self.osd.surface.take() else { return true };
        {
            let mut cv = surf.canvas();
            cv.clear();
            let th = ui::osd_theme(&self.cfg, &self.colors);
            let (name, sym, on) = self.osd_key;
            ui::draw_osd(&mut cv, &mut self.text, &self.cfg, &th, s, name, sym, on, t);
        }
        surf.present(self.osd.hwnd, x, y, e);
        self.osd.surface = Some(surf);
        self.osd.show(false);
        true
    }

    // ------------------------------------------------------------ entrada

    fn open_flyout(&mut self, now: Instant) {
        self.flyout_open = true;
        self.flyout_last_hover = now;
        self.popup_start = None;
    }

    fn frac(x: f32, track: Option<(f32, f32)>) -> Option<f32> {
        let (x0, x1) = track?;
        Some(((x - x0) / (x1 - x0)).clamp(0.0, 1.0))
    }

    fn process_input(&mut self, now: Instant) -> bool {
        let events: Vec<(Win, Ev)> = EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut()));
        if events.is_empty() {
            return false;
        }
        self.last_input = now;
        let s = self.scale();
        let wl = self.widget_layout().unwrap_or_default();
        let cl = self.card_layout();
        for (win, ev) in events {
            let hit_at = |x: f32, y: f32| match win {
                Win::Widget => wl.hit(x, y),
                Win::Flyout => cl.hit(x, y),
            };
            match ev {
                Ev::Move(x, y) => {
                    if win == Win::Flyout {
                        self.flyout_last_hover = now;
                    }
                    match self.drag {
                        Some(Hit::Vol) => {
                            if let Some(v) = Self::frac(x, cl.vol_track(s)) {
                                self.volume.set(v);
                            }
                        }
                        Some(Hit::Seek) => self.seek_preview = Self::frac(x, cl.seek_track(s)),
                        _ => match win {
                            Win::Widget => self.hover_w = hit_at(x, y),
                            Win::Flyout => self.hover_f = hit_at(x, y).or(Some(Hit::Info)),
                        },
                    }
                }
                Ev::Leave => {
                    if self.drag.is_none() {
                        match win {
                            Win::Widget => self.hover_w = None,
                            Win::Flyout => self.hover_f = None,
                        }
                    }
                }
                Ev::Down(x, y) => {
                    let h = hit_at(x, y);
                    self.pressed = h.map(|h| (win, h));
                    if win == Win::Flyout {
                        match h {
                            Some(Hit::Vol) => {
                                self.drag = Some(Hit::Vol);
                                if let Some(v) = Self::frac(x, cl.vol_track(s)) {
                                    self.volume.set(v);
                                }
                            }
                            Some(Hit::Seek) if self.media.can_seek => {
                                self.drag = Some(Hit::Seek);
                                self.seek_preview = Self::frac(x, cl.seek_track(s));
                            }
                            _ => {}
                        }
                    }
                }
                Ev::Up(x, y) => {
                    let up = hit_at(x, y);
                    match self.drag.take() {
                        Some(Hit::Seek) => {
                            if let Some(f) = self.seek_preview.take() {
                                let secs = f as f64 * self.media.dur;
                                self.seek_pending = Some((secs, now));
                                self.shared.send_media(MediaCmd::Seek(secs));
                            }
                        }
                        Some(_) => {}
                        None => {
                            if let (Some(u), Some((pw, ph))) = (up, self.pressed) {
                                if pw == win && ph == u {
                                    self.click(win, u, now);
                                }
                            }
                        }
                    }
                    self.pressed = None;
                }
                Ev::Wheel(d) => self.volume.step(d as f32 / 120.0 * 0.02),
                Ev::Deactivate | Ev::Escape => {
                    if self.drag.is_none() {
                        self.flyout_open = false;
                    }
                }
            }
        }
        true
    }

    fn click(&mut self, win: Win, hit: Hit, now: Instant) {
        let m = self.media.clone();
        match hit {
            Hit::Info if win == Win::Widget => {
                if self.flyout_open {
                    self.flyout_open = false;
                } else {
                    self.open_flyout(now);
                }
            }
            Hit::Prev if m.can_prev => self.shared.send_media(MediaCmd::Previous),
            Hit::Next if m.can_next => self.shared.send_media(MediaCmd::Next),
            Hit::Play if m.can_toggle => {
                let playing = self.play_pending.resolve(m.playing);
                self.play_pending.set(!playing);
                self.shared.send_media(MediaCmd::PlayPause);
            }
            Hit::Shuffle if m.can_shuffle => {
                let on = !self.shuffle_pending.resolve(m.shuffle);
                self.shuffle_pending.set(on);
                self.shared.send_media(MediaCmd::Shuffle(on));
            }
            Hit::Repeat if m.can_repeat => {
                // Ciclo igual ao Spotify: desligado -> playlist -> música -> desligado.
                let next = match self.repeat_pending.resolve(m.repeat) {
                    0 => 2,
                    2 => 1,
                    _ => 0,
                };
                self.repeat_pending.set(next);
                self.shared.send_media(MediaCmd::Repeat(next));
            }
            Hit::VolBtn => self.volume.toggle_mute(),
            _ => {}
        }
    }

    // ------------------------------------------------------------ card expandido

    fn update_flyout(&mut self, dt: f32, placement: Option<Placement>, pb: Playback) -> bool {
        let target = if self.flyout_open && placement.is_some() { 1.0 } else { 0.0 };
        let speed = if target > 0.5 { 1.0 / 0.22 } else { 1.0 / 0.14 };
        self.flyout_anim += (target - self.flyout_anim).clamp(-speed * dt, speed * dt);
        let Some(p) = placement.filter(|_| self.flyout_anim > 0.001) else {
            self.flyout.hide();
            self.hover_f = None;
            self.flyout_anim = 0.0;
            self.flyout_open = false;
            return false;
        };
        let s = p.scale;
        let l = self.card_layout();
        let m = ui::SHADOW_MARGIN * s;
        let e = ui::ease_out_cubic(self.flyout_anim);
        let wl = WLayout::build(s, p.h as f32, &self.cfg, self.panel, self.viz);
        let anchor = p.x + wl.art.map(|a| a.x as i32).unwrap_or(0) - m as i32 - (6.0 * s) as i32;
        let x = anchor.clamp(p.mon_left - m as i32 + (8.0 * s) as i32, p.mon_right - l.w as i32 + m as i32 - (8.0 * s) as i32);
        let slide = ((1.0 - e) * 14.0 * s) as i32;
        let y = if p.taskbar_bottom_edge {
            p.taskbar_top - (l.h - m) as i32 - (10.0 * s) as i32 + slide
        } else {
            p.y + p.h + (10.0 * s) as i32 - m as i32 - slide
        };

        self.text.set_scale(s);
        Surface::ensure(&mut self.flyout.surface, l.w.round() as i32, l.h.round() as i32);
        let Some(mut surf) = self.flyout.surface.take() else { return true };
        {
            let mut cv = surf.canvas();
            cv.clear();
            let pressed = self.pressed.filter(|(w, _)| *w == Win::Flyout).map(|(_, h)| h);
            let f = ui::CardFrame {
                cfg: &self.cfg,
                colors: &self.colors,
                media: &self.media,
                theme: ui::card_theme(&self.cfg, &self.colors, &self.media),
                it: Interact { hover: self.hover_f, pressed, drag: self.drag },
                pb,
                s,
            };
            ui::draw_card(&mut cv, &mut self.text, &l, &f);
        }
        surf.present(self.flyout.hwnd, x, y, e);
        self.flyout.surface = Some(surf);
        self.flyout.show(true);
        true
    }

    // ------------------------------------------------------------ "A seguir"

    fn update_popup(&mut self, now: Instant, placement: Option<Placement>) -> bool {
        let Some(start) = self.popup_start else { return false };
        let total = self.cfg.popup_secs.max(2) as f32 + 0.5;
        let t = (now - start).as_secs_f32();
        let Some(p) = placement.filter(|_| t < total) else {
            self.popup_start = None;
            self.popup.hide();
            return false;
        };
        let s = p.scale;
        let (w, h) = ui::upnext_size(s);
        let m = ui::SHADOW_MARGIN * s;
        let e = if t < 0.45 {
            ui::ease_out_cubic(t / 0.45)
        } else if t > total - 0.4 {
            1.0 - ui::ease_out_cubic((t - (total - 0.4)) / 0.4)
        } else {
            1.0
        };
        let wl = WLayout::build(s, p.h as f32, &self.cfg, self.panel, self.viz);
        let anchor = p.x + wl.art.map(|a| a.x as i32).unwrap_or(0) - m as i32 - (6.0 * s) as i32;
        let x = anchor.clamp(p.mon_left - m as i32 + (8.0 * s) as i32, p.mon_right - w as i32 + m as i32 - (8.0 * s) as i32);
        let slide = ((1.0 - e) * 18.0 * s) as i32;
        let y = if p.taskbar_bottom_edge {
            p.taskbar_top - (h - m) as i32 - (10.0 * s) as i32 + slide
        } else {
            p.y + p.h + (10.0 * s) as i32 - m as i32 - slide
        };

        self.text.set_scale(s);
        Surface::ensure(&mut self.popup.surface, w as i32, h as i32);
        let Some(mut surf) = self.popup.surface.take() else { return true };
        {
            let mut cv = surf.canvas();
            cv.clear();
            let th = ui::card_theme(&self.cfg, &self.colors, &self.media);
            ui::draw_upnext(&mut cv, &mut self.text, &self.cfg, &self.colors, &self.media, &th, s, t, total);
        }
        surf.present(self.popup.hwnd, x, y, e);
        self.popup.surface = Some(surf);
        self.popup.show(false);
        true
    }
}
