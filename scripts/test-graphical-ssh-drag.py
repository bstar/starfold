#!/usr/bin/env python3
"""Real Nautilus -> Kitty -> SSH mouse Copy/Move gate on Hyprland.

Explicitly injects host mouse input through /dev/uinput, and creates two private
floating fixture windows. Requires Kitty, Nautilus, OpenSSH, grim, writable
/dev/uinput and Hyprland's Lua dispatch API. All files, keys, SSH configuration,
and application sessions are temporary. This is a loopback test, not a WAN test.
"""
import getpass
import shlex
import socket
import argparse
import sys
import os
import pathlib
import subprocess
import tempfile
import time

if '--uinput-helper' in sys.argv:
 import fcntl
 import struct
 import signal
 signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
 fd=os.open('/dev/uinput',os.O_WRONLY|os.O_NONBLOCK)
 for ev in [0,1,2]:fcntl.ioctl(fd,0x40045564,ev)
 fcntl.ioctl(fd,0x40045565,272)
 for ax in [0,1]:fcntl.ioctl(fd,0x40045566,ax)
 os.write(fd,struct.pack('80sHHHHi',b'STAR isolated drag fixture',3,0x1234,1,1,0)+bytes(1024))
 fcntl.ioctl(fd,0x5501);time.sleep(1)
 def ev(t,c,v):os.write(fd,struct.pack('qqHHi',0,0,t,c,v))
 try:
  for line in sys.stdin:
   p=line.split()
   if p[0]=='M':
    subprocess.run(['hyprctl','eval',f'hl.dispatch(hl.dsp.cursor.move({{x={p[1]},y={p[2]}}}))'],stdout=subprocess.DEVNULL)
    ev(2,0,1);ev(0,0,0);time.sleep(.02);ev(2,0,-1);ev(0,0,0)
   elif p[0]=='B':ev(1,272,int(p[1]));ev(0,0,0)
   time.sleep(.1)
 finally:
  ev(1,272,0);ev(0,0,0)
  fcntl.ioctl(fd,0x5502);os.close(fd)
 sys.exit(0)

parser = argparse.ArgumentParser(description=__doc__)
for name in ('binary', 'kitty', 'kitten', 'output'):
 parser.add_argument('--' + name, required=True)
parser.add_argument('--inject-host-mouse', action='store_true', required=True, help='Run mouse injection on this desktop')
parser.add_argument('--origin',default='0,0',help='Fixture top-left coordinates, e.g. 2574,1106')
parser.add_argument('--tree',action='store_true',help='Copy a nested directory and symlink instead of a file')
parser.add_argument('--plain-ssh',action='store_true',help='Launch normally inside an existing SSH shell, through the Kitty integration')
parser.add_argument('--watcher',help='STAR/KIT Kitty watcher path; required for --plain-ssh')
parser.add_argument('--kitty-config',default='/dev/null',help='Kitty configuration, optionally with the integration already installed')
parser.add_argument('--payload-mib',type=int,help='Size of the transferred file, to exercise transport backpressure')
args = parser.parse_args()
if args.plain_ssh and not args.watcher and args.kitty_config=='/dev/null':parser.error('--plain-ssh requires --watcher or an installed --kitty-config')
origin_x,origin_y=map(int,args.origin.split(','))
pointer=subprocess.Popen([sys.executable,str(pathlib.Path(__file__).resolve()),'--uinput-helper'],stdin=subprocess.PIPE,text=True)
def motion(x,y):
 pointer.stdin.write(f'M {x} {y}\n');pointer.stdin.flush();time.sleep(.1)
def button(down):
 pointer.stdin.write(f'B {down}\n');pointer.stdin.flush();time.sleep(.1)
binary, output = args.binary, args.output
out=pathlib.Path(output);out.mkdir(exist_ok=True)
kitty=args.kitty
kitten=args.kitten
with tempfile.TemporaryDirectory(prefix='fold-drag-pixels-') as temp:
 root=pathlib.Path(temp);files=root/'files';files.mkdir()
 left=files/'left';left.mkdir();right=files/'right';right.mkdir()
 payload=b'Graphical drag identity\n'*100000
 if args.payload_mib:
  assert 1<=args.payload_mib<=256
  size=args.payload_mib*1024*1024
  payload=(payload*(size//len(payload)+1))[:size]
 if args.tree:
  (left/'payload.bin/nested').mkdir(parents=True)
  (left/'payload.bin/nested/leaf.bin').write_bytes(payload)
  (left/'payload.bin/link').symlink_to('nested/leaf.bin')
 else:(left/'payload.bin').write_bytes(payload)
 def delivered(base):
  if args.tree:
   assert os.readlink(base/'payload.bin/link')=='nested/leaf.bin'
   return (base/'payload.bin/nested/leaf.bin').read_bytes()
  return (base/'payload.bin').read_bytes()
 def wait_delivery():
  deadline=time.monotonic()+30
  while time.monotonic()<deadline:
   try:
    if delivered(right)==payload:return
   except OSError:pass
   time.sleep(.05)
  rc('screenshot',str(out/'failure.png'))
  raise AssertionError('Mouse-selected transfer did not deliver exact bytes')
 for i in range(60):(right/f'filler-{i:03}.txt').write_text('fixture\n')
 # Private loopback SSH server: no user keys or SSH configuration changed.
 for name in ['client','host']:
  subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(root/name)],check=True)
 (root/'authorized_keys').write_text((root/'client.pub').read_text())
 with socket.socket() as probe:
  probe.bind(('127.0.0.1',0));port=probe.getsockname()[1]
 (root/'known_hosts').write_text(f'[127.0.0.1]:{port} '+(root/'host.pub').read_text())
 (root/'sshd_config').write_text(f"""Port {port}
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
""")
 (root/'ssh_config').write_text(f"""Host star-graphics-test
  HostName 127.0.0.1
  Port {port}
  User {getpass.getuser()}
  IdentityFile {root}/client
  IdentitiesOnly yes
  StrictHostKeyChecking yes
  UserKnownHostsFile {root}/known_hosts
""")
 daemon=subprocess.Popen(['/usr/bin/sshd','-D','-e','-f',str(root/'sshd_config')],stdout=subprocess.DEVNULL,stderr=(out/'sshd.log').open('w'),env={k:v for k,v in os.environ.items() if k!='NOTIFY_SOCKET'})
 remote_executable=root/'remote-fold'
 remote_executable.write_text('#!/bin/sh\nexec env '+shlex.quote('STARFOLD_DIR='+str(root/'remote-app'))+' '+shlex.quote('STARFOLD_CONFIG_DIR='+str(root/'remote-config'))+' '+shlex.quote('STARFOLD_LOG=starfold=debug,starkit=debug')+' '+shlex.quote(str(pathlib.Path(binary).resolve()))+' "$@"\n')
 remote_executable.chmod(0o700)
 address=f'unix:{root}/rc.sock'
 log=(out/'kitty.log').open('w')
 command=([ 'ssh','-tt','-F',str(root/'ssh_config'),'star-graphics-test',
            ' '.join(map(shlex.quote,[str(remote_executable),'graphical','--session','dragpixels',str(files)])) ]
           if args.plain_ssh else
           [str(pathlib.Path(binary).resolve()),'graphical','--ssh','star-graphics-test','--ssh-config',str(root/'ssh_config'),'--remote-executable',str(remote_executable),'--session','dragpixels',str(files)])
 process=subprocess.Popen([kitty,'--class','star-drop-fixture','--config',args.kitty_config,'--hold','--listen-on',address,
  '-o','allow_remote_control=yes','-o','linux_display_server=wayland',
  '-o','font_family=JetBrainsMono Nerd Font','-o','font_size=9','-o','window_padding_width=15',
  '-o','remember_window_size=no','-o','initial_window_width=942','-o','initial_window_height=1040',
  *(['-o','watcher='+str(pathlib.Path(args.watcher).resolve())] if args.watcher else []),*command],
 env={**os.environ,'STARFOLD_DIR':str(root/'app'),'STARFOLD_CONFIG_DIR':str(root/'config'),'STARFOLD_LOG':'starfold=debug,starkit=debug'},stdout=log,stderr=log)
 def position(title,x,y,w,h):
  import json
  deadline=time.monotonic()+10
  while True:
   clients=json.loads(subprocess.check_output(['hyprctl','clients','-j']))
   matches=[c for c in clients if (c['class']=='star-drop-fixture' if title=='starfold-graphical' else c['title']==title and str(left).encode() in pathlib.Path(f'/proc/{c["pid"]}/cmdline').read_bytes())]
   if matches:break
   assert time.monotonic()<deadline
   time.sleep(.1)
  addr=matches[-1]['address']
  subprocess.run(['hyprctl','eval',f'hl.dispatch(hl.dsp.window.float({{action="set",window="address:{addr}"}}))'],check=True,stdout=subprocess.DEVNULL)
  subprocess.run(['hyprctl','eval',f'hl.dispatch(hl.dsp.window.resize({{x={w},y={h},relative=false,window="address:{addr}"}}))'],check=True,stdout=subprocess.DEVNULL)
  subprocess.run(['hyprctl','eval',f'hl.dispatch(hl.dsp.window.move({{x={x},y={y},relative=false,window="address:{addr}"}}))'],check=True,stdout=subprocess.DEVNULL)
  return addr
 def rc(*args):
  if args[0] == 'screenshot':return subprocess.check_output(['grim','-g',f'{origin_x},{origin_y} 1897x1040',args[1]],stderr=subprocess.PIPE,timeout=10)
  return subprocess.check_output([kitten,'@','--to',address,*args],stderr=subprocess.PIPE,timeout=10)
 def key(k):rc('send-key',k);time.sleep(.35)
 app_log=root/'remote-app/cache/starfold.log'
 client_log=root/'app/cache/starfold.log'
 try:
  deadline=time.monotonic()+30
  while not client_log.exists() or 'Graphical frame presented revision=' not in client_log.read_text():
   assert time.monotonic()<deadline
   time.sleep(.1)
  time.sleep(.7)
  position('starfold-graphical',origin_x,origin_y,942,1040)
  key('ctrl+t')
  for k in ['v','enter','tab','j','enter','tab']:key(k)
  rc('screenshot',str(out/'before.png'))

  source_process=subprocess.Popen(['dbus-run-session','--','nautilus','--new-window',str(left)],env={**os.environ,'GDK_BACKEND':'wayland','NO_AT_BRIDGE':'1','GDK_SCALE':'1','GDK_DPI_SCALE':'1','GTK_MODULES':'','XDG_CACHE_HOME':str(root/'nautilus-cache'),'XDG_DATA_HOME':str(root/'nautilus-data'),'XDG_DATA_DIRS':str(root/'empty-data'),'GSETTINGS_SCHEMA_DIR':'/usr/share/glib-2.0/schemas','GSETTINGS_BACKEND':'memory'})
  time.sleep(3)
  source_address=position("left",origin_x+960,origin_y,560,900)
  rc("screenshot",str(out/"source-ready.png"))
  # Actual external GTK drag.
  motion(origin_x+1046,origin_y+94);button(1)
  for x,y in [(origin_x+1056,origin_y+104),(origin_x+926,origin_y+294),(origin_x+626,origin_y+350),(origin_x+600,origin_y+350)]:motion(x,y)
  time.sleep(.4);button(0);time.sleep(.7)
  rc('screenshot',str(out/'dropped.png'))
  motion(origin_x+630,origin_y+352);time.sleep(.5);rc("screenshot",str(out/"hover.png"));button(1);button(0)
  wait_delivery()
  assert delivered(right)==payload
  assert delivered(left)==payload
  rc('screenshot',str(out/'copied.png'))
  # A second drop uses Move through the mouse menu.
  __import__('shutil').rmtree(right/'payload.bin') if args.tree else (right/'payload.bin').unlink()
  time.sleep(.6)
  motion(origin_x+1046,origin_y+94);button(1)
  for x,y in [(origin_x+1056,origin_y+104),(origin_x+626,origin_y+350),(origin_x+600,origin_y+350)]:motion(x,y)
  button(0);time.sleep(.7)
  motion(origin_x+630,origin_y+368);button(1);button(0)
  wait_delivery()
  assert delivered(right)==payload
  deadline=time.monotonic()+8
  while (left/'payload.bin').exists():
   assert time.monotonic()<deadline, 'Move did not remove the source after delivery'
   time.sleep(.05)
  trace = app_log.read_text()
  assert trace.count('receiving remote drop files')==2
  assert client_log.read_text().count('SSH drop captured; desktop pointer released')==2
  assert trace.count('Drop choice selected kind=Copy') == 1, 'Copy click chose wrong action'
  assert trace.count('Drop choice selected kind=Move') == 1, 'Move click chose wrong action'
  assert trace.count('own_sources=0') == 2, 'Both drops must originate outside FOLD'
  # The local bridge removes sources after remote publication succeeds.
  # Verify both delivery and removal, rather than only a protocol acknowledgement.


  assert trace.count('OSC 72 drag hover received') < 100, 'Hover acceptance feedback loop'

  key('q');rc('close-window');process.wait(timeout=10)
  print('External Copy and Move clicks selected their exact actions, delivered exact bytes and removed only the moved source.')
 finally:
  if 'source_address' in locals():
   subprocess.run(['hyprctl','dispatch',f'hl.dsp.window.close("address:{source_address}")'],stdout=subprocess.DEVNULL)
  daemon.terminate();daemon.wait()
  if client_log.exists():(out/'client.log').write_text(client_log.read_text())
  import signal
  for proc in pathlib.Path('/proc').iterdir():
   if not proc.name.isdigit():continue
   try:env=(proc/'environ').read_bytes()
   except OSError:continue
   if ('STARFOLD_DIR='+str(root/'remote-app')).encode()+b'\0' in env:
    try:os.kill(int(proc.name),signal.SIGTERM)
    except ProcessLookupError:pass
  pointer.terminate();pointer.wait()
  if "source_process" in locals():source_process.terminate();source_process.wait()
  if app_log.exists():(out/'application.log').write_text(app_log.read_text())
  if process.poll() is None:
   try:key('escape');key('q');rc('close-window')
   except Exception:pass
   process.terminate();process.wait(timeout=10)
