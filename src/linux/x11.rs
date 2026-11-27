//! Ilha no X11: janela de 32 bits (com transparência) sem moldura e fora do
//! controle do gerenciador de janelas (`override-redirect`), no topo e no meio
//! da tela principal — logo abaixo da barra de cima, se houver uma (a área
//! de trabalho do `_NET_WORKAREA`; no GNOME, abaixo do painel).
//!
//! Também é o caminho do GNOME no Wayland (que não tem layer-shell): a janela
//! vai pelo XWayland. A transparência precisa de um compositor (todos os
//! desktops atuais têm); sem ele, a janela é cortada na forma da ilha.

use super::window::{WinEv, Window};
use crate::render::RectF;
use std::os::fd::{AsRawFd, RawFd};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub struct X11 {
    conn: RustConnection,
    win: Window_,
    gc: Gcontext,
    root: Window_,
    scale: f32,
    /// Tela principal: x, y, largura, altura; `top`: onde a ilha começa.
    mon: (i32, i32, i32, i32),
    top: i32,
    size: (usize, usize),
    events: Vec<WinEv>,
    composited: bool,
    has_shape: bool,
}

type Window_ = u32;

fn atom(conn: &RustConnection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes()).ok().and_then(|c| c.reply().ok()).map(|r| r.atom).unwrap_or(0)
}

impl X11 {
    pub fn open() -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
        let screen = conn.setup().roots[screen_num].clone();
        let root = screen.root;
        // Visual de 32 bits (ARGB) para a transparência.
        let (depth, visual) = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter().map(move |v| (d.depth, v)))
            .find(|(_, v)| v.class == VisualClass::TRUE_COLOR)
            .map(|(d, v)| (d, v.visual_id))
            .ok_or("sem visual de 32 bits")?;
        let composited = {
            let a = atom(&conn, &format!("_NET_WM_CM_S{screen_num}"));
            a != 0 && conn.get_selection_owner(a).ok().and_then(|c| c.reply().ok()).is_some_and(|r| r.owner != 0)
        };
        let has_shape = conn.extension_information(shape::X11_EXTENSION_NAME).ok().flatten().is_some();
        let scale = x_scale(&conn, root);
        let mon = primary_monitor(&conn, root).unwrap_or((0, 0, screen.width_in_pixels as i32, screen.height_in_pixels as i32));
        let top = workarea_top(&conn, root, mon);

        let cmap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_colormap(ColormapAlloc::NONE, cmap, root, visual).map_err(|e| e.to_string())?;
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(cmap)
            .override_redirect(1)
            .event_mask(EventMask::EXPOSURE | EventMask::POINTER_MOTION | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::ENTER_WINDOW | EventMask::LEAVE_WINDOW);
        conn.create_window(depth, win, root, (mon.0 + mon.2 / 2) as i16, top as i16, 1, 1, 0, WindowClass::INPUT_OUTPUT, visual, &aux).map_err(|e| e.to_string())?;
        let utf8 = atom(&conn, "UTF8_STRING");
        let _ = conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_NAME, AtomEnum::STRING, b"Taskbeat");
        let _ = conn.change_property8(PropMode::REPLACE, win, atom(&conn, "_NET_WM_NAME"), utf8, b"Taskbeat");
        let _ = conn.change_property8(PropMode::REPLACE, win, AtomEnum::WM_CLASS, AtomEnum::STRING, b"taskbeat\0Taskbeat\0");
        // "Dock": compositores não põem sombra nem animação de janela.
        let dock = atom(&conn, "_NET_WM_WINDOW_TYPE_DOCK");
        let _ = conn.change_property32(PropMode::REPLACE, win, atom(&conn, "_NET_WM_WINDOW_TYPE"), AtomEnum::ATOM, &[dock]);
        let gc = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_gc(gc, win, &CreateGCAux::new()).map_err(|e| e.to_string())?;
        let mut x = Self { conn, win, gc, root, scale, mon, top, size: (1, 1), events: Vec::new(), composited, has_shape };
        x.set_input(None);
        x.conn.map_window(win).map_err(|e| e.to_string())?;
        x.conn.flush().map_err(|e| e.to_string())?;
        super::system::log::info(&format!("janela: X11 escala {scale} tela {mon:?} topo {top} compositor={composited}"));
        Ok(x)
    }

    fn shape(&self, kind: shape::SK, r: Option<RectF>) {
        if !self.has_shape {
            return;
        }
        let rects: Vec<Rectangle> = r
            .map(|r| {
                let (x0, y0) = (r.x.floor().max(0.0), r.y.floor().max(0.0));
                Rectangle { x: x0 as i16, y: y0 as i16, width: (r.right().ceil() - x0).max(0.0) as u16, height: (r.bottom().ceil() - y0).max(0.0) as u16 }
            })
            .into_iter()
            .collect();
        let _ = self.conn.shape_rectangles(shape::SO::SET, kind, ClipOrdering::UNSORTED, self.win, 0, 0, &rects);
    }
}

/// Escala do X: `Xft.dpi` (o que GNOME, KDE e XFCE ajustam) ou `GDK_SCALE`.
fn x_scale(conn: &RustConnection, root: u32) -> f32 {
    if let Some(s) = std::env::var("GDK_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|s| *s >= 1.0) {
        return s;
    }
    let res = conn.get_property(false, root, AtomEnum::RESOURCE_MANAGER, AtomEnum::STRING, 0, 1 << 16).ok().and_then(|c| c.reply().ok());
    let dpi = res.and_then(|r| {
        String::from_utf8_lossy(&r.value)
            .lines()
            .find_map(|l| l.strip_prefix("Xft.dpi:").map(|v| v.trim().parse::<f32>().ok()))
            .flatten()
    });
    dpi.map(|d| ((d / 96.0) * 4.0).round() / 4.0).filter(|s| *s >= 1.0).unwrap_or(1.0)
}

fn primary_monitor(conn: &RustConnection, root: u32) -> Option<(i32, i32, i32, i32)> {
    let r = conn.randr_get_monitors(root, true).ok()?.reply().ok()?;
    let m = r.monitors.iter().find(|m| m.primary).or_else(|| r.monitors.first())?;
    Some((m.x as i32, m.y as i32, m.width as i32, m.height as i32))
}

/// Início da área livre no topo (abaixo de painéis), dentro da tela.
fn workarea_top(conn: &RustConnection, root: u32, mon: (i32, i32, i32, i32)) -> i32 {
    if let Some(t) = std::env::var("TASKBEAT_TOP_OFFSET").ok().and_then(|v| v.parse::<i32>().ok()) {
        return mon.1 + t;
    }
    let a = atom(conn, "_NET_WORKAREA");
    let wa = conn.get_property(false, root, a, AtomEnum::CARDINAL, 0, 4).ok().and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()));
    match wa.as_deref() {
        Some([_, y, _, _, ..]) if (*y as i32) >= mon.1 && (*y as i32) < mon.1 + mon.3 / 4 => *y as i32,
        _ => mon.1,
    }
}

impl Window for X11 {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn fd(&self) -> RawFd {
        self.conn.stream().as_raw_fd()
    }

    fn before_poll(&mut self) {
        let _ = self.conn.flush();
        // Eventos já lidos do socket (o poll não avisaria deles).
        self.after_poll(false);
    }

    fn after_poll(&mut self, _readable: bool) {
        while let Ok(Some(ev)) = self.conn.poll_for_event() {
            match ev {
                Event::MotionNotify(e) => self.events.push(WinEv::Motion(e.event_x as f32, e.event_y as f32)),
                Event::EnterNotify(e) => self.events.push(WinEv::Motion(e.event_x as f32, e.event_y as f32)),
                Event::LeaveNotify(_) => self.events.push(WinEv::Leave),
                Event::ButtonPress(e) | Event::ButtonRelease(e) => {
                    let down = matches!(ev, Event::ButtonPress(_));
                    match e.detail {
                        1..=3 => self.events.push(WinEv::Button { down, button: e.detail as u32, x: e.event_x as f32, y: e.event_y as f32 }),
                        4 if down => self.events.push(WinEv::Scroll(4.0)),
                        5 if down => self.events.push(WinEv::Scroll(-4.0)),
                        _ => {}
                    }
                }
                Event::Expose(e) if e.count == 0 => self.events.push(WinEv::Expose),
                _ => {}
            }
        }
    }

    fn take_events(&mut self) -> Vec<WinEv> {
        std::mem::take(&mut self.events)
    }

    fn scale(&self) -> f32 {
        self.scale
    }

    fn fit(&self, w: f32, h: f32) -> (usize, usize) {
        (w.ceil() as usize, h.ceil() as usize)
    }

    fn set_size(&mut self, w: usize, h: usize) {
        if (w, h) == self.size {
            return;
        }
        self.size = (w, h);
        let x = self.mon.0 + (self.mon.2 - w as i32) / 2;
        let aux = ConfigureWindowAux::new().x(x).y(self.top).width(w as u32).height(h as u32).stack_mode(StackMode::ABOVE);
        let _ = self.conn.configure_window(self.win, &aux);
        let _ = self.conn.flush();
        let _ = self.root;
    }

    fn ready(&self) -> bool {
        true
    }

    fn present(&mut self, frame: &[u32], w: usize, h: usize, region: (usize, usize, usize, usize)) -> bool {
        let (x0, y0, x1, y1) = region;
        let (x1, y1) = (x1.min(w), y1.min(h));
        if x1 <= x0 || y1 <= y0 {
            return true;
        }
        let rw = x1 - x0;
        // Em faixas que caibam num pedido do X.
        let max_rows = ((self.conn.maximum_request_bytes().saturating_sub(64)) / (rw * 4)).max(1);
        let mut y = y0;
        while y < y1 {
            let n = max_rows.min(y1 - y);
            let mut bytes = Vec::with_capacity(rw * n * 4);
            for row in y..y + n {
                // BGRA em little-endian: os bytes do `u32` já são os do X.
                let px = &frame[row * w + x0..row * w + x1];
                bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(px.as_ptr().cast::<u8>(), px.len() * 4) });
            }
            let _ = self.conn.put_image(ImageFormat::Z_PIXMAP, self.win, self.gc, rw as u16, n as u16, x0 as i16, y as i16, 0, 32, &bytes);
            y += n;
        }
        let _ = self.conn.flush();
        true
    }

    fn set_input(&mut self, r: Option<RectF>) {
        self.shape(shape::SK::INPUT, r);
        if !self.composited {
            // Sem compositor não há transparência: só a ilha aparece.
            self.shape(shape::SK::BOUNDING, r);
        }
        let _ = self.conn.flush();
    }
}
