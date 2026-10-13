#!/usr/bin/env python3
"""Exercise shared KIT artwork and FOLD video controls with a silent temporary video."""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument('--staramp', required=True)
parser.add_argument('--starfold', required=True)
parser.add_argument('--video', required=True, help='Generated silent video fixture')
parser.add_argument('--pixel-layout', action='store_true')
parser.add_argument('--without-amp', action='store_true', help='Copy FOLD without its adjacent music helper and reject any AMP invocation')
parser.add_argument('--cells', action='store_true', help='Exercise cell grids and movie track pickers')
parser.add_argument('--movie-player', action='store_true')
parser.add_argument('--original', action='store_true', help='Negotiate original media and exercise quality modes')
args = parser.parse_args()

with tempfile.TemporaryDirectory(prefix='sf-video-', dir='/tmp') as temporary:
    root = Path(temporary)
    files = root / 'files'
    files.mkdir()
    shutil.copyfile(args.video, files / 'video.mp4')
    env = dict(os.environ, STARFOLD_DIR=str(root / 'fold'), STARAMP_DIR=str(root / 'amp'),
               STARAMP_CONFIG_DIR=str(root / 'amp-config'),
               PATH=str(Path(args.staramp).absolute().parent) + os.pathsep + os.environ['PATH'])
    binary = Path(args.starfold).resolve()
    if args.without_amp:
        private_bin = root / 'bin'
        private_bin.mkdir()
        for helper in binary.parent.glob('starfold-preview-*'):
            if helper.is_file() and os.access(helper, os.X_OK):
                (private_bin / helper.name).symlink_to(helper)
        shutil.copy2(binary, private_bin / 'starfold')
        binary = private_bin / 'starfold'
        rejected = private_bin / 'staramp'
        rejected.write_text('#!/bin/sh\necho invoked > "' + str(root / 'amp-invoked') + '"\nexit 77\n')
        rejected.chmod(0o700)
        env['PATH'] = str(private_bin) + os.pathsep + env['PATH']
    process = subprocess.Popen([str(binary), '--graphical-relay', 'video-proof', '--directory', str(files)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    messages = queue.Queue()
    def read():
        for line in process.stdout:
            messages.put(json.loads(line))
        messages.put(dict(type='error', message=process.stderr.read()))
    threading.Thread(target=read, daemon=True).start()
    scene = None
    server_pid = None
    sequence = 0
    media_kinds = set()
    def send(message):
        process.stdin.write(json.dumps(message) + '\n')
        process.stdin.flush()
    def wait(predicate):
        global scene
        deadline = time.monotonic() + 10
        while True:
            try:
                message = messages.get(timeout=max(.01, deadline - time.monotonic()))
            except queue.Empty:
                raise AssertionError('Timed out; latest scene: ' + texts(scene)[-2000:] + '\nSurface actions: ' + repr([[h['action'] for h in c['surface']['hits']] for c in (scene or {}).get('components', []) if c['kind']=='surface'])) from None
            if message['type'] == 'error':
                raise AssertionError(message)
            if message['type'] == 'media':
                media_kinds.add(message['message']['kind'])
            if message['type'] == 'scene':
                scene = message['scene']
                send(dict(type='presented', revision=scene['revision'], generation=scene['viewport']['generation']))
            if predicate(message):
                return message
    def controls(value):
        return next((c for c in value.get('components', []) if c['kind'] == 'surface'
                     and any(h['action'] in ('play', 'pause') for h in c['surface']['hits'])), None)
    def input(value):
        global sequence
        sequence += 1
        send(dict(type='input', id=sequence, revision=scene['revision'], generation=1, input=value))
        assert wait(lambda m: m['type'] == 'ack' and m['id'] == sequence)['accepted']
    def click(action):
        component = next((c for c in scene.get('components', []) if c['kind'] == 'surface' and any(h['action'] == action for h in c['surface']['hits'])), None)
        assert component, f'Transport surface missing for {action}'
        hit = next(h['rect'] for h in component['surface']['hits'] if h['action'] == action)
        rect = component['rect']
        x = rect['x'] + int((hit['x'] + hit['width'] / 2) * rect['width'] / component['surface']['width'])
        y = rect['y'] + int((hit['y'] + hit['height'] / 2) * rect['height'] / component['surface']['height'])
        pointer('down', x, y)
    def pointer(action, x, y):
        packet = dict(kind='pointer', action=action, button=0, x=x, y=y, modifiers=0)
        if args.pixel_layout and scene.get('placements'):
            placement = next(p for p in scene['placements'] if
                             p['source']['x'] <= x < p['source']['x'] + p['source']['width'] and
                             p['source']['y'] <= y < p['source']['y'] + p['source']['height'])
            source, target = placement['source'], placement['target']
            def edge(row):
                height, rows = target['height'], source['height']
                padding = placement.get('padding')
                if padding and rows >= 6:
                    inset = min(padding['inset'], height / 8)
                    gap = min(padding['gap'], height / 8)
                    unit = (height - 2 * inset - gap) / (rows - 3)
                    if row == 0: return 0
                    if row == 1: return inset
                    if row == 2: return inset + unit
                    if row == rows: return height
                    return inset + gap + (row - 2) * unit
                return height * row / rows
            row = y - source['y']
            packet['pixel'] = [int(target['x'] + (x - source['x'] + .5) * target['width'] / source['width']),
                                int(target['y'] + (edge(row) + edge(row + 1)) / 2)]
        input(packet)
    def texts(value):
        if isinstance(value, dict):
            return value.get('text', '') + ''.join(texts(v) for k, v in value.items() if k != 'text')
        if isinstance(value, list):
            return ''.join(texts(v) for v in value)
        return ''
    def playing_scene():
        def playing(value):
            component=controls(value)
            actions=[h['action'] for h in component['surface']['hits']] if component else []
            return component and ('Pause' in texts(value) or (args.movie_player and 'pause' in actions and 'play' not in actions))
        if scene and playing(scene): return scene
        return wait(lambda m: m['type']=='scene' and playing(m['scene']))
    def paused_scene():
        # This protocol fixture does not decode frontend media. Starting/buffering
        # notices may remain; the authoritative transport toggle still reports pause.
        def paused(value):
            component = controls(value)
            actions = [h['action'] for h in component['surface']['hits']] if component else []
            return component and ('paused' in texts(value) or (args.movie_player and 'play' in actions and 'pause' not in actions))
        if scene and paused(scene):
            return scene
        return wait(lambda m: m['type']=='scene' and paused(m['scene']))
    def scene_with(text):
        if scene and text in texts(scene) and controls(scene):
            return scene
        return wait(lambda m: m['type'] == 'scene' and text in texts(m['scene']) and controls(m['scene']))
    try:
        send(dict(type='hello', version=1, client='video-proof',
            viewport=dict(columns=100, rows=40, width=1000, height=800, generation=1),
            capabilities=dict(image_transport='none' if args.cells else 'kitty', cell_presentation=args.cells, pixel_geometry='measured', pointer_precision='cells',
                              keyboard=True, paste=True, native_surfaces=True, pixel_layout=args.pixel_layout,
                              presentation_ack=True, video=True, video_player=args.movie_player,
                              original_media=args.original, local_media=False)))
        hello = wait(lambda m: m['type'] == 'hello')
        server_pid = int(hello['epoch'].split('-')[0])
        wait(lambda m: m['type'] == 'scene')
        input(dict(kind='key', code='enter', modifiers=0))
        scene_with('SSH stream')
        if args.original:
            scene_with('SSH stream · Original')
            input(dict(kind='key', code='char:o', modifiers=0))
            scene_with('mode: Original')
            input(dict(kind='key', code='char:o', modifiers=0))
            scene_with('SSH stream · Preview')
            input(dict(kind='key', code='char:o', modifiers=0))
            scene_with('mode: Auto')
            assert 'original_open' in media_kinds
        click('pause')
        paused_scene()
        click('next')
        scene_with('00:05')
        click('previous')
        scene_with('00:00')
        click('volume')
        wait(lambda m: m['type'] == 'scene' and controls(m['scene']) and
             any(40 <= int(v) <= 60 for v in re.findall(r'(\d+)% volume', texts(m['scene']))))
        click('play')
        playing_scene()
        timeline = next(c['rect'] for c in scene['components'] if c['kind'] == 'surface'
                        and c['rect']['height']==1 and c['rect'] in scene.get('pointer_regions',[]) and len(c['surface']['nodes']) == 2
                        and all(n['kind'] == 'fill' for n in c['surface']['nodes']))
        pointer('down', timeline['x']+1, timeline['y'])
        # Scrubbing's pause is internal; its on-screen toggle reflects the saved
        # play state until release. Assert the seek result and restored playback.
        for offset in range(1, 6):
            pointer('drag', timeline['x'] + offset, timeline['y'])
        pointer('up', timeline['x'] + (timeline['width'] + 1) // 2, timeline['y'])
        wait(lambda m: m['type'] == 'scene' and controls(m['scene']) and
             '00:10' in texts(m['scene']))
        playing_scene()
        if args.movie_player:
            click('fullscreen')
            wait(lambda m: m['type']=='scene' and not m['scene'].get('placements') and controls(m['scene']))
            click('audio_tracks')
            scene_with('Audio tracks')
            click('track:1')
            wait(lambda m: m['type']=='scene' and not any(c['kind']=='surface' and any(h['action'].startswith('track:') for h in c['surface']['hits']) for c in m['scene']['components']))
            click('subtitle_tracks')
            wait(lambda m: m['type']=='scene' and any(c['kind']=='surface' and any(h['action']=='track:1' for h in c['surface']['hits']) for c in m['scene']['components']))
            click('track:1')
            wait(lambda m: m['type']=='scene' and not any(c['kind']=='surface' and any(h['action'].startswith('track:') for h in c['surface']['hits']) for c in m['scene']['components']))
            input(dict(kind='key', code='escape', modifiers=0))
            wait(lambda m: m['type']=='scene' and (not args.pixel_layout or m['scene'].get('placements')) and controls(m['scene']))
        click('stop')
        wait(lambda m: m['type']=='scene' and controls(m['scene']) is None)
        input(dict(kind='key', code='char:q', modifiers=0))
        wait(lambda m: m['type'] == 'closed')
        assert process.wait(timeout=3) == 0
        server_pid = None
        assert not (root / 'amp').exists() and not (root / 'amp-config').exists()
        assert not (root / 'amp-invoked').exists(), 'Generic video controls invoked the AMP music helper'
        print('Video transport: shared KIT artwork, play/pause, seek, drag/release, volume, stop, shutdown and no AMP files passed')
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        if server_pid:
            try:
                os.killpg(server_pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
