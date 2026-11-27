//! Taskbeat no macOS: widget de música no notch do MacBook.
//!
//! - thread principal: AppKit (ilha do notch, ícone na barra de menus,
//!   configurações), animações no ritmo da tela (CADisplayLink);
//! - media-nowplaying: o que está tocando (`media.rs`);
//! - audio-tap: som do sistema para as barras (`audio.rs`), só com elas ligadas.
//!
//! O desenho é o mesmo do Windows (`ui.rs`, `notch.rs`, `render.rs`): cada
//! quadro vira um `CGImage` que a camada da janela mostra.

pub mod audio;
mod compat;
mod gfx;
mod launchers;
mod lyrics;
pub mod media;
mod notch;
mod osd;
mod settings;
mod statusbar;
pub mod system;
pub(crate) mod update;
mod volume;

use crate::config::Config;
use crate::shared::Shared;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSMenu, NSMenuDelegate, NSRunningApplication};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSString};
use std::cell::OnceCell;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub const BUNDLE_ID: &str = "io.github.lu1sgabriel.taskbeat";

thread_local! {
    static SHARED: OnceCell<Arc<Shared>> = const { OnceCell::new() };
    static TARGET: OnceCell<Retained<Target>> = const { OnceCell::new() };
}

/// Estado compartilhado (só na thread principal).
pub(crate) fn shared() -> Arc<Shared> {
    SHARED.with(|s| s.get().expect("app iniciado").clone())
}

/// Objeto que recebe as ações de menus, botões, temporizadores e do vblank.
pub(crate) fn target() -> Retained<Target> {
    TARGET.with(|t| t.get().expect("app iniciado").clone())
}

pub fn main() {
    system::log::install_panic_hook();
    let mtm = MainThreadMarker::new().expect("Taskbeat precisa rodar na thread principal");
    // Uma instância só (o Finder já reaproveita a aberta; isto cobre `open -n`).
    let others = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(BUNDLE_ID));
    if others.count() > 1 {
        return;
    }

    let first_run = !Config::exists();
    let mut cfg = Config::load();
    // Abrir ao iniciar a sessão vem ligado (também para quem já usava o beta),
    // só para o app instalado como pacote .app.
    if system::app_bundle().is_some() && cfg.apply_login_default(first_run, true, system::autostart_enabled, || system::set_autostart(true)) {
        cfg.save();
    }
    crate::i18n::apply(cfg.language);
    let shared = Arc::new(Shared::new(cfg));
    SHARED.with(|s| {
        let _ = s.set(shared.clone());
    });
    TARGET.with(|t| {
        let _ = t.set(Target::new(mtm));
    });

    let app = NSApplication::sharedApplication(mtm);
    // Sem ícone no Dock nem menu de app: vive no notch e na barra de menus.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    media::spawn(shared.clone(), || {
        dispatch2::DispatchQueue::main().exec_async(notch::media_changed);
    });
    launchers::spawn(shared.clone(), || {
        dispatch2::DispatchQueue::main().exec_async(notch::launchers_changed);
    });
    let audio = audio::spawn(shared.clone());
    crate::update::spawn(shared.clone());

    app.run();

    shared.running.store(false, Ordering::Relaxed);
    media::shutdown();
    let _ = audio.join();
}

/// Sai do app (menu "Sair do Taskbeat").
pub(crate) fn quit(mtm: MainThreadMarker) {
    shared().running.store(false, Ordering::Relaxed);
    media::shutdown();
    audio::clean_exit();
    NSApplication::sharedApplication(mtm).terminate(None);
}

/// Configurações mudaram (tela de configurações ou menu): todos reagem.
pub(crate) fn config_changed() {
    notch::config_changed();
    statusbar::config_changed();
    if let Some(mtm) = MainThreadMarker::new() {
        lyrics::sync(mtm);
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            let mtm = self.mtm();
            notch::start(mtm);
            osd::start(mtm);
            statusbar::install(mtm);
            lyrics::sync(mtm);
            if std::env::var_os("TV_DEMO_SETTINGS").is_some() {
                settings::open(mtm);
            }
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            // Abriu o app de novo (Finder, Launchpad, Spotlight): mostra as configurações.
            settings::open(self.mtm());
            true
        }

        #[unsafe(method(applicationDidChangeScreenParameters:))]
        fn screens_changed(&self, _n: &NSNotification) {
            notch::screens_changed();
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _n: &NSNotification) {
            shared().running.store(false, Ordering::Relaxed);
            media::shutdown();
            audio::clean_exit();
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    pub(crate) struct Target;

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSMenuDelegate for Target {
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, menu: &NSMenu) {
            statusbar::refresh(menu);
        }
    }

    impl Target {
        /// Quadro do vblank (CADisplayLink da ilha).
        #[unsafe(method(frame:))]
        fn frame(&self, _link: &AnyObject) {
            notch::frame();
        }

        /// Temporizador lento da letra na tela (10 Hz).
        #[unsafe(method(lyricsTick:))]
        fn lyrics_tick(&self, _timer: &AnyObject) {
            lyrics::tick();
        }

        /// Quadro do vblank da letra na tela (animações).
        #[unsafe(method(lyricsFrame:))]
        fn lyrics_frame(&self, _link: &AnyObject) {
            lyrics::frame();
        }

        /// Quadro do vblank da tela de configurações (prévia).
        #[unsafe(method(settingsFrame:))]
        fn settings_frame(&self, _link: &AnyObject) {
            settings::frame();
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            settings::open(self.mtm());
        }

        #[unsafe(method(quit:))]
        fn quit_app(&self, _sender: Option<&AnyObject>) {
            quit(self.mtm());
        }

        #[unsafe(method(toggleAutostart:))]
        fn toggle_autostart(&self, _sender: Option<&AnyObject>) {
            system::set_autostart(!system::autostart_enabled());
        }

        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&AnyObject>) {
            shared().send_media(crate::shared::MediaCmd::PlayPause);
        }

        #[unsafe(method(nextTrack:))]
        fn next_track(&self, _sender: Option<&AnyObject>) {
            shared().send_media(crate::shared::MediaCmd::Next);
        }

        #[unsafe(method(previousTrack:))]
        fn previous_track(&self, _sender: Option<&AnyObject>) {
            shared().send_media(crate::shared::MediaCmd::Previous);
        }

        #[unsafe(method(toggleOption:))]
        fn toggle_option(&self, sender: Option<&AnyObject>) {
            statusbar::toggle_option(sender);
        }

        #[unsafe(method(licenses:))]
        fn licenses(&self, _sender: Option<&AnyObject>) {
            crate::licenses::open();
        }

        /// Controle da tela de configurações mudou (interruptor, lista, slider).
        #[unsafe(method(control:))]
        fn control(&self, sender: Option<&AnyObject>) {
            settings::control_changed(sender);
        }

        /// Botão da tela de configurações (restaurar, licenças, testar…).
        #[unsafe(method(button:))]
        fn button(&self, sender: Option<&AnyObject>) {
            settings::button(sender);
        }

        /// Quadro do vblank do aviso de Caps Lock.
        #[unsafe(method(osdFrame:))]
        fn osd_frame(&self, _link: &AnyObject) {
            osd::frame();
        }

        /// Leitura do Caps Lock.
        #[unsafe(method(osdTick:))]
        fn osd_tick(&self, _timer: &AnyObject) {
            osd::tick();
        }

        /// Temporizador lento da ilha (sem animação: estado do mouse, sessão).
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &AnyObject) {
            notch::tick();
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}
