# Visible Automation Contract

`cua-conductor` adds a user-visible automation state surface without changing cua-driver's existing agent-cursor semantics.

## Invariant

When an automation is launched through the conductor and overlay visibility is enabled, the warning overlay is started before the managed child process. The warning must remain visibly present while that process is active and is removed when the managed process exits.

The overlay is deliberately independent from the target application. It is:

- topmost across the Windows virtual desktop;
- non-activating, so it does not steal keyboard focus;
- click-through (`HTTRANSPARENT` plus `WS_EX_TRANSPARENT`);
- custom-drawn rather than dependent on the target UI framework;
- able to flash without moving the real mouse pointer.

The agent cursor answers **where the automation acts**. The warning overlay answers **whether automation is active at all**.
## Modes

The standalone warning renderer accepts these stable states:

- `observe` — amber: read-only observation or inspection;
- `active` — orange: an automation session is live;
- `acting` — flashing red: the automation is currently mutating/interacting;
- `paused` — blue: automation is deliberately waiting;
- `error` — red: automation lost its expected execution state.

The initial conductor keeps one warning process alive for the lifetime of a managed child. Later action-level event integration may change the mode in real time, but session visibility must not depend on those richer events.

## Acceptance

A visual-warning change is not accepted from process existence alone. Capture the exact warning HWND through cua-driver's visual path and verify the rendered warning. For flashing states, capture at least two timer phases and verify that the image pixels differ across the full warning surface.

The warning window intentionally has no actionable UIA descendants; it is a status surface, not an interactive control.
