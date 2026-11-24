//! Utilidades do sistema no Mac: cor de destaque e tema, abrir ao iniciar a
//! sessão, prioridade das threads de fundo e registro de diagnóstico.

use crate::render::{Colors, Rgb};
use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSColor, NSColorSpace};
use objc2_foundation::{MainThreadMarker, NSArray};
use std::path::PathBuf;

/// Thread de segundo plano: QoS "utility" (núcleos de eficiência no Apple Silicon).
pub fn background_thread() {
    // QOS_CLASS_UTILITY = 0x11
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos: u32, relative_priority: i32) -> i32;
    }
    unsafe {
        pthread_set_qos_class_self_np(0x11, 0);
    }
}

fn rgb_of(c: &NSColor) -> Option<Rgb> {
    let c = c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some([c.redComponent() as f32, c.greenComponent() as f32, c.blueComponent() as f32])
}

/// Modo escuro do sistema (Ajustes > Aparência).
pub fn dark_mode(mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe { NSAppearanceNameDarkAqua }]);
    app.effectiveAppearance().bestMatchFromAppearancesWithNames(&names).is_some_and(|n| &*n == unsafe { NSAppearanceNameDarkAqua })
}

/// Cor de destaque do macOS e uma versão clara (para fundos escuros).
pub fn colors(mtm: MainThreadMarker, palette: crate::config::Palette) -> Colors {
    let accent = rgb_of(&NSColor::controlAccentColor()).unwrap_or([0.0, 0.48, 1.0]);
    let light = crate::render::lerp3(accent, [1.0; 3], 0.25);
    Colors { palette, accent, accent_light: light, light_taskbar: !dark_mode(mtm), album: None }
}

// ---------------------------------------------------------------- abrir ao iniciar a sessão

const AGENT_LABEL: &str = "io.github.lu1sgabriel.taskbeat";

fn agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

/// Pasta `.app` em que o executável está (se estiver num pacote).
pub fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut p = exe.as_path();
    while let Some(parent) = p.parent() {
        if p.extension().is_some_and(|e| e == "app") {
            return Some(p.to_path_buf());
        }
        p = parent;
    }
    None
}

/// Abre ao iniciar a sessão? (agente do launchd em ~/Library/LaunchAgents.)
pub fn autostart_enabled() -> bool {
    agent_path().is_some_and(|p| p.exists())
}

/// Liga/desliga o início com a sessão. Usa um agente do launchd que roda
/// `open -a` no pacote: funciona sem assinatura da Apple (o SMAppService exige).
pub fn set_autostart(on: bool) {
    let Some(path) = agent_path() else { return };
    if !on {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let target = app_bundle().map(|p| p.display().to_string()).or_else(|| std::env::current_exe().ok().map(|p| p.display().to_string()));
    let Some(target) = target else { return };
    let esc = target.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let args = if target.ends_with(".app") {
        format!("<string>/usr/bin/open</string>\n        <string>-a</string>\n        <string>{esc}</string>")
    } else {
        format!("<string>{esc}</string>")
    };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        {args}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, plist);
}

// ---------------------------------------------------------------- registro

/// Diagnóstico: com `TASKBEAT_LOG=/caminho/arquivo.log` grava cada evento.
pub mod log {
    use std::io::Write;
    use std::sync::OnceLock;

    fn file() -> Option<&'static std::path::PathBuf> {
        static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
        PATH.get_or_init(|| std::env::var_os("TASKBEAT_LOG").map(Into::into)).as_ref()
    }

    fn write(level: &str, msg: &str) {
        #[cfg(debug_assertions)]
        eprintln!("[{level}] {msg}");
        if let Some(p) = file() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                let _ = writeln!(f, "{t:.3} [{level}] {msg}");
            }
        }
    }

    pub fn info(msg: &str) {
        write("info", msg);
    }

    pub fn warn(msg: &str) {
        write("warn", msg);
    }
}
