// CI: um "player" de teste que publica uma música no Tocando Agora do macOS
// (título, artista, álbum, capa, tempo) e toca um áudio baixinho em loop, para
// testar o Taskbeat de ponta a ponta sem Spotify nem Música.
// Registra no stdout os comandos que receber (play/pause/próxima…).
// Uso: swift macos/ci-nowplaying.swift <áudio> [segundos]
import AppKit
import AVFoundation
import MediaPlayer

let args = CommandLine.arguments
let player = try! AVAudioPlayer(contentsOf: URL(fileURLWithPath: args[1]))
player.numberOfLoops = -1
player.volume = 0.2
player.play()

let size = NSSize(width: 300, height: 300)
let art = NSImage(size: size, flipped: false) { r in
    NSGradient(colors: [.systemPink, .systemOrange])!.draw(in: r, angle: 45)
    NSColor.white.setFill()
    NSBezierPath(ovalIn: NSRect(x: 105, y: 105, width: 90, height: 90)).fill()
    return true
}
MPNowPlayingInfoCenter.default().nowPlayingInfo = [
    MPMediaItemPropertyTitle: "Taskbeat CI",
    MPMediaItemPropertyArtist: "GitHub Actions",
    MPMediaItemPropertyAlbumTitle: "macOS Runner",
    MPMediaItemPropertyPlaybackDuration: 180.0,
    MPNowPlayingInfoPropertyElapsedPlaybackTime: 42.0,
    MPNowPlayingInfoPropertyPlaybackRate: 1.0,
    MPMediaItemPropertyArtwork: MPMediaItemArtwork(boundsSize: size) { _ in art },
]
MPNowPlayingInfoCenter.default().playbackState = .playing

func log(_ s: String) -> MPRemoteCommandHandlerStatus {
    print("comando: \(s)")
    fflush(stdout)
    return .success
}
let cc = MPRemoteCommandCenter.shared()
cc.togglePlayPauseCommand.addTarget { _ in log("toggle") }
cc.playCommand.addTarget { _ in log("play") }
cc.pauseCommand.addTarget { _ in log("pause") }
cc.nextTrackCommand.addTarget { _ in log("next") }
cc.previousTrackCommand.addTarget { _ in log("previous") }
cc.changePlaybackPositionCommand.addTarget { e in
    log("seek \((e as! MPChangePlaybackPositionCommandEvent).positionTime)")
}

print("tocando")
fflush(stdout)
RunLoop.main.run(until: Date().addingTimeInterval(Double(args.count > 2 ? args[2] : "120") ?? 120))
