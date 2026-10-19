# Disposable Linux terminal compatibility lab. Versions come from flake.lock.
{ pkgs, source }:
let
  lib = pkgs.lib;
  specs = [
    { id = "kitty"; path = [ "kitty" ]; }
    { id = "ghostty"; path = [ "ghostty" ]; }
    { id = "wezterm"; path = [ "wezterm" ]; }
    { id = "alacritty"; path = [ "alacritty" ]; }
    { id = "foot"; path = [ "foot" ]; }
    { id = "konsole"; path = [ "kdePackages" "konsole" ]; }
    { id = "gnome-terminal"; path = [ "gnome-terminal" ]; }
    { id = "gnome-console"; path = [ "gnome-console" ]; }
    { id = "ptyxis"; path = [ "ptyxis" ]; }
    { id = "blackbox-terminal"; path = [ "blackbox-terminal" ]; }
    { id = "xfce4-terminal"; path = [ "xfce4-terminal" ]; }
    { id = "tilix"; path = [ "tilix" ]; }
    { id = "terminator"; path = [ "terminator" ]; }
    { id = "mate-terminal"; path = [ "mate-terminal" ]; }
    { id = "qterminal"; path = [ "lxqt" "qterminal" ]; }
    { id = "terminology"; path = [ "enlightenment" "terminology" ]; }
    { id = "rio"; path = [ "rio" ]; }
    { id = "contour"; path = [ "contour" ]; }
    { id = "cosmic-term"; path = [ "cosmic-term" ]; }
    { id = "xterm"; path = [ "xterm" ]; }
    { id = "mlterm"; path = [ "mlterm" ]; }
    { id = "st"; path = [ "st" ]; }
    { id = "cool-retro-term"; path = [ "cool-retro-term" ]; }
    { id = "hyper"; path = [ "hyper" ]; }
    # pkgs.tabby is an unrelated AI assistant, not Eugeny's terminal.
    { id = "tabby"; path = [ "tabby-terminal" ]; }
    { id = "warp-terminal"; path = [ "warp-terminal" ]; }
  ];
  resolve = spec:
    let
      package = lib.attrByPath spec.path null pkgs;
      evaluated = builtins.tryEval (if package == null then throw "missing package" else package.drvPath);
      available = evaluated.success;
      version = builtins.tryEval (if package == null then null else package.version or "unknown");
    in
    spec // {
      inherit available;
      package = if available then package else null;
      version = if version.success then version.value else null;
      store = if available then toString package else null;
      reason =
        if available then null
        else if package == null then "Not packaged in this locked nixpkgs revision."
        else if package.meta.broken or false then "Marked broken by this locked nixpkgs revision."
        else if lib.any (license: !(license.free or true)) (lib.toList (package.meta.license or [ ])) then "Unfree package; not enabled by the lab's default license policy."
        else "Not evaluable under this locked nixpkgs revision's normal security/platform policies.";
    };
  resolved = map resolve specs;
  manifest = pkgs.writeText "starfold-terminal-lab-manifest.json" (builtins.toJSON {
    schema = 1;
    system = pkgs.stdenv.hostPlatform.system;
    terminals = map (s: builtins.removeAttrs s [ "path" "package" ]) resolved;
  });
  python = pkgs.python3.withPackages (p: [ p.pillow p.pyte p.hypothesis ]);
  tools = [
    python
    pkgs.bash
    pkgs.coreutils
    pkgs.xorg-server
    pkgs.xauth
    pkgs.xwininfo
    pkgs.xprop
    pkgs.openbox
    pkgs.xdotool
    pkgs.wmctrl
    pkgs.dbus
    pkgs.glib
    pkgs.weston
    pkgs.mesa
    pkgs.fontconfig
    pkgs.dejavu_fonts
    pkgs.liberation_ttf
    pkgs.noto-fonts
    pkgs.nerd-fonts.symbols-only
    pkgs.tmux
    pkgs.zellij
    pkgs.ffmpeg
    pkgs.ncurses
    pkgs.util-linux
  ];
  terminals = map (s: s.package) (builtins.filter (s: s.available) resolved);
  package = pkgs.writeShellApplication {
    name = "starfold-terminal-lab";
    runtimeInputs = tools ++ terminals;
    text = ''
      export STARFOLD_TERMINAL_MANIFEST=${manifest}
      export STARFOLD_LAB_PATH=${lib.makeBinPath (tools ++ terminals)}
      export PYTHONDONTWRITEBYTECODE=1
      export STARFOLD_LAB_MESA=${pkgs.mesa}/lib/dri
      export STARFOLD_LAB_FONTS=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts pkgs.liberation_ttf pkgs.noto-fonts pkgs.nerd-fonts.symbols-only ]; }}
      exec ${python}/bin/python3 ${source}/scripts/terminal_lab/lab.py "$@"
    '';
  };
in
{
  inherit package manifest;
  check = pkgs.runCommand "starfold-terminal-lab-evidence-tests"
    {
      nativeBuildInputs = [ python ];
    } ''
    export PYTHONDONTWRITEBYTECODE=1
    python3 -m unittest discover -s ${source}/scripts/terminal_lab -v
    touch "$out"
  '';
  shell = pkgs.mkShell {
    packages = tools ++ terminals ++ [ package ];
    STARFOLD_TERMINAL_MANIFEST = manifest;
    STARFOLD_LAB_PATH = lib.makeBinPath (tools ++ terminals);
    PYTHONDONTWRITEBYTECODE = "1";
    STARFOLD_LAB_MESA = "${pkgs.mesa}/lib/dri";
    STARFOLD_LAB_FONTS = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts pkgs.liberation_ttf pkgs.noto-fonts pkgs.nerd-fonts.symbols-only ]; };
  };
}
