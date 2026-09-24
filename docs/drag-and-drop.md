# Drag and drop

STAR/FOLD enables native drag and drop when the terminal answers its
[OSC 72 capability query](https://sw.kovidgoyal.net/kitty/dnd-protocol/).
Kitty 0.47 or newer implements the protocol; other terminals can support the
same gesture if they implement OSC 72. There is no separate drag mode to turn on.

Ghostty does not currently implement OSC 72 ([tracking issue](https://github.com/ghostty-org/ghostty/issues/12852)).
On macOS its `+` drag cursor means Ghostty accepts an OS file drop; it does not
mean STAR/FOLD received a drag event. In an SSH session, a Mac-local path
pasted by Ghostty cannot transfer that file to the remote machine. Use a
terminal with OSC 72 support, such as Kitty, for native drag and drop over SSH.

Drag a file row onto a folder, the other Commander pane, or a Fold breadcrumb.
Empty space or an ordinary file row in a file pane targets that pane's current
directory. Dragging a marked row carries the marked set. You can also drag
files to or from desktop applications that provide file URIs. If Copy and Move
are both available, STAR/FOLD asks which one to perform after the drop.

Dropped files enter OPERATIONS and start when the worker is free. Existing
keyboard-queued entries remain paused. Conflicts still use the usual
overwrite, skip, or rename decision. Cancelling a drop stops the transfer and
cleans up temporary files.

Over SSH, a supported terminal can stream dropped files and directory trees
through the terminal connection. Dragging out of a remote session is copy-only.

If a desktop drop over SSH does nothing, first try `kitten dnd` in the same
session to check the terminal's transfer path. Then restart STAR/FOLD with
`starfold -v`, drop onto the file pane, and inspect
`~/.local/starfold/cache/starfold.log`. The log records whether OSC 72 was
enabled, whether the pane accepted the hover, and whether a remote transfer
started; it does not log file contents.
