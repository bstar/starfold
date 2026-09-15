# The command line

```
starfold [--verbose] [DIR]
starfold list DIR [--hidden] [--sort name|size|time|ext]
```

`--verbose` raises the log level to debug. It goes to the log file at
`~/.local/starfold/cache/starfold.log` and never to the terminal: the window
owns the alternate screen, and `list` writes its report to stdout, which a
script may be reading. `STARFOLD_LOG` overrides the filter entirely, with the
syntax `tracing`'s `EnvFilter` uses — `STARFOLD_LOG=starfold::fold::ops=trace`,
for instance.

With no subcommand, `starfold` opens the window on `DIR`, or on the session's
last directory (falling back to the current one on a first run) if none is
given.

## `starfold list`

Reads one directory and prints it, with no window involved — STAR/CORD's
`probe` equivalent. It exists because the core is written before there is a
terminal to drive it, and it stays useful afterwards: printing what a
directory holds with no TTY attached is a much shorter bug report than the
whole window.

```sh
starfold list ~/projects/starfold
```

```
drwxr-xr-x         -  Sep 12      src/
-rw-r--r--    1.2 KB  Sep 14      Cargo.toml
-rw-r--r--  139.5 KB  Sep 14      Cargo.lock
-rw-r--r--    3.1 KB  Sep 10      README.md
```

Four columns: the Unix mode (`ls -l`'s ten characters — type plus the three
`rwx` triads, with setuid/setgid/sticky folded into the executable position
the way `ls -l` draws them), the size (`-` for a directory, since a
directory's own inode size is not one), when it was last modified (a clock
time for today, month and day for this year, a full date for anything older
or for a time in the future), and the name. A directory's name is shown with
a trailing `/`; a symlink to one counts as a directory here too.

| Flag | Does |
| --- | --- |
| `--hidden` | include entries whose name starts with `.` |
| `--sort KEY` | `name`, `size`, `time` or `ext`. Default `name` |

If the directory holds more than `[ui] max_entries` entries (50000 by
default — the same limit the window's own listing uses), the read stops there
and the last line of output is `(truncated)`.

### Exit status

`0` on success. Non-zero on any failure, with the reason on stderr — most
often the directory cannot be read (permission denied) or does not exist, in
which case the line is `<dir>: <reason>` and the exit code is `1`.

## Environment variables

| Variable | Does |
| --- | --- |
| `STARFOLD_DIR` | relocates the whole of `~/.local/starfold` |
| `STARFOLD_CONFIG_DIR` | relocates just `config.toml`, leaving the cache and the session file where they were |
| `STARFOLD_LOG` | a `tracing` `EnvFilter`, overriding `--verbose` entirely |
