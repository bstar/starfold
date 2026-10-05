#!/usr/bin/env python3
"""Verify AMP's bounded relay output without opening an audio device.

Uses generated temporary audio and private configuration. Does not play sound.
The graphical relay/presentation machine still needs its separate listening gate.
"""
import argparse
import json
import math
import os
from pathlib import Path
import queue
import signal
import struct
import subprocess
import tempfile
import threading
import time
import wave

parser = argparse.ArgumentParser()
parser.add_argument('--staramp', required=True)
parser.add_argument('--starfold', help='Also verify the graphical controller relay with a simulated remote frontend')
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix='starfold-audio-proof-') as temporary:
    root = Path(temporary)
    song = root / 'tone.wav'
    with wave.open(str(song), 'wb') as wav:
        wav.setparams((1, 2, 44100, 0, 'NONE', 'not compressed'))
        wav.writeframes(b''.join(struct.pack('<h', int(10000 * math.sin(i * 2 * math.pi * 440 / 44100))) for i in range(44100 * 8)))
    env = dict(os.environ, STARAMP_DIR=str(root / 'amp'))
    process = subprocess.Popen([args.staramp, 'embed', '--stdio'], stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    messages = queue.Queue()
    def read():
        for line in process.stdout:
            messages.put(json.loads(line))
    threading.Thread(target=read, daemon=True).start()
    def send(**message):
        process.stdin.write(json.dumps(message) + '\n')
        process.stdin.flush()
    def wait(predicate, timeout=8):
        deadline = time.monotonic() + timeout
        while True:
            message = messages.get(timeout=max(.01, deadline - time.monotonic()))
            if message['type'] == 'error':
                raise AssertionError(message['message'])
            if predicate(message):
                return message
    try:
        hello = wait(lambda m: m['type'] == 'hello')
        assert 'audio_relay_v1' in hello['capabilities']
        palette = dict(bg=[0, 0, 0], fg=[255, 255, 255], muted=[100, 100, 100], accent=[0, 200, 255],
                       selected=[0, 50, 100], border=[100, 100, 100], error=[255, 0, 0])
        send(type='configure', generation=1, width=58, height=5, focused=True, theme=palette)
        send(type='control', action='audio_relay', value=1)
        send(type='play', generation=1, paths=[str(song)], index=0)
        epoch = wait(lambda m: m['type'] == 'audio_open')['epoch']
        send(type='control', action='audio_credit', value=8)
        blocks = [wait(lambda m: m['type'] == 'audio') for _ in range(8)]
        assert all(b['epoch'] == epoch and 0 < len(b['samples']) <= 1920 and len(b['samples']) % 2 == 0 for b in blocks)
        assert any(abs(s) > 100 for b in blocks for s in b['samples']), 'Relay was silent'
        # No more credits: neither data queues nor a blocked output may prevent controls.
        send(type='control', action='pause')
        wait(lambda m: m['type'] == 'status' and m['paused'])
        # Reattaching a frontend must replace a stream stalled on old credits.
        send(type='control', action='audio_relay', value=1)
        send(type='control', action='resume')
        reattached = wait(lambda m: m['type'] == 'audio_open')['epoch']
        assert reattached > epoch
        send(type='control', action='pause')
        wait(lambda m: m['type'] == 'status' and m['paused'])
        send(type='control', action='seek', value=2)
        send(type='control', action='resume')
        newer = wait(lambda m: m['type'] == 'audio_open')['epoch']
        assert newer > reattached, 'Seek did not invalidate queued audio'
        send(type='control', action='audio_credit', value=1)
        assert wait(lambda m: m['type'] == 'audio')['epoch'] == newer
        send(type='control', action='stop')
        wait(lambda m: m['type'] == 'stopped')
        send(type='shutdown')
        assert process.wait(timeout=3) == 0
        print('PCM relay: mono conversion, non-silent blocks, bounded credits, pause, seek epoch, resume, stop and shutdown passed')
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()

if args.starfold:
    with tempfile.TemporaryDirectory(prefix='sf-audio-', dir='/tmp') as temporary:
        root = Path(temporary)
        files = root / 'files'
        files.mkdir()
        song = files / 'silence.wav'
        with wave.open(str(song), 'wb') as wav:
            wav.setparams((2, 2, 48000, 0, 'NONE', 'not compressed'))
            wav.writeframes(b'\0' * 48000 * 4 * 8)
        amp_root = root / 'amp'
        amp_root.mkdir()
        (amp_root / 'config.toml').write_text('volume = 0.0\n')
        env = dict(os.environ, STARFOLD_DIR=str(root / 'fold'), STARAMP_DIR=str(amp_root),
                   PATH=str(Path(args.staramp).absolute().parent) + os.pathsep + os.environ['PATH'])
        process = subprocess.Popen([str(Path(args.starfold).resolve()), '--graphical-relay', 'audio-proof', '--directory', str(files)],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        messages = queue.Queue()
        def read_controller():
            for line in process.stdout:
                messages.put(json.loads(line))
            messages.put(dict(type='error', message=process.stderr.read() or 'Controller relay closed'))
        threading.Thread(target=read_controller, daemon=True).start()
        def send_controller(message):
            process.stdin.write(json.dumps(message) + '\n')
            process.stdin.flush()
        server_pid = None
        def scene_text(value):
            if isinstance(value, dict):
                return value.get('text', '') + ''.join(scene_text(v) for k, v in value.items() if k != 'text')
            if isinstance(value, list):
                return ''.join(scene_text(v) for v in value)
            return ''
        def wait_controller(predicate, timeout=10):
            deadline = time.monotonic() + timeout
            while True:
                message = messages.get(timeout=max(.01, deadline - time.monotonic()))
                if message['type'] == 'error':
                    raise AssertionError(message['message'])
                if message['type'] == 'scene':
                    scene = message['scene']
                    send_controller(dict(type='presented', revision=scene['revision'], generation=scene['viewport']['generation']))
                if predicate(message):
                    return message
        def key_controller(number, code, modifiers=0):
            send_controller(dict(type='input', id=number, revision=0, generation=1,
                                 input=dict(kind='key', code=code, modifiers=modifiers)))
            assert wait_controller(lambda m: m['type'] == 'ack' and m['id'] == number)['accepted']
        try:
            send_controller(dict(type='hello', version=1, client='audio-proof',
                viewport=dict(columns=100, rows=30, width=1000, height=600, generation=1),
                capabilities=dict(image_transport='kitty', pixel_geometry='measured', pointer_precision='cells',
                    keyboard=True, paste=True, presentation_ack=True, native_surfaces=True, pixel_layout=True,
                    audio_relay=True, local_media=False)))
            greeting = wait_controller(lambda m: m['type'] == 'hello')
            server_pid = int(greeting['epoch'].split('-')[0])
            wait_controller(lambda m: m['type'] == 'scene')
            key_controller(1, 'enter')
            wait_controller(lambda m: m['type'] == 'scene' and 'audio: host' in scene_text(m) and '0:0' in scene_text(m))
            key_controller(2, 'char:2', modifiers=2)
            key_controller(3, 'char:a')
            opened = wait_controller(lambda m: m['type'] == 'media' and m['message']['kind'] == 'audio_open')['message']
            send_controller(dict(type='media', message=dict(kind='audio_credit', session=opened['session'], epoch=opened['epoch'], blocks=8)))
            block = wait_controller(lambda m: m['type'] == 'media' and m['message']['kind'] == 'audio_chunk')['message']
            assert block['epoch'] == opened['epoch'] and 0 < len(block['samples']) <= 1920
            key_controller(4, 'char:a')
            wait_controller(lambda m: m['type'] == 'media' and m['message']['kind'] == 'audio_close')
            key_controller(5, 'char:q')
            wait_controller(lambda m: m['type'] == 'closed')
            assert process.wait(timeout=3) == 0
            server_pid = None
            print('FOLD controller: negotiated Preview toggle, PCM forwarding, route close and clean shutdown passed')
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            if server_pid:
                try:
                    os.killpg(server_pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
