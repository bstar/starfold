# testdata

Almost nothing lives here. The fixture tree the tests read — a home directory
with a project in it, a symlink, a broken symlink, an empty directory, fixed
modification times — is built fresh in a temporary directory by
`src/fold/testing.rs` for each test that needs one, rather than checked in.
That keeps the fixture and the tests that describe it beside each other, and
means a new case is a few lines in Rust rather than a new file to review.

This directory holds only what `testing.rs` cannot build for itself:
[`media/harbour.png`](media/README.md), a real, decodable picture for the
preview test. Everything a test helper can construct from path and byte
literals stays in the code.
