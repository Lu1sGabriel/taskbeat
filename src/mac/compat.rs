//! Compatibilidade com macOS mais antigos (o app roda a partir do macOS 12).
//!
//! - Ritmo das animações: `CADisplayLink` da vista (macOS 14+) ou, antes disso,
//!   um `NSTimer` no mesmo ritmo.
//! - Funções do Core Audio que só existem no macOS 14.2+ (process taps) são
//!   procuradas na hora (`dlsym`): o app abre em versões antigas e só as barras
//!   de áudio ficam sem o som do sistema.
//! - `TASKBEAT_FORCE_LEGACY=1` força os caminhos antigos (para testar no CI).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{ClassType, MainThreadMarker, Message};
use objc2_app_kit::{NSApplication, NSColor, NSView};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use std::ffi::{CStr, c_void};

pub fn force_legacy() -> bool {
    std::env::var_os("TASKBEAT_FORCE_LEGACY").is_some()
}

/// Quadros de animação em `target`/`selector`, no ritmo pedido (0 = parado).
pub struct Ticker {
    view: Retained<NSView>,
    selector: Sel,
    link: Option<Retained<CADisplayLink>>,
    timer: Option<Retained<NSTimer>>,
    rate: u32,
}

impl Ticker {
    pub fn new(view: &NSView, selector: Sel) -> Self {
        Self { view: view.retain(), selector, link: None, timer: None, rate: 0 }
    }

    fn modern(&self) -> bool {
        !force_legacy() && self.view.respondsToSelector(objc2::sel!(displayLinkWithTarget:selector:))
    }

    pub fn set_rate(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let target = super::target();
        if self.modern() {
            if self.link.is_none() && rate > 0 {
                let link = unsafe { self.view.displayLinkWithTarget_selector(&target, self.selector) };
                unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
                self.link = Some(link);
            }
            if let Some(link) = &self.link {
                if rate == 0 {
                    link.setPaused(true);
                } else {
                    let r = rate as f32;
                    link.setPreferredFrameRateRange(CAFrameRateRange { minimum: (r / 2.0).min(30.0), maximum: r, preferred: r });
                    link.setPaused(false);
                }
            }
            return;
        }
        // macOS 12/13: temporizador no ritmo pedido (60 Hz no máximo).
        if let Some(t) = self.timer.take() {
            t.invalidate();
        }
        if rate > 0 {
            let hz = rate.min(60) as f64;
            let t = unsafe { NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(1.0 / hz, &target, self.selector, None, true) };
            t.setTolerance(0.2 / hz);
            unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&t, NSRunLoopCommonModes) };
            self.timer = Some(t);
        }
    }
}

/// Traz o app para a frente (o `activate` sem argumento é do macOS 14+).
pub fn activate(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if !force_legacy() && app.respondsToSelector(objc2::sel!(activate)) {
        app.activate();
    } else {
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

/// Fundo dos grupos das configurações (as cores "system fill" são do macOS 14+).
pub fn group_fill() -> Retained<NSColor> {
    if !force_legacy() && NSColor::class().responds_to(objc2::sel!(quaternarySystemFillColor)) {
        NSColor::quaternarySystemFillColor()
    } else {
        NSColor::quaternaryLabelColor()
    }
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}

/// RTLD_DEFAULT no macOS.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

fn symbol(name: &CStr) -> *mut c_void {
    if force_legacy() {
        return std::ptr::null_mut();
    }
    unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) }
}

type CreateTap = unsafe extern "C" fn(*const AnyObject, *mut u32) -> i32;
type DestroyTap = unsafe extern "C" fn(u32) -> i32;

/// `AudioHardwareCreateProcessTap` (macOS 14.2+), se existir neste macOS.
pub fn create_process_tap() -> Option<CreateTap> {
    let p = symbol(c"AudioHardwareCreateProcessTap");
    (!p.is_null()).then(|| unsafe { std::mem::transmute::<*mut c_void, CreateTap>(p) })
}

/// `AudioHardwareDestroyProcessTap` (macOS 14.2+).
pub fn destroy_process_tap(tap: u32) {
    let p = symbol(c"AudioHardwareDestroyProcessTap");
    if !p.is_null() {
        let f = unsafe { std::mem::transmute::<*mut c_void, DestroyTap>(p) };
        unsafe { f(tap) };
    }
}
