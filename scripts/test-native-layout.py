#!/usr/bin/env python3
"""Capture native panes/player at normal and zoomed sizes in a private Kitty.

Requires Kitty >= 0.49. Pass --x11 under Xvfb to avoid an inherited Wayland
connection. All files, sessions and configuration belong to a temporary fixture.
This checks presentation/control; a silent WAV does not prove audible playback.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import struct
import tempfile
import time
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "staramp", "kitty", "kitten", "output"):
        parser.add_argument(f"--{name}", required=True)
    parser.add_argument("--theme", default="catppuccin-mocha",
                        choices=["catppuccin-mocha", "catppuccin-latte"])
    parser.add_argument("--x11", action="store_true")
    args = parser.parse_args()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fold-layout-") as temporary:
        root = Path(temporary)
        files = root / "files"
        files.mkdir()
        helper = root / "bin"
        helper.mkdir()
        (helper / "staramp").symlink_to(Path(args.staramp).resolve())
        config = root / "config"
        config.mkdir()
        (config / "config.toml").write_text(f'[ui]\ntheme = "{args.theme}"\n')
        with wave.open(str(files / "000-native-player.wav"), "wb") as audio:
            audio.setnchannels(2)
            audio.setsampwidth(2)
            audio.setframerate(44100)
            audio.writeframes(bytes(44100 * 4 * 60))
        # A real 24-bit BMP fixture, independent of the image encoder under test.
        width, height = 64, 48
        pixels = b"".join(bytes((210, 160, 50) if (x // 8 + y // 8) % 2 else (70, 90, 230))
                          for y in range(height) for x in range(width))
        header = struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
        dib = struct.pack("<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(pixels), 0, 0, 0, 0)
        (files / "000-native-preview.bmp").write_bytes(header + dib + pixels)
        for i in range(100):
            (files / f"{i+1:03}-long-file-name-for-native-commander-truncation.txt").write_text("fixture\n")
        address = f"unix:{root}/kitty.sock"
        env = dict(os.environ, PATH=str(helper) + ":" + os.environ["PATH"],
                   STARFOLD_DIR=str(root / "app"), STARFOLD_CONFIG_DIR=str(config),
                   STARAMP_DIR=str(root / "amp"), STARAMP_CONFIG_DIR=str(root / "amp-config"))
        command = [args.kitty, "--hold", "--config", "/dev/null", "--listen-on", address,
                   "-o", "allow_remote_control=yes", "-o", "remember_window_size=no",
                   "-o", "initial_window_width=1800", "-o", "initial_window_height=1000"]
        if args.x11:
            command += ["-o", "linux_display_server=x11"]
        command += [str(Path(args.binary).resolve()), "graphical", "--session", "layout", str(files)]
        with (output / "kitty.log").open("w") as log:
            terminal = subprocess.Popen(command, env=env, stdout=log, stderr=log)

            def rc(*words):
                return subprocess.check_output([args.kitten, "@", "--to", address, *words],
                                               stderr=subprocess.PIPE, timeout=15)

            def key(value):
                rc("send-key", "--match", "id:1", value)
                time.sleep(.4)

            def shot(name):
                rc("screenshot", "--match", "id:1", str(output / f"{name}.png"))

            try:
                deadline = time.monotonic() + 30
                while True:
                    assert terminal.poll() is None, "Kitty exited; inspect kitty.log"
                    try:
                        if json.loads(rc("ls"))[0]["tabs"][0]["windows"][0]["in_alternate_screen"]:
                            break
                    except (subprocess.SubprocessError, IndexError, KeyError):
                        pass
                    assert time.monotonic() < deadline, "Graphical startup timed out"
                    time.sleep(.2)
                time.sleep(1)
                shot("fold")
                key("ctrl+t")
                key("v")
                shot("commander-tabs")
                key("j")
                time.sleep(.8)
                shot("bmp-preview")
                key("home")
                key("c")
                shot("menu")
                key("escape")
                key("enter")
                time.sleep(3)
                shot("player")
                key("space")
                shot("paused")
                rc("set-font-size", "16")
                time.sleep(1)
                shot("zoomed")
                key("x")
                key("q")
                rc("close-window", "--match", "id:1")
                terminal.wait(timeout=10)
            finally:
                if terminal.poll() is None:
                    try:
                        key("escape")
                        key("x")
                        key("q")
                    except subprocess.SubprocessError:
                        pass
                    terminal.terminate()
                    terminal.wait(timeout=10)
    print(output)


if __name__ == "__main__":
    main()
