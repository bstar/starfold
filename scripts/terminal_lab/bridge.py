#!/usr/bin/env python3
"""Transparent PTY recorder inside a real emulator; never synthesizes replies.

The controller's Unix socket is read-only. Keys and pointer events must enter
through the emulator/display server, not through this observation channel.
"""
import codecs
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import socket
import struct
import sys
import termios
import threading
import time
import tty

import pyte


def main():
    directory = Path(sys.argv[1])
    command = sys.argv[2:]
    directory.mkdir(parents=True, exist_ok=True)
    old = termios.tcgetattr(0)
    size = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
    rows, columns, _, _ = struct.unpack('HHHH', size)
    screen = pyte.Screen(columns or 100, rows or 35)
    stream = pyte.Stream(screen)
    lock = threading.Lock()
    state = {'command': command, 'pid': None, 'exit_code': None,
             'output_bytes': 0, 'input_bytes': 0, 'resizes': [],
             'environment': {k: os.environ.get(k) for k in
                             ('TERM', 'COLORTERM', 'TERM_PROGRAM', 'LANG', 'NO_COLOR', 'STAR_KIT_ASCII')}}
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, size)
    # Enter raw mode before the child can issue queries. TCSAFLUSH after fork
    # can discard fast emulator replies and leave a multiplexer waiting forever.
    tty.setraw(0)
    child = os.fork()
    if child == 0:
        os.close(master)
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        for fd in (0, 1, 2):
            os.dup2(slave, fd)
        if slave > 2:
            os.close(slave)
        os.execvpe(command[0], command, os.environ)
    state['pid'] = child
    os.close(slave)
    stop = threading.Event()
    server = socket.socket(socket.AF_UNIX)
    server.bind(str(directory / 'observe.sock'))
    server.listen(4)
    server.settimeout(.2)

    def snapshot():
        with lock:
            result = dict(state, columns=screen.columns, rows=screen.lines,
                          screen='\n'.join(screen.display))
        (directory / 'state.json').write_text(json.dumps(result))
        return result

    def observe():
        while not stop.is_set():
            try:
                client, _ = server.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with client:
                client.settimeout(1)
                try:
                    client.recv(128)
                    client.sendall(json.dumps(snapshot()).encode() + b'\n')
                except (OSError, ValueError):
                    pass
    observer = threading.Thread(target=observe, daemon=True)
    observer.start()

    def resized(*_):
        current = fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))
        fcntl.ioctl(master, termios.TIOCSWINSZ, current)
        nr, nc, _, _ = struct.unpack('HHHH', current)
        with lock:
            screen.resize(nr or 1, nc or 1)
            state['resizes'].append([nc, nr])
    signal.signal(signal.SIGWINCH, resized)
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    decoder = codecs.getincrementaldecoder('utf-8')('replace')
    # Strip OSC/DCS/APC image payloads before pyte without dropping cell CSI.
    parsing = 'text'
    previous_escape = False

    def visible(data):
        nonlocal parsing, previous_escape
        result = []
        for char in data:
            if parsing == 'string':
                if char == '\x07' or (previous_escape and char == '\\'):
                    parsing = 'text'
                previous_escape = char == '\x1b'
            elif parsing == 'escape':
                if char in ']P_^X':
                    parsing = 'string'
                    previous_escape = False
                else:
                    result.extend(('\x1b', char))
                    parsing = 'text'
            elif char == '\x1b':
                parsing = 'escape'
            else:
                result.append(char)
        return ''.join(result)

    try:
        with (directory / 'output.bin').open('wb') as output, (directory / 'input.bin').open('wb') as inputs:
            while not stop.is_set():
                ready, _, _ = select.select([0, master], [], [], .05)
                if 0 in ready:
                    data = os.read(0, 65536)
                    if not data:
                        break
                    inputs.write(data)
                    inputs.flush()
                    state['input_bytes'] += len(data)
                    os.write(master, data)
                if master in ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        data = b''
                    if not data:
                        break
                    if state['output_bytes'] < 48 * 1024 * 1024:
                        output.write(data)
                        output.flush()
                    state['output_bytes'] += len(data)
                    with lock:
                        stream.feed(visible(decoder.decode(data)))
                    view = memoryview(data)
                    while view:
                        view = view[os.write(1, view):]
                pid, status = os.waitpid(child, os.WNOHANG)
                if pid:
                    state['exit_code'] = os.waitstatus_to_exitcode(status)
                    break
    finally:
        try:
            os.killpg(child, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if state['exit_code'] is None:
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                try:
                    pid, status = os.waitpid(child, os.WNOHANG)
                except ChildProcessError:
                    break
                if pid:
                    state['exit_code'] = os.waitstatus_to_exitcode(status)
                    break
                time.sleep(.02)
            else:
                os.killpg(child, signal.SIGKILL)
                _, status = os.waitpid(child, 0)
                state['exit_code'] = os.waitstatus_to_exitcode(status)
        stop.set()
        server.close()
        snapshot()
        os.close(master)
        termios.tcsetattr(0, termios.TCSANOW, old)
    return state['exit_code'] or 0


if __name__ == '__main__':
    sys.exit(main())
