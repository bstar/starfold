# The command line

```
starfold [--verbose] [DIR]
starfold list DIR [--hidden] [--sort KEY]
```

`--verbose` raises the log level to debug. It goes to the log file at
`~/.local/starfold/cache/starfold.log` and never to the terminal: the window
owns the alternate screen, and `list` writes a report to stdout that a script
may be reading. `STARFOLD_LOG` overrides the filter entirely, with the syntax
`tracing`'s `EnvFilter` uses — `STARFOLD_LOG=starfold::fold::ops=trace`, for
instance.

With no subcommand, `starfold` opens the window on `DIR`, or on the current
directory if none is given.

## `starfold list`

Reads one directory and prints it, with no window involved. It is how the
core is exercised while there is little UI to click through yet, and it is
meant to stay after there is one: a defect that reproduces with no terminal
attached is a defect with a much shorter report.

```sh
starfold list ~/projects/starfold
```

```
src/                              dir         -    Sep 12 14:02
Cargo.toml                        toml    1.2 KB   Sep 14 09:27
Cargo.lock                        lock  139.5 KB   Sep 14 09:39
README.md                         md    3.1 KB   Sep 10 09:41
```

| Flag | Does |
| --- | --- |
| `--hidden` | include dotfiles |
| `--sort KEY` | `name`, `size`, `modified` or `kind`. Default `name` |

### Exit status

`0` on a successful listing. Non-zero, with the reason on stderr, when the
directory cannot be read — permission denied, or it does not exist.

## Environment variables

| Variable | Does |
| --- | --- |
| `STARFOLD_DIR` | relocates the whole of `~/.local/starfold` |
| `STARFOLD_CONFIG_DIR` | relocates just `config.toml`, leaving the cache and the session file where they were |
| `STARFOLD_LOG` | a `tracing` `EnvFilter`, overriding `--verbose` entirely |
