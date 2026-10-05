#!/usr/bin/env python3
"""External GTK -> Kitty Copy/Move mouse regression in a private Xvfb display.

Requires GTK3/PyGObject, ImageMagick, JetBrainsMono Nerd Font, libX11 and libXtst.
Use a private 2400x1080 Xvfb screen. Uses a narrow 942x1040 tabbed window,
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

GTK_SOURCE = r"""
import gi,sys
from pathlib import Path
gi.require_version('Gtk','3.0')
from gi.repository import Gtk,Gdk
p=Path(sys.argv[1])
print("Creating GTK fixture", flush=True)
w=Gtk.Window(title='Private external drag fixture');w.set_default_size(300,180);w.move(1820,0)
b=Gtk.EventBox();b.add(Gtk.Label(label='Drag fixture file'))
b.drag_source_set(Gdk.ModifierType.BUTTON1_MASK,[Gtk.TargetEntry.new('text/uri-list',0,0)],Gdk.DragAction.COPY|Gdk.DragAction.MOVE)
b.connect('drag-data-get',lambda widget,context,data,info,t:data.set_uris([p.as_uri()]))
b.connect('drag-data-delete',lambda *a:p.unlink(missing_ok=True))
w.add(b);w.connect('destroy',Gtk.main_quit);w.show_all();w.get_window().move(1820,0);w.get_window().raise_();print("GTK mapped",w.get_position(),flush=True);Gtk.main()
"""

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
  '-o','remember_window_size=no','-o','initial_window_width=942','-o','initial_window_height=1040',
  str(pathlib.Path(binary).resolve()),'graphical','--session','dragpixels',str(files)],
 env={**os.environ,'STARFOLD_DIR':str(root/'app'),'STARFOLD_CONFIG_DIR':str(root/'config'),'STARFOLD_LOG':'starfold=debug,starkit=debug'},stdout=log,stderr=log)
 def rc(*args):
  if args[0] == 'screenshot':return subprocess.check_output(['magick','import','-window','root',args[1]],stderr=subprocess.PIPE,timeout=10)
  return subprocess.check_output([kitten,'@','--to',address,*args],stderr=subprocess.PIPE,timeout=10)
 def key(k):rc('send-key',k);time.sleep(.35)
 app_log=root/'app/cache/starfold.log'
 try:
  deadline=time.monotonic()+30
  while not app_log.exists() or 'Graphical frame presented revision=' not in app_log.read_text():
   assert time.monotonic()<deadline
   time.sleep(.1)
  time.sleep(.7)
  key('ctrl+t')
  for k in ['v','enter','tab','j','enter','tab']:key(k)
  rc('screenshot',str(out/'before.png'))

  source_process=subprocess.Popen(['python3','-c',GTK_SOURCE,str(left/'payload.bin')],env={**os.environ,'GDK_BACKEND':'x11','NO_AT_BRIDGE':'1','GDK_SCALE':'1','GDK_DPI_SCALE':'1','GTK_MODULES':''})
  time.sleep(1)
  rc("screenshot",str(out/"source-ready.png"))
  # Actual external GTK drag.
  motion(1900,90);button(1)
  for x,y in [(1900,100),(1800,200),(1200,350),(600,350)]:motion(x,y)
  time.sleep(.4);button(0);time.sleep(.7)
  rc('screenshot',str(out/'dropped.png'))
  motion(630,352);time.sleep(.5);rc("screenshot",str(out/"hover.png"));button(1);button(0)
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
  motion(1900,90);button(1)
  for x,y in [(1900,100),(1200,350),(600,350)]:motion(x,y)
  button(0);time.sleep(.7)
  motion(630,368);button(1);button(0)
  deadline=time.monotonic()+8
  while not (right/'payload.bin').exists():
   assert time.monotonic()<deadline, 'Mouse Move did not complete'
   time.sleep(.05)
  assert (right/'payload.bin').read_bytes()==payload
  deadline=time.monotonic()+8
  while (left/'payload.bin').exists():
   assert time.monotonic()<deadline, 'Move did not remove the source after delivery'
   time.sleep(.05)
  trace = app_log.read_text()
  assert trace.count('Drop choice selected kind=Copy') == 1, 'Copy click chose wrong action'
  assert trace.count('Drop choice selected kind=Move') == 1, 'Move click chose wrong action'
  assert trace.count('own_sources=0') == 2, 'Both drops must originate outside FOLD'
  # The local queue owns removal after the desktop offer has been cancelled.
  # Verify both delivery and removal, rather than only a protocol acknowledgement.


  assert trace.count('OSC 72 drag hover received') < 100, 'Hover acceptance feedback loop'

  key('q');rc('close-window');process.wait(timeout=10)
  print('External Copy and Move clicks selected their exact actions, delivered exact bytes and removed only the moved source.')
 finally:
  if "source_process" in locals():source_process.terminate();source_process.wait()
  if app_log.exists():(out/'application.log').write_text(app_log.read_text())
  if process.poll() is None:
   try:key('escape');key('q');rc('close-window')
   except Exception:pass
   process.terminate();process.wait(timeout=10)
