#!/usr/bin/env python3
"""Real mouse Copy/Move smoke test in a private Xvfb display (Kitty >= 0.49).

Requires JetBrainsMono Nerd Font, libX11 and libXtst. Uses a fixed 1800x1000,
9pt fixture so coordinates are explicit. Run ONLY under a private Xvfb display:
this script injects X11 pointer events. Unit tests cover varying geometry,
marked sets, fast releases, cancellation, self-drops and scrollbar ownership.
"""
import argparse
import ctypes
import ctypes.util
import os
import pathlib
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
for name in ('binary', 'kitty', 'kitten', 'output'):
 parser.add_argument('--' + name, required=True)
parser.add_argument('--private-xvfb', action='store_true', required=True,
                    help='Confirm DISPLAY belongs to an isolated Xvfb server')
args = parser.parse_args()
x11=ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6");xt=ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
x11.XOpenDisplay.restype=ctypes.c_void_p
display=x11.XOpenDisplay(None)
assert display, "Cannot open the private Xvfb display"
xt.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
xt.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
x11.XFlush.argtypes=[ctypes.c_void_p]
def motion(x,y):
 xt.XTestFakeMotionEvent(display,-1,x,y,0);x11.XFlush(display);time.sleep(.1)
def button(down):
 xt.XTestFakeButtonEvent(display,1,down,0);x11.XFlush(display);time.sleep(.1)
binary, output = args.binary, args.output
out=pathlib.Path(output);out.mkdir(exist_ok=True)
kitty=args.kitty
kitten=args.kitten
with tempfile.TemporaryDirectory(prefix='fold-drag-pixels-') as temp:
 root=pathlib.Path(temp);files=root/'files';files.mkdir()
 left=files/'left';left.mkdir();right=files/'right';right.mkdir()
 payload=b'Graphical drag identity\n'*100
 (left/'payload.bin').write_bytes(payload)
 for i in range(60):(right/f'filler-{i:03}.txt').write_text('fixture\n')
 address=f'unix:{root}/rc.sock'
 log=(out/'kitty.log').open('w')
 process=subprocess.Popen([kitty,'--config','/dev/null','--hold','--listen-on',address,
  '-o','allow_remote_control=yes','-o','linux_display_server=x11',
  '-o','font_family=JetBrainsMono Nerd Font','-o','font_size=9','-o','window_padding_width=15',
  '-o','remember_window_size=no','-o','initial_window_width=1800','-o','initial_window_height=1000',
  str(pathlib.Path(binary).resolve()),'graphical','--session','dragpixels',str(files)],
 env={**os.environ,'STARFOLD_DIR':str(root/'app'),'STARFOLD_CONFIG_DIR':str(root/'config'),'STARFOLD_LOG':'starfold=debug,starkit=debug'},stdout=log,stderr=log)
 def rc(*args):return subprocess.check_output([kitten,'@','--to',address,*args],stderr=subprocess.PIPE,timeout=10)
 def key(k):rc('send-key',k);time.sleep(.35)
 app_log=root/'app/cache/starfold.log'
 try:
  deadline=time.monotonic()+30
  while not app_log.exists() or 'Graphical frame presented revision=' not in app_log.read_text():
   assert time.monotonic()<deadline
   time.sleep(.1)
  time.sleep(.7)
  for k in ['v','enter','tab','j','enter','tab']:key(k)
  rc('screenshot',str(out/'before.png'))

  # Actual X11 events exercise Kitty's desktop drag protocol.
  motion(160,82);button(1)
  for x,y in [(190,82),(300,120),(600,160),(950,210),(1150,250)]:motion(x,y)
  time.sleep(.4);button(0);time.sleep(.7)
  rc('screenshot',str(out/'dropped.png'))
  motion(1180,268);button(1);button(0)
  deadline=time.monotonic()+8
  while not (right/'payload.bin').exists():
   if time.monotonic()>deadline:
    rc('screenshot',str(out/'failure.png'))
    raise AssertionError('actual Kitty mouse drag did not copy')
   time.sleep(.05)
  assert (right/'payload.bin').read_bytes()==payload
  assert (left/'payload.bin').read_bytes()==payload
  rc('screenshot',str(out/'copied.png'))
  # A second drop uses Move through the mouse menu.
  (right/'payload.bin').unlink()
  time.sleep(.6)
  motion(160,82);button(1)
  for x,y in [(190,82),(600,160),(1150,250)]:motion(x,y)
  button(0);time.sleep(.7)
  motion(1180,284);button(1);button(0)
  deadline=time.monotonic()+8
  while (left/'payload.bin').exists() or not (right/'payload.bin').exists():
   assert time.monotonic()<deadline, 'Mouse Move did not complete'
   time.sleep(.05)
  assert (right/'payload.bin').read_bytes()==payload
  trace = app_log.read_text()
  assert trace.count('OSC 72 drop received') == 2, 'Must exercise Kitty OSC 72, not only synthetic mouse input'
  assert trace.count('OSC 72 drag hover received') < 100, 'Hover acceptance feedback loop'

  key('q');rc('close-window');process.wait(timeout=10)
  print('Actual X11 drag Copy and Move succeeded through mouse menus with exact bytes.')
 finally:
  if app_log.exists():(out/'application.log').write_text(app_log.read_text())
  if process.poll() is None:
   try:key('escape');key('q');rc('close-window')
   except Exception:pass
   process.terminate();process.wait(timeout=10)
