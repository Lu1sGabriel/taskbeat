//! Aleatório e repetir em players de navegador (o YouTube Music instalado pelo
//! Brave, Chrome ou Edge, ou aberto numa aba).
//!
//! O Windows (SMTC) não deixa mudar isso nesses players: o navegador não
//! publica essas ações. Mas a página expõe os próprios botões pela UI
//! Automation (a mesma interface dos leitores de tela). Aqui eles são
//! "apertados" em segundo plano, sem trazer a janela para a frente, e o estado
//! do repetir é lido do nome do botão ("Repetição desativada", "Repetir
//! tudo", "Repetir uma"…). O elemento em volta de cada botão tem uma classe
//! que não muda com o idioma (`repeat style-scope ytmusic-player-bar`).
//!
//! O aleatório do YouTube Music liga e desliga, mas a página não conta isso à
//! acessibilidade (o botão se chama sempre "Ordem aleatória"): o ícone fica
//! branco ligado e cinza desligado, e é essa cor que é lida, num retrato da
//! janela (`PrintWindow`, sem trazê-la para a frente). O repetir é um ciclo
//! (desativado → tudo → uma), lido do nome do botão.
//!
//! Roda numa thread própria: o navegador pode levar alguns segundos para
//! responder à primeira consulta (ele só monta a árvore de acessibilidade
//! quando alguém pede).
//!
//! Com a extensão do Taskbeat instalada (`webext.rs`, pasta `extension/`), o
//! estado e os comandos vêm dela, lidos da própria página, sem UI Automation
//! nem cor de pixel. Tudo acima continua valendo sem ela (ou quando ela não
//! conhece a música da sessão).

use crate::audio::log;
use crate::shared::{Shared, WebCmd, WebState};
use crate::webext;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern, TreeScope_Descendants, UIA_ClassNamePropertyId,
    UIA_InvokePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow};

const SHUFFLE_CLASS: &str = "shuffle style-scope ytmusic-player-bar";
const REPEAT_CLASS: &str = "repeat style-scope ytmusic-player-bar";
/// Título da música na barra do player da página.
const TITLE_CLASS: &str = "title style-scope ytmusic-player-bar";
/// Sem botões numa janela: não procura de novo nela por este tempo.
const MISS_TTL: Duration = Duration::from_secs(60);
/// Com a extensão conectada, a música nova dela pode chegar um pouco depois
/// da do Windows: espera isto antes de ler a página pela UI Automation.
const EXT_WAIT: Duration = Duration::from_millis(1500);

pub fn spawn(shared: Arc<Shared>) {
    let (tx, rx) = channel::<WebCmd>();
    *shared.web_tx.lock().unwrap() = Some(tx);
    let _ = std::thread::Builder::new().name("web-player".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        crate::system::background_thread();
        let Ok(uia) = (unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }) else {
            log::warn("UI Automation indisponível: aleatório/repetir de navegador desligados");
            return;
        };
        let mut w = Worker { uia, found: None, miss: None, assumed: None, shuffle_on: None, shuffle_read: None, shuffle_assumed: None };
        let mut ext = ExtRoute::default();
        while shared.is_running() {
            let wait = ext.waiting.as_ref().map_or(Duration::from_secs(1), |(_, _, at)| at.saturating_duration_since(Instant::now()));
            let state = match rx.recv_timeout(wait) {
                Ok(cmd) => ext.handle(&mut w, &shared, cmd),
                Err(RecvTimeoutError::Timeout) => ext.expired(&mut w),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let Some(state) = state else { continue };
            let mut cur = shared.web.lock().unwrap();
            if *cur != state {
                *cur = state;
                drop(cur);
                // O overlay relê a música (e junta isto a ela).
                shared.media_version.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

/// Decide entre a extensão e a UI Automation para cada pedido.
#[derive(Default)]
struct ExtRoute {
    /// Leitura pedida para uma música que a extensão ainda não mostrou:
    /// (sessão, título, até quando esperar por ela).
    waiting: Option<(String, String, Instant)>,
    /// Sessão cujo estado publicado veio da extensão.
    from_ext: Option<String>,
}

/// O estado da aba que a extensão contou, no formato de `WebState`.
fn ext_state(aumid: &str, p: &webext::ExtPlayer) -> WebState {
    WebState { aumid: aumid.into(), shuffle: p.shuffle.is_some(), shuffle_on: p.shuffle, repeat: p.repeat }
}

impl ExtRoute {
    /// `None`: nada a publicar agora.
    fn handle(&mut self, w: &mut Worker, shared: &Shared, cmd: WebCmd) -> Option<WebState> {
        match cmd {
            // A extensão mandou algo: vale para a música que toca agora.
            WebCmd::Ext => {
                let (aumid, title, needed) = {
                    let m = shared.media.lock().unwrap();
                    (m.aumid.clone(), m.title.clone(), m.present() && !(m.can_shuffle && m.can_repeat))
                };
                if !needed {
                    return None;
                }
                if let Some(p) = webext::lookup(&aumid, &title) {
                    return Some(self.use_ext(&aumid, &p));
                }
                // A extensão contava esta sessão e não conta mais (aba fechada,
                // página mudou): volta à leitura pela UI Automation.
                if self.from_ext.as_deref() == Some(aumid.as_str()) && self.waiting.is_none() {
                    self.from_ext = None;
                    log::info(&format!("extensão não conta mais \"{title}\" ({aumid}): lendo a página pela UI Automation"));
                    return Some(w.run(&WebCmd::Probe { aumid, title }));
                }
                None
            }
            WebCmd::Probe { ref aumid, ref title } => {
                if let Some(p) = webext::lookup(aumid, title) {
                    return Some(self.use_ext(aumid, &p));
                }
                if webext::has_players(aumid) {
                    self.waiting = Some((aumid.clone(), title.clone(), Instant::now() + EXT_WAIT));
                    return None;
                }
                self.from_ext = None;
                Some(w.run(&cmd))
            }
            WebCmd::Shuffle { ref aumid, ref title } | WebCmd::Repeat { ref aumid, ref title } => {
                let name = if matches!(cmd, WebCmd::Shuffle { .. }) { "shuffle" } else { "repeat" };
                // Pela extensão: o estado novo chega quando a página mudar.
                if webext::lookup(aumid, title).is_some_and(|p| webext::command(&p, name)) {
                    return None;
                }
                self.from_ext = None;
                Some(w.run(&cmd))
            }
        }
    }

    /// Acabou a espera pela extensão: lê pela UI Automation.
    fn expired(&mut self, w: &mut Worker) -> Option<WebState> {
        let (aumid, title, at) = self.waiting.take()?;
        if Instant::now() < at {
            self.waiting = Some((aumid, title, at));
            return None;
        }
        if let Some(p) = webext::lookup(&aumid, &title) {
            return Some(self.use_ext(&aumid, &p));
        }
        log::info(&format!("extensão sem a música \"{title}\" ({aumid}): lendo a página pela UI Automation"));
        self.from_ext = None;
        Some(w.run(&WebCmd::Probe { aumid, title }))
    }

    fn use_ext(&mut self, aumid: &str, p: &webext::ExtPlayer) -> WebState {
        self.waiting = None;
        self.from_ext = Some(aumid.into());
        ext_state(aumid, p)
    }
}

/// Botões achados na página do player de uma sessão.
struct Found {
    aumid: String,
    hwnd: HWND,
    shuffle: Option<IUIAutomationElement>,
    repeat: Option<IUIAutomationElement>,
    /// Título da música na barra do player da página (numa aba, confere que é
    /// a mesma música da sessão antes de apertar).
    title: Option<IUIAutomationElement>,
}

struct Worker {
    uia: IUIAutomation,
    found: Option<Found>,
    /// Sessão e janela em que os botões não existem, e desde quando.
    miss: Option<(String, Instant)>,
    /// Clique no repetir que a página ainda não mostrou: (sessão, nome do
    /// botão na hora do clique, estado esperado).
    assumed: Option<(String, String, u8)>,
    /// Estado do aleatório que vale (sessão, ligado) e quando a página foi
    /// lida por último (o retrato da janela coberta é caro: não a cada abertura).
    shuffle_on: Option<(String, bool)>,
    shuffle_read: Option<Instant>,
    /// Clique no aleatório que a página ainda não mostrou: (sessão, o que a
    /// página mostrava na hora do clique, estado esperado). Vale até a página
    /// mostrar outra coisa (com a janela coberta, o clique pode só ser
    /// aplicado quando ela aparecer).
    shuffle_assumed: Option<(String, Option<bool>, bool)>,
}

/// Próximo estado do repetir no ciclo do YouTube Music: desativado → tudo → uma.
pub fn next_repeat(r: u8) -> u8 {
    match r {
        0 => 2,
        2 => 1,
        _ => 0,
    }
}

impl Worker {
    fn run(&mut self, cmd: &WebCmd) -> WebState {
        let (aumid, title) = match cmd {
            WebCmd::Probe { aumid, title } | WebCmd::Shuffle { aumid, title } | WebCmd::Repeat { aumid, title } => (aumid.as_str(), title.as_str()),
            // Só a extensão usa (veja `ExtRoute`).
            WebCmd::Ext => return WebState::default(),
        };
        let t0 = Instant::now();
        let Some(f) = self.find(aumid, title) else {
            if !matches!(cmd, WebCmd::Probe { .. }) {
                // Não deu pela página: traz o player para a frente (último recurso).
                crate::launchers::focus_player(aumid, title);
            }
            return WebState { aumid: aumid.into(), ..Default::default() };
        };
        let (hwnd, shuffle, repeat) = (f.hwnd, f.shuffle.clone(), f.repeat.clone());
        // O último estado que vale desta sessão (sem leitura nova, fica ele).
        let last = self.shuffle_on.as_ref().filter(|(a, _)| a == aumid).map(|(_, on)| *on);
        let fresh = last.is_some() && self.shuffle_read.is_some_and(|t| t.elapsed() < Duration::from_secs(30));
        // Lê a página: à vista é barato (da tela); coberta, o retrato da janela
        // inteira só se não houver leitura recente (ou depois de um clique).
        let read = |print: bool| shuffle.as_ref().and_then(|b| shuffle_lit(hwnd, b, print));
        let mut seen = None;
        match cmd {
            WebCmd::Shuffle { .. } => {
                let before = last;
                let ok = shuffle.as_ref().is_some_and(|b| invoke(b, hwnd));
                // Espera o ícone mudar de cor (o estado novo): no máximo duas leituras.
                for wait in [300, 500] {
                    if !ok {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(wait));
                    seen = read(true);
                    if seen.is_some() && seen != before {
                        break;
                    }
                }
                // A página não mostrou a mudança (minimizada, coberta, lenta):
                // vale o inverso do que estava, até ela mostrar outra coisa.
                if ok && (seen.is_none() || seen == before) {
                    self.shuffle_assumed = Some((aumid.to_string(), seen, !before.unwrap_or(false)));
                }
                log::info(&format!("aleatório pela página do player ({aumid}): {ok}, {before:?} → {seen:?} em {:?}", t0.elapsed()));
            }
            WebCmd::Repeat { .. } => {
                if let Some(b) = &repeat {
                    let before = name(b);
                    let base = self.repeat_now(aumid, &before);
                    let ok = invoke(b, hwnd);
                    // Espera o botão mudar de nome (o estado novo) para publicar.
                    let t = Instant::now();
                    while ok && t.elapsed() < Duration::from_millis(1500) && name(b) == before {
                        std::thread::sleep(Duration::from_millis(60));
                    }
                    let after = name(b);
                    // Com a janela do player escondida, a página pode só redesenhar
                    // (e trocar o nome do botão) quando aparecer: vale o próximo do
                    // ciclo até o nome mudar.
                    if ok && after == before {
                        self.assumed = base.map(|r| (aumid.to_string(), before.clone(), next_repeat(r)));
                    }
                    log::info(&format!("repetir pela página do player ({aumid}): {ok}, \"{before}\" → \"{after}\" em {:?}", t0.elapsed()));
                }
            }
            WebCmd::Probe { .. } | WebCmd::Ext => seen = read(!fresh),
        }
        if seen.is_some() {
            self.shuffle_read = Some(Instant::now());
        }
        let shuffle_on = self.shuffle_now(aumid, seen, last);
        if let Some(on) = shuffle_on {
            self.shuffle_on = Some((aumid.to_string(), on));
        }
        let repeat = repeat.as_ref().and_then(|b| self.repeat_now(aumid, &name(b)));
        WebState { aumid: aumid.into(), shuffle: shuffle.is_some(), shuffle_on, repeat }
    }

    /// Estado do aleatório: o esperado depois de um clique que a página ainda
    /// não mostrou; senão o lido agora; sem leitura, o último que valia.
    fn shuffle_now(&mut self, aumid: &str, seen: Option<bool>, last: Option<bool>) -> Option<bool> {
        shuffle_state(&mut self.shuffle_assumed, aumid, seen, last)
    }

    /// Estado do repetir: o do nome do botão, ou o esperado depois de um
    /// clique que a página ainda não mostrou.
    fn repeat_now(&mut self, aumid: &str, name: &str) -> Option<u8> {
        match &self.assumed {
            Some((a, n, r)) if a == aumid && n == name => Some(*r),
            _ => {
                self.assumed = None;
                repeat_state(name)
            }
        }
    }

    /// Botões do player da sessão `aumid` (guardados enquanto a janela e os
    /// elementos continuam valendo).
    fn find(&mut self, aumid: &str, title: &str) -> Option<&Found> {
        let alive = |f: &Found| unsafe { IsWindow(Some(f.hwnd)).as_bool() } && f.repeat.as_ref().or(f.shuffle.as_ref()).is_some_and(|e| unsafe { e.CurrentName() }.is_ok());
        // Numa aba (a sessão é o navegador), a página guardada só vale
        // enquanto a barra do player dela ainda mostra a música da sessão:
        // outra aba pode ter começado a tocar.
        let app = is_ytm_app(aumid);
        let same_song = |f: &Found| app || f.title.as_ref().is_some_and(|e| same_title(&name(e), title));
        if self.found.as_ref().is_some_and(|f| f.aumid == aumid && alive(f) && same_song(f)) {
            return self.found.as_ref();
        }
        self.found = None;
        if self.miss.as_ref().is_some_and(|(a, t)| a == aumid && t.elapsed() < MISS_TTL) {
            return None;
        }
        let t0 = Instant::now();
        let found = if app {
            // O app do YouTube Music: a janela dele (mesmo id).
            crate::launchers::player_window_exact(aumid, title).and_then(|hwnd| self.scan(aumid, hwnd))
        } else {
            // Numa aba: das janelas do navegador com o YouTube Music na frente,
            // só a que mostra na barra do player a música que o Windows diz
            // estar tocando. Com uma aba de outro site tocando e o YouTube
            // Music aberto noutra janela, nada é apertado (a fila dele fica
            // como está). Só o YouTube Music tem esses botões: outras páginas
            // nem são lidas, para não ligar a acessibilidade do navegador à toa.
            crate::launchers::app_windows(aumid)
                .into_iter()
                .filter(|(_, t)| t.contains("YouTube Music"))
                .find_map(|(hwnd, _)| self.scan(aumid, hwnd).filter(|f| f.title.as_ref().is_some_and(|e| same_title(&name(e), title))))
        };
        let Some(found) = found else {
            log::info(&format!("botões da página do player ({aumid}): não achados em {:?}", t0.elapsed()));
            self.miss = Some((aumid.into(), Instant::now()));
            return None;
        };
        log::info(&format!("botões da página do player ({aumid}): aleatório={} repetir={} em {:?}", found.shuffle.is_some(), found.repeat.is_some(), t0.elapsed()));
        self.miss = None;
        self.found = Some(found);
        self.found.as_ref()
    }

    /// Botões e título da barra do player na página da janela `hwnd`.
    fn scan(&self, aumid: &str, hwnd: HWND) -> Option<Found> {
        let root = unsafe { self.uia.ElementFromHandle(hwnd) }.ok()?;
        let (mut shuffle, mut repeat) = (None, None);
        // O navegador monta a árvore da página só depois do primeiro pedido.
        for attempt in 0..2 {
            shuffle = self.button(&root, SHUFFLE_CLASS);
            repeat = self.button(&root, REPEAT_CLASS);
            if shuffle.is_some() || repeat.is_some() || attempt == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(700));
        }
        if shuffle.is_none() && repeat.is_none() {
            return None;
        }
        let title = unsafe {
            self.uia
                .CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(TITLE_CLASS))
                .ok()
                .and_then(|c| root.FindFirst(TreeScope_Descendants, &c).ok())
        };
        Some(Found { aumid: aumid.into(), hwnd, shuffle, repeat, title })
    }

    /// O botão dentro do elemento com a classe `class`.
    fn button(&self, root: &IUIAutomationElement, class: &str) -> Option<IUIAutomationElement> {
        unsafe {
            let cond = self.uia.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(class)).ok()?;
            let group = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
            let walker = self.uia.RawViewWalker().ok()?;
            let first = walker.GetFirstChildElement(&group).ok();
            // O próprio grupo também aceita o clique, se o botão não vier.
            Some(first.filter(|e| e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).is_ok()).unwrap_or(group))
        }
    }
}

/// O aleatório está ligado na página? O ícone fica branco ligado e cinza
/// desligado (medido no YouTube Music: ligado, dezenas de pixels em 255;
/// desligado, nenhum acima de ~150). Lê só a área do botão: direto da tela
/// quando ele está à vista (barato, sem pedir nada ao navegador), ou pelo
/// retrato que o próprio navegador faz (`PrintWindow`, funciona com a janela
/// coberta), recortado numa imagem do tamanho do botão. `None`: minimizada
/// ou sem retrato.
fn shuffle_lit(hwnd: HWND, button: &IUIAutomationElement, print: bool) -> Option<bool> {
    let r = unsafe { button.CurrentBoundingRectangle() }.ok()?;
    let px = grab(hwnd, r, print)?;
    Some(bright_count(&px, r.right - r.left, r.bottom - r.top) >= 8)
}

/// Pixels claros (os três canais ≥ 215) no miolo de uma imagem `w`×`h` do
/// botão (sem a borda, onde fica o anel de foco).
fn bright_count(px: &[u32], w: i32, h: i32) -> usize {
    let (x0, x1, y0, y1) = (w / 5, w - w / 5, h / 5, h - h / 5);
    let mut n = 0;
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w) {
            let p = px[(y * w + x) as usize];
            if ((p >> 16) & 0xff).min((p >> 8) & 0xff).min(p & 0xff) >= 215 {
                n += 1;
            }
        }
    }
    n
}

/// Regra do estado do aleatório (ver `Worker::shuffle_now`): `assumed` é o
/// clique ainda não mostrado (sessão, o que a página mostrava, esperado).
fn shuffle_state(assumed: &mut Option<(String, Option<bool>, bool)>, aumid: &str, seen: Option<bool>, last: Option<bool>) -> Option<bool> {
    if let Some((a, at_click, expected)) = assumed.as_ref() {
        if a == aumid && (seen.is_none() || seen == *at_click) {
            return Some(*expected);
        }
        *assumed = None;
    }
    seen.or(last)
}

/// Os pixels do retângulo `r` (tela) da janela `hwnd`: da tela, se o botão
/// estiver à vista (≈5 ms, sem pedir nada ao navegador); senão, com `print`,
/// do retrato da janela inteira (`button_pixels_full`: o `PrintWindow` do
/// conteúdo do navegador ignora a origem deslocada, então não dá para
/// recortar antes; ≈28 ms e ~8 MB por um instante). Sem `print`: `None`.
fn grab(hwnd: HWND, r: windows::Win32::Foundation::RECT, print: bool) -> Option<Vec<u32>> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, IsIconic, WindowFromPoint};
    unsafe {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        if IsIconic(hwnd).as_bool() || w <= 0 || h <= 0 || w > 400 || h > 400 {
            return None;
        }
        let center = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
        // À vista: na tela, a janela de cima nesses pontos é a do player e
        // nenhuma janela visível acima dela cruza o botão (o `WindowFromPoint`
        // não vê as que deixam o clique passar, como o "A seguir" e o aviso de
        // teclas do próprio Taskbeat, ou overlays do Discord, Steam e NVIDIA:
        // a tela teria os pixels delas).
        let on_screen = [center, POINT { x: r.left + 2, y: r.top + 2 }, POINT { x: r.right - 3, y: r.bottom - 3 }]
            .iter()
            .all(|p| GetAncestor(WindowFromPoint(*p), GA_ROOT) == hwnd)
            && !covered_above(hwnd, r);
        if !on_screen {
            return if print { button_pixels_full(hwnd, r) } else { None };
        }
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            return None;
        };
        let old = SelectObject(dc, bmp.into());
        let ok = BitBlt(dc, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY).is_ok();
        let out = (ok && !bits.is_null()).then(|| std::slice::from_raw_parts(bits as *const u32, (w * h) as usize).to_vec());
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        out
    }
}

/// Alguma janela visível acima de `hwnd` (na ordem das janelas) cruza o
/// retângulo `r` da tela, inclusive as transparentes ao clique?
fn covered_above(hwnd: HWND, r: windows::Win32::Foundation::RECT) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{GW_HWNDPREV, GetWindow, GetWindowRect, IsIconic, IsWindowVisible};
    unsafe {
        let mut cur = hwnd;
        for _ in 0..1000 {
            let Ok(above) = GetWindow(cur, GW_HWNDPREV) else { return false };
            if above.is_invalid() {
                return false;
            }
            cur = above;
            if !IsWindowVisible(above).as_bool() || IsIconic(above).as_bool() {
                continue;
            }
            // Escondida pelo DWM (outra área de trabalho, app suspenso): não aparece.
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(above, DWMWA_CLOAKED, &mut cloaked as *mut u32 as _, 4);
            if cloaked != 0 {
                continue;
            }
            // O retângulo que aparece (sem as bordas invisíveis de redimensionar).
            let mut wr = RECT::default();
            if DwmGetWindowAttribute(above, DWMWA_EXTENDED_FRAME_BOUNDS, &mut wr as *mut RECT as _, std::mem::size_of::<RECT>() as u32).is_err() {
                let _ = GetWindowRect(above, &mut wr);
            }
            if wr.left < r.right && wr.right > r.left && wr.top < r.bottom && wr.bottom > r.top {
                return true;
            }
        }
        false
    }
}

/// Os pixels do retângulo `r` (tela) tirados do retrato da janela inteira
/// (funciona com ela coberta; não minimizada).
fn button_pixels_full(hwnd: HWND, r: windows::Win32::Foundation::RECT) -> Option<Vec<u32>> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    unsafe {
        let mut wr = RECT::default();
        GetWindowRect(hwnd, &mut wr).ok()?;
        let (ww, wh) = (wr.right - wr.left, wr.bottom - wr.top);
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: ww, biHeight: -wh, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bmp.into());
        let ok = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        let all = std::slice::from_raw_parts(bits as *const u32, (ww * wh) as usize);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (r.left - wr.left + x, r.top - wr.top + y);
                out.push(if sx >= 0 && sy >= 0 && sx < ww && sy < wh { all[(sy * ww + sx) as usize] } else { 0 });
            }
        }
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        ok.then_some(out)
    }
}

/// A música da barra do player é a da sessão: iguais, sem diferenciar
/// maiúsculas, com os espaços normalizados e sem a pontuação das pontas. "Contém" só vale para um título de 6 letras ou mais
/// que caia em fronteira de palavra (um sufixo como " - YouTube Music" ou
/// " (Official Video)"): "Intro" não bate com "Introduction", nem um título
/// de uma ou duas letras com quase tudo.
pub(crate) fn same_title(page: &str, session: &str) -> bool {
    let (a, b) = (norm_title(page), norm_title(session));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.chars().count() <= b.chars().count() { (&a, &b) } else { (&b, &a) };
    if short.chars().count() < 6 {
        return false;
    }
    long.match_indices(short.as_str()).any(|(i, m)| {
        let before = long[..i].chars().next_back();
        let after = long[i + m.len()..].chars().next();
        before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
    })
}

/// Minúsculas, espaços seguidos viram um, sem pontuação nem espaço nas pontas.
fn norm_title(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    s.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// A sessão é o app do YouTube Music instalado pelo navegador.
fn is_ytm_app(aumid: &str) -> bool {
    aumid.to_lowercase().contains("cinhimbnkkghhklpknlkffjgod")
}

fn name(e: &IUIAutomationElement) -> String {
    unsafe { e.CurrentName() }.map(|b| b.to_string()).unwrap_or_default()
}

/// Aperta o botão. O Chromium às vezes passa o primeiro plano para uma janela
/// escondida dele ao executar a ação: se isso acontecer, devolve para quem estava.
fn invoke(e: &IUIAutomationElement, player: HWND) -> bool {
    unsafe {
        let before = GetForegroundWindow();
        let ok = e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|p| p.Invoke()).is_ok();
        let mut browser = 0u32;
        GetWindowThreadProcessId(player, Some(&mut browser));
        for _ in 0..8 {
            let now = GetForegroundWindow();
            if now != before && !before.is_invalid() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(now, Some(&mut pid));
                if pid == browser {
                    let back = SetForegroundWindow(before).as_bool();
                    log::info(&format!("o navegador pegou o primeiro plano ao apertar o botão; devolvido: {back}"));
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        ok
    }
}

/// Estado do repetir pelo nome do botão, em qualquer idioma: "desativado" →
/// 0, "uma/one" → 1 (a música), o resto → 2 (a fila toda).
pub fn repeat_state(name: &str) -> Option<u8> {
    let n = name.trim().to_lowercase();
    if n.is_empty() {
        return None;
    }
    const OFF: &[&str] = &["off", "desativada", "desativado", "desactivada", "desactivado", "désactivée", "désactivé", "disattivata", "disattivato", "aus", "deaktiviert", "uit", "wyłączone", "kapalı", "выкл"];
    const ONE: &[&str] = &["one", "uma", "um", "una", "uno", "eine", "einen", "eins", "einzeln", "un", "une", "één", "jeden", "1"];
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| OFF.contains(w)) {
        Some(0)
    } else if words.iter().any(|w| ONE.contains(w)) {
        Some(1)
    } else {
        Some(2)
    }
}

#[cfg(test)]
mod tests {
    use super::{repeat_state, same_title, shuffle_state};

    /// A bolinha do aleatório: sem leitura, fica o último estado; depois de
    /// um clique que a página não mostrou, vale o inverso até ela mudar.
    #[test]
    fn estado_do_aleatorio() {
        let mut none = None;
        // Minimizada (sem leitura): continua ligado.
        assert_eq!(shuffle_state(&mut none, "yt", None, Some(true)), Some(true));
        assert_eq!(shuffle_state(&mut none, "yt", Some(false), Some(true)), Some(false));
        // Clique com a janela minimizada (leitura nenhuma): vale o inverso.
        let mut a = Some(("yt".to_string(), None, true));
        assert_eq!(shuffle_state(&mut a, "yt", None, Some(false)), Some(true));
        // Coberta: o retrato ainda mostra o de antes; continua o inverso.
        let mut b = Some(("yt".to_string(), Some(false), true));
        assert_eq!(shuffle_state(&mut b, "yt", Some(false), Some(false)), Some(true));
        // A página mudou: vale o que ela mostra, e o "esperado" some.
        assert_eq!(shuffle_state(&mut b, "yt", Some(true), Some(false)), Some(true));
        assert!(b.is_none());
        assert_eq!(shuffle_state(&mut b, "yt", Some(false), Some(true)), Some(false));
        // Outra sessão: o "esperado" não vale.
        let mut c = Some(("yt".to_string(), None, true));
        assert_eq!(shuffle_state(&mut c, "outra", None, None), None);
    }

    /// Numa aba, só aperta se a barra do player mostrar a música da sessão
    /// (lida de verdade: "Thanatos" na barra e no Windows).
    #[test]
    fn aba_so_com_a_mesma_musica() {
        assert!(same_title("Thanatos", "Thanatos"));
        assert!(same_title(" thanatos ", "Thanatos"));
        assert!(same_title("Thanatos.", "thanatos"));
        assert!(same_title("Midnight   City", "midnight city"));
        assert!(!same_title("Thanatos", "Midnight City"));
        assert!(!same_title("", "Thanatos"));
        assert!(!same_title("Thanatos", ""));
        // "Contém" só com 6+ letras e em fronteira de palavra.
        assert!(!same_title("Introduction", "Intro"));
        assert!(!same_title("A", "Alabama"));
        assert!(!same_title("Me", "Meet me halfway"));
        assert!(!same_title("Midnight Cityscape", "Midnight City"));
        assert!(!same_title("Midnights", "Midnight"));
        assert!(same_title("Midnight City - YouTube Music", "Midnight City"));
        assert!(same_title("Midnight City (Official Video)", "Midnight City"));
        assert!(same_title("Thanatos", "M83 - Thanatos"));
    }

    #[test]
    fn repetir_pelo_nome_do_botao() {
        // Nomes do YouTube Music (pt-BR lidos de verdade; os outros, os da página em cada idioma).
        assert_eq!(repeat_state("Repetição desativada"), Some(0));
        assert_eq!(repeat_state("Repetir tudo"), Some(2));
        assert_eq!(repeat_state("Repetir uma"), Some(1));
        assert_eq!(repeat_state("Repeat off"), Some(0));
        assert_eq!(repeat_state("Repeat all"), Some(2));
        assert_eq!(repeat_state("Repeat one"), Some(1));
        assert_eq!(repeat_state("Repetición desactivada"), Some(0));
        assert_eq!(repeat_state("Wiederholen: aus"), Some(0));
        assert_eq!(repeat_state(""), None);
    }

    /// Procura os botões na janela do player atual e lê o repetir (sem clicar).
    /// `cargo test --release web_probe_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn web_probe_here() {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        let m = Manager::RequestAsync().and_then(|op| op.join()).unwrap();
        let s = m.GetCurrentSession().unwrap();
        let aumid = s.SourceAppUserModelId().unwrap().to_string();
        let title = s.TryGetMediaPropertiesAsync().and_then(|op| op.join()).and_then(|p| p.Title()).map(|t| t.to_string()).unwrap_or_default();
        let uia = unsafe { windows::Win32::System::Com::CoCreateInstance::<_, super::IUIAutomation>(&super::CUIAutomation8, None, super::CLSCTX_INPROC_SERVER) }.unwrap();
        let mut w = super::Worker { uia, found: None, miss: None, assumed: None, shuffle_on: None, shuffle_read: None, shuffle_assumed: None };
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let st = w.run(&crate::shared::WebCmd::Probe { aumid: aumid.clone(), title: title.clone() });
            println!("{aumid} \"{title}\" → {st:?} em {:?}", t.elapsed());
        }
    }
}

#[cfg(test)]
mod covered {
    /// Uma janela transparente ao clique (como o "A seguir") por cima do
    /// botão: o `covered_above` a vê (o `WindowFromPoint` não).
    /// `cargo test --release covered_by_click_through -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn covered_by_click_through() {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::w;
        unsafe {
            let make = |ex: WINDOW_EX_STYLE, x: i32, y: i32, wd: i32, ht: i32| {
                let h = CreateWindowExW(ex | WS_EX_TOOLWINDOW, w!("STATIC"), w!("teste"), WS_POPUP | WS_VISIBLE, x, y, wd, ht, None, None, None, None).unwrap();
                let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
                h
            };
            let target = make(WS_EX_TOPMOST, 100, 100, 300, 200);
            let button = RECT { left: 200, top: 180, right: 240, bottom: 220 };
            let free = !super::covered_above(target, button);
            let overlay = make(WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT, 190, 170, 120, 80);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetLayeredWindowAttributes(overlay, windows::Win32::Foundation::COLORREF(0), 200, LWA_ALPHA);
            let _ = SetWindowPos(overlay, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let covered = super::covered_above(target, button);
            let elsewhere = super::covered_above(target, RECT { left: 110, top: 110, right: 140, bottom: 140 });
            let _ = DestroyWindow(overlay);
            let _ = DestroyWindow(target);
            println!("sem nada por cima: livre={free}; transparente por cima: coberto={covered}, fora dela coberto={elsewhere}");
            assert!(free && covered && !elsewhere);
        }
    }
}

#[cfg(test)]
mod crop {
    /// O recorte (tela ou PrintWindow com a origem deslocada) dá os mesmos
    /// pixels do botão que o retrato da janela inteira, e custa menos (só lê).
    /// `cargo test --release shuffle_crop_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn shuffle_crop_here() {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};
        // Janela Chromium à vista (TASKBEAT_CROP_TITLE, padrão "Claude").
        let title = std::env::var("TASKBEAT_CROP_TITLE").unwrap_or_else(|_| "Claude".into());
        unsafe {
            let hwnd = FindWindowW(windows::core::w!("Chrome_WidgetWin_1"), &windows::core::HSTRING::from(title.as_str())).unwrap();
            let mut wr = RECT::default();
            GetWindowRect(hwnd, &mut wr).unwrap();
            let (cx, cy) = ((wr.left + wr.right) / 2, (wr.top + wr.bottom) / 2);
            let r = RECT { left: cx - 20, top: cy - 20, right: cx + 20, bottom: cy + 20 };
            let full = super::button_pixels_full(hwnd, r).unwrap();
            let t = std::time::Instant::now();
            let px = super::grab(hwnd, r, false).expect("à vista");
            let diff = px.iter().zip(&full).filter(|(a, b)| (**a & 0xffffff) != (**b & 0xffffff)).count();
            println!("da tela: {:?}, pixels diferentes do retrato inteiro: {diff} de {}", t.elapsed(), px.len());
            let t = std::time::Instant::now();
            let _ = super::button_pixels_full(hwnd, r);
            println!("retrato da janela inteira: {:?} ({}x{})", t.elapsed(), wr.right - wr.left, wr.bottom - wr.top);
        }
    }
}

#[cfg(test)]
mod probe {
    /// Tudo que a UI Automation mostra dos botões de aleatório e repetir do
    /// YouTube Music (só lê). `cargo test --release ytm_buttons_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ytm_buttons_here() {
        use windows::Win32::System::Com::*;
        use windows::Win32::System::Variant::VARIANT;
        use windows::Win32::UI::Accessibility::*;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).unwrap();
            let root = uia.GetRootElement().unwrap();
            let cond = uia.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from("YouTube Music")).unwrap();
            let Ok(win) = root.FindFirst(TreeScope_Children, &cond) else { println!("sem janela"); return };
            let walker = uia.RawViewWalker().unwrap();
            for class in [super::SHUFFLE_CLASS, super::REPEAT_CLASS] {
                let c = uia.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(class)).unwrap();
                let Ok(g) = win.FindFirst(TreeScope_Descendants, &c) else { println!("[{class}] não achado"); continue };
                let mut stack = vec![(g, 0)];
                while let Some((e, depth)) = stack.pop() {
                    let s = |id| e.GetCurrentPropertyValue(id).map(|v| v.to_string()).unwrap_or_default();
                    let legacy = e.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId).ok();
                    let (lstate, lrole, ldesc, lval) = legacy.map(|l| (l.CurrentState().unwrap_or(0), l.CurrentRole().unwrap_or(0), l.CurrentDescription().map(|b| b.to_string()).unwrap_or_default(), l.CurrentValue().map(|b| b.to_string()).unwrap_or_default())).unwrap_or_default();
                    let toggle = e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).ok().map(|t| t.CurrentToggleState().map(|x| x.0).unwrap_or(-1));
                    println!(
                        "{}- nome='{}' classe='{}' aria='{}' estado=0x{lstate:x} papel={lrole} desc='{ldesc}' valor='{lval}' toggle={toggle:?} status='{}' full='{}' auto='{}'",
                        "  ".repeat(depth),
                        s(UIA_NamePropertyId),
                        s(UIA_ClassNamePropertyId),
                        s(UIA_AriaPropertiesPropertyId),
                        s(UIA_ItemStatusPropertyId),
                        s(UIA_FullDescriptionPropertyId),
                        s(UIA_AutomationIdPropertyId),
                    );
                    let mut kid = walker.GetFirstChildElement(&e).ok();
                    while let Some(k) = kid {
                        kid = walker.GetNextSiblingElement(&k).ok();
                        stack.push((k, depth + 1));
                    }
                }
            }
        }
    }
}
