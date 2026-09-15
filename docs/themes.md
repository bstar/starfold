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

`t` and `T` cycle the built-ins while it is running, and `"system"` follows the
desktop through Stylix. The sixteen built-in ids are listed on
[the configuration page](configuration.md#theming).

They are the same sixteen files STAR/AMP and STAR/CORD read, so setting a
theme in one gives you the same look in the others. A theme file written for
one works here and the other way round: each program ignores the tables that
belong to the others.

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
