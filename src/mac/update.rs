//! Versão nova no Mac: um aviso que abre a página da release.
//!
//! Por enquanto o app não se troca sozinho: sem conta de desenvolvedor da
//! Apple, um `.app` trocado por ele mesmo esbarra no Gatekeeper (e as
//! permissões podem ser pedidas de novo). A pessoa baixa o `.dmg` na página.
//!
//! O aviso é um `NSPanel` próprio, **não modal** (B12). O `runModal` de antes
//! rodava dentro de um bloco da fila principal: enquanto o aviso estava aberto,
//! a fila (serial) ficava presa, os outros avisos do app esperavam e o laço
//! modal ignorava a ilha. Agora `show` volta logo e a escolha chega pelos
//! botões (`Target`, em `mod.rs`).

use crate::i18n::tr;
use crate::update::{Offer, State, Version, log};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{NSBackingStoreType, NSButton, NSFloatingWindowLevel, NSFont, NSPanel, NSTextField, NSView, NSWindowStyleMask, NSWorkspace};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSNotification, NSRect, NSString, NSURL};
use std::cell::RefCell;

/// O que a pessoa escolheu no aviso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Open,
    /// "Depois", e também fechar pelo x ou pelo Esc.
    Later,
    Skip,
}

thread_local! {
    /// O aviso aberto e a sua oferta, até a escolha. Também é a guarda contra
    /// uma segunda oferta com o aviso na tela.
    static OPEN: RefCell<Option<(Retained<NSPanel>, Offer)>> = const { RefCell::new(None) };
    /// O último aviso fechado. Fica guardado até o próximo: soltar a janela de
    /// dentro do próprio `close` (ou da ação de um botão dela) não é seguro.
    static CLOSED: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

/// Chamado da thread da atualização: o aviso abre na thread principal.
pub fn offer(o: Offer) {
    dispatch2::DispatchQueue::main().exec_async(move || show(o));
}

/// "Procurar agora" ainda não existe no Mac: só o registro.
pub fn tell(result: Result<(), String>) {
    if let Err(e) = result {
        log(&format!("procura manual: {e}"));
    }
}

const W: f64 = 420.0;
const PAD: f64 = 20.0;
/// As notas da release podem ser longas: o aviso não passa disso.
const MAX_INFO_H: f64 = 320.0;

fn show(o: Offer) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    if OPEN.with(|s| s.borrow().is_some()) {
        log(&format!("versão {}: já há um aviso aberto", o.version));
        return;
    }
    let panel = build(mtm, &o);
    OPEN.with(|s| *s.borrow_mut() = Some((panel.clone(), o)));
    super::compat::activate(mtm);
    panel.makeKeyAndOrderFront(None);
}

fn label(mtm: MainThreadMarker, text: &str, wrap: bool) -> Retained<NSTextField> {
    let s = NSString::from_str(text);
    if wrap { NSTextField::wrappingLabelWithString(&s, mtm) } else { NSTextField::labelWithString(&s, mtm) }
}

fn button(mtm: MainThreadMarker, text: &'static str, action: objc2::runtime::Sel, key: &str) -> Retained<NSButton> {
    let target = super::target();
    let b = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(tr(text)), Some(&target), Some(action), mtm) };
    if !key.is_empty() {
        b.setKeyEquivalent(&NSString::from_str(key));
    }
    b.sizeToFit();
    b
}

fn build(mtm: MainThreadMarker, o: &Offer) -> Retained<NSPanel> {
    let title = label(mtm, &tr("Taskbeat {} disponível").replace("{}", &o.version.to_string()), false);
    title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    let mut text = tr("Você está na versão {}").replace("{}", &Version::current().to_string());
    if !o.notes.is_empty() {
        text = format!("{}\n\n{text}", o.notes);
    }
    let info = label(mtm, &text, true);
    info.setSelectable(true);
    info.setPreferredMaxLayoutWidth(W - 2.0 * PAD);
    let info_h = info.sizeThatFits(CGSize::new(W - 2.0 * PAD, 10_000.0)).height.clamp(16.0, MAX_INFO_H);
    // Enter abre a página; Esc é "Depois".
    let open = button(mtm, "Abrir página", sel!(updateOpen:), "\r");
    let later = button(mtm, "Depois", sel!(updateLater:), "\u{1b}");
    let skip = button(mtm, "Pular esta versão", sel!(updateSkip:), "");

    let btn_h = open.frame().size.height.max(28.0);
    let h = PAD + btn_h + 16.0 + info_h + 8.0 + 18.0 + PAD;
    let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(W, h)));
    title.setFrame(NSRect::new(CGPoint::new(PAD, h - PAD - 18.0), CGSize::new(W - 2.0 * PAD, 18.0)));
    info.setFrame(NSRect::new(CGPoint::new(PAD, PAD + btn_h + 16.0), CGSize::new(W - 2.0 * PAD, info_h)));
    // Botões da direita para a esquerda, como num alerta do sistema.
    let mut x = W - PAD;
    for b in [&open, &later] {
        let bw = b.frame().size.width.max(90.0);
        x -= bw;
        b.setFrame(NSRect::new(CGPoint::new(x, PAD), CGSize::new(bw, btn_h)));
        x -= 8.0;
    }
    let sw = skip.frame().size.width;
    skip.setFrame(NSRect::new(CGPoint::new(PAD, PAD), CGSize::new(sw, btn_h)));
    for v in [&*title as &NSView, &info, &open, &later, &skip] {
        view.addSubview(v);
    }

    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::UtilityWindow;
    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(NSPanel::alloc(mtm), NSRect::new(CGPoint::new(0.0, 0.0), CGSize::new(W, h)), style, NSBackingStoreType::Buffered, false);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setTitle(&NSString::from_str("Taskbeat"));
    panel.setLevel(NSFloatingWindowLevel);
    // Sem isso o aviso some quando a pessoa clica em outro app, e sem ícone no
    // Dock não haveria como trazê-lo de volta.
    panel.setHidesOnDeactivate(false);
    let target = super::target();
    panel.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    panel.setContentView(Some(&view));
    panel.center();
    panel
}

/// Um dos botões do aviso.
pub(super) fn choose(c: Choice) {
    let Some((panel, o)) = OPEN.with(|s| s.borrow_mut().take()) else { return };
    apply(c, &o);
    // O `windowWillClose:` chega já sem aviso aberto e não faz nada.
    panel.close();
    CLOSED.with(|s| *s.borrow_mut() = Some(panel));
}

/// `windowWillClose:` do aviso (o x da janela ou o Esc): conta como "Depois".
pub(super) fn window_closed(n: &NSNotification) {
    let obj = n.object();
    let mine = OPEN.with(|s| {
        s.borrow().as_ref().is_some_and(|(p, _)| obj.as_deref().is_some_and(|w: &AnyObject| std::ptr::eq(w, Retained::as_ptr(p).cast::<AnyObject>())))
    });
    if !mine {
        return;
    }
    if let Some((panel, o)) = OPEN.with(|s| s.borrow_mut().take()) {
        apply(Choice::Later, &o);
        CLOSED.with(|s| *s.borrow_mut() = Some(panel));
    }
}

fn apply(c: Choice, o: &Offer) {
    match c {
        Choice::Open => {
            if o.page.starts_with("https://github.com/Lu1sGabriel/taskbeat/")
                && let Some(u) = NSURL::URLWithString(&NSString::from_str(&o.page))
            {
                NSWorkspace::sharedWorkspace().openURL(&u);
            }
            log(&format!("versão {}: página aberta", o.version));
        }
        Choice::Later => log(&format!("versão {}: depois", o.version)),
        Choice::Skip => {
            let mut st = State::load();
            st.skipped = o.version.to_string();
            st.save();
            log(&format!("versão {} pulada", o.version));
        }
    }
}
