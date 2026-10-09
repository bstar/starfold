# STAR/FOLD Commander visual target

A local, buildless design review for the native terminal graphics direction (`tiny-skia` / `cosmic-text`). HTML/CSS is used only for review. The native application now carries the rack palette, beveled chrome, workspace readout and lower module deck; the browser study remains the target for further spacing and content refinements.

Open `index.html`, or check a free port and serve this directory:

```sh
ss -ltn 'sport = :3000'
python3 -m http.server 3000 --bind 0.0.0.0 --directory docs/graphical/treatments
```

Study 03 follows [STAR/AMP Option 1](http://192.168.86.57:4000/#option-1): slate rack frames, beveled square buttons, recessed green-black displays and lime readouts. Commander keeps two equal directory panes; inspection and operations sit in matching modules below. The workspace display shows marked-file count and size, the active location and illustrative volume capacity. Classic is the default, with Mocha and Latte alternatives.

Files use 28 px rows and 13–14 px names at desktop sizes. Narrow screens retain both panes, progressively hiding metadata columns and stacking the inspection and operations modules. The bundled JetBrains Mono Nerd Font is from the same family used by STAR/AMP's design studies; its SIL Open Font License is in `FONT-LICENSE.txt`. Local bundling makes the review font consistent across devices.

Click the terminal to use keyboard controls. Tab/Shift+Tab switch panes; j/k/arrows navigate; Home/End jump; Space marks; Enter/l or double-click enters a sample directory; h/Backspace returns; y yanks; p simulates copying into the active directory; m simulates moving to the opposite pane. Slash filters; b opens searchable Places; i expands/folds inspection; v switches Fold/Commander; ? opens help; Ctrl+X pauses the sample queue; X resumes. Escape dismisses overlays, clears a filter or clears sample waiting requests. Browser controls outside the terminal retain normal Tab navigation.

Marks, cursors and directory history are independent per pane and workspace. The two workspace tabs retain their contexts. Places supports typing, arrows and Enter. All filesystem paths, file contents, metadata and progress are illustrative; actions update local presentation state only. No file operations run.

`target.png`, `commander-mocha.png` and `commander-latte.png` capture the terminal area at a 1536 px viewport. `alpine.svg` is an original illustrative preview asset. There are no runtime network or package dependencies; only the optional reference link leaves the local study.

Chromium review on 2026-10-09 covered pane and workspace switching, independent marks, yank/paste queueing, stop/resume, clearing waiting entries, filtering, searched Places, text previews and double-click navigation. Layout checks passed at 1536, 1280, 930, 768 and 390 px widths, including the mobile Places dialog, with no page script exceptions.

`native-classic.png`, `native-mocha.png` and `native-latte.png` are captures from STAR/KIT's Rust renderer using FOLD's real application scene over `Fixture::tree()`, including a marked file, text preview and completed copy. These are application captures, separate from the HTML target. Recreate them with:

```sh
STARFOLD_RACK_CAPTURES=/tmp/starfold-native-rack-review \
  CARGO_NET_GIT_FETCH_WITH_CLI=true nix develop -c cargo test --locked \
  --all-features --bin starfold rack_native_frames
```
