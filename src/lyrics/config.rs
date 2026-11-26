//! Opções da sobreposição de letra (tabela `[lyrics]` do config.toml).

use crate::config::Size3;
use serde::{Deserialize, Serialize};

/// O que a sobreposição mostra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// Só a letra (cinco linhas, a atual em destaque).
    Lyrics,
    /// Só o player (o card expandido).
    Player,
    /// O card em cima, a letra embaixo.
    PlayerLyrics,
    /// Mini player (capa pequena, anterior/play/próximo) com a letra.
    Compact,
    /// Legenda: a linha atual e a próxima, numa pílula.
    Caption,
}

impl Layout {
    pub fn has_lyrics(self) -> bool {
        self != Layout::Player
    }
}

/// Onde a janela fica no monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
    /// Onde o usuário soltou (`free_x`, `free_y`).
    Free,
}

impl Position {
    pub const GRID: [Position; 9] =
        [Position::TopLeft, Position::Top, Position::TopRight, Position::Left, Position::Center, Position::Right, Position::BottomLeft, Position::Bottom, Position::BottomRight];

    /// Frações (0, 0,5 ou 1) da área livre em x e y; `None` para "Livre".
    pub fn frac(self) -> Option<(f32, f32)> {
        let i = Self::GRID.iter().position(|p| *p == self)?;
        Some(((i % 3) as f32 * 0.5, (i / 3) as f32 * 0.5))
    }
}

/// Fundo da sobreposição.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Capa desfocada, como o card.
    Artwork,
    /// Cor extraída da capa.
    AlbumColor,
    /// Escuro translúcido.
    Dark,
    /// Sem fundo: só o texto, com sombra.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Centro em "Só a letra" e "Legenda"; à esquerda com o player.
    Auto,
    Left,
    Center,
}

/// Atalho global para mostrar/esconder (com Shift: travar/destravar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hotkey {
    WinAltL,
    CtrlAltL,
    CtrlShiftL,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsConfig {
    /// Sobreposição na tela (o atalho liga e desliga).
    pub show: bool,
    pub layout: Layout,
    pub position: Position,
    /// Lugar "Livre": centro da janela em frações da área de trabalho do monitor.
    pub free_x: f32,
    pub free_y: f32,
    /// Monitor: 0 = o principal; 1, 2… = os outros, da esquerda para a direita.
    pub monitor: i32,
    /// Travada: a posição fica presa (não arrasta); os botões continuam valendo.
    pub locked: bool,
    /// Travada, o clique atravessa a janela e vai para o que está atrás (jogos).
    pub click_through: bool,
    /// Tamanho da janela (%), além da escala do monitor.
    pub zoom: i32,
    /// Volume e mixer de volume nos layouts com player (como no card).
    pub volume: bool,
    pub mixer: bool,
    /// Fundo animado na tela cheia (a capa desfocada girando devagar).
    pub animated_bg: bool,
    /// Tempo decorrido e restante embaixo do progresso (a barra sempre aparece).
    pub show_time: bool,
    /// Procurar a letra no LRCLIB (manda título, artista, álbum e duração).
    pub online: bool,
    /// Ajuste de tempo (ms): positivo adianta a letra.
    pub offset_ms: i32,
    pub size: Size3,
    pub align: Align,
    pub background: Background,
    /// Opacidade do fundo (%).
    pub opacity: i32,
    pub hotkey: Hotkey,
    /// Some quando um app em tela cheia está na frente, no mesmo monitor.
    pub hide_fullscreen: bool,
    /// Some quando nada está tocando (sem player aberto).
    pub hide_idle: bool,
    /// Botão no card aberto (Windows) que mostra e esconde a sobreposição.
    pub card_button: bool,
}

/// Limites do tamanho da janela (%).
pub const ZOOM_MIN: i32 = 60;
pub const ZOOM_MAX: i32 = 200;

impl Default for LyricsConfig {
    fn default() -> Self {
        Self {
            show: false,
            layout: Layout::Compact,
            position: Position::TopRight,
            free_x: 0.5,
            free_y: 0.5,
            monitor: 0,
            locked: false,
            click_through: false,
            zoom: 100,
            volume: true,
            mixer: true,
            animated_bg: true,
            show_time: false,
            online: true,
            offset_ms: 0,
            size: Size3::Normal,
            align: Align::Auto,
            background: Background::Artwork,
            opacity: 100,
            hotkey: Hotkey::WinAltL,
            hide_fullscreen: false,
            hide_idle: true,
            card_button: true,
        }
    }
}

impl LyricsConfig {
    pub fn sanitize(&mut self) {
        self.free_x = if self.free_x.is_finite() { self.free_x.clamp(0.0, 1.0) } else { 0.5 };
        self.free_y = if self.free_y.is_finite() { self.free_y.clamp(0.0, 1.0) } else { 0.5 };
        self.monitor = self.monitor.clamp(0, 8);
        self.offset_ms = self.offset_ms.clamp(-5000, 5000);
        self.opacity = self.opacity.clamp(10, 100);
        self.zoom = self.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Letra centralizada?
    pub fn centered(&self) -> bool {
        match self.align {
            Align::Auto => matches!(self.layout, Layout::Lyrics | Layout::Caption),
            a => a == Align::Center,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_sem_lyrics_e_ida_e_volta() {
        // Config antiga (sem a tabela): os padrões.
        let c: crate::config::Config = toml::from_str("show_media = true\nversion = 5\n").unwrap();
        assert_eq!(c.lyrics, LyricsConfig::default());
        let mut c2 = c.clone();
        c2.lyrics = LyricsConfig { show: true, layout: Layout::Caption, position: Position::Free, free_x: 0.25, monitor: 1, locked: true, ..Default::default() };
        let text = toml::to_string_pretty(&c2).unwrap();
        assert!(text.contains("[lyrics]"));
        let back: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(back.lyrics, c2.lyrics);
    }

    #[test]
    fn grade_de_posicoes() {
        assert_eq!(Position::TopLeft.frac(), Some((0.0, 0.0)));
        assert_eq!(Position::Bottom.frac(), Some((0.5, 1.0)));
        assert_eq!(Position::Right.frac(), Some((1.0, 0.5)));
        assert_eq!(Position::Free.frac(), None);
    }
}
