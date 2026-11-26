# STAR/FOLD on Omarchy

STAR/FOLD runs in a terminal, so on Omarchy it launches the same way Omarchy's
own TUIs do. Nothing here changes system files. Every step goes in your own
config and is undone by deleting the lines.

## Install

Add the signed [star] package repository once (its
[setup page](https://github.com/bstar/star-packages) has the steps), then:

```sh
sudo pacman -S starfold
```

STAR/FOLD then updates with `pacman -Syu` and `omarchy update`. The package
adds a desktop entry, so STAR/FOLD shows up in the app launcher
(Super + Alt + Space), and pacman tracks removal. Packages are also on the
[releases page](https://github.com/bstar/starfold/releases); see
[Installing](installing.md#arch-linux) for building from the supplied
`PKGBUILD`.

With `theme = "system"`, STAR/FOLD takes its colours from the active Omarchy
theme when it starts.

## Put it on a key

Omarchy binds Super + Shift + F to its graphical file manager. To open
STAR/FOLD there instead, add this to `~/.config/hypr/bindings.lua`:

```lua
o.rebind("SUPER + SHIFT + F", "File manager", { tui = "starfold", focus = true })
```

With `focus = true`, pressing the key again focuses the STAR/FOLD window that's
already open instead of starting another one. To keep Omarchy's file manager
and add STAR/FOLD beside it, use a free key with `o.bind` instead.

To open STAR/FOLD in the directory of the focused terminal, the way
Super + Alt + Shift + F does for the default file manager:

```lua
o.rebind("SUPER + ALT + SHIFT + F", "File manager (cwd)",
  'omarchy-launch-tui starfold "$(omarchy-cmd-terminal-cwd)"')
```

Hyprland reloads the bindings when you save the file.

## Graphical or cell mode

In Kitty, STAR/FOLD starts with graphical presentation; in other terminals it
uses cells. Add `--cells` or `--graphical` to the command above to choose for
that launch. F9 switches while it runs. See
[Terminal graphics](terminal-graphics.md).

## What it doesn't do

STAR/FOLD isn't registered as the handler for folders (`inode/directory`), so
"Show in folder" from a browser still opens the default file manager. It also
isn't a file picker: open and save dialogs come from the desktop portal, not
from STAR/FOLD.
