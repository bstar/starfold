# Graphical interface inside the terminal

This is a fresh experiment from STAR/FOLD main `6cbee38` (v0.0.2), on
`experiment/terminal-graphics`. The separate native GPUI experiment remains on
`experiment/graphical-presentation`.

## Direction

Use Awrit's offscreen-browser-to-terminal rendering approach. The graphical
interface must appear inside the existing terminal and preserve STAR/FOLD's
stack navigation and workflows. Preview and Operations retain their established
placement; Commander splits the browser only.

The reusable renderer, component model, terminal image transport, input routing,
capability negotiation and lifecycle belong in STAR/KIT. STAR/FOLD supplies its
application state and semantic actions. Copy, delete, rename and other filesystem
work continue through the existing controller and workers.

The authoritative foundation design, reference analysis, acceptance gates and
implementation sequence are in
[STAR/KIT's terminal graphics plan](https://github.com/bstar/starkit/blob/experiment/terminal-graphics/docs/terminal-graphics.md).
It incorporates Awrit's rendering path and cmux's surface identities, frame guards,
queue bounds, protocol boundaries and SSH transport lessons.

## Current status

Fresh branches and the architecture/reference investigation are complete.
There is no new graphical executable on this branch yet. The first implementation
milestone is a shared STAR/KIT example with actual offscreen browser rendering and
interactive pixels inside Kitty, including a headless/SSH viability test.

After that gate passes, integrate the shared components with STAR/FOLD's existing
controller and complete the workflow/compatibility matrix. A prototype with fewer
file actions will not count as completed feature alignment. Default app builds
and the regular local debug launcher remain available throughout the experiment.
