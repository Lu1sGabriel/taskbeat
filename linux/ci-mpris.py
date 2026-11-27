#!/usr/bin/env python3
"""Player de teste para o CI do Linux: publica uma música no MPRIS (D-Bus),
com capa, linha do tempo, aleatório e repetir, e toca um som pelo
PulseAudio/PipeWire (paplay) para as barras se mexerem.

Cada comando recebido (PlayPause, Next, Shuffle…) é escrito na saída, para o
script de capturas conferir que o Taskbeat controla o player de verdade.

Uso: ci-mpris.py <capa.png> <capa2.png>
"""
import math
import os
import struct
import subprocess
import sys
import tempfile
import time
import wave

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

ROOT = "org.mpris.MediaPlayer2"
PLAYER = "org.mpris.MediaPlayer2.Player"
PROPS = "org.freedesktop.DBus.Properties"
LENGTH = 243_000_000  # 4:03 em microssegundos


def log(*a):
    print(*a, flush=True)


def make_wav(path):
    """10 s de um acorde que pulsa (as barras sobem e descem)."""
    rate = 48000
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(rate)
        frames = bytearray()
        for i in range(rate * 10):
            t = i / rate
            env = 0.35 + 0.65 * abs(math.sin(math.pi * 2.0 * t))
            v = sum(math.sin(2 * math.pi * f * t) / (k + 1) for k, f in enumerate((110, 220, 330, 660, 1320, 2640)))
            s = int(max(-1.0, min(1.0, v * 0.22 * env)) * 32767)
            frames += struct.pack("<hh", s, s)
        w.writeframes(bytes(frames))


class Player(dbus.service.Object):
    def __init__(self, bus, arts):
        super().__init__(bus, "/org/mpris/MediaPlayer2")
        self.tracks = [
            ("Midnight City", ["M83"], "Hurry Up, We're Dreaming", arts[0]),
            ("Wait", ["M83"], "Hurry Up, We're Dreaming", arts[1]),
        ]
        self.track = 0
        self.status = "Playing"
        self.shuffle = False
        self.loop = "None"
        self.pos = 110_000_000
        self.since = time.monotonic()
        self.wav = os.path.join(tempfile.gettempdir(), "taskbeat-ci.wav")
        make_wav(self.wav)
        self.audio = None
        self.start_audio()

    # ---- som
    def start_audio(self):
        if self.audio is None:
            self.audio = subprocess.Popen(
                ["bash", "-c", f'while true; do paplay --client-name="CI Player" "{self.wav}" || sleep 1; done'],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=False,
            )

    def stop_audio(self):
        if self.audio is not None:
            subprocess.run(["pkill", "-P", str(self.audio.pid)])
            self.audio.kill()
            self.audio = None

    # ---- estado
    def position(self):
        p = self.pos
        if self.status == "Playing":
            p += int((time.monotonic() - self.since) * 1e6)
        return min(p, LENGTH)

    def metadata(self):
        title, artist, album, art = self.tracks[self.track]
        return dbus.Dictionary(
            {
                "mpris:trackid": dbus.ObjectPath(f"/io/github/taskbeat/ci/track{self.track}"),
                "mpris:length": dbus.Int64(LENGTH),
                "xesam:title": title,
                "xesam:artist": dbus.Array(artist, signature="s"),
                "xesam:album": album,
                "mpris:artUrl": "file://" + art,
            },
            signature="sv",
        )

    def player_props(self):
        return {
            "PlaybackStatus": self.status,
            "LoopStatus": self.loop,
            "Rate": 1.0,
            "Shuffle": dbus.Boolean(self.shuffle),
            "Metadata": self.metadata(),
            "Volume": 1.0,
            "Position": dbus.Int64(self.position()),
            "MinimumRate": 1.0,
            "MaximumRate": 1.0,
            "CanGoNext": True,
            "CanGoPrevious": True,
            "CanPlay": True,
            "CanPause": True,
            "CanSeek": True,
            "CanControl": True,
        }

    def root_props(self):
        return {
            "CanQuit": False,
            "CanRaise": False,
            "HasTrackList": False,
            "Identity": "CI Player",
            "DesktopEntry": "ci-player",
            "SupportedUriSchemes": dbus.Array([], signature="s"),
            "SupportedMimeTypes": dbus.Array([], signature="s"),
        }

    def changed(self, **props):
        self.PropertiesChanged(PLAYER, props, [])

    # ---- Properties
    @dbus.service.method(PROPS, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return (self.player_props() if iface == PLAYER else self.root_props())[prop]

    @dbus.service.method(PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return self.player_props() if iface == PLAYER else self.root_props() if iface == ROOT else {}

    @dbus.service.method(PROPS, in_signature="ssv")
    def Set(self, iface, prop, value):
        log(f"comando: Set {prop}={value}")
        if prop == "Shuffle":
            self.shuffle = bool(value)
            self.changed(Shuffle=dbus.Boolean(self.shuffle))
        elif prop == "LoopStatus":
            self.loop = str(value)
            self.changed(LoopStatus=self.loop)

    @dbus.service.signal(PROPS, signature="sa{sv}as")
    def PropertiesChanged(self, iface, changed, invalidated):
        pass

    # ---- Player
    def set_status(self, s):
        self.pos = self.position()
        self.since = time.monotonic()
        self.status = s
        if s == "Playing":
            self.start_audio()
        else:
            self.stop_audio()
        self.changed(PlaybackStatus=s)

    @dbus.service.method(PLAYER)
    def PlayPause(self):
        log("comando: PlayPause")
        self.set_status("Paused" if self.status == "Playing" else "Playing")

    @dbus.service.method(PLAYER)
    def Play(self):
        log("comando: Play")
        self.set_status("Playing")

    @dbus.service.method(PLAYER)
    def Pause(self):
        log("comando: Pause")
        self.set_status("Paused")

    @dbus.service.method(PLAYER)
    def Stop(self):
        log("comando: Stop")
        self.set_status("Stopped")

    def go(self, d):
        self.track = (self.track + d) % len(self.tracks)
        self.pos = 0
        self.since = time.monotonic()
        self.changed(Metadata=self.metadata())

    @dbus.service.method(PLAYER)
    def Next(self):
        log("comando: Next")
        self.go(1)

    @dbus.service.method(PLAYER)
    def Previous(self):
        log("comando: Previous")
        self.go(-1)

    @dbus.service.method(PLAYER, in_signature="x")
    def Seek(self, off):
        log(f"comando: Seek {off}")
        self.pos = max(0, self.position() + int(off))
        self.since = time.monotonic()
        self.Seeked(dbus.Int64(self.pos))

    @dbus.service.method(PLAYER, in_signature="ox")
    def SetPosition(self, track, pos):
        log(f"comando: SetPosition {pos}")
        self.pos = int(pos)
        self.since = time.monotonic()
        self.Seeked(dbus.Int64(self.pos))

    @dbus.service.method(PLAYER, in_signature="s")
    def OpenUri(self, uri):
        pass

    @dbus.service.signal(PLAYER, signature="x")
    def Seeked(self, pos):
        pass

    # ---- MediaPlayer2
    @dbus.service.method(ROOT)
    def Raise(self):
        pass

    @dbus.service.method(ROOT)
    def Quit(self):
        pass


def main():
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName("org.mpris.MediaPlayer2.ciplayer", bus)
    arts = [os.path.abspath(a) for a in sys.argv[1:3]] if len(sys.argv) >= 3 else ["/nonexistent.png"] * 2
    p = Player(bus, arts)
    log("tocando: Midnight City (pid %d)" % os.getpid())
    loop = GLib.MainLoop()
    try:
        loop.run()
    finally:
        p.stop_audio()
        del name


if __name__ == "__main__":
    main()
