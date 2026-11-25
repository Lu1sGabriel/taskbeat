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
//! No YouTube Music o aleatório não é um modo: cada clique embaralha a fila de
//! novo. O repetir é um ciclo: desativado → tudo → uma.
//!
//! Roda numa thread própria: o navegador pode levar alguns segundos para
//! responder à primeira consulta (ele só monta a árvore de acessibilidade
//! quando alguém pede).

use crate::audio::log;
use crate::shared::{Shared, WebCmd, WebState};
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
        let mut w = Worker { uia, found: None, miss: None, assumed: None };
        while shared.is_running() {
            let cmd = match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let state = w.run(&cmd);
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
        match cmd {
            WebCmd::Shuffle { .. } => {
                let ok = shuffle.as_ref().is_some_and(|b| invoke(b, hwnd));
                log::info(&format!("aleatório pela página do player ({aumid}): {ok} em {:?}", t0.elapsed()));
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
            WebCmd::Probe { .. } => {}
        }
        let repeat = repeat.as_ref().and_then(|b| self.repeat_now(aumid, &name(b)));
        WebState { aumid: aumid.into(), shuffle: shuffle.is_some(), repeat }
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

/// A música da barra do player é a da sessão (sem diferenciar maiúsculas).
fn same_title(page: &str, session: &str) -> bool {
    let (a, b) = (page.trim().to_lowercase(), session.trim().to_lowercase());
    !a.is_empty() && !b.is_empty() && (a == b || a.contains(&b) || b.contains(&a))
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
    use super::{repeat_state, same_title};

    /// Numa aba, só aperta se a barra do player mostrar a música da sessão
    /// (lida de verdade: "Thanatos" na barra e no Windows).
    #[test]
    fn aba_so_com_a_mesma_musica() {
        assert!(same_title("Thanatos", "Thanatos"));
        assert!(same_title(" thanatos ", "Thanatos"));
        assert!(!same_title("Thanatos", "Midnight City"));
        assert!(!same_title("", "Thanatos"));
        assert!(!same_title("Thanatos", ""));
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
        let mut w = super::Worker { uia, found: None, miss: None, assumed: None };
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let st = w.run(&crate::shared::WebCmd::Probe { aumid: aumid.clone(), title: title.clone() });
            println!("{aumid} \"{title}\" → {st:?} em {:?}", t.elapsed());
        }
    }
}
