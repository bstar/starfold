# Archive format verification

This records the actual extension checks on Linux on 2026-10-08. It is a
fixture matrix, not a promise that every variant of a format works. MacPacker's
published list supplies the comparison labels; TAR.ZST is an additional FOLD
format. Finder integration, system Quick Look and translated interfaces remain
outside the current archive workspace.

Each **Pass** exercises listing, exact-member reads, extraction with byte
comparison, Test archive, and inspection after renaming the fixture to an
unrecognized suffix. **Not verified** means no representative fixture was
available; suffix recognition and codec availability alone do not establish support.

| Format | Linux fixture checks | Create | Edit in place |
| --- | --- | --- | --- |
| 7z | Pass | Yes | — |
| AR | Pass | — | — |
| ARJ | Not verified | — | — |
| BZ2 | Pass | — | — |
| CAB | Pass | — | — |
| CHM | Not verified | — | — |
| CPIO | Pass | — | — |
| DMG | Not verified | — | — |
| EXE | Pass | — | — |
| FAT | Not verified | — | — |
| GZ | Pass | — | — |
| ISO | Pass | — | — |
| LHA | Pass | — | — |
| LZ4 | Not verified | — | — |
| LZH | Pass | — | — |
| LZX | Not verified | — | — |
| MSI | Not verified | — | — |
| NTFS | Not verified | — | — |
| PKG | Not verified | — | — |
| QCOW2 | Not verified | — | — |
| RAR | Pass | — | — |
| RPM | Pass | — | — |
| SEA | Not verified | — | — |
| SIT | Not verified | — | — |
| SITX | Not verified | — | — |
| SquashFS | Not verified | — | — |
| TAR | Pass | Yes | — |
| TAR.BZ2 | Pass | — | — |
| TAR.GZ | Pass | Yes | — |
| TAR.LZ4 | Pass | — | — |
| TAR.XZ | Pass | — | — |
| TAR.Z | Pass | — | — |
| VDI | Not verified | — | — |
| VHD | Not verified | — | — |
| VHDX | Not verified | — | — |
| VMDK | Not verified | — | — |
| WIM | Not verified | — | — |
| XAR | Pass | — | — |
| XZ | Pass | — | — |
| Z | Pass | — | — |
| ZIP | Pass | Yes | ZIP chains |
| ZIPX | Pass | — | — |
| TAR.ZST | Pass | Yes | — |

Read-only formats can use **Save archive as…** to convert to ZIP, subject to the
same codec, expansion and free-space limits. ZIP member editing, encrypted
replacement, nested publication, cancellation and source-change guards have
separate protocol/controller regression tests. Password and numbered-volume
round trips are tested for ZIP/7z; this matrix's fixture checks are unencrypted.
Legacy encrypted XAD formats remain limited. Native archives with duplicate
member names are rejected when indexed decoding cannot distinguish them; ZIP/7z
selective reads retain header identity. ZIP rebuild currently rejects duplicate
output names rather than silently replacing a different header.

## Reproduce

Build the extension through the Nix shell, then run:

```sh
nix develop -c cargo build -p starfold-archive
nix develop -c python3 scripts/test-archive-matrix.py \
  --binary target/debug/starfold-archive \
  --libarchive /path/to/libarchive \
  --output /tmp/starfold-archive-matrix.json
```

The script generates fixtures in temporary storage and optionally decodes the
selected upstream libarchive `.uu` corpus. It records source links, SHA-256 hashes,
platform, individual checks and errors in JSON. No downloaded archive bytes are
committed here. Additional samples can be supplied with repeated
`--fixtures FORMAT=/path/to/sample` arguments. Treat supplied samples as untrusted
codec input; the matrix runner is intended for bounded regression fixtures.

Generated fixtures cover ZIP, 7z, TAR, gzip/bzip2/XZ streams and compressed TARs;
AR uses the system writer, and ISO/XAR use `bsdtar` when present. Corpus fixtures
were checked at libarchive revision
`8bb3bbdc7b117a1e22086a2260f2087aafa90687`.

macOS codec execution, desktop drag into other applications and physical Kitty
SSH acceptance remain separate verification gates. Linux headless checks do not
establish those results. Missing fixtures are explicitly retained in this table.
