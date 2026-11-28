//! Ilha no Wayland com layer-shell (`zwlr_layer_shell_v1`): uma superfície
//! presa no topo da tela, acima das janelas comuns, que não aparece na barra
//! de tarefas nem no Alt+Tab. O compositor a centraliza.
//!
//! - Escala fracionada (125%, 150%…) com `wp_fractional_scale_v1` +
//!   `wp_viewporter`: o buffer tem exatamente os pixels da tela (nítido).
//! - Dois buffers em memória compartilhada (`wl_shm`); cada quadro copia só o
//!   que mudou e avisa o compositor só dessa região.
//! - Ritmo dos quadros pelo próprio compositor (`frame callback`, o vblank).
//! - Região de entrada = a forma visível da ilha: fora dela os cliques vão
//!   para as janelas de trás.

use super::system::log;
use super::window::{WinEv, Window, union};
use crate::render::RectF;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Quanto esperar pelo `configure` de um tamanho novo. O protocolo não
/// promete resposta ao `set_size`, e um compositor que só responde quando o
/// tamanho muda deixaria a ilha congelada (B11).
const CONFIGURE_WAIT: Duration = Duration::from_millis(500);
/// Recriações da superfície aceitas em `RECREATE_WINDOW` antes de desistir
/// (um compositor que fecha toda superfície nova não vira um laço).
const MAX_RECREATES: usize = 5;
const RECREATE_WINDOW: Duration = Duration::from_secs(10);

/// Uma tela (`wl_output`) anunciada pelo registro.
struct Out {
    /// Nome no registro (o que o `global_remove` traz).
    name: u32,
    /// Id do objeto (o que o `enter`/`leave` da superfície traz).
    id: u32,
    scale: i32,
    proxy: wl_output::WlOutput,
}

/// Estado mexido pelos eventos do compositor.
#[derive(Default)]
struct St {
    events: Vec<WinEv>,
    /// O compositor confirmou o último tamanho pedido.
    configured: bool,
    /// A superfície atual já recebeu o primeiro `configure`. Antes disso
    /// anexar um buffer é erro de protocolo, então aqui não há timeout.
    first_configure: bool,
    /// Quando o último tamanho novo foi pedido (para o `CONFIGURE_WAIT`).
    size_sent: Option<Instant>,
    /// Geração da superfície: eventos de uma superfície já destruída são
    /// ignorados.
    surface_gen: u32,
    closed: bool,
    /// Quadro enviado e ainda não mostrado (espera o vblank).
    frame_pending: Option<Instant>,
    /// Buffers em uso pelo compositor (geração do pool, índice).
    busy: [bool; 2],
    pool_gen: u32,
    /// Escala fracionada (×120) ou inteira preferida.
    scale120: Option<u32>,
    int_scale: i32,
    outputs: Vec<Out>,
    entered: Vec<u32>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    /// Escala em uso (para converter as posições do mouse).
    scale: f32,
    pos: (f32, f32),
    wheel: f32,
    smooth: f32,
    frames: bool,
}

impl St {
    fn effective_scale(&self) -> f32 {
        match self.scale120 {
            Some(s) => s as f32 / 120.0,
            None => self.int_scale.max(1) as f32,
        }
    }

    fn rescale(&mut self) {
        if self.scale120.is_none() {
            // Sem escala fracionada: a maior das telas em que a superfície está.
            let s = self.outputs.iter().filter(|o| self.entered.contains(&o.id)).map(|o| o.scale).max().unwrap_or(1);
            if s != self.int_scale {
                self.int_scale = s;
            }
        }
        let s = self.effective_scale();
        if (s - self.scale).abs() > 0.001 {
            self.events.push(WinEv::Scale(s));
        }
    }
}

struct Pool {
    pool: wl_shm_pool::WlShmPool,
    buffers: [wl_buffer::WlBuffer; 2],
    mem: *mut u8,
    len: usize,
    _fd: OwnedFd,
    w: usize,
    h: usize,
    /// Regiões que cada buffer perdeu (desenhadas no outro).
    stale: [Option<(usize, usize, usize, usize)>; 2],
}

impl Drop for Pool {
    fn drop(&mut self) {
        for b in &self.buffers {
            b.destroy();
        }
        self.pool.destroy();
        unsafe { libc::munmap(self.mem.cast(), self.len) };
    }
}

/// Os objetos de uma superfície da ilha (recriados juntos quando o
/// compositor a fecha).
struct Surf {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    viewport: Option<wp_viewport::WpViewport>,
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
}

/// Os globais que criam uma superfície.
struct Makers {
    compositor: wl_compositor::WlCompositor,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
}

impl Makers {
    /// Superfície nova com os `set_*` iniciais, região de entrada vazia e um
    /// commit **sem buffer**; o `configure` chega depois, pelo laço normal.
    /// `size`: o tamanho lógico já em uso (`None` na primeira vez).
    fn surface(&self, above: bool, size: Option<(u32, u32)>, int_scale: i32, generation: u32, qh: &QueueHandle<St>) -> Surf {
        let surface = self.compositor.create_surface(qh, generation);
        let (viewport, fractional) = match (&self.fractional, &self.viewporter) {
            (Some(f), Some(v)) => (Some(v.get_viewport(&surface, qh, ())), Some(f.get_fractional_scale(&surface, qh, ()))),
            _ => (None, None),
        };
        let layer = self.layer_shell.get_layer_surface(&surface, None, layer_of(above), "taskbeat".into(), qh, generation);
        layer.set_anchor(zwlr_layer_surface_v1::Anchor::Top);
        layer.set_exclusive_zone(0);
        layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        let (lw, lh) = size.unwrap_or((1, 1));
        layer.set_size(lw, lh);
        if size.is_some() {
            match &viewport {
                Some(v) => {
                    surface.set_buffer_scale(1);
                    v.set_destination(lw as i32, lh as i32);
                }
                None => surface.set_buffer_scale(int_scale.max(1)),
            }
        }
        // Sem entrada até a ilha dizer onde fica.
        let empty = self.compositor.create_region(qh, ());
        surface.set_input_region(Some(&empty));
        empty.destroy();
        surface.commit();
        Surf { surface, layer, viewport, fractional }
    }
}

pub struct Wl {
    conn: Connection,
    queue: EventQueue<St>,
    qh: QueueHandle<St>,
    st: St,
    makers: Makers,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    layer_version: u32,
    viewport: Option<wp_viewport::WpViewport>,
    fractional: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    guard: Option<wayland_client::backend::ReadEventsGuard>,
    pool: Option<Pool>,
    size: (usize, usize),
    logical: (u32, u32),
    /// Acima de janelas em tela cheia (para recriar na mesma camada).
    above: bool,
    /// A superfície foi fechada e espera uma tela para ser recriada.
    lost: bool,
    /// Quando foram as últimas recriações (limite de `MAX_RECREATES`).
    recreated: Vec<Instant>,
}

impl Wl {
    pub fn open(above_fullscreen: bool) -> Result<Self, String> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err("sem WAYLAND_DISPLAY".into());
        }
        let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
        let (globals, queue) = registry_queue_init::<St>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ()).map_err(|_| "o compositor não tem layer-shell".to_string())?;
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).map_err(|e| e.to_string())?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        let mut st = St { int_scale: 1, scale: 1.0, ..Default::default() };
        // Telas (escala inteira, quando não há escala fracionada). As que
        // entram e saem depois chegam pelo `Dispatch` do registro.
        for g in globals.contents().clone_list() {
            if g.interface == wl_output::WlOutput::interface().name {
                let proxy: wl_output::WlOutput = globals.registry().bind(g.name, g.version.min(4), &qh, g.name);
                st.outputs.push(Out { name: g.name, id: proxy.id().protocol_id(), scale: 1, proxy });
            }
        }
        if let Ok(seat) = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 1..=8, ()) {
            let _ = seat;
        }
        st.cursor_mgr = globals.bind(&qh, 1..=1, ()).ok();
        let fractional_mgr: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1> = globals.bind(&qh, 1..=1, ()).ok();
        let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();

        let layer_version = layer_shell.version();
        let makers = Makers { compositor: compositor.clone(), layer_shell, fractional: fractional_mgr, viewporter };
        let Surf { surface, layer, viewport, fractional } = makers.surface(above_fullscreen, None, 1, st.surface_gen, &qh);

        let mut w = Self {
            conn,
            queue,
            qh,
            st,
            makers,
            compositor,
            shm,
            surface,
            layer,
            layer_version,
            viewport,
            fractional,
            guard: None,
            pool: None,
            size: (0, 0),
            logical: (1, 1),
            above: above_fullscreen,
            lost: false,
            recreated: Vec::new(),
        };
        // Primeira configuração e escala (só aqui a espera é bloqueante).
        let t = Instant::now();
        while !w.st.first_configure && t.elapsed() < Duration::from_secs(3) {
            w.queue.roundtrip(&mut w.st).map_err(|e| e.to_string())?;
        }
        if !w.st.first_configure {
            return Err("o compositor não configurou a superfície".into());
        }
        w.st.scale = w.st.effective_scale();
        w.st.events.clear();
        Ok(w)
    }

    /// O compositor fechou a superfície (tela removida, laptop desencaixado):
    /// em vez de fechar o app (B10), cria outra. Um `wl_surface` **novo**,
    /// porque o antigo já teve buffer e o `get_layer_surface` daria
    /// `already_constructed`; viewport e escala fracionada novos junto.
    fn recreate(&mut self) {
        // Ordem de destruição: layer, viewport, escala fracionada, superfície.
        self.layer.destroy();
        if let Some(v) = self.viewport.take() {
            v.destroy();
        }
        if let Some(f) = self.fractional.take() {
            f.destroy();
        }
        self.surface.destroy();
        let st = &mut self.st;
        st.surface_gen = st.surface_gen.wrapping_add(1);
        st.configured = false;
        st.first_configure = false;
        st.size_sent = None;
        st.frame_pending = None;
        st.entered.clear();
        // Pool novo: um `release` atrasado do pool antigo não marca nada.
        st.pool_gen = st.pool_gen.wrapping_add(1);
        st.busy = [false; 2];
        self.pool = None;
        let size = (self.size != (0, 0)).then_some(self.logical);
        let s = self.makers.surface(self.above, size, st.int_scale, st.surface_gen, &self.qh);
        self.surface = s.surface;
        self.layer = s.layer;
        self.viewport = s.viewport;
        self.fractional = s.fractional;
        let _ = self.conn.flush();
    }

    fn ensure_pool(&mut self, w: usize, h: usize) -> bool {
        if self.pool.as_ref().is_some_and(|p| p.w == w && p.h == h) {
            return true;
        }
        self.pool = None;
        let one = w * h * 4;
        let len = one * 2;
        let fd = unsafe { libc::memfd_create(c"taskbeat".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return false;
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
            return false;
        }
        let mem = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if mem == libc::MAP_FAILED {
            return false;
        }
        self.st.pool_gen += 1;
        self.st.busy = [false; 2];
        let pool = self.shm.create_pool(fd.as_fd(), len as i32, &self.qh, ());
        let generation = self.st.pool_gen;
        let mk = |i: usize| pool.create_buffer((i * one) as i32, w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888, &self.qh, (generation, i));
        let buffers = [mk(0), mk(1)];
        let full = Some((0, 0, w, h));
        self.pool = Some(Pool { pool, buffers, mem: mem.cast(), len, _fd: fd, w, h, stale: [full, full] });
        true
    }
}

fn layer_of(above_fullscreen: bool) -> zwlr_layer_shell_v1::Layer {
    if above_fullscreen { zwlr_layer_shell_v1::Layer::Overlay } else { zwlr_layer_shell_v1::Layer::Top }
}

impl Window for Wl {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn fd(&self) -> RawFd {
        self.conn.backend().poll_fd().as_raw_fd()
    }

    fn before_poll(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.st);
        let _ = self.conn.flush();
        self.guard = self.queue.prepare_read();
        if self.guard.is_none() {
            // Já há eventos na fila: processa e tenta de novo.
            let _ = self.queue.dispatch_pending(&mut self.st);
            self.guard = self.queue.prepare_read();
        }
    }

    fn after_poll(&mut self, readable: bool) {
        if let Some(g) = self.guard.take() {
            if readable {
                let _ = g.read();
            }
        }
        let _ = self.queue.dispatch_pending(&mut self.st);
        let _ = self.conn.flush();
    }

    fn take_events(&mut self) -> Vec<WinEv> {
        if self.st.closed {
            self.st.closed = false;
            self.lost = true;
            log::info("janela: superfície fechada pelo compositor; recriando quando houver uma tela");
        }
        // Só com alguma tela: sem nenhuma, espera o `global` de uma nova em vez
        // de entrar num laço de criar e fechar.
        if self.lost && !self.st.outputs.is_empty() {
            self.lost = false;
            self.recreated.retain(|t| t.elapsed() < RECREATE_WINDOW);
            if self.recreated.len() >= MAX_RECREATES {
                log::warn("janela: o compositor fecha toda superfície nova; desistindo");
                self.st.events.push(WinEv::Closed);
            } else {
                self.recreated.push(Instant::now());
                self.recreate();
            }
        }
        std::mem::take(&mut self.st.events)
    }

    fn scale(&self) -> f32 {
        self.st.effective_scale()
    }

    fn fit(&self, w: f32, h: f32) -> (usize, usize) {
        let s = self.scale();
        let (lw, lh) = ((w / s).ceil(), (h / s).ceil());
        ((lw * s).round() as usize, (lh * s).round() as usize)
    }

    fn set_size(&mut self, w: usize, h: usize) {
        let s = self.scale();
        self.st.scale = s;
        let logical = ((w as f32 / s).round().max(1.0) as u32, (h as f32 / s).round().max(1.0) as u32);
        if (w, h) == self.size && logical == self.logical {
            return;
        }
        let resized = logical != self.logical;
        self.size = (w, h);
        self.logical = logical;
        self.layer.set_size(logical.0, logical.1);
        match &self.viewport {
            Some(v) => {
                self.surface.set_buffer_scale(1);
                v.set_destination(logical.0 as i32, logical.1 as i32);
            }
            None => self.surface.set_buffer_scale(self.st.int_scale.max(1)),
        }
        // Espera o compositor confirmar o tamanho novo, só se o tamanho
        // **lógico** mudou (uma troca de escala não muda o que foi pedido), e
        // no máximo `CONFIGURE_WAIT`.
        if resized {
            self.st.configured = false;
            self.st.size_sent = Some(Instant::now());
        }
        self.surface.commit();
        let _ = self.conn.flush();
    }

    fn ready(&self) -> bool {
        let confirmed = self.st.configured || self.st.size_sent.is_some_and(|t| t.elapsed() > CONFIGURE_WAIT);
        self.st.first_configure && confirmed && self.st.frame_pending.is_none_or(|t| t.elapsed() > Duration::from_millis(500))
    }

    fn present(&mut self, frame: &[u32], w: usize, h: usize, region: (usize, usize, usize, usize)) -> bool {
        if !self.ready() || w == 0 || h == 0 || !self.ensure_pool(w, h) {
            return false;
        }
        let Some(i) = (0..2).find(|&i| !self.st.busy[i]) else { return false };
        let pool = self.pool.as_mut().unwrap();
        let (x0, y0, x1, y1) = union(pool.stale[i], region);
        let (x1, y1) = (x1.min(w), y1.min(h));
        let base = unsafe { pool.mem.cast::<u32>().add(i * w * h) };
        for y in y0..y1 {
            unsafe { std::ptr::copy_nonoverlapping(frame.as_ptr().add(y * w + x0), base.add(y * w + x0), x1 - x0) };
        }
        pool.stale[i] = None;
        pool.stale[1 - i] = Some(union(pool.stale[1 - i], region));
        self.surface.attach(Some(&pool.buffers[i]), 0, 0);
        let (rx0, ry0, rx1, ry1) = region;
        self.surface.damage_buffer(rx0 as i32, ry0 as i32, (rx1 - rx0) as i32, (ry1 - ry0) as i32);
        self.surface.frame(&self.qh, ());
        self.st.frame_pending = Some(Instant::now());
        self.st.busy[i] = true;
        self.surface.commit();
        let _ = self.conn.flush();
        true
    }

    fn set_input(&mut self, r: Option<RectF>) {
        let s = self.st.scale.max(0.5);
        let region = self.compositor.create_region(&self.qh, ());
        if let Some(r) = r {
            let (x0, y0) = ((r.x / s).floor() as i32, (r.y / s).floor() as i32);
            let (x1, y1) = ((r.right() / s).ceil() as i32, (r.bottom() / s).ceil() as i32);
            region.add(x0.max(0), y0.max(0), (x1 - x0.max(0)).max(0), (y1 - y0.max(0)).max(0));
        }
        self.surface.set_input_region(Some(&region));
        region.destroy();
        self.surface.commit();
        let _ = self.conn.flush();
    }

    fn set_above_fullscreen(&mut self, on: bool) {
        self.above = on;
        if self.layer_version >= 2 {
            self.layer.set_layer(layer_of(on));
            self.surface.commit();
        }
    }
}

// ---------------------------------------------------------------- eventos

/// Telas que entram e saem depois do `open` (monitor ligado, desligado,
/// laptop encaixado).
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for St {
    fn event(st: &mut Self, registry: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &GlobalListContents, _: &Connection, qh: &QueueHandle<Self>) {
        match ev {
            wl_registry::Event::Global { name, interface, version } if interface == wl_output::WlOutput::interface().name => {
                if !st.outputs.iter().any(|o| o.name == name) {
                    let proxy: wl_output::WlOutput = registry.bind(name, version.min(4), qh, name);
                    st.outputs.push(Out { name, id: proxy.id().protocol_id(), scale: 1, proxy });
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(i) = st.outputs.iter().position(|o| o.name == name) {
                    let o = st.outputs.remove(i);
                    if o.proxy.version() >= 3 {
                        o.proxy.release();
                    }
                    st.entered.retain(|&id| id != o.id);
                    st.rescale();
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(St: wl_compositor::WlCompositor);
delegate_noop!(St: wl_region::WlRegion);
delegate_noop!(St: wl_shm_pool::WlShmPool);
delegate_noop!(St: ignore wl_shm::WlShm);
delegate_noop!(St: zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(St: wp_viewporter::WpViewporter);
delegate_noop!(St: wp_viewport::WpViewport);
delegate_noop!(St: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(St: wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(St: wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

impl Dispatch<wl_surface::WlSurface, u32> for St {
    fn event(st: &mut Self, _: &wl_surface::WlSurface, ev: wl_surface::Event, generation: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if *generation != st.surface_gen {
            return;
        }
        match ev {
            wl_surface::Event::Enter { output } => {
                st.entered.push(output.id().protocol_id());
                st.rescale();
            }
            wl_surface::Event::Leave { output } => {
                st.entered.retain(|&id| id != output.id().protocol_id());
                st.rescale();
            }
            wl_surface::Event::PreferredBufferScale { factor } => {
                if st.scale120.is_none() {
                    st.int_scale = factor.max(1);
                    let s = st.effective_scale();
                    if (s - st.scale).abs() > 0.001 {
                        st.events.push(WinEv::Scale(s));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for St {
    fn event(st: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, name: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Scale { factor } = ev
            && let Some(o) = st.outputs.iter_mut().find(|o| o.name == *name)
        {
            o.scale = factor;
            st.rescale();
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, (u32, usize)> for St {
    fn event(st: &mut Self, _: &wl_buffer::WlBuffer, ev: wl_buffer::Event, data: &(u32, usize), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = ev {
            if data.0 == st.pool_gen {
                st.busy[data.1] = false;
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for St {
    fn event(st: &mut Self, _: &wl_callback::WlCallback, ev: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = ev {
            st.frame_pending = None;
            st.frames = true;
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, u32> for St {
    fn event(st: &mut Self, l: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ev: zwlr_layer_surface_v1::Event, generation: &u32, _: &Connection, _: &QueueHandle<Self>) {
        // Evento atrasado de uma superfície que já foi trocada.
        if *generation != st.surface_gen {
            return;
        }
        match ev {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                l.ack_configure(serial);
                st.configured = true;
                st.first_configure = true;
                st.size_sent = None;
                st.events.push(WinEv::Expose);
            }
            zwlr_layer_surface_v1::Event::Closed => {
                st.closed = true;
                // Nada mais é anexado à superfície fechada.
                st.first_configure = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for St {
    fn event(st: &mut Self, _: &wp_fractional_scale_v1::WpFractionalScaleV1, ev: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            st.scale120 = Some(scale.max(60));
            let s = st.effective_scale();
            if (s - st.scale).abs() > 0.001 {
                st.events.push(WinEv::Scale(s));
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for St {
    fn event(st: &mut Self, seat: &wl_seat::WlSeat, ev: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = ev {
            let has = caps.contains(wl_seat::Capability::Pointer);
            if has && st.pointer.is_none() {
                let p = seat.get_pointer(qh, ());
                st.cursor = st.cursor_mgr.as_ref().map(|m| m.get_pointer(&p, qh, ()));
                st.frames = seat.version() >= 5;
                st.pointer = Some(p);
            } else if !has {
                if let Some(p) = st.pointer.take() {
                    if p.version() >= 3 {
                        p.release();
                    }
                }
                st.cursor = None;
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for St {
    fn event(st: &mut Self, p: &wl_pointer::WlPointer, ev: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let s = st.scale.max(0.5);
        let framed = p.version() >= 5;
        match ev {
            wl_pointer::Event::Enter { serial, surface_x, surface_y, .. } => {
                if let Some(c) = &st.cursor {
                    c.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                }
                st.pos = (surface_x as f32 * s, surface_y as f32 * s);
                st.events.push(WinEv::Motion(st.pos.0, st.pos.1));
            }
            wl_pointer::Event::Leave { .. } => st.events.push(WinEv::Leave),
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                st.pos = (surface_x as f32 * s, surface_y as f32 * s);
                st.events.push(WinEv::Motion(st.pos.0, st.pos.1));
            }
            wl_pointer::Event::Button { button, state, .. } => {
                let b = match button {
                    0x110 => 1,
                    0x112 => 2,
                    0x111 => 3,
                    _ => return,
                };
                let down = state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                st.events.push(WinEv::Button { down, button: b, x: st.pos.0, y: st.pos.1 });
            }
            wl_pointer::Event::Axis { axis: WEnum::Value(wl_pointer::Axis::VerticalScroll), value, .. } => {
                // Touchpad (rolagem contínua, em pontos da tela).
                st.smooth -= value as f32 * 0.25;
                if !framed {
                    st.events.push(WinEv::Scroll(st.smooth));
                    st.smooth = 0.0;
                }
            }
            wl_pointer::Event::AxisDiscrete { axis: WEnum::Value(wl_pointer::Axis::VerticalScroll), discrete } => st.wheel -= discrete as f32 * 4.0,
            wl_pointer::Event::AxisValue120 { axis: WEnum::Value(wl_pointer::Axis::VerticalScroll), value120 } => st.wheel -= value120 as f32 / 120.0 * 4.0,
            wl_pointer::Event::Frame => {
                // Roda do mouse: em degraus; senão, a rolagem contínua.
                let v = if st.wheel != 0.0 { st.wheel } else { st.smooth };
                if v != 0.0 {
                    st.events.push(WinEv::Scroll(v));
                }
                st.wheel = 0.0;
                st.smooth = 0.0;
            }
            _ => {}
        }
    }
}
