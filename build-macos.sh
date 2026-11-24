#!/bin/bash
# Gera dist/Taskbeat.app e dist/Taskbeat-<versão>.dmg (num Mac com Xcode CLT e Rust).
# Binário universal (Apple Silicon + Intel), adaptador do "Tocando Agora"
# (mediaremote-adapter, BSD-3) embutido e assinatura ad-hoc.
# Uso: ./build-macos.sh
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ADAPTER_REPO=https://github.com/ungive/mediaremote-adapter.git
ADAPTER_REV=2ef957c736256cee7905c9f3f1c7e279087bf195
export MACOSX_DEPLOYMENT_TARGET=14.2

rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# Adaptador do Tocando Agora (versão fixa).
AD=build/mediaremote-adapter
if [ ! -d "$AD/.git" ] || [ "$(git -C "$AD" rev-parse HEAD)" != "$ADAPTER_REV" ]; then
    rm -rf "$AD"
    mkdir -p "$AD"
    git -C "$AD" init -q
    git -C "$AD" remote add origin "$ADAPTER_REPO"
    git -C "$AD" fetch -q --depth 1 origin "$ADAPTER_REV"
    git -C "$AD" checkout -q FETCH_HEAD
fi
cmake -S "$AD" -B "$AD/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$AD/build" --config Release >/dev/null

APP=dist/Taskbeat.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
lipo -create -output "$APP/Contents/MacOS/Taskbeat" \
    target/aarch64-apple-darwin/release/taskbeat target/x86_64-apple-darwin/release/taskbeat
sed "s/__VERSION__/$VERSION/g" macos/Info.plist > "$APP/Contents/Info.plist"
for l in en pt-BR es de; do
    cp -R "macos/$l.lproj" "$APP/Contents/Resources/"
done
cp -R "$AD/build/MediaRemoteAdapter.framework" "$APP/Contents/Frameworks/"
cp "$AD/bin/mediaremote-adapter.pl" "$APP/Contents/Resources/"
cp LICENSE "$APP/Contents/Resources/LICENSE.txt"
cp THIRD-PARTY-NOTICES.md "$APP/Contents/Resources/THIRD-PARTY-NOTICES.txt"
cp "$AD/LICENSE" "$APP/Contents/Resources/mediaremote-adapter-LICENSE.txt" 2>/dev/null || true

# Ícone (.icns) a partir do PNG 1024 (assets/make_icon.py --mac).
ICONSET=build/Taskbeat.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s assets/taskbeat-mac-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d assets/taskbeat-mac-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Taskbeat.icns"

# Assinatura ad-hoc (sem conta de desenvolvedor da Apple).
codesign --force --deep --sign - "$APP"

# .dmg com atalho para Aplicativos.
STAGE=build/dmg
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
DMG="dist/Taskbeat-$VERSION.dmg"
rm -f "$DMG"
hdiutil create -quiet -volname "Taskbeat $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
echo "Pronto: $APP e $DMG ($(du -h "$DMG" | cut -f1))"
