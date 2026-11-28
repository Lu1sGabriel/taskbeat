//! Player de teste do soak no Windows: toca um WAV em loop pelo
//! `Windows.Media.Playback.MediaPlayer`, que aparece nos controles de mídia do
//! sistema (SMTC) como qualquer player. É o equivalente do
//! `macos/ci-nowplaying.swift` e do `linux/ci-mpris.py`.
//!
//! Uso: `cargo run --release --example ci_player [segundos]` (sem número: até
//! ser fechado). Escreve "tocando" quando começa.
//!
//! Num runner sem placa de som o `MediaPlayer` pode não tocar; aí o soak mede
//! o app parado (o script registra o que aconteceu).

#[cfg(windows)]
fn main() {
    let secs: Option<u64> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    if let Err(e) = run(secs) {
        eprintln!("ci_player: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ci_player: só no Windows (no Mac e no Linux há macos/ci-nowplaying.swift e linux/ci-mpris.py)");
}

/// 10 s de um acorde que pulsa (as barras sobem e descem), PCM 16 bits estéreo.
#[cfg(windows)]
fn wav() -> Vec<u8> {
    const RATE: u32 = 48_000;
    let frames = RATE * 10;
    let data_len = frames * 4;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // estéreo
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    let tau = std::f32::consts::TAU;
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        let env = 0.35 + 0.65 * (tau * t / 2.0).sin().abs();
        let v: f32 = [110.0f32, 220.0, 330.0, 660.0, 1320.0, 2640.0].iter().enumerate().map(|(k, f)| (tau * f * t).sin() / (k + 1) as f32).sum();
        let s = ((v * 0.22 * env).clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&s.to_le_bytes());
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(windows)]
fn run(secs: Option<u64>) -> windows::core::Result<()> {
    use windows::Foundation::Uri;
    use windows::Media::Core::MediaSource;
    use windows::Media::MediaPlaybackType;
    use windows::Media::Playback::MediaPlayer;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    // O WinRT pede o COM ligado na thread (MTA: o MediaPlayer tem as próprias).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let path = std::env::temp_dir().join(format!("taskbeat-ci-{}.wav", std::process::id()));
    std::fs::write(&path, wav()).map_err(|e| windows::core::Error::new(windows::core::HRESULT(0x8000_4005u32 as i32), e.to_string()))?;
    let url = format!("file:///{}", path.display().to_string().replace('\\', "/"));

    let player = MediaPlayer::new()?;
    player.SetSource(&MediaSource::CreateFromUri(&Uri::CreateUri(&HSTRING::from(url))?)?)?;
    player.SetIsLoopingEnabled(true)?;
    player.SetVolume(0.2)?;
    // Título e artista nos controles do sistema (o que o Taskbeat lê).
    let smtc = player.SystemMediaTransportControls()?;
    let du = smtc.DisplayUpdater()?;
    du.SetType(MediaPlaybackType::Music)?;
    let mp = du.MusicProperties()?;
    mp.SetTitle(&HSTRING::from("Taskbeat CI"))?;
    mp.SetArtist(&HSTRING::from("GitHub Actions"))?;
    mp.SetAlbumTitle(&HSTRING::from("Windows Runner"))?;
    du.Update()?;
    player.Play()?;
    println!("tocando {}", path.display());

    let start = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if secs.is_some_and(|s| start.elapsed().as_secs() >= s) {
            break;
        }
    }
    player.Pause()?;
    let _ = std::fs::remove_file(&path);
    Ok(())
}
