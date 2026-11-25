//! Estado compartilhado entre as threads (áudio, mídia, overlay, bandeja).

use crate::config::Config;
use crate::render::Image;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};

/// Buffer circular com as amostras mono mais recentes capturadas por loopback.
pub struct AudioRing {
    buf: Vec<f32>,
    pos: usize,
    /// Total de amostras já escritas (permite detectar "nada novo chegou").
    pub written: u64,
}

impl AudioRing {
    pub const CAPACITY: usize = 16384;

    pub fn new() -> Self {
        Self { buf: vec![0.0; Self::CAPACITY], pos: 0, written: 0 }
    }

    pub fn push(&mut self, s: f32) {
        self.buf[self.pos] = s;
        self.pos = (self.pos + 1) % self.buf.len();
        self.written += 1;
    }

    /// Copia as últimas `out.len()` amostras (da mais antiga para a mais nova).
    pub fn latest(&self, out: &mut [f32]) {
        let n = out.len().min(self.buf.len());
        let start = (self.pos + self.buf.len() - n) % self.buf.len();
        for (i, o) in out.iter_mut().take(n).enumerate() {
            *o = self.buf[(start + i) % self.buf.len()];
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MediaCmd {
    Previous,
    PlayPause,
    Next,
    /// Liga/desliga a ordem aleatória.
    Shuffle(bool),
    /// 0 = não repetir, 1 = repetir a música, 2 = repetir a playlist.
    Repeat(u8),
    /// Ir para a posição (segundos desde o início da faixa).
    Seek(f64),
    /// Windows: aviso interno de que algo mudou (sessão, música, play/pause).
    /// Mac: o card abriu (relê aleatório/repetir dos players por AppleScript).
    Refresh,
}

#[derive(Clone, Default)]
pub struct MediaInfo {
    pub playing: bool,
    /// AppUserModelId da sessão (ex.: "Spotify.exe", "MSEdge").
    pub aumid: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    /// Nome do álbum (nem todo player informa).
    pub album: String,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_toggle: bool,
    pub can_shuffle: bool,
    pub can_repeat: bool,
    pub shuffle: bool,
    /// 0 = não repetir, 1 = música, 2 = playlist.
    pub repeat: u8,
    /// Capa do álbum / miniatura do vídeo (quadrada, BGRA pré-multiplicado).
    pub art: Option<Arc<Image>>,
    /// Ícone do aplicativo que está tocando.
    pub icon: Option<Arc<Image>>,
    /// Incrementa a cada troca de faixa *automática* (a anterior terminou).
    pub auto_change_seq: u64,
    /// A faixa acabou de mudar e a capa ainda é a da faixa anterior (ou nenhuma):
    /// o player costuma publicar a imagem nova um pouco depois do título.
    pub art_pending: bool,
    /// Linha do tempo: posição (s) medida em `pos_at`, duração (s; 0 = desconhecida).
    pub pos: f64,
    pub dur: f64,
    pub pos_at: Option<std::time::Instant>,
    pub can_seek: bool,
    /// Aleatório e repetir vêm dos botões da página do player (navegador),
    /// não do Windows. No YouTube Music o aleatório embaralha a fila de novo a
    /// cada clique (não é um modo ligado/desligado).
    pub web_shuffle: bool,
    pub web_repeat: bool,
}

/// O que se sabe dos botões da página de um player de navegador (Windows, `webplayer.rs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebState {
    /// Sessão (AUMID) a que isto se refere.
    pub aumid: String,
    /// Botão de embaralhar achado.
    pub shuffle: bool,
    /// Estado do repetir lido do botão (0 = desligado, 1 = uma, 2 = tudo).
    pub repeat: Option<u8>,
}

impl WebState {
    /// Junta à música os controles da página que o Windows não oferece.
    pub fn merge_into(&self, m: &mut MediaInfo) {
        if self.aumid.is_empty() || self.aumid != m.aumid {
            return;
        }
        if !m.can_shuffle && self.shuffle {
            m.can_shuffle = true;
            m.shuffle = false;
            m.web_shuffle = true;
        }
        if let (false, Some(r)) = (m.can_repeat, self.repeat) {
            m.can_repeat = true;
            m.repeat = r;
            m.web_repeat = true;
        }
    }
}

/// Linhas do mixer de volume no máximo (o card não cresce sem fim).
pub const MIXER_ROWS: usize = 6;

/// Um app no mixer de volume do card (sessões de áudio do mesmo app juntas).
#[derive(Clone)]
pub struct MixApp {
    /// Identifica o app (caminho do .exe; "system" para os sons do sistema).
    pub key: String,
    pub name: String,
    pub icon: Option<Arc<Image>>,
    pub volume: f32,
    pub muted: bool,
    /// Tocando som agora (as outras sessões estão abertas, mas em silêncio).
    pub active: bool,
    /// Sons do sistema (avisos do Windows).
    pub system: bool,
}

/// O que o mixer mostra: o dispositivo de saída e os apps com som nele.
#[derive(Clone, Default)]
pub struct MixerState {
    pub device: String,
    pub apps: Vec<MixApp>,
}

/// Pedidos para a thread do mixer (só trabalha com ele aberto).
#[derive(Clone, Debug)]
pub enum MixCmd {
    Open,
    Close,
    /// Volume (0..1) de um app (`MixApp::key`).
    Volume(String, f32),
    Mute(String, bool),
    /// Algo mudou (aviso do Windows): relê.
    Refresh,
}

/// Pedidos para a thread que aperta os botões da página do player.
#[derive(Clone, Debug)]
pub enum WebCmd {
    /// Procura os botões e lê o repetir.
    Probe { aumid: String, title: String },
    Shuffle { aumid: String, title: String },
    Repeat { aumid: String, title: String },
}

impl MediaInfo {
    /// Posição estimada agora (avança sozinha enquanto toca).
    pub fn position_now(&self) -> f64 {
        let extra = match (self.playing, self.pos_at) {
            (true, Some(t)) => t.elapsed().as_secs_f64(),
            _ => 0.0,
        };
        let p = self.pos + extra;
        if self.dur > 0.0 { p.clamp(0.0, self.dur) } else { p.max(0.0) }
    }
}

impl MediaInfo {
    pub fn present(&self) -> bool {
        !self.title.is_empty() || !self.aumid.is_empty()
    }
}

fn same_img(a: &Option<Arc<Image>>, b: &Option<Arc<Image>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl PartialEq for MediaInfo {
    fn eq(&self, o: &Self) -> bool {
        self.playing == o.playing
            && self.aumid == o.aumid
            && self.app == o.app
            && self.title == o.title
            && self.artist == o.artist
            && self.album == o.album
            && self.art_pending == o.art_pending
            && self.can_prev == o.can_prev
            && self.can_next == o.can_next
            && self.can_toggle == o.can_toggle
            && self.can_shuffle == o.can_shuffle
            && self.can_repeat == o.can_repeat
            && self.shuffle == o.shuffle
            && self.repeat == o.repeat
            && same_img(&self.art, &o.art)
            && same_img(&self.icon, &o.icon)
            && self.auto_change_seq == o.auto_change_seq
            && self.can_seek == o.can_seek
            && (self.dur - o.dur).abs() < 0.5
    }
}

/// App de música instalado (atalho para abrir no card).
#[derive(Clone)]
pub struct Launcher {
    pub name: String,
    /// Como abrir: o AUMID da pasta "Aplicativos" do Windows.
    pub id: String,
    pub icon: Option<Arc<Image>>,
}

pub struct Shared {
    pub running: AtomicBool,
    /// Apps de música instalados (atalhos do card), do mais conhecido ao menos.
    pub launchers: Mutex<Arc<Vec<Launcher>>>,
    pub audio: Mutex<AudioRing>,
    pub sample_rate: AtomicU32,
    pub config: Mutex<Config>,
    pub config_version: AtomicU64,
    pub media: Mutex<MediaInfo>,
    /// Incrementa a cada mudança em `media` (o overlay só copia quando muda).
    pub media_version: AtomicU64,
    /// HWND da janela oculta da bandeja (para a thread de mídia avisar mudanças).
    pub tray_hwnd: AtomicIsize,
    /// Canal para a thread de mídia executar anterior/play/próximo.
    pub media_tx: Mutex<Option<Sender<MediaCmd>>>,
    /// Pedido da tela de configurações para mostrar o aviso de Caps Lock agora
    /// (botão "Mostrar agora" e ajuste de posição ao vivo).
    pub osd_test: AtomicBool,
    /// Botões da página do player de navegador (aleatório/repetir).
    pub web: Mutex<WebState>,
    pub web_tx: Mutex<Option<Sender<WebCmd>>>,
    /// Mixer de volume do card: estado, versão (muda a cada leitura) e canal.
    pub mixer: Mutex<MixerState>,
    pub mixer_version: AtomicU64,
    pub mixer_tx: Mutex<Option<Sender<MixCmd>>>,
}

impl Shared {
    pub fn new(config: Config) -> Self {
        Self {
            running: AtomicBool::new(true),
            launchers: Mutex::new(Arc::new(Vec::new())),
            audio: Mutex::new(AudioRing::new()),
            sample_rate: AtomicU32::new(48000),
            config: Mutex::new(config),
            config_version: AtomicU64::new(1),
            media: Mutex::new(MediaInfo::default()),
            media_version: AtomicU64::new(0),
            tray_hwnd: AtomicIsize::new(0),
            media_tx: Mutex::new(None),
            osd_test: AtomicBool::new(false),
            web: Mutex::new(WebState::default()),
            web_tx: Mutex::new(None),
            mixer: Mutex::new(MixerState::default()),
            mixer_version: AtomicU64::new(0),
            mixer_tx: Mutex::new(None),
        }
    }

    pub fn send_mixer(&self, cmd: MixCmd) {
        if let Some(tx) = self.mixer_tx.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    pub fn send_web(&self, cmd: WebCmd) {
        if let Some(tx) = self.web_tx.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn send_media(&self, cmd: MediaCmd) {
        if let Some(tx) = self.media_tx.lock().unwrap().as_ref() {
            let _ = tx.send(cmd);
        }
    }

    /// Aplica sem gravar no disco (enquanto o usuário arrasta um slider).
    pub fn preview_config(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.config.lock().unwrap());
        self.config_version.fetch_add(1, Ordering::Relaxed);
    }

    pub fn save_config(&self) {
        self.config().save();
    }

    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        let snapshot = {
            let mut cfg = self.config.lock().unwrap();
            f(&mut cfg);
            cfg.clone()
        };
        snapshot.save();
        self.config_version.fetch_add(1, Ordering::Relaxed);
    }
}
