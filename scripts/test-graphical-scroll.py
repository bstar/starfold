#!/usr/bin/env python3
"""Exercise sustained wheel input while a graphical frame is in flight.

Uses private files and a real application session. Presentation is deliberately
withheld during the burst to model a slow renderer or SSH transport. Timings
measure controller ACKs, not terminal rasterization or physical display latency.
"""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True)
parser.add_argument('--fold', action='store_true', help='Exercise Fold instead of Commander')
parser.add_argument('--allow-drops', action='store_true', help='Measure a pre-fix executable')
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix='fold-scroll-') as private:
    root = Path(private)
    files = root / 'files'
    files.mkdir()
    for i in range(1000):
        (files / f'file-{i:04}.txt').touch()
    process = subprocess.Popen([str(Path(args.binary).resolve()), '--graphical-relay',
        'scroll', '--directory', str(files)], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
        env={**os.environ, 'STARFOLD_DIR': str(root / 'app'),
             'STARFOLD_CONFIG_DIR': str(root / 'config')})
    messages = queue.Queue()
    def read():
        for line in process.stdout:
            messages.put((time.monotonic(), json.loads(line)))
    threading.Thread(target=read, daemon=True).start()
    def send(message):
        process.stdin.write(json.dumps(message) + '\n')
        process.stdin.flush()
    def present(scene):
        send({'type': 'presented', 'revision': scene['revision'], 'generation': 1})
    def input_event(identity, scene, event):
        send({'type': 'input', 'id': identity, 'revision': scene['revision'],
              'generation': 1, 'input': event})
    scene = None
    try:
        send({'type': 'hello', 'version': 1, 'client': 'scroll-proof',
              'viewport': {'columns': 160, 'rows': 60, 'width': 1600,
                           'height': 960, 'generation': 1},
              'capabilities': {'presentation_ack': True, 'native_surfaces': True,
                               'pixel_layout': True, 'image_transport': 'kitty',
                               'pixel_geometry': 'measured', 'pointer_precision': 'cells',
                               'keyboard': True, 'paste': True}})
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            _, message = messages.get(timeout=15)
            assert message['type'] != 'error', message
            if message['type'] == 'scene':
                scene = message['scene']
                present(scene)
                if any(c.get('label') == 'file-0000.txt' for c in scene['components']):
                    break
        assert scene and any(c.get('label') == 'file-0000.txt' for c in scene['components'])
        if not args.fold:
            input_event(1, scene, {'kind': 'key', 'code': 'char:v', 'modifiers': 0})
        # Settle initial listing/volume updates before holding presentation.
        until = time.monotonic() + .5
        while time.monotonic() < until:
            try:
                _, m = messages.get(timeout=.05)
                if m['type'] == 'scene':
                    scene = m['scene']
                    present(scene)
            except queue.Empty:
                pass
        assert any(c["kind"] == "list_row" for c in scene["components"])
        sent = {}
        for identity in range(2, 62):
            sent[identity] = time.monotonic()
            input_event(identity, scene, {'kind': 'pointer', 'action': 'scroll_down',
                        'button': 0, 'x': 20, 'y': 12, 'modifiers': 0})
            time.sleep(.008)
        acks = {}
        latest = scene
        while len(acks) < len(sent):
            received, message = messages.get(timeout=10)
            if message['type'] == 'ack' and message['id'] in sent:
                acks[message['id']] = (message['accepted'], (received-sent[message['id']])*1000)
            elif message['type'] == 'scene':
                latest = message['scene']
        accepted = sum(ok for ok, _ in acks.values())
        timings = sorted(ms for _, ms in acks.values())
        print(json.dumps({'events': len(sent), 'accepted': accepted,
                          'ack_median_ms': round(timings[len(timings)//2], 2),
                          'ack_p95_ms': round(timings[int(len(timings)*.95)-1], 2)}), flush=True)
        if not args.allow_drops:
            assert accepted == len(sent), 'Continuous wheel events were discarded'
        present(latest)
        # A stale file click must still be rejected after the list moved.
        input_event(62, scene, {'kind': 'pointer', 'action': 'down',
                    'button': 0, 'x': 20, 'y': 12, 'modifiers': 0})
        while True:
            _, message = messages.get(timeout=10)
            if message['type'] == 'scene':
                latest = message['scene']
                present(latest)
            if message['type'] == 'ack' and message['id'] == 62:
                assert not message['accepted'], 'A stale file click was accepted'
                break
        if not args.allow_drops:
            deadline = time.monotonic() + 10
            while not any(c.get('selected') and c.get('label') == 'file-0180.txt'
                          for c in latest['components']):
                _, message = messages.get(timeout=max(.01, deadline-time.monotonic()))
                if message['type'] == 'scene':
                    latest = message['scene']
                    present(latest)
                assert time.monotonic() < deadline, 'Accepted wheel steps did not reach the listing'
    finally:
        if process.poll() is None:
            input_event(1000, scene or {'revision': 0},
                        {'kind': 'key', 'code': 'char:q', 'modifiers': 0})
            process.stdin.close()
            process.wait(timeout=10)
