# Installing

Built packages for the current release are on the
[releases page](https://github.com/bstar/starfold/releases/latest). Pick the
route that matches your machine.

| Route | For | Needs on the machine |
| --- | --- | --- |
| [Nix](#nix-and-nixos) | NixOS, or Nix on Linux or macOS | Nix with flakes |
| [AppImage](#appimage) | any desktop Linux | nothing |
| [macOS](#macos) | Apple Silicon | release archive or Nix |
| [From source](#from-source) | anything else | a Rust toolchain |

"Nothing" in that table is meant literally, and it is worth saying out loud
because it is unusual for a program that draws pictures and deletes to the
trash. There are no system libraries at all: trash is the freedesktop
specification in pure Rust on Linux and `NSFileManager` on macOS, and nothing
in the tree runs bindgen. A C runtime is the whole of it.

## Nix and NixOS

```sh
nix run github:bstar/starfold
```

Declaratively, with the home-manager module:

```nix
{
  inputs.starfold.url = "github:bstar/starfold";

  # in your home-manager config:
  imports = [ inputs.starfold.homeManagerModules.starfold ];
  programs.starfold = {
    enable = true;
    theme = "catppuccin-mocha";
    stylix.enable = true;    # derive the theme from your base16 scheme instead
    settings.ui.show_hidden = true;
  };
}
```

`settings` is merged into `~/.local/starfold/config.toml` last, so anything on
[the configuration page](configuration.md) can be set from there.

There is also an overlay, if you would rather have the package in `pkgs`:

```nix
nixpkgs.overlays = [ inputs.starfold.overlays.default ];
```

## AppImage

The one that needs nothing installed. Download it, make it executable, run it:

```sh
chmod +x starfold-*-x86_64.AppImage
./starfold-*-x86_64.AppImage
```

> [!TIP]
> On a distribution that no longer ships libfuse2, run it as
> `./starfold-*.AppImage --appimage-extract-and-run`.

Each release's AppImage is started on eight distributions in CI before the
release is drafted, so "it runs on yours" is tested rather than hoped for.




## macOS

Download `starfold-<version>-aarch64-apple-darwin.tar.gz` from the releases page,
extract it, and run the enclosed `starfold` executable. The archive is unsigned.


Apple Silicon. With Nix:

```sh
nix run github:bstar/starfold
```

Or from source, with nothing but a Rust toolchain:

```sh
cargo build --release
```

What is different on a Mac: deleting to the trash goes through
`NSFileManager` rather than the freedesktop trash directories. Pictures depend
on the terminal, as they do everywhere — iTerm2, kitty, WezTerm and Ghostty
all draw them.

Intel Macs need the plain `cargo` build above. There is no Nix package for
them, because nixpkgs 26.11 dropped `x86_64-darwin`.

## From source

You need a Rust toolchain, 1.90 or newer, and nothing else.

```sh
nix develop -c cargo build --release   # or supply the toolchain yourself
```

The build fetches [STAR/KIT](https://github.com/bstar/starkit) from git — this
project's own shared foundation, pinned to a tag rather than published to
crates.io — so a first build needs network. `nix build` needs it too; the flake
sets `cargoLock.allowBuiltinFetchGit` for that reason, which
[CONTRIBUTING.md](../CONTRIBUTING.md) explains.

## Terminals, and whether you get a picture in the preview

The preview panel draws an image as real pixels where the terminal has a
graphics protocol, and as half-blocks where it does not. Half-blocks always
work; they are simply two rows of colour per character cell rather than an
image.

| Terminal | Pictures |
| --- | --- |
| kitty | yes, kitty protocol |
| Ghostty | yes, kitty protocol |
| WezTerm | yes |
| foot | yes, sixel |
| iTerm2 | yes |
| xterm with sixel enabled | yes |
| Alacritty, GNOME Terminal, Konsole | half-blocks |
| inside tmux | half-blocks |

tmux is on that list for a reason that is not a shortcoming in tmux: a
graphics protocol writes pixels at a cursor position that the multiplexer is
also moving, and most of the time the picture is transmitted and simply never
appears. Half-blocks are drawn out of ordinary characters and survive it.

[Configuration](configuration.md) has the `[ui] graphics` setting for insisting
on a protocol the detection could not see, which is what you want over ssh.
