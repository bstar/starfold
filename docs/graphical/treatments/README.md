# STAR/FOLD Commander visual target

A local, buildless design review for the native terminal graphics direction (`tiny-skia` / `cosmic-text`). No application code or renderer behavior is changed. HTML/CSS is used only for review.

Open `index.html`, or check a free port and serve this directory:

```sh
ss -ltn 'sport = :3000'
python3 -m http.server 3000 --bind 0.0.0.0 --directory docs/graphical/treatments
```

Study 02 prioritizes Commander: equal panes, no directory stack, compact inspection, 16 px filenames, 36 px rows, 2 px panel borders and 3 px outer/center borders. The bundled JetBrains Mono Nerd Font is from the same family used by STAR/AMP's design studies; its SIL Open Font License is in `FONT-LICENSE.txt`. Local bundling makes the review font consistent across devices.

Click the terminal to use keyboard controls. Tab/Shift+Tab switch panes; j/k/arrows navigate; Home/End jump; Space marks; Enter/l enters a sample directory; h/Backspace returns; y yanks; p simulates copying into the active directory; m simulates moving to the opposite pane. Slash filters; b opens searchable Places; i expands/folds inspection; ? opens help; Ctrl+X pauses the sample queue; X resumes. Escape dismisses overlays, clears a filter or clears sample waiting requests. Browser controls outside the terminal retain normal Tab navigation.

Marks and cursors are independent per pane. Places supports typing, arrows and Enter. All filesystem paths, file contents, metadata and progress are illustrative; actions update local presentation state only. No file operations run.

`target.png`, `commander-mineral.png` and `commander-paper.png` are full-size browser captures at a 1536 px viewport. `alpine.svg` is an original illustrative preview asset. There are no external network or package dependencies.
