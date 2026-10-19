# Themes

A theme is one TOML file. The format belongs to
[STAR/KIT](https://github.com/bstar/starkit), the crate STAR/AMP, STAR/CORD
and STAR/FOLD share, and its
[`docs/themes.md`](https://github.com/bstar/starkit/blob/main/docs/themes.md)
is the reference: what the file holds, how a palette is derived from eight
colours, and how a base16 scheme becomes a theme.

This page is only what is particular to STAR/FOLD.

## Picking one

```toml
[ui]
theme = "catppuccin-mocha"
```

`F8` and `Shift+F8` cycle the built-ins while it is running (`alt+t` and
`alt+shift+t` also work). The footer
briefly shows the selected theme. `"system"` follows the desktop through
Stylix. The sixteen built-in ids are listed on
[the configuration page](configuration.md#theming).

The shared themes are the same files STAR/AMP and STAR/CORD read, so setting a
theme in one gives you the same look in the others. A theme file written for
one works here and the other way round: each program ignores the tables that
belong to the others.

## Light and classic palettes

FOLD also ships these palettes:

| ID | Treatment |
| --- | --- |
| `gruvbox-light` | Warm cream paper with dark brown text and teal accents. |
| `gruvbox-light-soft` | Softer cream with the same Gruvbox accent colors. |
| `dos-shell` | Light gray surfaces, black text and DOS blue accents. |
| `norton-commander` | Deep blue wells with cream text and cyan accents. |
| `windows-311` | Gray frames, white file areas and navy accents. |

They use the existing KIT controls and layout. Set `ui.theme` to an ID above,
or cycle to it live. The choice is saved automatically. Gruvbox colors follow
the [original palette](https://github.com/morhetz/gruvbox/blob/master/colors/gruvbox.vim).

Theme files can optionally supply a `[graphical]` table with `bg`, `inset_bg`,
`fg`, `muted`, `accent`, `selected`, `border` and `error` hex colors. These roles
keep native frame and file-area colors independent; ordinary cell clients keep
the shared theme's roles. `graphical_palette = "classic"` still overrides them.

## Your own

Put `<id>.toml` in `~/.local/starfold/themes/`. A file there wins over a
built-in with the same id, which is how you adjust one without forking it:
copy it, change the two colours you wanted changed, keep the id.

## The `[fold]` table

The one table STAR/FOLD adds. It is optional, and usually absent: the roles a
file listing needs are derived from the same palette everything else comes
from, and held to a contrast floor afterwards.

State one when the derivation gets it wrong for your palette:

```toml
[fold]
marked_fg = "#f5c518"
conflict_fg = "#e06c75"
```

A role you state is used exactly as written — the contrast floor applies only
to derived colours, because a derivation is a way of not writing a dozen
colours, not a committee sitting over the ones somebody did write.

[Configuration](configuration.md#the-fold-roles) lists every role and what it
colours.
