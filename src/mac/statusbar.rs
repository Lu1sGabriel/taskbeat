//! Ícone na barra de menus e o menu do Taskbeat (também no botão direito da ilha).

use super::{shared, system, target};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSView};
use objc2_foundation::{NSPoint, NSString};
use std::cell::RefCell;

const TAG_NOW: isize = 1;
const TAG_PLAY: isize = 2;
const TAG_HOVER: isize = 10;
const TAG_VIZ: isize = 11;
const TAG_AUTOSTART: isize = 20;

thread_local! {
    static ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    static MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
}

fn tr(s: &'static str) -> Retained<NSString> {
    NSString::from_str(crate::i18n::tr(s))
}

fn item(mtm: MainThreadMarker, title: &'static str, action: Option<Sel>, key: &str, tag: isize) -> Retained<NSMenuItem> {
    let it = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &tr(title), action, &NSString::from_str(key)) };
    if action.is_some() {
        let t = target();
        unsafe { it.setTarget(Some(&t)) };
    }
    it.setTag(tag);
    it
}

fn build(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    let t = target();
    menu.setDelegate(Some(ProtocolObject::from_ref(&*t)));
    let now = item(mtm, "Nada tocando", None, "", TAG_NOW);
    now.setEnabled(false);
    menu.addItem(&now);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(mtm, "Tocar / pausar", Some(sel!(playPause:)), "", TAG_PLAY));
    menu.addItem(&item(mtm, "Próxima", Some(sel!(nextTrack:)), "", 0));
    menu.addItem(&item(mtm, "Anterior", Some(sel!(previousTrack:)), "", 0));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(mtm, "Abrir ao passar o mouse", Some(sel!(toggleOption:)), "", TAG_HOVER));
    menu.addItem(&item(mtm, "Barras de áudio dinâmicas", Some(sel!(toggleOption:)), "", TAG_VIZ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(mtm, "Configurações…", Some(sel!(openSettings:)), ",", 0));
    menu.addItem(&item(mtm, "Abrir ao iniciar a sessão", Some(sel!(toggleAutostart:)), "", TAG_AUTOSTART));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&item(mtm, "Sair do Taskbeat", Some(sel!(quit:)), "q", 0));
    menu
}

fn menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    MENU.with(|m| m.borrow_mut().get_or_insert_with(|| build(mtm)).clone())
}

pub fn install(mtm: MainThreadMarker) {
    config_changed_with(mtm);
}

/// Mostra ou tira o ícone conforme a opção.
pub fn config_changed() {
    if let Some(mtm) = MainThreadMarker::new() {
        config_changed_with(mtm);
    }
}

fn config_changed_with(mtm: MainThreadMarker) {
    let want = shared().config().menu_bar_icon;
    ITEM.with(|i| {
        let mut i = i.borrow_mut();
        match (want, i.is_some()) {
            (true, false) => {
                let it = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
                if let Some(b) = it.button(mtm) {
                    if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("music.note"), Some(&NSString::from_str("Taskbeat"))) {
                        img.setTemplate(true);
                        b.setImage(Some(&img));
                    }
                    b.setToolTip(Some(&NSString::from_str("Taskbeat")));
                }
                it.setMenu(Some(&menu(mtm)));
                *i = Some(it);
            }
            (false, true) => {
                if let Some(it) = i.take() {
                    NSStatusBar::systemStatusBar().removeStatusItem(&it);
                }
            }
            _ => {}
        }
    });
}

/// Atualiza a música e as marcas antes de o menu abrir.
pub fn refresh(menu: &NSMenu) {
    let sh = shared();
    let m = sh.media.lock().unwrap().clone();
    let cfg = sh.config();
    let state = |on: bool| if on { NSControlStateValueOn } else { NSControlStateValueOff };
    if let Some(it) = menu.itemWithTag(TAG_NOW) {
        let title = if !m.present() {
            crate::i18n::tr("Nada tocando").to_string()
        } else if m.artist.is_empty() {
            m.title.clone()
        } else {
            format!("{} — {}", m.title, m.artist)
        };
        let title: String = if title.chars().count() > 48 { title.chars().take(47).chain(std::iter::once('…')).collect() } else { title };
        it.setTitle(&NSString::from_str(&title));
    }
    if let Some(it) = menu.itemWithTag(TAG_PLAY) {
        it.setTitle(&tr(if m.playing { "Pausar" } else { "Tocar" }));
        it.setEnabled(m.present());
    }
    if let Some(it) = menu.itemWithTag(TAG_HOVER) {
        it.setState(state(cfg.notch_hover));
    }
    if let Some(it) = menu.itemWithTag(TAG_VIZ) {
        it.setState(state(cfg.show_visualizer));
    }
    if let Some(it) = menu.itemWithTag(TAG_AUTOSTART) {
        it.setState(state(system::autostart_enabled()));
    }
}

/// Liga/desliga uma opção direto pelo menu.
pub fn toggle_option(sender: Option<&AnyObject>) {
    let Some(it) = sender.and_then(|s| s.downcast_ref::<NSMenuItem>()) else { return };
    let sh = shared();
    match it.tag() {
        TAG_HOVER => sh.update_config(|c| c.notch_hover = !c.notch_hover),
        TAG_VIZ => sh.update_config(|c| c.show_visualizer = !c.show_visualizer),
        _ => return,
    }
    super::config_changed();
    super::settings::refresh();
}

/// Menu no botão direito da ilha.
pub fn popup(mtm: MainThreadMarker, at: NSPoint, view: &NSView) {
    let m = menu(mtm);
    refresh(&m);
    m.popUpMenuPositioningItem_atLocation_inView(None, at, Some(view));
}
