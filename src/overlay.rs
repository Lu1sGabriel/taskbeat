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
use crate::config::{Config, OsdAnim, OsdPosition};
use crate::dsp::{Analyzer, FFT_SIZE};
use crate::render::{self, Canvas, Colors, Image, RectF};
use crate::shared::{Launcher, MediaCmd, MediaInfo, MixCmd, Shared, WebCmd};
use crate::system;
use crate::taskbar::{self, Placement};
use crate::text::TextRenderer;
use crate::ui::{self, CLayout, Hit, Interact, Playback, WLayout};
use crate::volume::Volume;
use crate::vsync::{Pacer, Target};
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
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
/// Com o ritmo do monitor ligado, o timer só serve de reserva (e para a
/// checagem periódica da posição da barra).
const FALLBACK_PERIOD_MS: i32 = 200;
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
    /// (x, y) na janela e x na tela, na hora da mensagem.
    Move(f32, f32, i32),
    Leave,
    Down(f32, f32, i32),
    Up(f32, f32, i32),
    Wheel(i16),
    Deactivate,
    Escape,
    /// Outra janela tomou a captura do mouse no meio de um arrasto (Alt+Tab,
    /// um aviso do sistema): sem o "soltar", o arrasto ficaria preso.
    CaptureLost,
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

/// x do mouse na tela quando a mensagem atual foi gerada.
fn screen_x() -> i32 {
    (unsafe { GetMessagePos() } & 0xffff) as u16 as i16 as i32
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
                push(win, Ev::Move(x, y, screen_x()));
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
                push(win, Ev::Down(x, y, screen_x()));
            }
            WM_LBUTTONUP => {
                // O "soltar" vai antes: soltar a captura manda WM_CAPTURECHANGED
                // na hora, e ele não pode cancelar um arrasto que terminou bem.
                let (x, y) = xy(lp);
                push(win, Ev::Up(x, y, screen_x()));
                let _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => push(win, Ev::CaptureLost),
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

    pub fn pixels(&self) -> &[u32] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h) as usize) }
    }

    /// Desloca o conteúdo `frac` (0..1) de pixel para baixo, misturando linhas
    /// vizinhas: o cartão desliza em subpixel em vez de andar aos saltos de 1 px.
    fn shift_down(&mut self, frac: f32) {
        let k = (frac.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        if k == 0 {
            return;
        }
        let (w, h) = (self.w as usize, self.h as usize);
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, w * h) };
        for y in (1..h).rev() {
            let (top, bot) = px.split_at_mut(y * w);
            let above = &top[(y - 1) * w..];
            for (d, &a) in bot[..w].iter_mut().zip(above) {
                if *d != 0 || a != 0 {
                    *d = render::lerp_px(*d, a, k);
                }
            }
        }
        for d in &mut px[..w] {
            *d = render::scale_px(*d, 255 - k);
        }
    }

    /// Mostra na posição (x, y) com y fracionário (desliza em subpixel).
    pub(crate) fn present_at(&mut self, hwnd: HWND, x: i32, y: f32, alpha: f32) {
        let yi = y.floor();
        self.shift_down(y - yi);
        self.present(hwnd, x, yi as i32, alpha);
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

/// Fundo acrílico de uma janela: o que está atrás do cartão, já desfocado.
struct Backdrop {
    /// Retângulo do cartão na tela (x, y, w, h) e desfoque usados na captura.
    key: (i32, i32, i32, i32, i32),
    img: Option<Arc<Image>>,
}

/// Captura a tela em volta de `rect` (com folga para o desfoque) e monta o
/// fundo acrílico. Feita com a nossa janela escondida, então só pega o que está
/// atrás dela.
pub(crate) fn capture_backdrop((x, y, w, h): (i32, i32, i32, i32), blur: f32) -> Option<Image> {
    let margin = (blur * 1.2).ceil() as i32 + 2;
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let (x0, y0) = ((x - margin).max(vx), (y - margin).max(vy));
    let (x1, y1) = ((x + w + margin).min(vx + vw), (y + h + margin).min(vy + vh));
    if x1 - x0 < 2 || y1 - y0 < 2 {
        return None;
    }
    let surf = Surface::new(x1 - x0, y1 - y0)?;
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let ok = BitBlt(surf.dc, 0, 0, x1 - x0, y1 - y0, Some(screen), x0, y0, SRCCOPY).is_ok();
        ReleaseDC(None, screen);
        let _ = GdiFlush();
        if !ok {
            return None;
        }
    }
    let rect = RectF::new((x - x0) as f32, (y - y0) as f32, w as f32, h as f32);
    Some(render::acrylic_backdrop(surf.pixels(), surf.w as usize, surf.h as usize, rect, blur))
}

/// Janela animada simples (card, "A seguir", aviso de teclas).
struct Layer {
    hwnd: HWND,
    surface: Option<Surface>,
    shown: bool,
    backdrop: Option<Backdrop>,
    /// Desde quando o cartão está num lugar diferente do fundo capturado.
    moved_at: Option<Instant>,
}

impl Layer {
    fn new(hwnd: HWND) -> Self {
        Self { hwnd, surface: None, shown: false, backdrop: None, moved_at: None }
    }

    fn hide(&mut self) {
        if self.shown {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
            self.surface = None;
            self.backdrop = None;
        }
    }

    /// Fundo acrílico para o cartão em `rect` (tela). Capturado antes de a
    /// janela aparecer. Se o cartão mudar de lugar com a janela visível (ajuste
    /// ao vivo nas configurações), espera parar de mexer e, se `may_hide`,
    /// esconde a janela por um quadro para capturar de novo.
    fn backdrop(&mut self, rect: (i32, i32, i32, i32), blur: f32, now: Instant, may_hide: bool) -> Option<Arc<Image>> {
        let key = (rect.0, rect.1, rect.2, rect.3, blur.round() as i32);
        let old = self.backdrop.as_ref().and_then(|b| b.img.clone());
        if self.backdrop.as_ref().is_some_and(|b| b.key == key) {
            self.moved_at = None;
            return old;
        }
        if self.shown {
            let since = *self.moved_at.get_or_insert(now);
            if !may_hide || now - since < Duration::from_millis(250) {
                return old;
            }
            self.hide();
            unsafe {
                let _ = DwmFlush();
            }
        }
        self.moved_at = None;
        let img = capture_backdrop(rect, blur).map(Arc::new);
        self.backdrop = Some(Backdrop { key, img: img.clone() });
        img
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
    /// Barra de tarefas que é "dona" do widget (ver `attach_to_taskbar`).
    owner: Option<HWND>,
    timer: HANDLE,
    period: i32,
    /// Quadros no ritmo do monitor durante animações (None: só o timer).
    pacer: Option<Pacer>,
    /// Animação leve (letreiro, indicador): desenha a cada 2 quadros do monitor.
    half_rate: bool,
    pacer_skip: bool,
    last_pacer_frame: Instant,
    monitor: isize,
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
    /// Superfície do card na posição final (x, y, largura, altura), quando visível.
    flyout_box: Option<(i32, i32, i32, i32)>,
    /// Aviso rápido no card ("o player não deixa", "fila embaralhada") e desde quando.
    hint: Option<(Instant, &'static str)>,
    /// Último estado registrado de visibilidade (barra visível, painel, música).
    vis_log: Option<(Option<bool>, bool, bool)>,
    // "A seguir"
    popup: Layer,
    popup_start: Option<Instant>,
    /// Troca automática detectada; esperando a capa nova chegar (no máximo ~1,5 s).
    popup_wait: Option<Instant>,
    /// Posição final (x, y) e sentido da entrada, fixados quando o cartão aparece.
    popup_pos: Option<(i32, i32, i32)>,
    // Caps/Num Lock
    osd: Layer,
    osd_start: Option<Instant>,
    /// (nome da tecla, é Num Lock, ligada)
    osd_key: (&'static str, bool, bool),
    /// Buffer reaproveitado para a animação de zoom do aviso.
    osd_tmp: Vec<u32>,
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
    /// Assinatura do último quadro desenhado do card (pula se nada mudou).
    flyout_sig: Option<FlyoutSig>,
    /// Troca de música no widget: música anterior e quando o texto/a capa mudaram.
    prev_media: MediaInfo,
    text_change: Option<Instant>,
    art_change: Option<Instant>,
    trimmed: bool,
    /// Animação dos ícones dos botões (widget e card).
    icons: ui::IconAnims,
    /// Atalhos dos apps de música à mostra (ordem e escolhas do usuário).
    apps: Arc<Vec<Launcher>>,
    /// Atalho do card apertado: (índice, x) — vira arrasto se o mouse andar.
    launch_press: Option<(u8, f32)>,
    /// Posição x do mouse arrastando um atalho.
    drag_x: f32,
    /// Widget apertado/arrastado pela barra.
    wdrag: Option<WidgetDrag>,
    /// A sessão não tem aleatório/repetir pelo Windows (navegador): os botões
    /// da página do player podem servir. E a última música para a qual eles
    /// foram procurados.
    web_needed: bool,
    web_probed: Option<(String, String)>,
    /// Mixer de volume do card: aberto (alvo) e quanto já abriu (0..1).
    mixer_open: bool,
    mixer_anim: f32,
    /// O que o usuário acabou de mudar no mixer, por app: vale por cima do que
    /// a thread do mixer publica até ela alcançar. Sem isso, arrastando, uma
    /// leitura de um instante antes voltava o slider ("vai e volta").
    mix_hold: Vec<MixHold>,
    /// App cujo slider do mixer está sendo arrastado.
    mix_drag: Option<String>,
    /// Barras como aparecem agora: sobem e descem suaves até o volume (e
    /// enchem ao abrir, esvaziam ao fechar).
    vol_fill: f32,
    mix_fill: Vec<(String, f32)>,
    /// Dica do botão sob o mouse no card: qual, desde quando e se já foi
    /// clicado (aí some até o mouse sair, como as dicas do Windows).
    tip: Option<(Hit, Instant, bool)>,
    /// Ordem das linhas do mixer enquanto ele está aberto: a da abertura, com
    /// os apps novos no fim. A thread reordena a lista a cada aviso do Windows
    /// (tocando primeiro); se as linhas trocassem de lugar com o mouse em
    /// cima, o slider, a roda e o mudo passariam a valer para outro app.
    mix_order: Vec<String>,
    /// O mixer como aparece, refeito só quando algo muda (a thread publicou,
    /// o usuário mexeu, a ordem mudou): o card não clona o estado por quadro.
    mix_view: crate::shared::MixerState,
    mix_view_key: Option<(u64, usize, usize)>,
    /// Lista do mixer rolada (px lógicos): alvo e como aparece (anima).
    mix_scroll_to: f32,
    mix_scroll: f32,
    /// Último x, y do mouse no card (a roda rola a lista do mixer embaixo dele).
    mouse_f: (f32, f32),
    /// Falhas ao trocar a saída de áudio já avisadas.
    out_failed_seen: u64,
}

/// Linhas do mixer na ordem `order` (a da abertura; os que não estão nela,
/// no fim), com os sons do sistema em cima, como no Mixer do Windows.
fn mix_arrange(apps: &mut [crate::shared::MixApp], order: &[String]) {
    apps.sort_by_key(|a| (!a.system, order.iter().position(|k| *k == a.key).unwrap_or(usize::MAX)));
}

/// Volume e/ou mudo que o usuário mudou num app do mixer, e quando.
struct MixHold {
    key: String,
    volume: Option<f32>,
    muted: Option<bool>,
    at: Instant,
}

/// Quanto tempo o valor mudado no mixer vale por cima do publicado.
const MIX_HOLD: Duration = Duration::from_millis(1200);

/// Arrasto do widget pela barra: começa ao apertar; vira arrasto quando o
/// mouse anda alguns pixels (antes disso, soltar é um clique normal).
#[derive(Clone, Copy, Debug)]
struct WidgetDrag {
    /// Mouse (x na tela) e widget (x) quando apertou.
    cursor0: i32,
    x0: i32,
    dx: i32,
    active: bool,
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
    /// Segundo atual quando a segunda linha mostra o tempo.
    clock: i64,
}

/// Tudo que muda a aparência do card aberto quando nada está animando.
#[derive(Clone, Copy, PartialEq, Debug)]
struct FlyoutSig {
    place: (i32, i32, i32, i32),
    media: u64,
    cfg: u64,
    colors: u64,
    hover: Option<Hit>,
    pressed: Option<(Win, Hit)>,
    drag: Option<Hit>,
    drag_x: i32,
    seek: Option<i32>,
    playing: bool,
    shuffle: bool,
    repeat: u8,
    /// Segundo da música (o tempo e a barra de progresso).
    clock: i64,
    volume: (u16, bool),
    /// Mixer: o que ele mostra e quanto está aberto.
    mixer: (u64, u16),
    /// Opacidade da dica do botão sob o mouse.
    tip: u8,
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
        owner: None,
        timer,
        period: 0,
        pacer: Pacer::new(Target::Event),
        half_rate: false,
        pacer_skip: false,
        last_pacer_frame: now,
        monitor: 0,
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
        flyout: Layer::new(flyout),
        flyout_open: false,
        flyout_anim: 0.0,
        flyout_last_hover: now,
        flyout_box: None,
        hint: None,
        vis_log: None,
        popup: Layer::new(popup),
        popup_start: None,
        popup_wait: None,
        popup_pos: None,
        osd: Layer::new(osd),
        osd_start: None,
        osd_key: (tr("Caps Lock"), false, false),
        osd_tmp: Vec::new(),
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
        flyout_sig: None,
        prev_media: MediaInfo::default(),
        text_change: None,
        art_change: None,
        trimmed: false,
        icons: ui::IconAnims::default(),
        apps: Arc::new(Vec::new()),
        launch_press: None,
        drag_x: 0.0,
        wdrag: None,
        web_needed: false,
        web_probed: None,
        mixer_open: false,
        mixer_anim: 0.0,
        mix_hold: Vec::new(),
        mix_drag: None,
        vol_fill: 0.0,
        mix_fill: Vec::new(),
        tip: None,
        mix_order: Vec::new(),
        mix_view: Default::default(),
        mix_view_key: None,
        mix_scroll_to: 0.0,
        mix_scroll: 0.0,
        mouse_f: (0.0, 0.0),
        out_failed_seen: 0,
        shared,
    };
    ov.refresh_launchers();
    ov.set_period(ACTIVE_PERIOD_MS);
    ov.update_hook();
    ov.attach_to_taskbar();

    let mut msg = MSG::default();
    let handles: Vec<HANDLE> = std::iter::once(ov.timer).chain(ov.pacer.as_ref().map(|p| p.event())).collect();
    'outer: loop {
        let r = unsafe { MsgWaitForMultipleObjects(Some(&handles), false, 1000, QS_ALLINPUT) };
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
        let vblank = handles.len() > 1 && r.0 == WAIT_OBJECT_0.0 + 1;
        let pacing = ov.pacer.as_ref().is_some_and(|p| p.active());
        if vblank {
            // Animação leve: um quadro sim, outro não (~30 fps, alinhado ao monitor).
            ov.pacer_skip = ov.half_rate && !ov.pacer_skip;
            if !ov.pacer_skip || has_input {
                ov.last_pacer_frame = Instant::now();
                ov.frame();
            }
        } else if r == WAIT_OBJECT_0 || r == WAIT_TIMEOUT || has_input {
            // Com o ritmo do monitor ativo, o timer só entra se ele parou de bater.
            if !pacing || has_input || ov.last_pacer_frame.elapsed() > Duration::from_millis(120) {
                ov.frame();
            }
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
    /// Ritmo dos quadros: animação forte (cada vblank), leve (vblank sim, vblank
    /// não) ou repouso (timer lento). Sem o ritmo do monitor, volta aos timers.
    fn set_rate(&mut self, busy: bool, decorative: bool) {
        let Some(pacer) = &self.pacer else {
            self.set_period(if busy {
                ACTIVE_PERIOD_MS
            } else if decorative {
                ANIM_PERIOD_MS
            } else {
                IDLE_PERIOD_MS
            });
            return;
        };
        let on = busy || decorative;
        self.half_rate = !busy;
        pacer.set_active(on);
        self.set_period(if on { FALLBACK_PERIOD_MS } else { IDLE_PERIOD_MS });
    }

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

    /// Faz a barra de tarefas ser a "dona" do widget. O Windows mantém janelas
    /// possuídas sempre acima da dona: quando a barra é ativada (abrir/fechar os
    /// ícones ocultos, clicar no relógio) ela sobe junto com o widget, em vez de
    /// cobri-lo até o próximo quadro (o widget "piscava"). Se o Explorer
    /// reiniciar, a barra nova vira a dona; se a janela tiver sido destruída
    /// junto com a barra antiga, é recriada.
    fn attach_to_taskbar(&mut self) {
        let alive = unsafe { IsWindow(Some(self.hwnd)).as_bool() };
        if !alive {
            match create_window(w!("TaskbeatWidget"), Some(widget_proc), WS_EX_NOACTIVATE) {
                Ok(h) => {
                    self.hwnd = h;
                    self.shown = false;
                    self.surface = None;
                    self.last_sig = None;
                    self.owner = None;
                }
                Err(e) => {
                    log::warn(&format!("widget: não deu para recriar a janela: {e}"));
                    return;
                }
            }
        }
        let tb = taskbar::taskbar_hwnd();
        if tb == self.owner {
            return;
        }
        if let Some(tb) = tb {
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, tb.0 as isize);
            }
        }
        self.owner = tb;
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

    /// Atalhos à mostra, na ordem escolhida (a lista dos apps achados muda raramente).
    fn refresh_launchers(&mut self) {
        let found = self.shared.launchers.lock().unwrap().clone();
        self.apps = Arc::new(crate::launchers::shown(&found, &self.cfg));
    }

    /// Quantos atalhos vão no card e na barra.
    fn card_launchers(&self) -> usize {
        if self.cfg.card_launchers && self.cfg.launcher_place.card() { self.apps.len() } else { 0 }
    }

    fn bar_launchers(&self) -> usize {
        if self.cfg.card_launchers && self.cfg.launcher_place.taskbar() { self.apps.len() } else { 0 }
    }

    fn card_layout(&self) -> CLayout {
        let s = self.scale();
        let rows = self.cfg.card_mixer.then(|| self.shared.mixer.lock().unwrap().apps.len());
        let base = CLayout::build(s, &self.cfg, self.media.dur > 0.0).with_launchers(self.card_launchers(), s);
        // Linhas à vista: até 7, menos se o card aberto não couber na tela.
        let room = self.placement.map(|p| (p.mon_bottom - p.mon_top) as f32 - (p.h as f32) - 40.0 * s).unwrap_or(f32::MAX);
        let max_rows = ((room - base.h - 50.0 * s) / (36.0 * s)).floor().clamp(3.0, crate::shared::MIXER_ROWS as f32) as usize;
        base.with_mixer(rows, ui::ease_move(self.mixer_anim), self.mix_scroll, max_rows, s)
    }

    /// Abre ou fecha o mixer de volume (a thread dele só trabalha aberto).
    fn set_mixer(&mut self, open: bool) {
        if open != self.mixer_open {
            self.mixer_open = open;
            // Abre sempre do começo da lista.
            self.mix_scroll_to = 0.0;
            self.mix_scroll = 0.0;
            self.shared.send_mixer(if open { MixCmd::Open } else { MixCmd::Close });
        }
    }

    /// O mixer como aparece: o publicado pela thread com o que o usuário
    /// acabou de mudar por cima.
    fn mixer_view(&self) -> crate::shared::MixerState {
        let mut st = self.shared.mixer.lock().unwrap().clone();
        for a in &mut st.apps {
            if let Some(h) = self.mix_hold.iter().find(|h| h.key == a.key) {
                a.volume = h.volume.unwrap_or(a.volume);
                a.muted = h.muted.unwrap_or(a.muted);
            }
        }
        mix_arrange(&mut st.apps, &self.mix_order);
        st
    }

    /// Guarda a ordem das linhas do mixer aberto (e esquece ao fechar).
    fn keep_mix_order(&mut self) {
        if !self.mixer_open {
            self.mix_order.clear();
            return;
        }
        let st = self.shared.mixer.lock().unwrap();
        self.mix_order.retain(|k| st.apps.iter().any(|a| a.key == *k));
        for a in &st.apps {
            if !self.mix_order.contains(&a.key) {
                self.mix_order.push(a.key.clone());
            }
        }
    }

    /// Lista das saídas de áudio (menu do app, embaixo do nome da saída no
    /// mixer); escolher uma a torna a padrão.
    fn pick_output(&mut self, now: Instant) {
        let (Some((x, y, _, _)), devices, current) = (self.flyout_box, self.mix_view.devices.clone(), self.mix_view.device_id.clone()) else { return };
        if devices.is_empty() {
            return;
        }
        let d = self.card_layout().mixer_dev;
        let at = RECT { left: x + d.x as i32, top: y + d.y as i32, right: x + d.right() as i32, bottom: y + d.bottom() as i32 };
        let th = ui::card_theme(&self.cfg, &self.colors, &self.media);
        let entries: Vec<crate::menu::Entry> = devices.iter().enumerate().map(|(k, o)| crate::menu::Entry::radio(k as u32 + 1, &o.name, o.id == current)).collect();
        // Largo o bastante para os nomes inteiros ("Speakers (Logitech G635/G633s Gaming Headset)").
        let opts = crate::menu::Options { dark: th.dark, accent: th.accent, min_width: 380.0 };
        let pick = crate::menu::popup(self.flyout.hwnd, entries, crate::menu::Anchor::Below(at), opts, &mut |_| None);
        // O menu é modal e toma o foco: o "card perdeu o foco" que isso gerou
        // não pode fechar o card; o foco volta para ele.
        EVENTS.with(|e| e.borrow_mut().retain(|(_, ev)| !matches!(ev, Ev::Deactivate)));
        unsafe {
            let _ = SetForegroundWindow(self.flyout.hwnd);
        }
        self.flyout_last_hover = now;
        self.pressed = None;
        if let Some(o) = pick.and_then(|k| devices.get(k as usize - 1)).filter(|o| o.id != current) {
            log::info(&format!("trocando a saída de áudio para {}", o.name));
            self.shared.send_mixer(MixCmd::SetDevice(o.id.clone()));
        }
    }

    /// Chave, volume e mudo do app da linha `i` do mixer (como aparece).
    fn mix_row(&self, i: usize) -> Option<(String, f32, bool)> {
        self.mixer_view().apps.get(i).map(|a| (a.key.clone(), a.volume, a.muted))
    }

    /// Volume (ou mudo) do app `key`: mostra na hora e manda para a thread.
    fn set_mix(&mut self, key: &str, volume: Option<f32>, mute: Option<bool>, now: Instant) {
        let muted_now = self.mixer_view().apps.iter().find(|a| a.key == key).map(|a| a.muted).unwrap_or(false);
        let i = match self.mix_hold.iter().position(|h| h.key == key) {
            Some(i) => i,
            None => {
                self.mix_hold.push(MixHold { key: key.to_string(), volume: None, muted: None, at: now });
                self.mix_hold.len() - 1
            }
        };
        let h = &mut self.mix_hold[i];
        h.at = now;
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
        self.shared.mixer_version.fetch_add(1, Ordering::Relaxed);
    }

    fn widget_layout(&self) -> Option<WLayout> {
        let p = self.placement?;
        Some(WLayout::build_with(p.scale, p.h as f32, &self.cfg, self.panel, self.viz, self.bar_launchers()))
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
            self.refresh_launchers();
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
            let mut new = self.shared.media.lock().unwrap().clone();
            self.web_needed = new.present() && !(new.can_shuffle && new.can_repeat);
            self.shared.web.lock().unwrap().merge_into(&mut new);
            // Troca de música com o widget à vista: anima texto e capa.
            if self.cfg.track_anim && self.shown && self.fade > 0.99 {
                let old = &self.media;
                let text = old.present() && new.present() && (old.title != new.title || old.artist != new.artist);
                let art = !new.art_pending && !same_art(&old.art, &new.art) && (old.art.is_some() || new.art.is_some());
                // Texto e capa animam separados: o player costuma mandar a capa
                // nova um pouco depois do título.
                if text {
                    let art_shown = self.prev_media.art.take();
                    self.prev_media = old.clone();
                    self.prev_media.art = art_shown;
                    self.text_change = Some(now);
                }
                if art {
                    self.prev_media.art = old.art.clone();
                    self.art_change = Some(now);
                }
            }
            self.media = new;
            // Aleatório/repetir à vista (card aberto ou botões na barra): confere a página.
            if self.flyout_open || (self.cfg.taskbar_controls && self.cfg.taskbar_shuffle_repeat) {
                self.probe_web(false);
            }
        }
        self.colors.album = self.media.art.as_ref().map(|a| a.vibrant);
        let visible = self.placement.is_some_and(|p| p.visible);

        // Cartão "A seguir": só troca automática. Também com o card aberto (aí
        // aparece empilhado acima dele). Espera um instante pela capa nova (o
        // player costuma publicar o título antes da imagem) e, se o widget
        // estiver escondido naquele instante, espera ele voltar.
        if self.media.auto_change_seq != self.seen_seq {
            self.seen_seq = self.media.auto_change_seq;
            if self.cfg.next_track_popup {
                self.popup_wait = Some(now);
                log::info(&format!("\"A seguir\": troca automática (widget visível={visible}, música={})", self.media.present()));
            }
        }
        if let Some(t0) = self.popup_wait {
            let waited = now - t0;
            let ready = !self.media.art_pending || waited > Duration::from_millis(1500);
            if ready && visible && self.media.present() {
                self.popup_wait = None;
                self.start_popup(now);
                log::info("\"A seguir\": mostrado");
            } else if waited > Duration::from_secs(5) {
                self.popup_wait = None;
                log::info(&format!("\"A seguir\": desistiu (widget visível={visible}, música={})", self.media.present()));
            }
        }
        // Demonstração (capturas de tela): o card abre antes, para dar para ver
        // o "A seguir" empilhado acima dele com as duas variáveis ligadas.
        if self.demo_flyout && self.media.present() && visible {
            self.demo_flyout = false;
            self.open_flyout(now);
        }
        if self.demo_popup && self.media.present() && visible {
            self.demo_popup = false;
            self.start_popup(now);
        }
        if self.demo_osd && visible {
            self.demo_osd = false;
            self.osd_key = (tr("Caps Lock"), false, true);
            self.osd_start = Some(now);
        }
        self.process_lock_keys(now);
        if self.shared.osd_test.swap(false, Ordering::Relaxed) {
            // Já visível (ex.: arrastando o ajuste de posição): mantém aberto sem
            // reiniciar a animação de entrada.
            let shown = self.osd_start.is_some_and(|t| (now - t).as_secs_f32() > 0.3);
            self.osd_key = (tr("Caps Lock"), false, self.lock_state[0]);
            self.osd_start = Some(if shown { now - Duration::from_millis(350) } else { now });
        }

        // Quais partes aparecem e onde.
        let horizontal = self.placement.map(|p| p.horizontal).unwrap_or(true);
        let panel = self.cfg.show_media && horizontal && (self.media.present() || self.cfg.always_show_widget);
        let viz = self.cfg.show_visualizer;
        let layout_every = Duration::from_millis(if self.period == IDLE_PERIOD_MS { 500 } else { 250 });
        if panel != self.panel || viz != self.viz || now - self.last_layout > layout_every {
            self.panel = panel;
            self.viz = viz;
            self.last_layout = now;
            let logical = WLayout::build_with(1.0, 40.0, &self.cfg, panel, viz, self.bar_launchers()).width.ceil() as i32;
            self.placement = taskbar::compute(&self.cfg, logical.max(1));
            self.attach_to_taskbar();
            let mon = unsafe { MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTOPRIMARY) };
            if mon.0 as isize != self.monitor {
                self.monitor = mon.0 as isize;
                if let Some(p) = &self.pacer {
                    p.set_monitor(mon);
                }
            }
        }

        if let (Some(d), Some(p)) = (self.wdrag.filter(|d| d.active), self.placement.as_mut()) {
            p.x = (d.x0 + d.dx).clamp(p.min_x, p.max_x.max(p.min_x));
        }

        // Áudio -> espectro: com o visualizador da barra ligado ou com o card
        // aberto com as barras dele (aí a captura roda só enquanto ele está aberto).
        let card_bars = self.cfg.card_viz && self.flyout_open;
        self.shared.card_bars.store(card_bars, Ordering::Relaxed);
        if viz || card_bars {
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
        // Diagnóstico: registra quando o widget some/volta e por quê.
        let vis = (self.placement.map(|p| p.visible), panel, self.media.present());
        if self.vis_log != Some(vis) {
            self.vis_log = Some(vis);
            let why = self.placement.and_then(|p| p.why_hidden).unwrap_or(if self.placement.is_none() { "barra de tarefas não encontrada" } else { "" });
            log::info(&format!("widget: {} [barra visível={:?}, painel={panel}, música={}] {why}", if want { "visível" } else { "escondido" }, vis.0, vis.2));
        }
        let target = if want { 1.0 } else { 0.0 };
        let speed = if want { 6.0 } else { 2.5 };
        self.fade += (target - self.fade).clamp(-speed * dt, speed * dt);

        let idle_close = Duration::from_secs(self.cfg.card_autoclose.max(2) as u64);
        if self.flyout_open && (!want || (self.hover_f.is_none() && self.drag.is_none() && !self.mixer_open && now - self.flyout_last_hover > idle_close)) {
            self.flyout_open = false;
        }
        let pb = self.playback();
        // Ícones: anima o que mudou (clique ou o próprio player).
        if self.media.present() {
            self.icons.observe(&pb);
        } else {
            self.icons.reset_seen();
        }
        let icons_active = self.cfg.icon_anim && self.icons.running();
        // Barras no card mexendo (card aberto, barras ligadas e som tocando).
        let live_bars = card_bars && sound;
        let flyout_active = self.update_flyout(now, dt, placement, pb, live_bars);
        let popup_active = self.update_popup(now, placement);
        let osd_active = self.update_osd(now, placement);

        // Ritmo: 60 fps quando há interação/animação forte, 30 fps para o
        // letreiro e o indicador, 10 fps em repouso.
        let busy = (viz && sound)
            || self.hover_w.is_some()
            || self.drag.is_some()
            || self.wdrag.is_some()
            || flyout_active
            || popup_active
            || self.popup_wait.is_some()
            || osd_active
            || had_input
            || icons_active
            || (self.fade > 0.0 && self.fade < 1.0)
            || now - self.last_input < Duration::from_millis(500);
        let changing = self.change_progress(now).is_some();
        let busy = busy || changing;
        let decorative = panel && pb.playing && (self.cfg.playing_indicator || self.cfg.marquee || self.cfg.taskbar_progress);
        self.set_rate(busy, decorative);

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
        let animating = (viz && sound) || decorative || changing || icons_active || (self.fade > 0.0 && self.fade < 1.0);
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
            clock: if self.cfg.second_line == crate::config::SecondLine::Time { pb.pos as i64 } else { 0 },
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
        let layout = WLayout::build_with(p.scale, p.h as f32, &self.cfg, panel, viz, self.bar_launchers());
        {
            let mut cv = surface.canvas();
            cv.fill_raw(0x0100_0000); // alfa 1/255: invisível, mas recebe cliques
            let theme = ui::widget_theme(&self.cfg, &self.colors, &self.media);
            let pressed = self.pressed.filter(|(w, _)| *w == Win::Widget).map(|(_, h)| h);
            let change = self.change_progress(now);
            let f = ui::WidgetFrame {
                cfg: &self.cfg,
                media: &self.media,
                theme,
                colors: &self.colors,
                it: Interact { hover: self.hover_w, pressed, drag: None, fx: self.icons.fx(self.cfg.icon_anim), ..Default::default() },
                pb,
                time: (now - self.started).as_secs_f32(),
                viz: Some((&self.analyzer.values, &self.analyzer.peaks)),
                flyout_open: self.flyout_open,
                s: p.scale,
                change: change.map(|(a, b)| (&self.prev_media, a, b)),
                launchers: &self.apps,
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

    /// Progresso (0..1) da troca de música: (texto, capa); `None` se nada anima.
    fn change_progress(&mut self, now: Instant) -> Option<(f32, f32)> {
        let p = |t: &mut Option<Instant>| {
            let v = t.map(|t| ((now - t).as_secs_f32() / 0.42).min(1.0)).unwrap_or(1.0);
            if v >= 1.0 {
                *t = None;
            }
            v
        };
        let (a, b) = (p(&mut self.text_change), p(&mut self.art_change));
        (a < 1.0 || b < 1.0).then_some((a, b))
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
            let (i, name, num, enabled) = if vk == VK_CAPITAL.0 {
                (0, "Caps Lock", false, self.cfg.caps_lock_osd)
            } else {
                (1, tr("Num Lock"), true, self.cfg.num_lock_osd)
            };
            // A tecla alterna o estado; o hook vê a tecla antes do Windows aplicar.
            self.lock_state[i] = !self.lock_state[i];
            let show_state = self.lock_state[i] || self.cfg.osd_show_off;
            if enabled && show_state && self.placement.is_some_and(|p| p.visible) {
                self.osd_key = (name, num, self.lock_state[i]);
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
        let e = if t < 0.3 {
            ui::ease_enter(t / 0.3)
        } else if t > secs - 0.22 {
            1.0 - ui::ease_exit((t - (secs - 0.22)) / 0.22)
        } else {
            1.0
        };
        let m = ui::SHADOW_MARGIN * s;
        let gap = (12.0 * s) as i32;
        let (wi, hi, mi) = (w as i32, h as i32, m as i32);
        let center_x = (p.mon_left + p.mon_right) / 2 - wi / 2;
        // Posição final e de que lado o aviso entra (+1: de baixo, -1: de cima).
        let (x, y, dir) = match self.cfg.osd_position {
            OsdPosition::Bottom if p.taskbar_bottom_edge => (center_x, p.taskbar_top - hi + mi - (20.0 * s) as i32, 1),
            OsdPosition::Bottom => (center_x, p.mon_bottom - hi + mi - (60.0 * s) as i32, 1),
            OsdPosition::Center => (center_x, (p.mon_top + p.mon_bottom) / 2 - hi / 2, 1),
            OsdPosition::Top => (center_x, p.mon_top + (40.0 * s) as i32 - mi, -1),
            OsdPosition::AbovePlayer => {
                let x = p.x + p.w / 2 - wi / 2;
                if p.taskbar_bottom_edge { (x, p.taskbar_top - hi + mi - gap, 1) } else { (x, p.y + p.h + gap - mi, -1) }
            }
        };
        // Ajuste fino do usuário; nunca sai do monitor.
        let x = (x + (self.cfg.osd_offset_x as f32 * s) as i32).clamp(p.mon_left - mi, p.mon_right - wi + mi);
        let y = (y + (self.cfg.osd_offset_y as f32 * s) as i32).clamp(p.mon_top - mi, p.mon_bottom - hi + mi);
        let (slide, zoom) = match self.cfg.osd_anim {
            OsdAnim::Slide => ((1.0 - e) * 18.0 * s, 1.0),
            OsdAnim::Zoom => (0.0, 0.88 + 0.12 * e),
            OsdAnim::Fade => (0.0, 1.0),
        };

        // Acrílico: o que está atrás do cartão na posição final.
        let backdrop = if ui::osd_wants_backdrop(&self.cfg) {
            let (cw, ch) = ui::osd_card(s, &self.cfg);
            self.osd.backdrop((x + mi, y + mi, cw.round() as i32, ch.round() as i32), ui::osd_blur_px(&self.cfg, s), now, true)
        } else {
            None
        };

        self.text.set_scale(s);
        Surface::ensure(&mut self.osd.surface, w as i32, h as i32);
        let Some(mut surf) = self.osd.surface.take() else { return true };
        {
            let th = ui::osd_theme(&self.cfg, &self.colors);
            let (name, num, on) = self.osd_key;
            if zoom < 0.999 {
                // Zoom: desenha em tamanho normal e reduz a partir do centro.
                let (sw, sh) = (surf.w as usize, surf.h as usize);
                let mut px = std::mem::take(&mut self.osd_tmp);
                px.clear();
                px.resize(sw * sh, 0);
                ui::draw_osd(&mut Canvas::new(sw, sh, &mut px), &mut self.text, &self.cfg, &self.colors, &th, s, name, num, on, t, backdrop.as_deref());
                let img = Image::plain(sw, sh, px);
                let (zw, zh) = (sw as f32 * zoom, sh as f32 * zoom);
                let mut cv = surf.canvas();
                cv.clear();
                cv.draw_image(RectF::new((sw as f32 - zw) / 2.0, (sh as f32 - zh) / 2.0, zw, zh), &img, 0.0, 1.0);
                self.osd_tmp = img.px;
            } else {
                let mut cv = surf.canvas();
                cv.clear();
                ui::draw_osd(&mut cv, &mut self.text, &self.cfg, &self.colors, &th, s, name, num, on, t, backdrop.as_deref());
            }
        }
        surf.present_at(self.osd.hwnd, x, y as f32 + dir as f32 * slide, e);
        self.osd.surface = Some(surf);
        self.osd.show(false);
        true
    }

    // ------------------------------------------------------------ entrada

    /// Pede para procurar os botões de aleatório/repetir na página do player
    /// (só nos players sem esses controles pelo Windows). `force`: mesmo que a
    /// música não tenha mudado (o card acabou de abrir: relê o repetir).
    fn probe_web(&mut self, force: bool) {
        if !self.web_needed {
            return;
        }
        let key = (self.media.aumid.clone(), self.media.title.clone());
        if !force && self.web_probed.as_ref() == Some(&key) {
            return;
        }
        self.web_probed = Some(key.clone());
        self.shared.send_web(WebCmd::Probe { aumid: key.0, title: key.1 });
    }

    fn open_flyout(&mut self, now: Instant) {
        self.probe_web(true);
        self.flyout_open = true;
        self.flyout_last_hover = now;
        self.popup_start = None;
        self.popup_wait = None;
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
            if win == Win::Widget && self.widget_drag_event(ev, s, now) {
                continue;
            }
            let hit_at = |x: f32, y: f32| match win {
                Win::Widget => wl.hit(x, y),
                Win::Flyout => cl.hit(x, y),
            };
            match ev {
                Ev::Move(x, y, _) => {
                    if win == Win::Flyout {
                        self.flyout_last_hover = now;
                        self.mouse_f = (x, y);
                        // Atalho apertado e o mouse andou: vira arrasto (reordena).
                        if let Some((i, x0)) = self.launch_press
                            && self.drag.is_none()
                            && (x - x0).abs() > 5.0 * s
                        {
                            self.drag = Some(Hit::Launch(i));
                        }
                    }
                    match self.drag {
                        Some(Hit::Launch(_)) => self.drag_x = x,
                        Some(Hit::MixVol(i)) => {
                            if let (Some(v), Some(key)) = (Self::frac(x, cl.mix_track(i as usize, s)), self.mix_drag.clone()) {
                                self.set_mix(&key, Some(v), None, now);
                            }
                        }
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
                Ev::Down(x, y, _) => {
                    let h = hit_at(x, y);
                    self.pressed = h.map(|h| (win, h));
                    // Apertou: a dica some (e só volta depois de o mouse sair).
                    if let Some(t) = &mut self.tip {
                        t.2 = true;
                    }
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
                            Some(Hit::Launch(i)) => {
                                self.launch_press = Some((i, x));
                                self.drag_x = x;
                            }
                            Some(Hit::MixVol(i)) => {
                                if let Some((key, _, _)) = self.mix_row(i as usize) {
                                    self.drag = Some(Hit::MixVol(i));
                                    self.mix_drag = Some(key.clone());
                                    if let Some(v) = Self::frac(x, cl.mix_track(i as usize, s)) {
                                        self.set_mix(&key, Some(v), None, now);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ev::Up(x, y, _) => {
                    let up = hit_at(x, y);
                    self.launch_press = None;
                    if let Some(key) = self.mix_drag.take() {
                        // O valor solto vale por mais um tempo, até a thread alcançar.
                        if let Some(h) = self.mix_hold.iter_mut().find(|h| h.key == key) {
                            h.at = now;
                        }
                    }
                    match self.drag.take() {
                        Some(Hit::Launch(i)) => self.drop_launcher(i, &cl.launch, x),
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
                Ev::Wheel(d) => match self.hover_f {
                    Some(Hit::MixVol(i)) if win == Win::Flyout => {
                        if let Some((key, v, _)) = self.mix_row(i as usize) {
                            self.set_mix(&key, Some(((v + d as f32 / 120.0 * 0.04) * 50.0).round() / 50.0), None, now);
                        }
                    }
                    // Sobre a lista do mixer (fora dos sliders): rola, uma linha por "clique".
                    _ if win == Win::Flyout && cl.mixer_open > 0.5 && cl.mixer_scroll_max > 0.0 && cl.mixer_list.contains(self.mouse_f.0, self.mouse_f.1) => {
                        self.mix_scroll_to = (self.mix_scroll_to - d as f32 / 120.0 * 36.0).clamp(0.0, cl.mixer_scroll_max);
                    }
                    _ => self.volume.step(d as f32 / 120.0 * 0.02),
                },
                Ev::Deactivate | Ev::Escape => {
                    if self.drag.is_none() {
                        self.flyout_open = false;
                    }
                }
                // Captura perdida no meio de um arrasto: termina sem aplicar o
                // que dependia de soltar (pular, mudar a ordem dos atalhos); o
                // volume já foi aplicado enquanto arrastava.
                Ev::CaptureLost => {
                    if self.drag.take().is_some() {
                        self.seek_preview = None;
                    }
                    if let Some(key) = self.mix_drag.take()
                        && let Some(h) = self.mix_hold.iter_mut().find(|h| h.key == key)
                    {
                        h.at = now;
                    }
                    self.launch_press = None;
                    self.pressed = None;
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
            Hit::Prev if m.can_prev => {
                self.icons.kick(Hit::Prev);
                self.shared.send_media(MediaCmd::Previous);
            }
            Hit::Next if m.can_next => {
                self.icons.kick(Hit::Next);
                self.shared.send_media(MediaCmd::Next);
            }
            Hit::Play if m.can_toggle => {
                let playing = self.play_pending.resolve(m.playing);
                self.play_pending.set(!playing);
                self.shared.send_media(MediaCmd::PlayPause);
            }
            // Navegador: aperta o botão da própria página do player.
            Hit::Shuffle if m.web_shuffle => {
                self.icons.kick(Hit::Shuffle);
                self.shared.send_web(WebCmd::Shuffle { aumid: m.aumid.clone(), title: m.title.clone() });
                self.hint = Some((now, tr("Fila embaralhada no player")));
            }
            Hit::Repeat if m.web_repeat => {
                let next = crate::webplayer::next_repeat(self.repeat_pending.resolve(m.repeat));
                self.repeat_pending.set(next);
                self.shared.send_web(WebCmd::Repeat { aumid: m.aumid.clone(), title: m.title.clone() });
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
            // O player não deixa (navegadores): traz o player para a frente,
            // para usar o botão dele. Sem achar a janela, explica no card.
            Hit::Shuffle | Hit::Repeat => {
                if crate::launchers::focus_player(&m.aumid, &m.title) {
                    self.flyout_open = false;
                } else {
                    if !self.flyout_open {
                        self.open_flyout(now);
                    }
                    self.hint = Some((now, tr("Este player não deixa mudar isso pelo Windows")));
                }
            }
            Hit::VolBtn => self.volume.toggle_mute(),
            Hit::MixToggle => {
                let open = !self.mixer_open;
                self.set_mixer(open);
                self.flyout_last_hover = now;
            }
            Hit::MixDevice => self.pick_output(now),
            Hit::MixMute(i) => {
                if let Some((key, _, m)) = self.mix_row(i as usize) {
                    self.set_mix(&key, None, Some(!m), now);
                }
            }
            // Abre o app de música e fecha o card (a janela do app aparece).
            Hit::Launch(i) => {
                if let Some(app) = self.apps.get(i as usize) {
                    log::info(&format!("abrindo {} ({})", app.name, app.id));
                    crate::launchers::open(&app.id);
                }
                self.flyout_open = false;
            }
            _ => {}
        }
    }

    /// Arrastar o widget pela barra. Devolve `true` quando o evento foi do
    /// arrasto (não vira clique nem mexe no "mouse em cima").
    fn widget_drag_event(&mut self, ev: Ev, s: f32, now: Instant) -> bool {
        match ev {
            Ev::Down(_, _, sx) => {
                // Só com a barra na horizontal: na vertical (Windows 10) não há
                // limites calculados e o widget pularia para o canto.
                if let Some(p) = self.placement.filter(|p| self.cfg.widget_drag && p.horizontal) {
                    self.wdrag = Some(WidgetDrag { cursor0: sx, x0: p.x, dx: 0, active: false });
                }
                false
            }
            Ev::Move(_, _, sx) => {
                let Some(d) = self.wdrag.as_mut() else { return false };
                d.dx = sx - d.cursor0;
                if !d.active && d.dx.abs() as f32 > 6.0 * s {
                    d.active = true;
                    self.pressed = None;
                    self.flyout_open = false;
                    self.popup_start = None;
                }
                d.active
            }
            Ev::Up(_, _, sx) => {
                let Some(mut d) = self.wdrag.take() else { return false };
                d.dx = sx - d.cursor0;
                if !d.active {
                    return false;
                }
                self.pressed = None;
                if let Some(p) = self.placement {
                    let x = (d.x0 + d.dx).clamp(p.min_x, p.max_x.max(p.min_x));
                    let off = (((x - p.base_x) as f32) / p.scale).round() as i32;
                    log::info(&format!("widget arrastado para x={x} (deslocamento {off})"));
                    self.shared.update_config(|c| c.offset = off.clamp(-4000, 4000));
                    self.last_layout = now - Duration::from_secs(10);
                }
                true
            }
            Ev::Leave => self.wdrag.is_some_and(|d| d.active),
            // Captura perdida: o widget volta para onde estava (nada é gravado).
            Ev::CaptureLost => {
                let was = self.wdrag.take().is_some_and(|d| d.active);
                if was {
                    self.pressed = None;
                    self.last_layout = now - Duration::from_secs(10);
                }
                was
            }
            _ => false,
        }
    }

    /// Solta o atalho `i` arrastado na linha do card: muda a ordem e grava.
    fn drop_launcher(&mut self, i: u8, slots: &[RectF], x: f32) {
        let Some(app) = self.apps.get(i as usize).cloned() else { return };
        let to = ui::launch_drop_slot(&slots[..slots.len().min(self.apps.len())], x);
        if to == i as usize {
            return;
        }
        let found = self.shared.launchers.lock().unwrap().clone();
        let all = crate::launchers::arranged(&found, &self.cfg);
        let Some(full) = crate::launchers::full_index(&all, to) else { return };
        let order = crate::launchers::moved(&all, &app.id, full);
        let (_, hidden) = crate::launchers::lists_of(&all);
        log::info(&format!("atalho {} movido para a posição {}", app.name, to + 1));
        self.shared.update_config(|c| {
            c.launcher_order = order;
            c.launcher_hidden = hidden;
        });
    }

    // ------------------------------------------------------------ card expandido

    /// Desenha o card (abrindo, aberto ou fechando). Devolve se ele precisa
    /// de quadros seguidos (animação, barras tocando); parado, só é
    /// redesenhado quando algo nele muda.
    fn update_flyout(&mut self, now: Instant, dt: f32, placement: Option<Placement>, pb: Playback, live_bars: bool) -> bool {
        let target = if self.flyout_open && placement.is_some() { 1.0 } else { 0.0 };
        let opening = target > 0.5;
        let speed = if opening { 1.0 / 0.28 } else { 1.0 / 0.16 };
        self.flyout_anim += (target - self.flyout_anim).clamp(-speed * dt, speed * dt);
        let Some(p) = placement.filter(|_| self.flyout_anim > 0.001) else {
            if self.flyout.shown {
                ui::forget_layers(ui::LayerTag::Card);
            }
            self.flyout.hide();
            self.hover_f = None;
            self.flyout_anim = 0.0;
            self.flyout_open = false;
            self.flyout_box = None;
            self.flyout_sig = None;
            // O card fechou: o mixer fecha junto (abre recolhido da próxima vez).
            self.set_mixer(false);
            self.mixer_anim = 0.0;
            self.vol_fill = 0.0;
            self.mix_fill.clear();
            self.mix_hold.clear();
            self.mix_drag = None;
            self.mix_view = Default::default();
            self.mix_view_key = None;
            return false;
        };
        // Mixer abrindo/fechando: o card cresce ou encolhe junto.
        let target = if self.mixer_open && self.cfg.card_mixer { 1.0 } else { 0.0 };
        self.mixer_anim += (target - self.mixer_anim).clamp(-dt / 0.24, dt / 0.24);
        self.keep_mix_order();
        // Barras de volume: sobem e descem suaves até o valor (arrastando, na
        // hora); enchem ao abrir o card ou o mixer e esvaziam ao fechar.
        let drag_key = self.mix_drag.clone();
        self.mix_hold.retain(|h| now - h.at < MIX_HOLD || drag_key.as_deref() == Some(&h.key));
        let k = 1.0 - (-dt / 0.07).exp();
        let ease = |cur: &mut f32, target: f32, snap: bool| -> bool {
            if snap || (target - *cur).abs() < 0.002 {
                *cur = target;
                false
            } else {
                *cur += (target - *cur) * k;
                true
            }
        };
        let vol_target = if opening && !pb.muted { pb.volume } else { 0.0 };
        let mut fills_moving = ease(&mut self.vol_fill, vol_target, self.drag == Some(Hit::Vol));
        // O mixer só é copiado quando algo nele muda (e só com ele à vista).
        if self.cfg.card_mixer && (self.mixer_open || self.mixer_anim > 0.0) {
            let key = (self.shared.mixer_version.load(Ordering::Relaxed), self.mix_hold.len(), self.mix_order.len());
            if self.mix_view_key != Some(key) {
                self.mix_view = self.mixer_view();
                self.mix_view_key = Some(key);
            }
        }
        let mix_on = opening && self.mixer_open;
        let mut fill = std::mem::take(&mut self.mix_view.fill);
        fill.clear();
        for a in &self.mix_view.apps {
            let target = if mix_on && !a.muted { a.volume } else { 0.0 };
            let i = match self.mix_fill.iter().position(|(k, _)| *k == a.key) {
                Some(i) => i,
                None => {
                    self.mix_fill.push((a.key.clone(), 0.0));
                    self.mix_fill.len() - 1
                }
            };
            fills_moving |= ease(&mut self.mix_fill[i].1, target, drag_key.as_deref() == Some(&a.key));
            fill.push(self.mix_fill[i].1);
        }
        let apps = &self.mix_view.apps;
        self.mix_fill.retain(|(k, _)| apps.iter().any(|a| &a.key == k));
        self.mix_view.fill = fill;
        fills_moving |= ease(&mut self.mix_scroll, self.mix_scroll_to, false);
        // Trocar a saída de áudio falhou: aviso discreto no card.
        let failed = self.shared.output_failed.load(Ordering::Relaxed);
        if failed != self.out_failed_seen {
            self.out_failed_seen = failed;
            self.hint = Some((now, tr("Não deu para trocar a saída de áudio")));
        }
        // Dica da setinha do mixer: quase na hora (um quarto de segundo em cima).
        let tip_hit = self.hover_f.filter(|h| *h == Hit::MixToggle && self.drag.is_none());
        match (tip_hit, self.tip) {
            (Some(h), Some((t, _, _))) if t == h => {}
            (Some(h), _) => self.tip = Some((h, now, false)),
            (None, _) => self.tip = None,
        }
        let tip_a = match self.tip {
            Some((_, t0, false)) => (((now - t0).as_secs_f32() - 0.25) / 0.12).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let tip_moving = matches!(self.tip, Some((_, _, false))) && tip_a < 1.0;
        let s = p.scale;
        let l = self.card_layout();
        let m = ui::SHADOW_MARGIN * s;
        // Abre com "decelerate" (Fluent) e fecha com "accelerate".
        let e = if opening { ui::ease_enter(self.flyout_anim) } else { 1.0 - ui::ease_exit(1.0 - self.flyout_anim) };
        let wl = WLayout::build_with(s, p.h as f32, &self.cfg, self.panel, self.viz, self.bar_launchers());
        let anchor = p.x + wl.art.map(|a| a.x as i32).unwrap_or(0) - m as i32 - (6.0 * s) as i32;
        let x = anchor.clamp(p.mon_left - m as i32 + (8.0 * s) as i32, p.mon_right - l.w as i32 + m as i32 - (8.0 * s) as i32);
        let slide = (1.0 - e) * if opening { 24.0 } else { 12.0 } * s;
        let (y, dir) = if p.taskbar_bottom_edge {
            (p.taskbar_top - (l.h - m) as i32 - (10.0 * s) as i32, 1)
        } else {
            (p.y + p.h + (10.0 * s) as i32 - m as i32, -1)
        };
        self.flyout_box = Some((x, y, l.w.round() as i32, l.h.round() as i32));
        // Parado (aberto, sem animação): só redesenha se algo mudou.
        let hint_on = self.hint.is_some_and(|(t, _)| (now - t).as_secs_f32() < 2.8);
        let mixer_moving = self.mixer_anim > 0.0 && self.mixer_anim < 1.0;
        let moving = self.flyout_anim < 0.999 || live_bars || hint_on || mixer_moving || fills_moving || tip_moving || (self.cfg.icon_anim && self.icons.running());
        let sig = FlyoutSig {
            place: (x, y, l.w.round() as i32, l.h.round() as i32),
            media: self.media_version,
            cfg: self.cfg_version,
            colors: self.last_colors.elapsed().as_secs() / 30,
            hover: self.hover_f,
            pressed: self.pressed,
            drag: self.drag,
            drag_x: self.drag_x.round() as i32,
            seek: self.seek_preview.map(|v| (v * 1000.0) as i32),
            playing: pb.playing,
            shuffle: pb.shuffle,
            repeat: pb.repeat,
            clock: pb.pos as i64,
            volume: ((pb.volume * 1000.0) as u16, pb.muted),
            mixer: (self.shared.mixer_version.load(Ordering::Relaxed), (self.mixer_anim * 1000.0) as u16),
            tip: (tip_a * 255.0) as u8,
        };
        if !moving && self.flyout.shown && self.flyout_sig == Some(sig) {
            return false;
        }
        self.flyout_sig = Some(sig);
        // Acrílico: captura antes de o card aparecer (o card tem o foco: escondê-lo
        // para recapturar o fecharia, então mudanças com ele aberto usam a mesma imagem).
        let backdrop = if ui::card_wants_backdrop(&self.cfg) {
            let c = l.card;
            let rect = (x + c.x.round() as i32, y + c.y.round() as i32, c.w.round() as i32, c.h.round() as i32);
            self.flyout.backdrop(rect, ui::GLASS_BLUR * s, now, false)
        } else {
            None
        };

        self.text.set_scale(s);
        Surface::ensure(&mut self.flyout.surface, l.w.round() as i32, l.h.round() as i32);
        let Some(mut surf) = self.flyout.surface.take() else { return true };
        {
            let mut cv = surf.canvas();
            cv.clear();
            let pressed = self.pressed.filter(|(w, _)| *w == Win::Flyout).map(|(_, h)| h);
            let launchers = self.apps.clone();
            let f = ui::CardFrame {
                cfg: &self.cfg,
                colors: &self.colors,
                media: &self.media,
                theme: ui::card_theme(&self.cfg, &self.colors, &self.media),
                it: Interact { hover: self.hover_f, pressed, drag: self.drag, drag_x: self.drag_x, fx: self.icons.fx(self.cfg.icon_anim), vol_fill: Some(self.vol_fill), tip: tip_a, ..Default::default() },
                pb,
                s,
                backdrop: backdrop.as_deref(),
                hint: self.hint.and_then(|(t, msg)| {
                    let k = (now - t).as_secs_f32();
                    (k < 2.8).then(|| (msg, (1.0 - (k - 2.4) / 0.4).clamp(0.0, 1.0)))
                }),
                launchers: &launchers,
                viz: self.cfg.card_viz.then_some(&self.analyzer.values[..]),
                mixer: self.cfg.card_mixer.then_some(&self.mix_view),
            };
            ui::draw_card(&mut cv, &mut self.text, &l, &f);
        }
        surf.present_at(self.flyout.hwnd, x, y as f32 + dir as f32 * slide, e);
        self.flyout.surface = Some(surf);
        self.flyout.show(true);
        moving
    }

    // ------------------------------------------------------------ "A seguir"

    fn start_popup(&mut self, now: Instant) {
        self.popup_start = Some(now);
        self.popup_pos = None;
        self.popup.hide();
    }

    /// Onde o "A seguir" fica: acima da música na barra ou, com o card aberto,
    /// empilhado acima (ou abaixo, com a barra no topo) dele.
    fn popup_home(&self, p: &Placement, w: f32, h: f32) -> (i32, i32, i32) {
        let s = p.scale;
        let m = ui::SHADOW_MARGIN * s;
        let gap = 10.0 * s;
        let wl = WLayout::build_with(s, p.h as f32, &self.cfg, self.panel, self.viz, self.bar_launchers());
        let anchor = p.x + wl.art.map(|a| a.x as i32).unwrap_or(0) - m as i32 - (6.0 * s) as i32;
        let x = anchor.clamp(p.mon_left - m as i32 + (8.0 * s) as i32, p.mon_right - w as i32 + m as i32 - (8.0 * s) as i32);
        let (y, dir) = match (self.flyout_box, p.taskbar_bottom_edge) {
            (Some((_, fy, _, _)), true) => ((fy as f32 + m - gap - (h - m)) as i32, 1),
            (Some((_, fy, _, fh)), false) => ((fy as f32 + fh as f32 - m + gap - m) as i32, -1),
            (None, true) => (p.taskbar_top - (h - m) as i32 - gap as i32, 1),
            (None, false) => (p.y + p.h + gap as i32 - m as i32, -1),
        };
        let y = y.clamp(p.mon_top - m as i32, p.mon_bottom - h as i32 + m as i32);
        (x, y, dir)
    }

    fn update_popup(&mut self, now: Instant, placement: Option<Placement>) -> bool {
        let Some(start) = self.popup_start else {
            // Dispensado (ex.: o card foi aberto): some na hora, sem ficar congelado.
            self.popup.hide();
            return false;
        };
        let total = self.cfg.popup_secs.max(2) as f32 + 0.5;
        let t = (now - start).as_secs_f32();
        let Some(p) = placement.filter(|_| t < total) else {
            self.popup_start = None;
            self.popup_pos = None;
            ui::forget_layers(ui::LayerTag::UpNext);
            self.popup.hide();
            return false;
        };
        let s = p.scale;
        let (w, h) = ui::upnext_size(s);
        let e = if t < 0.5 {
            ui::ease_enter(t / 0.5)
        } else if t > total - 0.3 {
            1.0 - ui::ease_exit((t - (total - 0.3)) / 0.3)
        } else {
            1.0
        };
        // A posição é fixada quando o cartão aparece (não pula se o card fechar).
        let (x, y, dir) = match self.popup_pos {
            Some(v) => v,
            None => {
                let v = self.popup_home(&p, w, h);
                self.popup_pos = Some(v);
                v
            }
        };
        let slide = (1.0 - e) * 26.0 * s;
        let backdrop = if ui::card_wants_backdrop(&self.cfg) {
            let c = ui::upnext_card(s);
            let rect = (x + c.x.round() as i32, y + c.y.round() as i32, c.w.round() as i32, c.h.round() as i32);
            self.popup.backdrop(rect, ui::GLASS_BLUR * s, now, false)
        } else {
            None
        };

        self.text.set_scale(s);
        Surface::ensure(&mut self.popup.surface, w as i32, h as i32);
        let Some(mut surf) = self.popup.surface.take() else { return true };
        {
            let mut cv = surf.canvas();
            cv.clear();
            let th = ui::card_theme(&self.cfg, &self.colors, &self.media);
            ui::draw_upnext(&mut cv, &mut self.text, &self.cfg, &self.colors, &self.media, &th, s, t, total, backdrop.as_deref());
        }
        surf.present_at(self.popup.hwnd, x, y as f32 + dir as f32 * slide, e);
        self.popup.surface = Some(surf);
        self.popup.show(false);
        true
    }
}

fn same_art(a: &Option<Arc<Image>>, b: &Option<Arc<Image>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::mix_arrange;
    use crate::shared::MixApp;

    fn app(key: &str, system: bool) -> MixApp {
        MixApp { key: key.into(), name: key.into(), icon: None, volume: 1.0, muted: false, active: false, system }
    }

    fn keys(apps: &[MixApp]) -> Vec<&str> {
        apps.iter().map(|a| a.key.as_str()).collect()
    }

    /// Com o mixer aberto, as linhas não trocam de lugar quando a thread
    /// reordena (um app começou a tocar); app novo vai para o fim; os sons do
    /// sistema ficam em cima. Nenhum app é cortado (a lista rola).
    #[test]
    fn mixer_mantem_a_ordem_das_linhas() {
        let order: Vec<String> = ["system", "a", "b"].iter().map(|s| s.to_string()).collect();
        // A thread pôs "b" (que começou a tocar) e o novo "n" na frente.
        let mut apps = vec![app("n", false), app("b", false), app("system", true), app("a", false)];
        mix_arrange(&mut apps, &order);
        assert_eq!(keys(&apps), ["system", "a", "b", "n"]);
        let mut apps: Vec<MixApp> = (0..12).map(|i| app(&format!("p{i}"), false)).collect();
        apps.push(app("system", true));
        mix_arrange(&mut apps, &[]);
        assert_eq!((apps[0].key.as_str(), apps.len()), ("system", 13));
    }
}
