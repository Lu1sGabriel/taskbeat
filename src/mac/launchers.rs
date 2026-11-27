//! Atalhos para abrir os apps de música instalados (linha embaixo do card
//! aberto no notch), como no Windows.
//!
//! O macOS acha o app pelo identificador do pacote (Launch Services), esteja
//! ele em /Applications, ~/Applications ou em outro lugar. Os apps instalados
//! pelo navegador (YouTube Music no Chrome, Brave ou Edge) também têm um. A
//! lista é refeita de tempos em tempos numa thread própria; abrir é o mesmo que
//! `open -b <id>` (traz para a frente se já estiver aberto).

use super::system::log;
use crate::config::{Config, NotchLaunchers, NotchWing};
use crate::shared::{Launcher, Shared};
use std::sync::Arc;
use std::time::Duration;

/// No máximo tantos atalhos (a linha cabe no card).
pub const MAX: usize = 7;
/// A lista é refeita de tempos em tempos (instalou ou removeu um app).
const RESCAN: Duration = Duration::from_secs(10 * 60);

/// Players conhecidos: (identificadores do pacote, na ordem de preferência).
/// A ordem da lista é também a dos atalhos no card.
const KNOWN: &[&[&str]] = &[
    &["com.spotify.client"],
    // YouTube Music instalado pelo navegador (mesmo id do app em todos).
    &["com.google.Chrome.app.cinhimbnkkghhklpknlkffjgod", "com.brave.Browser.app.cinhimbnkkghhklpknlkffjgod", "com.microsoft.edgemac.app.cinhimbnkkghhklpknlkffjgod"],
    &["com.apple.Music"],
    &["com.deezer.deezer-desktop"],
    &["com.tidal.desktop"],
    &["com.amazon.music"],
    &["com.qobuz.QobuzDesktop"],
    &["com.shazam.mac.Shazam"],
    &["tv.plex.plexamp"],
    &["com.coppertino.Vox"],
    &["com.swinsian.Swinsian"],
    &["com.foobar2000.mac"],
];

/// Apps de música instalados, na ordem de `KNOWN` (um de cada).
pub fn scan() -> Vec<Launcher> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    let ws = NSWorkspace::sharedWorkspace();
    let mut out = Vec::new();
    for ids in KNOWN {
        let Some(id) = ids.iter().find(|id| ws.URLForApplicationWithBundleIdentifier(&NSString::from_str(id)).is_some()) else { continue };
        let (name, icon) = super::media::app_info(id);
        out.push(Launcher { name, id: id.to_string(), icon });
    }
    out
}

/// O app "principal": o que está tocando agora (`playing`: identificador do
/// pacote do player), se for um dos achados; senão o primeiro da lista.
pub fn main_app(found: &[Launcher], cfg: &Config, playing: &str) -> Option<Launcher> {
    let visible = || found.iter().filter(|l| !cfg.launcher_hidden.contains(&l.id));
    visible().find(|l| !playing.is_empty() && l.id == playing).or_else(|| visible().next()).cloned()
}

/// O que aparece: (atalhos do card aberto, ícone da ilha fechada).
pub fn pick(found: &[Launcher], cfg: &Config, playing: &str) -> (Vec<Launcher>, Option<Launcher>) {
    let main = main_app(found, cfg, playing);
    let card = match cfg.notch_launchers {
        NotchLaunchers::All => found.iter().filter(|l| !cfg.launcher_hidden.contains(&l.id)).take(MAX).cloned().collect(),
        NotchLaunchers::One => main.iter().cloned().collect(),
        NotchLaunchers::Off => Vec::new(),
    };
    let wing = if cfg.notch_wing == NotchWing::Bars { None } else { main };
    (card, wing)
}

/// Abre (ou traz para a frente) o app.
pub fn open(id: &str) {
    if id.is_empty() || id.starts_with('-') {
        return;
    }
    match std::process::Command::new("/usr/bin/open").arg("-b").arg(id).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn(&format!("atalho: não abriu {id}: {e}")),
    }
}

/// Procura os apps agora e depois de tempos em tempos; `changed` avisa a ilha.
pub fn spawn(shared: Arc<Shared>, changed: fn()) {
    let _ = std::thread::Builder::new().name("launchers".into()).spawn(move || {
        super::system::background_thread();
        let mut last: Vec<String> = Vec::new();
        while shared.is_running() {
            let found = scan();
            let ids: Vec<String> = found.iter().map(|l| l.id.clone()).collect();
            if ids != last {
                log::info(&format!("atalhos: {}", found.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")));
                last = ids;
                *shared.launchers.lock().unwrap() = Arc::new(found);
                changed();
            }
            // Dorme em passos curtos para sair logo quando o app fecha.
            let mut slept = Duration::ZERO;
            while slept < RESCAN && shared.is_running() {
                std::thread::sleep(Duration::from_secs(2));
                slept += Duration::from_secs(2);
            }
        }
    });
}
