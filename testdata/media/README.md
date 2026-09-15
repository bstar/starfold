# A picture for the preview

One tiny file, and the only binary blob in the tree.

| File | What it is |
|---|---|
| `harbour.png` | 64x48, a sky, a waterline and three boats. What the image-preview test decodes and draws. |

It is **synthetic**, like every other fixture here: no picture was taken from
anywhere. It is a few coloured rectangles and a circle, written out as an SVG
by hand and rendered to PNG with `rsvg-convert`. Nothing about it is worth
regenerating; if it ever has to change, any image of the same size and format
does.

It is committed rather than built by the test fixture because it is a real,
decodable image, and `fold/testing.rs` builds the rest of the fixture tree —
directories, plain files, symlinks — out of nothing but path and byte
literals. A PNG is not something a test helper should be encoding by hand.
