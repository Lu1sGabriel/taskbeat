// CI: põe a tela principal num modo HiDPI (2 pixels por ponto), se o monitor
// virtual do runner tiver um, para os prints do app rodando saírem como num
// Mac com tela Retina (sem isso o runner fica em 1x e não mostra se algo é
// desenhado em 1x e ampliado). Sem modo HiDPI, não muda nada.
// Uso: swift macos/ci-hidpi.swift [list]
import AppKit
import CoreGraphics

let display = CGMainDisplayID()
let opts = [kCGDisplayShowDuplicateLowResolutionModes: kCFBooleanTrue] as CFDictionary
let modes = (CGDisplayCopyAllDisplayModes(display, opts) as? [CGDisplayMode]) ?? []
if let cur = CGDisplayCopyDisplayMode(display) {
    print("atual: \(cur.width)x\(cur.height) pt, \(cur.pixelWidth)x\(cur.pixelHeight) px")
}
for m in modes {
    print("modo: \(m.width)x\(m.height) pt, \(m.pixelWidth)x\(m.pixelHeight) px")
}
if CommandLine.arguments.contains("list") {
    exit(0)
}
// O maior modo 2x (a ilha aberta e as configurações precisam de espaço).
guard let mode = modes.filter({ $0.pixelWidth == 2 * $0.width && $0.width >= 800 }).max(by: { $0.width < $1.width }) else {
    print("sem modo HiDPI: os prints seguem em 1x")
    exit(0)
}
var config: CGDisplayConfigRef?
CGBeginDisplayConfiguration(&config)
CGConfigureDisplayWithDisplayMode(config, display, mode, nil)
let err = CGCompleteDisplayConfiguration(config, .forSession)
Thread.sleep(forTimeInterval: 2)
let scale = NSScreen.main?.backingScaleFactor ?? 0
print("HiDPI: \(mode.width)x\(mode.height) pt em \(mode.pixelWidth)x\(mode.pixelHeight) px, erro \(err.rawValue), escala \(scale)")
