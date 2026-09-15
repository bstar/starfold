# testdata

Almost nothing lives here. The fixture tree the tests read is built fresh in
a temporary directory by `Fixture::tree()` in `src/fold/testing.rs`, for each
test that needs one, rather than checked in:

```
home/
  projects/
    starwire/
      src/main.rs            "fn main() {}\n"
      Cargo.toml              a few lines of toml
      README.md                markdown, 31 lines
      .gitignore              hidden
      target/                 empty directory
  pictures/
    harbour.png               testdata/media/harbour.png, 64x48
  notes.txt -> projects/starwire/README.md    symlink
  dangling -> nowhere         broken symlink
  blob.bin                    256 bytes, with NULs in them
  empty/                      empty directory
```

`src/main.rs` is stamped two minutes before the fixture's fixed instant
(`Fixture::now()`, 2026-09-11 12:00:00 UTC), `Cargo.toml` an hour before, and
everything else a day before — fixed rather than read from the clock, so a
test asserting `"14:02"` gets the same string on every machine and on every
day it runs. The symlink and the broken symlink are only built on Unix.

Building the tree fresh, rather than checking it in, keeps the fixture and
the tests that describe it beside each other, and means a new case is a few
lines of Rust rather than a new file to review — and a checked-in tree could
not carry a symlink on every platform git runs on, or have its mtimes pinned,
anyway.

This directory holds only what `testing.rs` cannot build for itself:
[`media/harbour.png`](media/README.md), a real, decodable picture for the
preview test. Everything a test helper can construct from path and byte
literals stays in the code.
