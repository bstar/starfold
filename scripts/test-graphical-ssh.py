#!/usr/bin/env python3
"""Real loopback SSH gate with private keys/config and a headless application host.

No user's SSH configuration, authorized_keys, HOME, or desktop is changed.
Optional delay/bandwidth flags model a slower transport; they are not a WAN test.
"""
import argparse
import getpass
import hashlib
import json
import os
from pathlib import Path
import queue
import shlex
import shutil
import socket
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', default='target/debug/starfold')
parser.add_argument('--rtt-ms', type=float, default=0)
parser.add_argument('--bandwidth-mbps', type=float, default=0)
options = parser.parse_args()
binary = Path(options.binary).resolve()
sshd = shutil.which('sshd')
if not sshd:
    raise SystemExit('Install openssh-server to run this optional integration gate')

def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()

def delay(length):
    seconds = options.rtt_ms / 2000
    if options.bandwidth_mbps > 0:
        seconds += length * 8 / (options.bandwidth_mbps * 1_000_000)
    time.sleep(seconds)

class Endpoint:
    def __init__(self, config, command, hello):
        self.process = subprocess.Popen(['ssh', '-F', str(config), 'star-graphics-test', command],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.messages = queue.Queue(maxsize=32)
        def read():
            try:
                for line in self.process.stdout:
                    if len(line) > 16 * 1024 * 1024:
                        raise RuntimeError('Unbounded SSH message')
                    delay(len(line))
                    self.messages.put(json.loads(line))
            except Exception as error:
                self.messages.put({'type': 'error', 'message': str(error)})
        threading.Thread(target=read, daemon=True).start()
        self.send(hello)
    def send(self, message):
        line = json.dumps(message) + '\n'
        delay(len(line))
        self.process.stdin.write(line)
        self.process.stdin.flush()
    def next(self, predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while True:
            message = self.messages.get(timeout=max(.01, deadline-time.monotonic()))
            if message['type'] == 'error':
                raise RuntimeError(message['message'])
            if predicate(message):
                return message
    def key(self, number, code):
        self.send({'type':'input','id':number,'revision':0,'generation':1,
            'input':{'kind':'key','code':code,'modifiers':0}})
        return self.next(lambda m: m['type']=='ack' and m['id']==number)['accepted']
    def detach(self):
        self.process.stdin.close()
        self.process.wait(timeout=5)

with tempfile.TemporaryDirectory(prefix='star-ssh-', dir='/tmp') as temporary:
    root = Path(temporary)
    for name in ['client', 'host']:
        subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(root/name)],check=True)
    (root/'authorized_keys').write_text((root/'client.pub').read_text())
    with socket.socket() as probe:
        probe.bind(('127.0.0.1',0))
        port = probe.getsockname()[1]
    (root/'known_hosts').write_text(f'[127.0.0.1]:{port} '+(root/'host.pub').read_text())
    # StrictModes is disabled only in this isolated loopback test, since the
    # authorized key is below /tmp; the temporary directory itself is private.
    (root/'sshd_config').write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {root}/host
PidFile {root}/sshd.pid
AuthorizedKeysFile {root}/authorized_keys
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM yes
AllowUsers {getpass.getuser()}
PerSourcePenalties no
LogLevel ERROR
''')
    (root/'ssh_config').write_text(f'''Host star-graphics-test
  HostName 127.0.0.1
  Port {port}
  User {getpass.getuser()}
  IdentityFile {root}/client
  IdentitiesOnly yes
  StrictHostKeyChecking yes
  UserKnownHostsFile {root}/known_hosts
''')
    environment = os.environ.copy()
    environment.pop('NOTIFY_SOCKET',None)
    daemon = subprocess.Popen([sshd,'-D','-e','-f',str(root/'sshd_config')],env=environment,
        stdout=subprocess.DEVNULL,stderr=open(root/'sshd.log','w'))
    remote_pid = None
    try:
        deadline = time.monotonic()+5
        while True:
            try:
                with socket.create_connection(('127.0.0.1',port),timeout=.1):
                    break
            except OSError:
                if time.monotonic()>deadline:
                    raise RuntimeError((root/'sshd.log').read_text())
                time.sleep(.02)
        files = root/'files'
        files.mkdir()
        (files/'destination').mkdir()
        with (files/'one 日本語.bin').open('wb') as file:
            for _ in range(64):
                file.write(b'\x07'*(1024*1024))
        (files/"two ' quoted.bin").write_bytes(b'captured marked path')
        command=' '.join(shlex.quote(s) for s in ['env','-u','DISPLAY','-u','WAYLAND_DISPLAY',
            f'STARFOLD_DIR={root}/app',str(binary),'--graphical-relay','proof','--directory',str(files)])
        hello={'type':'hello','version':1,'client':'ssh-proof','viewport':
            {'columns':100,'rows':40,'width':1200,'height':800,'generation':1}}
        start=time.monotonic()
        endpoint=Endpoint(root/'ssh_config',command,hello)
        greeting=endpoint.next(lambda m:m['type']=='hello')
        remote_pid=int(greeting['epoch'].split('-')[0])
        scene=endpoint.next(lambda m:m['type']=='scene' and any(
            c.get('label','').startswith('one') for c in m['scene']['components']))
        print(f'Headless SSH first listing: {(time.monotonic()-start)*1000:.1f} ms; scene {len(json.dumps(scene))} bytes',flush=True)
        timings=[]
        for number,code in enumerate(['down','char: ','down','char: ','char:y','home','enter','char:p'],1):
            start=time.monotonic()
            assert endpoint.key(number,code)
            timings.append((time.monotonic()-start)*1000)
        endpoint.detach()
        deadline=time.monotonic()+10
        while not (files/"destination/two ' quoted.bin").exists():
            assert time.monotonic()<deadline,'Detached remote copy did not complete'
            time.sleep(.02)
        for name in ['one 日本語.bin',"two ' quoted.bin"]:
            assert digest(files/name)==digest(files/'destination'/name)
        endpoint=Endpoint(root/'ssh_config',command+' --attach-only',hello)
        assert endpoint.next(lambda m:m['type']=='hello')['epoch']==greeting['epoch']
        assert not endpoint.key(8,'char:p'),'A repeated operation was admitted'
        assert endpoint.key(9,'char:q')
        endpoint.next(lambda m:m['type']=='closed')
        endpoint.detach()
        remote_pid=None
        timings.sort()
        print(f'Copy checksums, disconnect, reattach and duplicate rejection passed; input→ack p95 {timings[int((len(timings)-1)*.95)]:.1f} ms',flush=True)
    finally:
        daemon.terminate()
        daemon.wait(timeout=5)
        if remote_pid:
            try:
                os.kill(remote_pid,15)
            except ProcessLookupError:
                pass
