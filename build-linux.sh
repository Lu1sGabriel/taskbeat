#!/bin/bash
# Gera dist/taskbeat_<versão>_amd64.deb e dist/Taskbeat-<versão>-x86_64.AppImage
# (num Linux com Rust e as bibliotecas de desenvolvimento: dbus, pulse,
# freetype, fontconfig e harfbuzz). Para rodar em distros mais antigas,
# compile na mais antiga que quiser atender (o CI usa o Ubuntu 22.04).
# Uso: ./build-linux.sh
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1 | tr -d '\r')
DEB_VERSION=${VERSION//-/\~}
ID=io.github.lu1sgabriel.taskbeat
# appimagetool e runtime type2 em versão fixa, conferidos pelo sha256 (o
# projeto não publica checksum: os valores foram tirados baixando uma vez). Sem
# o `--runtime-file`, o appimagetool baixaria o runtime "continuous" a cada build.
APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

cargo build --release --locked
BIN=target/release/taskbeat

# Ícones em vários tamanhos (o tema hicolor escolhe o certo).
icons() { # pasta-raiz
    for s in 32 48 64 128 256; do
        mkdir -p "$1/usr/share/icons/hicolor/${s}x${s}/apps"
        if command -v convert >/dev/null; then
            convert assets/taskbeat-256.png -resize ${s}x${s} "$1/usr/share/icons/hicolor/${s}x${s}/apps/taskbeat.png"
        else
            cp assets/taskbeat-256.png "$1/usr/share/icons/hicolor/${s}x${s}/apps/taskbeat.png"
        fi
    done
}

# ---- .deb
DEB=build/deb
rm -rf "$DEB" && mkdir -p "$DEB/DEBIAN" "$DEB/usr/bin" "$DEB/usr/share/applications" "$DEB/usr/share/doc/taskbeat"
install -m 755 "$BIN" "$DEB/usr/bin/taskbeat"
install -m 644 linux/$ID.desktop "$DEB/usr/share/applications/$ID.desktop"
icons "$DEB"
{
    echo "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/"
    echo "Upstream-Name: Taskbeat"
    echo "Source: https://github.com/Lu1sGabriel/taskbeat"
    echo
    echo "Files: *"
    echo "License: MIT"
    sed 's/^$/./; s/^/ /' LICENSE
} >"$DEB/usr/share/doc/taskbeat/copyright"
gzip -9n <THIRD-PARTY-NOTICES.md >"$DEB/usr/share/doc/taskbeat/THIRD-PARTY-NOTICES.md.gz"
SIZE=$(du -sk "$DEB/usr" | cut -f1)
cat >"$DEB/DEBIAN/control" <<EOF
Package: taskbeat
Version: $DEB_VERSION
Architecture: amd64
Maintainer: Lu1sGabriel <https://github.com/Lu1sGabriel/taskbeat>
Installed-Size: $SIZE
Depends: libc6 (>= 2.35), libdbus-1-3, libpulse0, libfreetype6, libfontconfig1, libharfbuzz0b
Recommends: curl
Section: sound
Priority: optional
Homepage: https://github.com/Lu1sGabriel/taskbeat
Description: Music widget at the top of the screen
 A small "island" at the top of the screen with the cover, the title and
 live audio bars of what is playing (any MPRIS player: Spotify, browsers,
 VLC, Rhythmbox…). It expands on hover into a card with the timeline,
 controls, shuffle, repeat and volume.
EOF
mkdir -p dist
dpkg-deb --root-owner-group --build "$DEB" "dist/taskbeat_${DEB_VERSION}_amd64.deb" >/dev/null

# ---- AppImage
APPDIR=build/Taskbeat.AppDir
rm -rf "$APPDIR" && mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications"
install -m 755 "$BIN" "$APPDIR/usr/bin/taskbeat"
install -m 644 linux/$ID.desktop "$APPDIR/usr/share/applications/$ID.desktop"
icons "$APPDIR"
cp linux/$ID.desktop "$APPDIR/$ID.desktop"
cp assets/taskbeat-256.png "$APPDIR/taskbeat.png"
cp LICENSE THIRD-PARTY-NOTICES.md "$APPDIR/"
ln -sf usr/bin/taskbeat "$APPDIR/AppRun"
# Baixa (se faltar) e confere o hash; um arquivo que não confere é apagado.
fetch() { # url sha256 destino
    if [ ! -f "$3" ] || ! echo "$2  $3" | sha256sum -c --status; then
        curl -sSfL -o "$3" "$1"
    fi
    if ! echo "$2  $3" | sha256sum -c --status; then
        echo "sha256 não confere: $3 ($1)" >&2
        rm -f "$3"
        exit 1
    fi
}
TOOL=build/appimagetool-1.9.1-x86_64.AppImage
RUNTIME=build/runtime-20251108-x86_64
fetch "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "$TOOL"
fetch "$RUNTIME_URL" "$RUNTIME_SHA256" "$RUNTIME"
chmod +x "$TOOL"
# --appimage-extract-and-run: funciona sem FUSE (CI, contêineres).
ARCH=x86_64 "$TOOL" --appimage-extract-and-run --no-appstream --runtime-file "$RUNTIME" "$APPDIR" "dist/Taskbeat-$VERSION-x86_64.AppImage" >/dev/null

echo "Pronto:"
ls -lh dist/taskbeat_*"$DEB_VERSION"*.deb "dist/Taskbeat-$VERSION-x86_64.AppImage"
