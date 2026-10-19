#!/usr/bin/env python3
"""Reproducible emulator/STAR-FOLD compatibility lab (Linux, private X11/Wayland)."""
import argparse
import collections
import contextlib
import csv
import fcntl
import hashlib
import html
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid
import zipfile

from PIL import Image, ImageChops
from terminals import EXTERNAL, MANUAL, WAYLAND, launch

HERE = Path(__file__).resolve().parent
STATUSES = ('pass', 'fail', 'observed', 'unsupported', 'unavailable', 'manual', 'not-run')


def command(args, env=None, timeout=15, check=True):
    result = subprocess.run([str(a) for a in args], env=env, timeout=timeout,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if check and result.returncode:
        raise RuntimeError(f'{args[0]} exited {result.returncode}: ' + result.stderr.decode(errors='replace')[-2500:])
    return result


def wait_until(predicate, timeout=10, reason='condition'):
    deadline = time.monotonic() + timeout
    while True:
        value = predicate()
        if value:
            return value
        if time.monotonic() >= deadline:
            raise TimeoutError('Timed out waiting for ' + reason)
        time.sleep(.08)


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def executable_target(path):
    if path.read_bytes()[:2] != b'#!':
        return path
    # Recognize this workstation's simple media wrapper without executing shell
    # text or treating the wrapper's checksum as the executable's checksum.
    for line in path.read_text(errors='replace').splitlines():
        if line.startswith('exec '):
            words = shlex.split(line)
            if len(words) > 1 and words[1].startswith('/') and Path(words[1]).is_file():
                return Path(words[1]).resolve()
    return path


def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=4)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=4)


def private_children(home):
    """Reap only processes bearing this case's exact disposable HOME marker."""
    marker = b'STARFOLD_DIR=' + os.fsencode(home) + b'\0'
    for entry in Path('/proc').glob('[0-9]*/environ'):
        try:
            if marker in entry.read_bytes():
                os.kill(int(entry.parent.name), signal.SIGTERM)
        except (OSError, ValueError):
            pass


class Display:
    def __init__(self, root):
        self.root, self.processes, self.logs = root, [], []
        # Avoid host GTK/Qt plugins and desktop-specific service assumptions.
        self.env = {k: v for k, v in os.environ.items()
                    if k in ('PATH', 'USER', 'LOGNAME', 'SHELL', 'TZ', 'TMPDIR')}
        self.env['PATH'] = os.environ['STARFOLD_LAB_PATH']
        self.env['SHELL'] = shutil.which('bash', path=self.env['PATH'])
        for key in list(self.env):
            if key.startswith(('KITTY_', 'WEZTERM_', 'GHOSTTY_', 'SSH_')) or key in (
                    'DISPLAY', 'WAYLAND_DISPLAY', 'TMUX', 'STY', 'WINDOWID',
                    'TERM', 'COLORTERM', 'COLUMNS', 'LINES', 'DBUS_SESSION_BUS_ADDRESS',
                    'NO_COLOR', 'STAR_KIT_ASCII', 'STAR_GRAPHICS_SCALE'):
                self.env.pop(key, None)
        self.runtime = tempfile.TemporaryDirectory(prefix='foldlab-display-', dir='/tmp')
        runtime = Path(self.runtime.name)
        self.env.update(XDG_RUNTIME_DIR=str(runtime), LANG='C.UTF-8', LC_ALL='C.UTF-8',
                        LIBGL_ALWAYS_SOFTWARE='1',
                        GALLIUM_DRIVER='llvmpipe', GTK_A11Y='none', NO_AT_BRIDGE='1',
                        GSETTINGS_BACKEND='keyfile', FONTCONFIG_FILE=os.environ['STARFOLD_LAB_FONTS'])
        self.env['LIBGL_DRIVERS_PATH'] = os.environ['STARFOLD_LAB_MESA']
        mesa = Path(os.environ['STARFOLD_LAB_MESA']).parent
        self.env['LD_LIBRARY_PATH'] = str(mesa) + ':' + self.env.get('LD_LIBRARY_PATH', '')
        self.env['__GLX_VENDOR_LIBRARY_NAME'] = 'mesa'
        self.env['__EGL_VENDOR_LIBRARY_FILENAMES'] = str(mesa.parent / 'share/glvnd/egl_vendor.d/50_mesa.json')
        lavapipe = list((mesa.parent / 'share/vulkan/icd.d').glob('lvp*.json'))
        if lavapipe:
            self.env['VK_DRIVER_FILES'] = str(lavapipe[0])
        self.env['XDG_CACHE_HOME'] = str(root / 'display-cache')
        self.env['HOME'] = str(root / 'display-home')
        Path(self.env['HOME']).mkdir()
        read, write = os.pipe()
        log = self.log('xvfb.log')
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write), '-screen', '0',
                                 '1280x900x24', '-nolisten', 'tcp', '-ac'],
                                pass_fds=(write,), env=self.env, stdout=log, stderr=log)
        self.processes.append(xvfb)
        os.close(write)
        import select
        if not select.select([read], [], [], 15)[0]:
            raise RuntimeError('Xvfb failed; inspect infrastructure/xvfb.log')
        self.env['DISPLAY'] = ':' + os.read(read, 32).decode().strip()
        os.close(read)
        self.spawn(['openbox', '--sm-disable'], 'openbox.log')
        time.sleep(.5)
        self.wayland = None
        self.wayland_error = None

    def log(self, name):
        f = (self.root / name).open('wb')
        self.logs.append(f)
        return f

    def spawn(self, args, logfile, env=None):
        log = self.log(logfile)
        p = subprocess.Popen(args, env=env or self.env, stdout=log, stderr=log)
        self.processes.append(p)
        return p

    def start_wayland(self):
        if self.wayland:
            return
        if self.wayland_error:
            raise RuntimeError(self.wayland_error)
        try:
            self.spawn(['weston', '--backend=x11-backend.so', '--renderer=pixman',
                        '--shell=kiosk-shell.so', '--socket=fold-lab-wayland',
                        '--width=1200', '--height=820', '--idle-time=0', '--no-config'], 'weston.log')
            wait_until(lambda: (Path(self.env['XDG_RUNTIME_DIR']) / 'fold-lab-wayland').exists(),
                       timeout=15, reason='private nested Wayland compositor')
            self.wayland = 'fold-lab-wayland'
        except Exception as e:
            self.wayland_error = str(e)
            raise

    def windows(self):
        result = command(['wmctrl', '-l'],
                         self.env, check=False)
        return [line.split()[0] for line in result.stdout.decode().splitlines() if line.split()]

    def key(self, window, key):
        command(['xdotool', 'windowactivate', '--sync', window], self.env, check=False)
        command(['xdotool', 'windowfocus', '--sync', window], self.env)
        time.sleep(.08)
        command(['xdotool', 'key', '--clearmodifiers', key], self.env)
        time.sleep(.18)

    def type(self, window, text):
        command(['xdotool', 'windowactivate', '--sync', window], self.env, check=False)
        command(['xdotool', 'windowfocus', '--sync', window], self.env)
        time.sleep(.08)
        command(['xdotool', 'type', '--clearmodifiers', '--delay', '12', text], self.env)
        time.sleep(.18)

    def screenshot(self, path):
        # Xvfb is private. This captures no pixels from the user's desktop.
        from PIL import ImageGrab
        image = ImageGrab.grab(xdisplay=self.env['DISPLAY'])
        image.save(path)
        return image.convert('RGB')

    def close(self):
        for p in reversed(self.processes):
            stop(p)
        for f in self.logs:
            f.close()
        self.runtime.cleanup()


def fixtures(directory):
    files = directory / 'files'
    files.mkdir()
    (files / '00-destination').mkdir()
    (files / '10-source.txt').write_text('STARFOLD COPY FIXTURE\n')
    image = Image.new('RGB', (160, 100))
    colors = [(250, 30, 70), (30, 220, 110), (50, 110, 240), (245, 210, 40)]
    for index, color in enumerate(colors):
        x, y = (index % 2) * 80, (index // 2) * 50
        image.paste(color, (x, y, x + 80, y + 50))
    image.save(files / '20-picture.png')
    (files / '30-unicode-λ-日本語.txt').write_text('STARFOLD TEXT PREVIEW\nUnicode: λ 日本語 café\n')
    with zipfile.ZipFile(files / '40-archive.zip', 'w') as archive:
        archive.writestr('inside-archive.txt', 'STARFOLD ARCHIVE FIXTURE\n')
    for index in range(70):
        (files / f'80-scroll-{index:03}.txt').write_text('scroll fixture\n')
    command(['ffmpeg', '-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=12',
             '-t', '45', '-c:v', 'mpeg4', '-q:v', '6', '-an', files / '50-movie.mp4'], timeout=30)
    # Silent audio exercises the helper/UI without making sound on the workstation.
    import wave
    with wave.open(str(files / '60-silent.wav'), 'wb') as sound:
        sound.setnchannels(2)
        sound.setsampwidth(2)
        sound.setframerate(48000)
        sound.writeframes(bytes(48000 * 4 * 30))
    return files


def observe(directory):
    endpoint = directory / 'observe.sock'
    try:
        with socket.socket(socket.AF_UNIX) as s:
            s.settimeout(1)
            s.connect(str(endpoint))
            s.sendall(b'snapshot\n')
            chunks = bytearray()
            while not chunks.endswith(b'\n'):
                data = s.recv(65536)
                if not data:
                    break
                chunks.extend(data)
            return json.loads(chunks)
    except (OSError, ValueError):
        state = directory / 'state.json'
        return json.loads(state.read_text()) if state.exists() else {}


def read_logs(app):
    return '\n'.join(p.read_text(errors='replace') for p in app.rglob('*.log'))


def theme_id(app):
    try:
        return tomllib.loads((app / 'config.toml').read_text())['ui']['theme']
    except (OSError, KeyError, ValueError):
        return None


def image_markers(image):
    colors = image.get_flattened_data()
    counts = collections.Counter(colors)
    targets = [(250, 30, 70), (30, 220, 110), (50, 110, 240), (245, 210, 40)]
    hits = []
    for target in targets:
        hits.append(sum(n for color, n in counts.items()
                        if max(abs(color[i] - target[i]) for i in range(3)) <= 12))
    return hits


def quadrant_pattern(image):
    """Find the fixture's four adjacent solid quadrants after palette reduction.

    Requiring shared edges, similar widths and four different saturated colors
    avoids counting unrelated colored header text as a successful image preview.
    This tests geometry/content separately from fidelity to source RGB values.
    """
    width, height = image.size
    pixels = image.load()
    rows = {}
    for y in range(0, height, 8):
        segments = []
        start, color = 0, pixels[0, y]
        for x in range(1, width + 1):
            next_color = pixels[x, y] if x < width else None
            if next_color != color:
                if x - start >= 20 and max(color) - min(color) >= 50:
                    segments.append((start, x, color))
                start, color = x, next_color
        rows[y] = segments
    for y, segments in rows.items():
        for left, right in zip(segments, segments[1:]):
            if left[1] != right[0] or left[2] == right[2]:
                continue
            if abs((left[1] - left[0]) - (right[1] - right[0])) > 8:
                continue
            for bottom in range(y + 24, min(height, y + 300), 8):
                lower = rows.get(bottom, [])
                for bl, br in zip(lower, lower[1:]):
                    if (bl[:2], br[:2]) != (left[:2], right[:2]):
                        continue
                    if len({left[2], right[2], bl[2], br[2]}) == 4:
                        return {'bounds': [left[0], y, right[1], bottom],
                                'colors': [left[2], right[2], bl[2], br[2]]}
    return None


def image_protocol(data):
    # A graphics capability query is not an image transmission.
    if re.search(rb'\x1b_G(?:[^;\x1b]*,)?a=[tT](?:,|;)', data):
        return 'kitty'
    if b'\x1b]1337;File=' in data:
        return 'iterm2'
    if re.search(rb'\x1bP[0-9;?]*q', data):
        return 'sixel'
    if '▀'.encode() in data or '▄'.encode() in data:
        return 'half-blocks'
    return 'none-observed'


def sgr_colors(data):
    colors = []
    for parameters in re.findall(rb'\x1b\[([0-9;:]{1,512})m', data):
        for channels in re.findall(rb'(?:^|[;:])(?:38|48)[;:]2[;:](\d{1,3})[;:](\d{1,3})[;:](\d{1,3})(?=[;:]|$)', parameters):
            if all(int(v) < 256 for v in channels):
                colors.append('#%02x%02x%02x' % tuple(map(int, channels)))
    return list(dict.fromkeys(colors))


def help_open(screen):
    # Lower categories may be below the viewport at larger terminal fonts.
    return 'KEYS' in screen and 'new tab' in screen


def reassess(output, saved):
    """Re-evaluate stored observations; retain every replaced assertion."""
    history = output / 'history'
    history.mkdir(exist_ok=True)
    (history / ('results-before-review-' + uuid.uuid4().hex[:8] + '.json')).write_text(json.dumps(saved, indent=2))
    corrections = []
    for result in saved['results']:
        directory = output / result.get('evidence', 'missing')
        checks = result.get('checks', {})
        changed = False
        startup = directory / 'startup.txt'
        if startup.exists() and 'enter rename' in startup.read_text().lower():
            checks['startup-input-clean'] = {'status': 'fail',
                'detail': 'Stored startup frame contains an unsolicited Rename dialog before the first test key.',
                'evidence': 'startup.png'}
            changed = True
        if result.get('profile') == 'ascii' and checks.get('unicode', {}).get('status') != 'unsupported':
            checks['unicode'] = {'status': 'unsupported',
                'detail': 'ASCII was explicitly requested; this run does not establish Unicode glyph rendering.',
                'previous_assertion': checks.get('unicode')}
            corrections.append(result['terminal'] + '/ascii: Unicode coverage label')
            changed = True
        help_check = checks.get('help', {})
        steps = directory / 'steps.jsonl'
        if help_check.get('status') == 'fail' and steps.exists():
            for line in steps.read_text().splitlines():
                step = json.loads(line)
                if step.get('key') == 'question' and help_open(step.get('screen') or ''):
                    checks['help'] = {'status': 'pass',
                        'detail': 'Stored key-step capture shows the KEYS overlay and new-tab binding. Earlier predicate incorrectly required an offscreen category.',
                        'evidence': 'steps.jsonl', 'previous_assertion': help_check}
                    corrections.append(result['terminal'] + '/' + result.get('profile', '') + ': help predicate')
                    changed = True
                    break
        if changed:
            (directory / 'result.json').write_text(json.dumps(result, indent=2))
    saved['metadata'].setdefault('evidence_reviews', []).append({
        'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'parser_sha256': digest(Path(__file__)), 'corrections': corrections})
    return saved


def collect_evidence(output, saved, manifest):
    # Case records are authoritative even if an aggregate write was interrupted.
    results = {(r['terminal'], r.get('profile')): r for r in saved['results']}
    for path in sorted(output.glob('*/result.json')):
        result = json.loads(path.read_text())
        results[result['terminal'], result.get('profile')] = result
    for terminal in manifest['terminals']:
        if terminal.get('available') is False:
            results[terminal['id'], None] = {
                'terminal': terminal['id'], 'version': terminal.get('version'),
                'status': 'unavailable', 'detail': terminal.get('reason', 'Unavailable in locked package set')}
    order = {t['id']: i for i, t in enumerate(manifest['terminals'])}
    saved['results'] = sorted(results.values(), key=lambda r: (order.get(r['terminal'], 999), r.get('profile', '')))
    saved['metadata']['manifest'] = manifest
    saved['metadata']['collected_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    return saved


class Case:
    def __init__(self, terminal, profile, root, display, binary, args):
        self.terminal, self.profile, self.display = terminal, profile, display
        self.directory = root / (terminal['id'] + '-' + profile)
        self.directory.mkdir()
        # Unix session sockets must fit sockaddr_un even when reports live in
        # a deeply nested checkout. Preserve this HOME as evidence on exit.
        self.runtime = tempfile.TemporaryDirectory(prefix='foldlab-', dir='/tmp')
        self.bridge = Path(self.runtime.name)
        self.home = self.bridge / 'home'
        self.home.mkdir()
        self.app = self.home / '.local/starfold'
        self.app.mkdir(parents=True)
        self.args, self.binary = args, binary
        self.process = None
        self.dbus = None
        self.result = {'terminal': terminal['id'], 'version': terminal['version'],
                       'profile': profile, 'checks': {}, 'themes': [], 'evidence': self.directory.name}
        self.window = None
        self.log = None
        self.files = fixtures(self.directory)

    def check(self, name, status, detail, evidence=None):
        assert status in STATUSES
        self.result['checks'][name] = {'status': status, 'detail': detail}
        if evidence:
            self.result['checks'][name]['evidence'] = evidence

    def screen(self):
        return observe(self.bridge).get('screen', '')

    def key(self, key):
        self.display.key(self.window, key)
        state = observe(self.bridge)
        with (self.directory / 'steps.jsonl').open('a') as journal:
            journal.write(json.dumps({'key': key, 'input_bytes': state.get('input_bytes'), 'screen': state.get('screen')}) + '\n')

    def shot(self, name):
        filename = name + '.png'
        image = self.display.screenshot(self.directory / filename)
        (self.directory / (name + '.txt')).write_text(self.screen())
        return filename, image

    def expect(self, text, timeout=6):
        return wait_until(lambda: text in self.screen(), timeout=timeout, reason=repr(text))

    def filter(self, name):
        self.key('alt+1')
        self.key('slash')
        self.key('ctrl+e')
        self.key('ctrl+u')
        self.display.type(self.window, name)
        self.key('Return')
        self.expect(name)
        time.sleep(.45)

    def clear_filter(self):
        self.key('slash')
        self.key('ctrl+e')
        self.key('ctrl+u')
        self.key('BackSpace')
        self.key('Return')
        time.sleep(.2)

    def start(self):
        if digest(self.args.executable) != self.args.executable_sha256:
            raise RuntimeError('FOLD executable changed during the run; restart with a fresh report directory.')
        self.app.joinpath('config.toml').write_text('[ui]\ntheme = "catppuccin-mocha"\ngraphical_palette = "theme"\n')
        env = dict(self.display.env)
        env.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / '.config'),
                   XDG_CACHE_HOME=str(self.home / '.cache'), XDG_DATA_HOME=str(self.home / '.local/share'),
                   STARFOLD_DIR=str(self.app), STARFOLD_CONFIG_DIR=str(self.app), STARFOLD_LOG='debug',
                   STAR_KIT_ASCII='1' if self.profile == 'ascii' else '0',
                   QT_QPA_PLATFORM='xcb', GDK_BACKEND='x11', WINIT_UNIX_BACKEND='x11')
        if self.profile == 'mono':
            env['NO_COLOR'] = '1'
        if self.profile == 'rgb':
            env['COLORTERM'] = 'truecolor'
        if self.terminal['id'] in WAYLAND:
            self.display.start_wayland()
            env.update(WAYLAND_DISPLAY=self.display.wayland, GDK_BACKEND='wayland',
                       QT_QPA_PLATFORM='wayland', WINIT_UNIX_BACKEND='wayland')
        if self.terminal['id'] == 'terminology':
            env.update(ELM_SCALE='1.0', ELM_PROFILE='standard')
        # Non-NixOS hosts need not have /etc/dbus-1/session.conf. Do not use
        # host activation services or the user's desktop bus for this lab.
        manifest = json.loads(Path(os.environ['STARFOLD_TERMINAL_MANIFEST']).read_text())
        service_dirs = ''.join('<servicedir>' + html.escape(t['store'] + '/share/dbus-1/services') + '</servicedir>'
                               for t in manifest['terminals'] if t['available'])
        bus_config = self.home / 'bus.conf'
        bus_config.write_text('<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>'
                              '<auth>EXTERNAL</auth><policy context="default"><allow own="*"/>'
                              '<allow send_destination="*"/><allow receive_sender="*"/></policy>'
                              + service_dirs + '</busconfig>')
        bus = command(['dbus-daemon', '--config-file=' + str(bus_config), '--fork',
                       '--print-address=1', '--print-pid=1'], env).stdout.decode().splitlines()
        env['DBUS_SESSION_BUS_ADDRESS'], buspid = bus
        self.dbus = int(buspid)
        if self.terminal['id'] == 'gnome-terminal':
            schemas = next(Path(self.terminal['store']).glob('share/gsettings-schemas/*/glib-2.0/schemas'))
            command(['gsettings', '--schemadir', schemas, 'set',
                     'org.gnome.Terminal.Legacy.Settings', 'always-check-default-terminal', 'false'], env)
        session = 'lab-' + uuid.uuid4().hex[:12]
        fold = [str(self.binary)] + ([] if self.profile == 'auto' else ['--cells'])
        fold += ['--session', session, str(self.files)]
        if self.profile == 'tmux':
            fold = ['tmux', '-L', session, '-f', '/dev/null', 'new-session', '-s', 'lab', *fold]
        elif self.profile == 'zellij':
            fold = ['zellij', '--session', session, 'options', '--default-shell', shlex.join(fold)]
        inner = [sys.executable, str(HERE / 'bridge.py'), str(self.bridge), *fold]
        # Qt parses some global options (notably --session) before an emulator
        # can pass -e's arguments through. Give every terminal one executable
        # instead of exposing the recorder/application arguments to its parser.
        entry = self.bridge / 'run-fixture'
        entry.write_text('#!' + sys.executable + '\nimport os\nos.execv(' + repr(inner[0]) + ', ' + repr(inner) + ')\n')
        entry.chmod(0o700)
        args = launch(self.terminal, self.home, [str(entry)])
        self.result['launch'] = args
        before = set(self.display.windows())
        self.log = (self.directory / 'terminal.log').open('wb')
        self.process = subprocess.Popen(args, env=env, stdout=self.log, stderr=self.log)
        wait_until(lambda: (self.bridge / 'observe.sock').exists(), timeout=self.args.startup_timeout,
                   reason='emulator to execute PTY recorder')
        def find_window():
            windows = self.display.windows()
            if self.terminal['id'] in WAYLAND:
                result = command(['xdotool', 'search', '--onlyvisible', '--class', 'weston'], self.display.env, check=False)
                return (result.stdout.decode().split() or [None])[-1]
            return next((w for w in reversed(windows) if w not in before), None)
        self.window = wait_until(find_window, reason='terminal window')
        if self.terminal['id'] not in WAYLAND:
            command(['xdotool', 'windowsize', self.window, '1200', '820'], self.display.env)
            command(['xdotool', 'windowmove', self.window, '20', '20'], self.display.env)
        command(['xdotool', 'windowactivate', '--sync', self.window], self.display.env, check=False)
        command(['xdotool', 'windowfocus', '--sync', self.window], self.display.env)
        if self.terminal['id'] == 'terminology':
            # EFL presents a scale chooser for a fresh HOME. Record it and
            # accept the default inside the disposable window only.
            time.sleep(.5)
            self.display.screenshot(self.directory / 'emulator-first-run.png')
            command(['xdotool', 'mousemove', '--window', self.window, '600', '450', 'click', '1'], self.display.env)
            time.sleep(.7)
            command(['xdotool', 'mousemove', '--window', self.window, '400', '160', 'click', '1'], self.display.env)
            time.sleep(.3)
        wait_until(lambda: observe(self.bridge).get('output_bytes', 0) > 2000,
                   timeout=self.args.startup_timeout, reason='FOLD output')
        time.sleep(1.2)
        initial_input = observe(self.bridge).get('input_bytes', 0)
        self.display.key(self.window, 'ctrl+l')
        try:
            wait_until(lambda: observe(self.bridge).get('input_bytes', 0) > initial_input,
                       timeout=1, reason='Ctrl+L keyboard delivery')
            self.result['keyboard_probe'] = 'ctrl+l'
        except TimeoutError:
            self.display.key(self.window, 'F24')
            wait_until(lambda: observe(self.bridge).get('input_bytes', 0) > initial_input,
                       timeout=2, reason='real keyboard delivery to the terminal')
            self.result['keyboard_probe'] = 'F24 (Ctrl+L did not reach PTY)'
        state = observe(self.bridge)
        if state.get('exit_code') is not None:
            raise RuntimeError('FOLD exited at startup: ' + str(state['exit_code']))
        self.check('startup', 'pass', 'Real emulator executed FOLD and produced a live frame.')
        self.result['terminal_environment'] = state.get('environment', {})
        shot, _ = self.shot('startup')
        unsolicited = 'enter rename' in self.screen().lower()
        self.check('startup-input-clean', 'fail' if unsolicited else 'pass',
                   'Unexpected rename dialog before user input; recovered with Escape to continue testing.' if unsolicited else
                   'No unexpected rename dialog after startup queries.', shot)
        if unsolicited:
            self.key('Escape')
        self.result['environment'] = {k: env.get(k) for k in ('DISPLAY', 'WAYLAND_DISPLAY', 'LIBGL_ALWAYS_SOFTWARE')}
        logs = read_logs(self.app)
        native = 'Starting STAR graphical renderer' in (self.bridge / 'output.bin').read_bytes().decode(errors='replace')
        self.native = native
        self.result['presentation'] = 'native' if native else 'cells'
        self.check('native-ui', 'observed' if native else ('not-run' if self.profile != 'auto' else 'unsupported'),
                   'FOLD selected native pixels.' if native else 'FOLD selected cell fallback; not a startup failure.', shot)
        capabilities = next((line.split('Terminal presentation capabilities', 1)[-1]
                             for line in logs.splitlines() if 'Terminal presentation capabilities' in line), None)
        self.result['capabilities'] = capabilities

    def functional(self):
        if self.native:
            self.key('F9')
            self.expect('10-source.txt')
        self.key('alt+1')
        self.expect('10-source.txt')
        self.key('Home')
        self.key('j')
        self.key('space')
        try:
            self.expect('1 marked', timeout=2)
            self.check('mark', 'pass', 'Space marked a file; footer showed 1 marked.')
        except TimeoutError:
            self.check('mark', 'fail', 'No marked count after real Space key.')
        # Unmark before copying; navigation policy must not hide a wrong source.
        self.key('u')
        self.key('Home')
        self.key('j')
        self.key('y')
        self.key('Home')
        self.key('Return')
        self.key('p')
        copied = self.files / '00-destination/10-source.txt'
        try:
            wait_until(copied.exists, timeout=8, reason='queued copy')
            assert copied.read_bytes() == (self.files / '10-source.txt').read_bytes()
            self.check('copy-operations', 'pass', 'Yank/paste through the UI copied the exact bytes into the destination.')
        except (TimeoutError, AssertionError) as e:
            self.check('copy-operations', 'fail', str(e))
        self.key('h')
        self.key('v')
        try:
            wait_until(lambda: 'LEFT' in self.screen() and 'RIGHT' in self.screen(), reason='Commander panes')
            self.key('Tab')
            shot, _ = self.shot('commander')
            self.check('commander', 'pass', 'Both file panes rendered; Tab was delivered.', shot)
        except TimeoutError as e:
            self.check('commander', 'fail', str(e))
        self.key('v')
        self.key('question')
        try:
            wait_until(lambda: help_open(self.screen()), timeout=3, reason='help overlay')
            shot, _ = self.shot('help')
            self.check('help', 'pass', 'Help opened from the emulator keyboard.', shot)
        except TimeoutError as e:
            self.check('help', 'fail', str(e))
        self.key('Escape')
        self.filter('30-unicode')
        try:
            self.expect('STARFOLD TEXT PREVIEW')
            shot, _ = self.shot('text-unicode')
            self.check('text-preview', 'pass', 'Text preview contains the fixture body.', shot)
            self.check('unicode', 'unsupported' if self.profile == 'ascii' else 'observed',
                       'ASCII was explicitly requested; Unicode rendering is not established.' if self.profile == 'ascii' else
                       'Unicode bytes reached the cell grid; font coverage/width needs visual review.', shot)
        except TimeoutError as e:
            self.check('text-preview', 'fail', str(e))
        self.clear_filter()
        self.filter('40-archive')
        self.key('Return')
        try:
            self.expect('inside-archive.txt')
            shot, _ = self.shot('archive')
            self.check('archive', 'pass', 'Entered a generated ZIP and listed its member.', shot)
        except TimeoutError as e:
            self.check('archive', 'fail', str(e))
        self.key('h')
        self.clear_filter()
        # Actual wheel events, with enough files to change the viewport.
        self.key('Home')
        before = self.screen()
        command(['xdotool', 'mousemove', '--window', self.window, '400', '220', 'click', '--repeat', '8', '--delay', '50', '5'], self.display.env)
        time.sleep(.5)
        mouse = (self.bridge / 'input.bin').read_bytes()
        delivered = b'\x1b[<' in mouse or b'\x1b[M' in mouse
        changed = self.screen() != before
        self.check('mouse-wheel', 'pass' if delivered and changed else 'fail',
                   f'Emulator emitted mouse reports: {delivered}; visible grid changed: {changed}.')
        self.key('Home')
        if self.terminal['id'] in WAYLAND:
            self.check('resize', 'manual', 'Nested Wayland size negotiation requires compositor-specific control.')
        else:
            old = observe(self.bridge)
            command(['xdotool', 'windowsize', self.window, '850', '600'], self.display.env)
            time.sleep(.6)
            new = observe(self.bridge)
            resized = (old.get('columns'), old.get('rows')) != (new.get('columns'), new.get('rows'))
            shot, _ = self.shot('resized')
            self.check('resize', 'pass' if resized and new.get('exit_code') is None else 'fail',
                       f"PTY grid {old.get('columns')}x{old.get('rows')} → {new.get('columns')}x{new.get('rows')}; remained alive.", shot)
            command(['xdotool', 'windowsize', self.window, '1200', '820'], self.display.env)
            time.sleep(.5)
        self.filter('20-picture')
        time.sleep(.7)
        shot, image = self.shot('image-cells')
        hits = image_markers(image)
        pattern = quadrant_pattern(image)
        raw = (self.bridge / 'output.bin').read_bytes()
        if self.profile in ('ascii', 'mono'):
            self.check('image-preview', 'unsupported' if self.profile == 'ascii' else 'observed',
                       'Restricted presentation requested; source RGB image fidelity is intentionally unavailable.', shot)
        else:
            self.check('image-preview', 'pass' if all(n >= 12 for n in hits) or pattern else 'fail',
                       f'Emitted {image_protocol(raw)}; source-RGB pixels {hits}; four-quadrant pattern {pattern}.', shot)
        self.check('image-colors', 'observed',
                   'Source RGB colors visible.' if all(n >= 12 for n in hits) else
                   'Image present with palette-reduced colors.' if pattern else 'Source colors and fixture pattern not verified.', shot)
        self.result['image_protocol'] = image_protocol(raw)
        self.clear_filter()
        if self.native:
            self.key('F9')
            time.sleep(.6)
            self.filter_native_image()
        self.check('clipboard', 'manual', 'OSC 52 permission prompts and readback need an explicit clipboard test; not inferred from terminal identity.')
        self.check('desktop-dnd', 'manual', 'Real desktop OSC 72 drag/drop requires a separate source/target interaction.')
        self.check('ssh', 'not-run', 'This local emulator suite does not establish SSH latency, remote audio, or bridge correctness.')
        self.check('audio', 'not-run', 'Silent fixture is supplied for manual AMP/device testing; no audible correctness claim.')
        self.check('video', 'not-run', 'Use --media for pause/resume and visual movie captures.')

    def filter_native_image(self):
        # Use the same real keys; native text lives in image payloads, not pyte.
        self.key('alt+1')
        self.key('slash')
        self.display.type(self.window, '20-picture')
        self.key('Return')
        time.sleep(.8)
        shot, image = self.shot('image-native')
        hits = image_markers(image)
        self.check('native-image', 'pass' if all(n >= 12 for n in hits) else 'fail',
                   f'Native screenshot contains fixture colors: {hits}.', shot)
        self.key('slash')
        self.key('ctrl+e')
        self.key('ctrl+u')
        self.key('BackSpace')
        self.key('Return')

    def themes(self):
        seen = set()
        maximum = 40 if self.args.themes == 'all' else int(self.args.themes)
        previous = None
        for _ in range(maximum):
            name = theme_id(self.app)
            if name is None or name in seen:
                break
            seen.add(name)
            # Leave enough time for asynchronous native shaping/presentation.
            time.sleep(.3)
            path, image = self.shot('theme-' + name)
            colors = image.getcolors(image.width * image.height) or []
            dominant = sorted(colors, reverse=True)[:6]
            signature = [rgb for _, rgb in dominant[:3]]
            raw = (self.bridge / 'output.bin').read_bytes()
            emitted = sgr_colors(raw)
            exact = {rgb for _, rgb in colors}
            matching = sum(tuple(bytes.fromhex(c[1:])) in exact for c in emitted)
            changed = previous is None or ImageChops.difference(previous, image).getbbox() is not None
            item = {'id': name, 'status': 'pass' if changed else 'fail', 'screenshot': path,
                    'dominant_colors': ['#%02x%02x%02x' % rgb for rgb in signature],
                    'rgb_colors_emitted': len(emitted), 'emitted_colors_visible': matching,
                    'visual_review': 'pending',
                    'detail': 'Configured theme captured; pixel change and RGB matches are measurements, not a legibility approval.'}
            self.result['themes'].append(item)
            previous = image
            self.key('F8')
            try:
                wait_until(lambda: theme_id(self.app) != name, timeout=2, reason='theme persistence after F8')
            except TimeoutError:
                self.check('theme-switch', 'fail', 'F8 did not persist a different theme.')
                break
        if len(seen) > 1 and self.result['checks'].get('theme-switch', {}).get('status') != 'fail':
            self.check('theme-switch', 'pass', f'{len(seen)} distinct themes changed live and persisted; screenshots saved.')
        rgb_count = len(sgr_colors((self.bridge / 'output.bin').read_bytes()))
        self.check('theme-colors', 'observed',
                   f'{rgb_count} distinct explicit RGB colors emitted. '
                   + ('Native pixels carry their own colors.' if self.native else
                      ('RGB SGR output observed.' if rgb_count else 'Cell colors may be quantized to the terminal palette; see screenshots.')))
        self.check('theme-visual-quality', 'manual', 'Review theme screenshots for glyph coverage, contrast, clipping and light-theme consistency.')

    def media(self):
        # Same sequence in native/cells; screen evidence retained for review.
        self.key('alt+1')
        self.key('slash')
        self.key('ctrl+e')
        self.key('ctrl+u')
        self.display.type(self.window, '50-movie')
        self.key('Return')
        self.key('Return')
        time.sleep(2)
        a, playing = self.shot('movie-playing')
        self.key('space')
        time.sleep(.6)
        b, paused = self.shot('movie-paused')
        time.sleep(self.args.idle_seconds)
        c, idle = self.shot('movie-idle')
        self.key('space')
        time.sleep(2)
        d, resumed = self.shot('movie-resumed')
        logs = read_logs(self.app)
        frame_changed = ImageChops.difference(idle, resumed).getbbox() is not None
        self.check('video', 'observed' if frame_changed else 'fail',
                   f'Pause {self.args.idle_seconds}s; resume screenshot changed={frame_changed}. Movie-content motion and preserved paused frame require visual review.', d)
        if 'Graphical attachment disconnected' in logs:
            self.check('video-connection', 'fail', 'Attachment disconnected during movie check.')
        else:
            self.check('video-connection', 'pass', 'No logged attachment disconnect during this bounded run.')
        self.key('Escape')

    def run(self):
        try:
            self.start()
            self.functional()
            self.themes()
            if self.args.media:
                self.media()
            self.key('Escape')
            self.key('q')
            wait_until(lambda: observe(self.bridge).get('exit_code') is not None, timeout=7, reason='clean FOLD exit')
            status = observe(self.bridge).get('exit_code')
            self.check('clean-exit', 'pass' if status == 0 else 'fail', f'FOLD exit status {status}.')
        except Exception as error:
            self.check('scenario', 'fail', f'{type(error).__name__}: {error}')
            if 'startup' not in self.result['checks']:
                self.check('startup', 'fail', str(error), 'terminal.log')
            with contextlib.suppress(Exception):
                self.shot('failure')
        finally:
            private_children(self.app)
            stop(self.process)
            if self.dbus:
                with contextlib.suppress(ProcessLookupError):
                    os.kill(self.dbus, signal.SIGTERM)
            if self.log:
                self.log.close()
            for name in ('input.bin', 'output.bin', 'state.json'):
                source = self.bridge / name
                if source.exists():
                    shutil.copy2(source, self.directory / name)
            logs = read_logs(self.app)
            self.result['errors'] = [line for line in logs.splitlines()
                                     if ' ERROR ' in line or 'panicked at' in line][-15:]
            self.result['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
            shutil.copytree(self.home, self.directory / 'home',
                            ignore=shutil.ignore_patterns('*.sock'), dirs_exist_ok=True)
            self.runtime.cleanup()
            (self.directory / 'result.json').write_text(json.dumps(self.result, indent=2))
        return self.result


def protocol_observations(output, result):
    """Report wire evidence without inferring support from emulator identity."""
    incoming = b''
    if result.get('evidence'):
        capture = output / result['evidence'] / 'input.bin'
        if capture.is_file():
            incoming = capture.read_bytes()
    image = result.get('image_protocol')
    observed = {
        'Kitty graphics': image == 'kitty',
        'Sixel': image == 'sixel',
        'iTerm2 inline images': image == 'iterm2',
        'RGB SGR': any(t.get('rgb_colors_emitted', 0) for t in result.get('themes', [])),
        'SGR mouse': bool(re.search(rb'\x1b\[<\d+;\d+;\d+[Mm]', incoming)),
        'Kitty keyboard': bool(re.search(rb'\x1b\[\d+(?:[;:][0-9:;]*)?u', incoming)),
        'Bracketed paste': b'\x1b[200~' in incoming and b'\x1b[201~' in incoming,
    }
    values = {name: 'observed' if seen else 'not-observed' for name, seen in observed.items()}
    values['OSC 52 clipboard'] = 'manual'
    values['OSC 72 drag/drop'] = 'manual'
    if result.get('status') == 'unavailable':
        values = dict.fromkeys(values, 'unavailable')
    return values


def report(output, results, metadata):
    data = {'schema': 1, 'metadata': metadata, 'results': results, 'external_targets': EXTERNAL}
    (output / 'results.json').write_text(json.dumps(data, indent=2))
    features = ['startup', 'native-ui', 'copy-operations', 'commander', 'text-preview',
                'image-preview', 'mouse-wheel', 'desktop-dnd', 'resize', 'theme-switch', 'theme-colors', 'video', 'clean-exit']
    rows = []
    csv_rows = []
    measured_headers = ['Terminal / version / profile', 'Presentation', 'Images', 'Colors', 'Themes']
    headers = measured_headers + ['Drag and drop' if f == 'desktop-dnd' else f for f in features] + list(protocol_observations(output, {}))
    protocol_note = ('Protocol columns show observed wire traffic, not a complete terminal support inventory. '
                     'not-observed means no evidence in this scenario; manual means a dedicated interaction is needed. '
                     'Graphics alternatives need not all be active. Native RGB does not require RGB SGR.')
    markdown = ['# STAR/FOLD terminal lab', '',
                'Measured Linux runs with locked Nix packages and disposable profiles. Software rendering; no user configuration changed.', '',
                '**pass** = a stated assertion passed; **observed** = evidence recorded without complete behavioral proof; '
                '**unsupported** = feature unavailable in the selected presentation/profile; **manual/not-run** = no support claim; **unavailable** = provisioning policy/package limit.', '',
                protocol_note, '',
                '| ' + ' | '.join(headers) + ' |',
                '| ' + ' | '.join('---' for _ in headers) + ' |']
    for result in results:
        checks = result.get('checks', {})
        statuses = [checks.get(f, {}).get('status', result.get('status', 'not-run')) for f in features]
        label = f"{result['terminal']} {result.get('version') or ''} / {result.get('profile', 'provisioning')}"
        presentation = result.get('presentation') or ('native' if checks.get('native-ui', {}).get('status') == 'observed' else 'cells' if checks.get('startup', {}).get('status') == 'pass' else '—')
        rgb = max((t.get('rgb_colors_emitted', 0) for t in result.get('themes', [])), default=0)
        environment = result.get('terminal_environment', {})
        palette = 'monochrome' if environment.get('NO_COLOR') else '256-color palette' if '256color' in environment.get('TERM', '') else '16-color palette'
        colors = 'native RGB' if presentation == 'native' else 'RGB SGR' if rgb else palette if result.get('themes') else '—'
        measured = [label, presentation, result.get('image_protocol', '—'), colors, str(len(result.get('themes', [])))]
        protocols = list(protocol_observations(output, result).values())
        markdown.append('| ' + ' | '.join(measured + statuses + protocols) + ' |')
        csv_rows.append(measured + statuses + protocols)
        link = result.get('evidence')
        title = '<a href="#' + html.escape(result['terminal'] + '-' + result.get('profile', '')) + '">' + html.escape(label) + '</a>'
        rows.append('<tr><th>' + title + '</th>' + ''.join('<td>' + html.escape(v) + '</td>' for v in measured[1:])
                    + ''.join(f'<td class="{s}" title="{html.escape(checks.get(f, {}).get("detail", ""))}">{s}</td>'
                              for f, s in zip(features, statuses))
                    + ''.join(f'<td class="{s}">{s}</td>' for s in protocols) + '</tr>')
    details = []
    for result in results:
        case = result.get('evidence', '')
        section = [f'<section data-terminal="{html.escape(result["terminal"])}" id="{html.escape(result["terminal"] + "-" + result.get("profile", ""))}">',
                   f'<h2>{html.escape(result["terminal"] + " / " + result.get("profile", "provisioning"))}</h2>']
        if result.get('detail'):
            section.append('<p>' + html.escape(result['detail']) + '</p>')
        for note in result.get('review_notes', []):
            section.append('<p class="observed">Visual review: ' + html.escape(note) + '</p>')
        section.append('<details><summary>Functional results and evidence</summary>')
        for name, check in result.get('checks', {}).items():
            evidence = check.get('evidence')
            link = f' <a href="{html.escape(case + "/" + evidence)}">evidence</a>' if evidence else ''
            section.append(f'<p><b>{html.escape(name)}</b>: <span class="{check["status"]}">{check["status"]}</span> — {html.escape(check["detail"])}{link}</p>')
        section.append('</details>')
        if result.get('capabilities'):
            section.append('<details><summary>Actual FOLD capability negotiation</summary><pre>' + html.escape(result['capabilities']) + '</pre></details>')
        if result.get('themes'):
            section.append('<div class="gallery">')
            for theme in result['themes']:
                path = html.escape(case + '/' + theme['screenshot'])
                section.append(f'<figure data-theme="{html.escape(theme["id"])}"><a href="{path}"><img loading="lazy" src="{path}"></a><figcaption>{html.escape(theme["id"])} · {theme["status"]} · visual review pending</figcaption></figure>')
            section.append('</div>')
        section.append('</section>')
        details.append(''.join(section))
    markdown += ['', 'macOS/Windows targets (not installed on Linux): ' + ', '.join(EXTERNAL) + '.', '',
                 'Clipboard permissions, desktop drag/drop, audible output, SSH, physical GPU/HDR, '
                 'font legibility and platform-native fullscreen require separate checks. '
                 'Consult index.html, each case/result.json, input/output captures and screenshots.']
    (output / 'REPORT.md').write_text('\n'.join(markdown) + '\n')
    with (output / 'matrix.csv').open('w', newline='') as csv_file:
        writer = csv.writer(csv_file)
        writer.writerow(headers)
        writer.writerows(csv_rows)
    style = 'body{font:14px system-ui;margin:24px;background:#181a20;color:#e5e7eb}a{color:#91b7ff}table{border-collapse:collapse;display:block;max-width:100%;overflow:auto}td,th{padding:8px;border:1px solid #454955;text-align:left}td.pass,.pass{color:#8de0aa}td.fail,.fail,.unsupported{color:#ff8b91}.observed{color:#f0cf79}pre{white-space:pre-wrap}.gallery{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:12px}figure{margin:0}img{width:100%}section{border-top:1px solid #555;margin-top:32px;padding-top:16px}figcaption{padding:8px}.manual,.not-run,.not-observed,.unavailable{color:#aab1be}select{padding:8px;margin:8px;background:#252934;color:inherit;border:1px solid #666}summary{cursor:pointer;padding:12px 0}[hidden]{display:none!important}'
    terminal_options = ''.join('<option>' + html.escape(t) + '</option>' for t in sorted({r['terminal'] for r in results}))
    theme_options = ''.join('<option>' + html.escape(t) + '</option>' for t in sorted({t['id'] for r in results for t in r.get('themes', [])}))
    controls = '<h2>Compare captured themes</h2><label>Terminal <select id="terminal-filter"><option value="">All</option>' + terminal_options + '</select></label><label>Theme <select id="theme-filter"><option value="">All</option>' + theme_options + '</select></label>'
    content = '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>STAR/FOLD terminal lab</title><style>' + style + '</style><h1>STAR/FOLD terminal lab</h1><p>Real emulator runs, locked Nix versions, private profiles and software-rendered displays. Green means the stated assertion passed. Theme screenshots still require visual approval. Native capability detection alone is not a feature pass.</p><p><a href="results.json">JSON evidence</a> · <a href="matrix.csv">CSV</a> · <a href="REPORT.md">Markdown matrix</a></p><p>' + html.escape(protocol_note) + '</p><table><tr>' + ''.join('<th>' + html.escape(f) + '</th>' for f in headers) + '</tr>' + ''.join(rows) + '</table>' + ''.join(details) + '<h2>External / manual coverage</h2><p>' + html.escape(', '.join(EXTERNAL)) + '</p><p>Clipboard prompts, desktop drag/drop, listening, SSH, real GPU/HDR, physical fullscreen, font coverage and visual legibility are not established by this run.</p></html>'
    script = "<script>const terminal=document.querySelector('#terminal-filter'),theme=document.querySelector('#theme-filter');function filter(){for(const s of document.querySelectorAll('section[data-terminal]'))s.hidden=!!terminal.value&&s.dataset.terminal!==terminal.value;for(const f of document.querySelectorAll('figure[data-theme]'))f.hidden=!!theme.value&&f.dataset.theme!==theme.value;}terminal.addEventListener('change',filter);theme.addEventListener('change',filter);</script>"
    cases = [r for r in results if 'profile' in r]
    failures = sum(any(c['status'] == 'fail' for c in r.get('checks', {}).values()) for r in cases)
    captures = sum(len(r.get('themes', [])) for r in cases)
    rollup = f'<p><strong>{len(cases)} cases · {captures} theme captures · {failures} cases with failed assertions.</strong> See individual assertions for the actual coverage.</p>'
    for related in metadata.get('related_reports', []):
        rollup += '<p><a href="' + html.escape(related['href']) + '">' + html.escape(related['label']) + '</a></p>'
    content = content.replace('<h1>STAR/FOLD terminal lab</h1>', '<h1>STAR/FOLD terminal lab</h1>' + rollup, 1)
    if details:
        content = content.replace(''.join(details), controls + ''.join(details), 1)
    else:
        content = content.replace('<h2>External / manual coverage', controls + '<h2>External / manual coverage', 1)
    content = content.replace('</html>', script + '</html>')
    (output / 'index.html').write_text(content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='Release executable or installed wrapper; defaults to ./target/release/starfold')
    parser.add_argument('--output', type=Path, default=Path('target/terminal-lab/latest'))
    parser.add_argument('--terminals', default='all', help='Comma-separated emulator IDs, or all')
    parser.add_argument('--profiles', default='cells,auto', help='cells,auto,ascii,mono,tmux,rgb (comma separated; rgb explicitly supplies COLORTERM)')
    parser.add_argument('--themes', default='all', help='all, or maximum themes to capture in each case')
    parser.add_argument('--media', action='store_true', help='Add movie pause/resume captures')
    parser.add_argument('--idle-seconds', type=float, default=15)
    parser.add_argument('--startup-timeout', type=float, default=25)
    parser.add_argument('--list', action='store_true', help='List pinned provisioning inventory without starting displays')
    parser.add_argument('--resume', action='store_true', help='Retry incomplete/failed cases; archive their previous evidence')
    parser.add_argument('--report-only', action='store_true', help='Regenerate HTML/Markdown/CSV from existing results.json')
    parser.add_argument('--reassess-evidence', action='store_true', help='Audit saved startup/help observations, retaining original assertions; implies report-only')
    parser.add_argument('--collect-evidence', action='store_true', help='Regenerate aggregate results from completed case/result.json records')
    args = parser.parse_args()
    manifest = json.loads(Path(os.environ['STARFOLD_TERMINAL_MANIFEST']).read_text())
    if args.list:
        print(json.dumps(manifest, indent=2))
        return 0
    output = args.output.resolve()
    if args.report_only or args.reassess_evidence or args.collect_evidence:
        saved = json.loads((output / 'results.json').read_text())
        if args.collect_evidence:
            saved = collect_evidence(output, saved, manifest)
        if args.reassess_evidence:
            saved = reassess(output, saved)
        report(output, saved['results'], saved['metadata'])
        print(output / 'index.html')
        return 0
    binary = (args.binary or Path.cwd() / 'target/release/starfold').resolve()
    if not binary.is_file():
        parser.error('Build through nix develop -c cargo build --release, or supply --binary.')
    args.executable = executable_target(binary)
    args.executable_sha256 = digest(args.executable)
    profiles = args.profiles.split(',')
    if any(p not in ('cells', 'auto', 'ascii', 'mono', 'tmux', 'rgb') for p in profiles):
        parser.error('Unknown profile')
    if args.themes != 'all' and (not args.themes.isdigit() or int(args.themes) < 1):
        parser.error('--themes must be all or a positive integer')
    selected = manifest['terminals']
    if args.terminals != 'all':
        ids = args.terminals.split(',')
        unknown = set(ids) - {t['id'] for t in selected}
        if unknown:
            parser.error('Unknown terminals: ' + ', '.join(sorted(unknown)))
        selected = [t for t in selected if t['id'] in ids]
    if output.exists() and any(output.iterdir()) and not args.resume:
        parser.error('Output directory must be empty; preserve previous run evidence by choosing a new path.')
    output.mkdir(parents=True, exist_ok=True)
    run_lock = (output / '.lab.lock').open('a')
    try:
        fcntl.flock(run_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        parser.error('Another lab controller is using this output directory; choose a different one.')
    infrastructure = output / 'infrastructure'
    infrastructure.mkdir(exist_ok=True)
    if args.resume:
        infrastructure = infrastructure / ('retry-' + uuid.uuid4().hex[:8])
        infrastructure.mkdir()
    metadata = {'created_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                'binary': str(binary), 'binary_sha256': digest(binary), 'system': manifest['system'],
                'executable': str(args.executable), 'executable_sha256': args.executable_sha256,
                'flake_lock_sha256': digest(HERE.parent.parent / 'flake.lock'),
                'software_rendering': True, 'profiles': profiles,
                'manifest': manifest, 'command': sys.argv}
    results = []
    if args.resume:
        saved = json.loads((output / 'results.json').read_text())
        if saved['metadata'].get('executable_sha256', args.executable_sha256) != args.executable_sha256:
            parser.error('Cannot resume with a different executable; use a new output directory.')
        metadata['previous_attempts'] = saved['metadata'].get('previous_attempts', []) + [
            {k: v for k, v in saved['metadata'].items() if k != 'previous_attempts'}]
        results = saved['results']
    display = None
    try:
        display = Display(infrastructure)
        for terminal in selected:
            if not terminal['available'] or terminal['id'] in MANUAL:
                result = {'terminal': terminal['id'], 'version': terminal['version'],
                          'status': 'unavailable' if not terminal['available'] else 'manual',
                          'detail': terminal['reason'] or MANUAL[terminal['id']]}
                results = [r for r in results if r['terminal'] != terminal['id']]
                results.append(result)
                report(output, results, metadata)
                continue
            for profile in profiles:
                previous = next((r for r in results if r['terminal'] == terminal['id'] and r.get('profile') == profile), None)
                if previous and 'clean-exit' in previous.get('checks', {}) and not any(
                        c['status'] == 'fail' for c in previous['checks'].values()):
                    print(f"KEEP {terminal['id']} {profile}", flush=True)
                    continue
                old = output / (terminal['id'] + '-' + profile)
                if old.exists():
                    history = output / 'history'
                    history.mkdir(exist_ok=True)
                    old.rename(history / (old.name + '-' + uuid.uuid4().hex[:8]))
                print(f"RUN {terminal['id']} {terminal['version']} {profile}", flush=True)
                case = Case(terminal, profile, output, display, binary, args)
                result = case.run()
                results = [r for r in results if not (r['terminal'] == terminal['id'] and r.get('profile') == profile)]
                results.append(result)
                report(output, results, metadata)
                failed = [n for n, c in result['checks'].items() if c['status'] == 'fail']
                print(f"DONE {terminal['id']} {profile}: " + (', '.join(failed) if failed else 'assertions passed'), flush=True)
    finally:
        if display:
            display.close()
        report(output, results, metadata)
    print(str(output / 'index.html'), flush=True)
    return int(any(c['status'] == 'fail' for r in results for c in r.get('checks', {}).values()))


if __name__ == '__main__':
    sys.exit(main())
