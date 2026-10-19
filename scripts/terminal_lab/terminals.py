"""CLI adapters. Config files belong only to the case's disposable HOME."""
import shlex
from pathlib import Path

# Linux packages which require an interactive application/session workflow.
MANUAL = {'hyper': 'Its CLI opens windows; no reliable isolated command adapter yet.',
          'warp-terminal': 'Proprietary build/login and interactive command setup require external review.'}
WAYLAND = {'foot', 'cosmic-term'}
EXTERNAL = ['iTerm2 (macOS)', 'Apple Terminal (macOS)', 'Windows Terminal (Windows/WSL)',
            'mintty (Windows/MSYS2)', 'PuTTY (Windows)']


def launch(terminal, home, command):
    name, store = terminal['id'], Path(terminal['store'])
    executable = store / 'bin' / {'gnome-console': 'kgx', 'contour': 'contour',
                                 'blackbox-terminal': 'blackbox', 'cosmic-term': 'cosmic-term'}.get(name, name)
    font = 'DejaVu Sans Mono'
    if name == 'kitty':
        return [str(executable), '--config', '/dev/null', '-o', 'linux_display_server=x11',
                '-o', 'confirm_os_window_close=0', '-o', f'font_family={font}',
                '-o', 'font_size=11', '-o', 'remember_window_size=no',
                '-o', 'initial_window_width=1200', '-o', 'initial_window_height=800', *command]
    if name == 'ghostty':
        return [str(executable), '--gtk-single-instance=false', '--config-default-files=false',
                f'--font-family={font}', '--font-size=11', '-e', *command]
    if name == 'wezterm':
        config = home / 'wezterm.lua'
        config.write_text("local w = require 'wezterm'; return {font=w.font('DejaVu Sans Mono'), font_size=11, enable_wayland=false, check_for_updates=false, front_end='OpenGL'}\n")
        return [str(executable), '--config-file', str(config), 'start', '--always-new-process', '--', *command]
    if name == 'alacritty':
        config = home / 'alacritty.toml'
        config.write_text('[font]\nsize = 11\n[font.normal]\nfamily = "DejaVu Sans Mono"\n')
        return [str(executable), '--config-file', str(config), '-e', *command]
    if name == 'foot':
        config = home / 'foot.ini'
        config.write_text('[main]\nfont=DejaVu Sans Mono:size=11\n')
        return [str(executable), '--config', str(config), '--window-size-pixels=1200x800', *command]
    if name == 'konsole':
        return [str(executable), '--separate', '--nofork', '--hide-menubar', '-e', *command]
    if name in ('gnome-terminal', 'gnome-console'):
        return [str(executable), '--wait', '--', *command]
    if name == 'ptyxis':
        return [str(executable), '--standalone', '--', *command]
    if name == 'blackbox-terminal':
        return [str(executable), '--command', shlex.join(command)]
    if name == 'xfce4-terminal':
        return [str(executable), '--disable-server', '--hide-menubar', '--execute', *command]
    if name == 'tilix':
        return [str(executable), '--new-process', '-e', *command]
    if name == 'terminator':
        return [str(executable), '--no-dbus', '-x', *command]
    if name == 'mate-terminal':
        return [str(executable), '--disable-factory', '--execute', *command]
    if name == 'contour':
        return [str(executable), 'terminal', 'execute', *command]
    if name == 'cosmic-term':
        return [str(executable), '--no-daemon', '--', *command]
    if name == 'cool-retro-term':
        return [str(executable), '--default-settings', '-e', *command]
    if name == 'tabby':
        return [str(executable), 'run', *command]
    if name == 'xterm':
        return [str(executable), '-fa', font, '-fs', '11', '-geometry', '120x40', '-e', *command]
    if name == 'st':
        return [str(executable), '-f', font + ':size=11', '-e', *command]
    return [str(executable), '-e', *command]
