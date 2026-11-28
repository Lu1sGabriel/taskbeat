//! Janela da ilha: a mesma interface para os dois jeitos de mostrar algo
//! fixo na tela no Linux.
//!
//! - **Wayland com layer-shell** (KDE Plasma, Sway, Hyprland, COSMIC, niri…):
//!   uma "camada" presa no topo da tela, acima das janelas; o compositor
//!   centraliza; escala fracionada nítida. Ver `wayland.rs`.
//! - **X11** (XFCE, Cinnamon, MATE, i3… e o GNOME pelo XWayland, que não tem
//!   layer-shell): janela transparente sem moldura, posicionada por nós,
//!   logo abaixo da barra de cima se houver uma. Ver `x11.rs`.
//!
//! `TASKBEAT_BACKEND=x11` ou `=wayland` força um deles.

use crate::render::RectF;
use std::os::fd::RawFd;

/// Eventos da janela, já em pixels do buffer.
#[derive(Clone, Copy, Debug)]
pub enum WinEv {
    Motion(f32, f32),
    Leave,
    /// Botão do mouse: 1 = esquerdo, 2 = meio, 3 = direito.
    Button { down: bool, button: u32, x: f32, y: f32 },
    /// Rolagem (positivo = para cima), em "pontos" de volume.
    Scroll(f32),
    /// A escala da tela mudou (outra tela, outra configuração).
    Scale(f32),
    /// O conteúdo precisa ser redesenhado inteiro.
    Expose,
    /// O compositor fechou a superfície (tela removida).
    Closed,
}

pub trait Window {
    fn name(&self) -> &'static str;
    /// Descritor para o `poll` da thread principal.
    fn fd(&self) -> RawFd;
    /// Antes do `poll`: processa o que já chegou e envia o que falta.
    fn before_poll(&mut self);
    /// Depois do `poll` (`readable`: o descritor tem dados).
    fn after_poll(&mut self, readable: bool);
    fn take_events(&mut self) -> Vec<WinEv>;
    /// Pixels por ponto da tela.
    fn scale(&self) -> f32;
    /// Tamanho do buffer (px) que cabe um conteúdo de `w`×`h` px.
    fn fit(&self, w: f32, h: f32) -> (usize, usize);
    fn set_size(&mut self, w: usize, h: usize);
    /// Dá para mostrar um quadro agora (configurada, sem quadro pendente)?
    fn ready(&self) -> bool;
    /// Mostra o quadro `frame` (`w`×`h`); só a região `x0..x1, y0..y1` mudou.
    fn present(&mut self, frame: &[u32], w: usize, h: usize, region: (usize, usize, usize, usize)) -> bool;
    /// Onde o mouse conta (fora disso os cliques vão para o que está atrás).
    fn set_input(&mut self, r: Option<RectF>);
    /// Ficar acima de janelas em tela cheia.
    fn set_above_fullscreen(&mut self, _on: bool) {}
    /// Sem superfície para mostrar (fechada, esperando uma tela, ou recém-criada
    /// esperando o primeiro `configure`). Não adianta pedir quadros: o próximo
    /// evento chega pelo descritor da janela.
    fn suspended(&self) -> bool {
        false
    }
}

pub fn open(above_fullscreen: bool) -> Option<Box<dyn Window>> {
    let force = std::env::var("TASKBEAT_BACKEND").unwrap_or_default().to_lowercase();
    if force != "x11" {
        match super::wayland::Wl::open(above_fullscreen) {
            Ok(w) => return Some(Box::new(w)),
            Err(e) => super::system::log::info(&format!("janela: Wayland/layer-shell indisponível ({e})")),
        }
    }
    if force != "wayland" {
        match super::x11::X11::open() {
            Ok(w) => return Some(Box::new(w)),
            Err(e) => super::system::log::warn(&format!("janela: X11 indisponível ({e})")),
        }
    }
    None
}

/// União de duas regiões `(x0, y0, x1, y1)`.
pub fn union(a: Option<(usize, usize, usize, usize)>, b: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    match a {
        Some(a) => (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)),
        None => b,
    }
}
