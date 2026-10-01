#!/usr/bin/env python3
"""Measure API input to observed Kitty pixels on an isolated STAR/FOLD fixture.

Requires Pillow, Kitty >= 0.49, a graphical build and STAR_GRAPHICS_ELECTRON.
Includes remote-control and screenshot overhead; not pure display latency.
Does not change user configuration or operate on user files.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--kitty", required=True)
    parser.add_argument("--kitten", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--samples", type=int, default=30)
    options = parser.parse_args()
    assert 3 <= options.samples <= 300
    output = Path(options.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    binary = str(Path(options.binary).resolve())
    with tempfile.TemporaryDirectory(prefix="fold-pixels-") as private:
        root = Path(private)
        files = root / "files"
        files.mkdir()
        for index in range(8):
            image = Image.new("RGB", (1600, 900), (index * 30, 80, 140))
            image.save(files / f"{index:02}-preview.png")
        for index in range(8, 80):
            (files / f"{index:02}-preview.txt").write_text(f"STAR/FOLD latency preview {index}\n" * 50)
        environment = dict(os.environ, STARFOLD_DIR=str(root / "app"),
                           STARFOLD_CONFIG_DIR=str(root / "config"))
        address = f"unix:{root}/kitty.sock"
        session_path = root / "app/graphical/latency.sock"
        with (output / "kitty.log").open("w") as log:
            terminal = subprocess.Popen([
                options.kitty, "--hold", "--config", "/dev/null", "--listen-on", address,
                "-o", "allow_remote_control=yes", "--title", "STAR-FOLD-LATENCY-PROOF",
                binary, "graphical", "--session", "latency", str(files),
            ], env=environment, stdout=log, stderr=log)

            def rc(*command):
                return subprocess.check_output([options.kitten, "@", "--to", address, *command],
                                               stderr=subprocess.PIPE, timeout=15)

            def window():
                return json.loads(rc("ls"))[0]["tabs"][0]["windows"][0]

            def pixels():
                rc("screenshot", "--match", "id:1", str(output / "current.png"))
                with Image.open(output / "current.png") as image:
                    return image.size, image.convert("RGB").tobytes()

            def quiet_pixels():
                # Preview assets arrive asynchronously. Settle the preceding
                # action before measuring the next, so its preview cannot be
                # mistaken for the new menu's pixel change.
                previous = pixels()
                stable = 0
                deadline = time.monotonic() + 5
                while stable < 2:
                    time.sleep(.2)
                    current = pixels()
                    stable = stable + 1 if current == previous else 0
                    previous = current
                    assert time.monotonic() < deadline, "Pixels did not settle before the next input"
                return previous

            try:
                deadline = time.monotonic() + 30
                while True:
                    assert terminal.poll() is None, "Kitty exited; inspect kitty.log"
                    try:
                        current = window()
                        ready = current["in_alternate_screen"] and session_path.exists() and any(
                            "/main.cjs" in " ".join(p["cmdline"])
                            and "electron" in " ".join(p["cmdline"]).lower()
                            for p in current["foreground_processes"])
                        if ready:
                            break
                    except (subprocess.SubprocessError, IndexError):
                        pass
                    assert time.monotonic() < deadline, "Pixel frontend startup timed out"
                    time.sleep(.1)
                time.sleep(1)
                results = []
                for index in range(options.samples):
                    if index % 3 == 0:
                        rc("send-key", "--match", "id:1", "alt+1")
                        time.sleep(.2)
                    before = quiet_pixels()
                    action, key = [("navigation/preview", "j"), ("menu open", "shift+f10"),
                                   ("menu close", "escape")][index % 3]
                    started = time.monotonic()
                    rc("send-key", "--match", "id:1", key)
                    while pixels() == before:
                        assert time.monotonic() - started < 5, f"No pixel change for {action}"
                    milliseconds = (time.monotonic() - started) * 1000
                    results.append({"action": action, "observed_ms": milliseconds})
                    (output / "samples.json").write_text(json.dumps(results, indent=2) + "\n")
                    if index < 3:
                        (output / f"action-{index}.png").write_bytes((output / "current.png").read_bytes())
                    time.sleep(.2)
                ordered = sorted(r["observed_ms"] for r in results)
                report = {"samples": results, "p95_observed_ms": ordered[(len(ordered)-1)*95//100],
                          "median_observed_ms": ordered[len(ordered)//2], "terminal_pixels": before[0],
                          "method": "input API through pixel change; includes API/capture overhead"}
                (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
                print(json.dumps({k: v for k, v in report.items() if k != "samples"}))
                rc("send-text", "--match", "id:1", "q")
                deadline = time.monotonic() + 15
                while session_path.exists():
                    assert time.monotonic() < deadline, "Controller leaked after exit"
                    time.sleep(.1)
                rc("close-window", "--match", "id:1")
                terminal.wait(timeout=15)
            finally:
                log_path = root / "app/cache/starfold.log"
                if log_path.exists():
                    (output / "application.log").write_bytes(log_path.read_bytes())
                if terminal.poll() is None:
                    try:
                        (output / "terminal-error.txt").write_bytes(rc("get-text", "--match", "id:1"))
                        rc("send-text", "--match", "id:1", "q")
                        rc("close-window", "--match", "id:1")
                        terminal.wait(timeout=5)
                    except subprocess.SubprocessError:
                        terminal.terminate()
                        terminal.wait(timeout=5)
                # If an attachment failed while a modal was open, closing its
                # terminal leaves the persistent session alive. Close only this
                # fixture's controller through its private socket before the
                # temporary fixture is removed.
                if session_path.exists():
                    with socket.socket(socket.AF_UNIX) as connection:
                        connection.settimeout(5)
                        connection.connect(str(session_path))
                        with connection.makefile("rwb", buffering=0) as stream:
                            def send(message):
                                stream.write((json.dumps(message) + "\n").encode())
                            send({"type": "hello", "version": 1, "client": "close-owned-latency-probe",
                                  "viewport": {"columns": 100, "rows": 40, "width": 1200,
                                               "height": 800, "generation": 1}})
                            for identity, key in [(1, "escape"), (2, "char:q")]:
                                while True:
                                    raw = stream.readline(16 * 1024 * 1024 + 1)
                                    assert raw and len(raw) <= 16 * 1024 * 1024
                                    message = json.loads(raw)
                                    if message["type"] == "scene":
                                        send({"type": "input", "id": identity,
                                              "revision": message["scene"]["revision"], "generation": 1,
                                              "input": {"kind": "key", "code": key, "modifiers": 0}})
                                        break
                    deadline = time.monotonic() + 5
                    while session_path.exists():
                        assert time.monotonic() < deadline, "Fixture controller did not close"
                        time.sleep(.1)


if __name__ == "__main__":
    main()
