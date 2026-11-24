//! Ritmo das animações sincronizado com o monitor (vblank).
//!
//! Um timer comum (SetTimer, timer de espera) anda em passos de ~15,6 ms e não
//! acompanha a taxa do monitor: de tempos em tempos um quadro do app cai entre
//! dois do monitor e a animação "engasga". Aqui uma thread espera o vblank
//! (DXGI `WaitForVBlank` no monitor da janela; sem DXGI, `DwmFlush`) e avisa
//! quem está animando. Em monitores acima de ~90 Hz avisa a cada 2 (ou mais)
//! vblanks: continua alinhado (sem engasgos) e gasta menos CPU.
//!
//! Sem animação a thread fica bloqueada num evento: 0% de CPU.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Intervalo mínimo entre avisos (~75 fps): acima disso pula vblanks inteiros.
const MIN_INTERVAL: f32 = 1.0 / 76.0;

pub enum Target {
    /// Sinaliza um evento (para quem espera com `MsgWaitForMultipleObjects`).
    Event,
    /// Posta uma mensagem para a janela (no máximo uma pendente; ver `ack`).
    Post(HWND, u32),
}

struct Inner {
    active: AtomicBool,
    stop: AtomicBool,
    pending: AtomicBool,
    monitor: AtomicIsize,
    /// Acorda a thread (ativação ou parada).
    wake: HANDLE,
    /// Sinalizado a cada quadro (alvo `Event`).
    tick: HANDLE,
    post: Option<(isize, u32)>,
}

// HANDLE/HWND são só números para o sistema; o acesso é por chamadas thread-safe.
unsafe impl Send for Inner {}
unsafe impl Sync for Inner {}

pub struct Pacer {
    inner: Arc<Inner>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Pacer {
    pub fn new(target: Target) -> Option<Pacer> {
        unsafe {
            let wake = CreateEventW(None, false, false, None).ok()?;
            let tick = CreateEventW(None, false, false, None).ok()?;
            let post = match target {
                Target::Event => None,
                Target::Post(h, m) => Some((h.0 as isize, m)),
            };
            let inner = Arc::new(Inner {
                active: AtomicBool::new(false),
                stop: AtomicBool::new(false),
                pending: AtomicBool::new(false),
                monitor: AtomicIsize::new(0),
                wake,
                tick,
                post,
            });
            let i2 = inner.clone();
            let thread = std::thread::Builder::new().name("vsync".into()).spawn(move || run(&i2)).ok()?;
            Some(Pacer { inner, thread: Some(thread) })
        }
    }

    /// Evento sinalizado a cada quadro (alvo `Event`).
    pub fn event(&self) -> HANDLE {
        self.inner.tick
    }

    pub fn active(&self) -> bool {
        self.inner.active.load(Ordering::Relaxed)
    }

    pub fn set_active(&self, on: bool) {
        if self.inner.active.swap(on, Ordering::Relaxed) != on && on {
            unsafe {
                let _ = SetEvent(self.inner.wake);
            }
        }
    }

    /// A mensagem postada foi tratada: libera a próxima.
    pub fn ack(&self) {
        self.inner.pending.store(false, Ordering::Relaxed);
    }

    /// Monitor cujo vblank deve ser seguido (o da janela que anima).
    pub fn set_monitor(&self, m: HMONITOR) {
        self.inner.monitor.store(m.0 as isize, Ordering::Relaxed);
    }
}

impl Drop for Pacer {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::Relaxed);
        unsafe {
            let _ = SetEvent(self.inner.wake);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        unsafe {
            let _ = CloseHandle(self.inner.wake);
            let _ = CloseHandle(self.inner.tick);
        }
    }
}

/// Saída de vídeo (DXGI) do monitor `mon`; sem correspondência, a primeira.
fn find_output(mon: isize) -> Option<IDXGIOutput> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut first = None;
        for i in 0.. {
            let Ok(adapter) = factory.EnumAdapters1(i) else { break };
            for j in 0.. {
                let Ok(out) = adapter.EnumOutputs(j) else { break };
                if let Ok(d) = out.GetDesc() {
                    if d.Monitor.0 as isize == mon {
                        return Some(out);
                    }
                }
                first.get_or_insert(out);
            }
        }
        first
    }
}

fn run(inner: &Inner) {
    let mut output: Option<(isize, Option<IDXGIOutput>)> = None;
    let mut last = Instant::now();
    let mut interval = 1.0 / 60.0f32;
    let mut count = 0u32;
    loop {
        if inner.stop.load(Ordering::Relaxed) {
            break;
        }
        if !inner.active.load(Ordering::Relaxed) {
            unsafe {
                WaitForSingleObject(inner.wake, INFINITE);
            }
            last = Instant::now();
            continue;
        }
        let mon = inner.monitor.load(Ordering::Relaxed);
        if output.as_ref().map(|o| o.0) != Some(mon) {
            output = Some((mon, find_output(mon)));
        }
        let t0 = Instant::now();
        let waited = match output.as_ref().and_then(|o| o.1.as_ref()) {
            Some(o) => unsafe { o.WaitForVBlank().is_ok() },
            None => false,
        };
        if !waited {
            unsafe {
                let _ = DwmFlush();
            }
        }
        // Não esperou de verdade (sem composição, monitor desligado): segura o
        // ritmo em ~60 fps em vez de girar em falso.
        if t0.elapsed() < Duration::from_millis(2) {
            unsafe {
                WaitForSingleObject(inner.wake, 15);
            }
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().clamp(0.002, 0.05);
        last = now;
        interval += (dt - interval) * 0.1;
        let every = ((MIN_INTERVAL / interval).round() as u32).max(1);
        count = count.wrapping_add(1);
        if count % every != 0 || !inner.active.load(Ordering::Relaxed) {
            continue;
        }
        match inner.post {
            None => unsafe {
                let _ = SetEvent(inner.tick);
            },
            Some((h, msg)) => {
                if !inner.pending.swap(true, Ordering::Relaxed) {
                    let ok = unsafe { PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(0), LPARAM(0)).is_ok() };
                    if !ok {
                        inner.pending.store(false, Ordering::Relaxed);
                    }
                }
            }
        }
    }
}
