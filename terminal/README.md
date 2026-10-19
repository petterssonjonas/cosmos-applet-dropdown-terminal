# COSMIC Dropdown Terminal backend

This is a GPL-3.0-only fork of [System76 cosmic-term](https://github.com/pop-os/cosmic-term),
including its terminal renderer, profiles, translations, resources, and licenses.
It uses native Wayland layer-shell surfaces, not window positioning or reparenting.

## Dropdown lifecycle and control

`cosmic-dropdown-terminal --daemon` runs in the foreground, initially hidden.
`cosmic-dropdown-terminal start` ensures the backend exists without changing its
visibility. With no arguments, the controller behaves like `toggle`. `toggle` and
`show` launch a backend when absent. `hide`, `status`, and `quit` do not launch one.
Automatically launched backends detach from the controller's session and write
diagnostics to `backend.log` in the private runtime directory.
Use `--help` for shell-command and working-directory options.

The backend owns `$XDG_RUNTIME_DIR/cosmic-dropdown-terminal/control.sock`.
The directory must be owned by the current user and private (0700); the socket is
0600. A persistent `owner.lock` inode serializes ownership and stale-socket recovery.
Each connection accepts one of `toggle`, `show`, `hide`, `status`, or `quit`, followed
by newline. Replies are `visible`, `hidden`, `bye`, or `error MESSAGE`, with newline.
CLI commands print `stopped` when no backend exists and no startup was requested.
Visibility replies describe the requested state; a show request before output
discovery is applied once usable output information arrives.
CLI `quit` waits up to five seconds after acknowledgement for the old backend to
release ownership. A replacement backend does not extend this wait.

The surface anchors to the top, left, and right edges of the first advertised
usable output, occupies 40% of its logical height rounded to the nearest pixel,
and uses the overlay layer. It respects reserved panel zones so the panel icon
remains clickable, and does not reserve additional desktop space. Output scale,
rotation, dimensions, and hot-unplug changes update
the geometry; an unplugged selected output falls back to another available output.
There is no application animation. Compositor-managed effects are outside this
application's control.

Hiding releases keyboard focus and destroys only the Wayland surface. All terminal
models, PTYs, child processes, scrollback, tabs, and panes remain alive. Surface
creation and destruction are serialized through compositor open/close
acknowledgements. Rapid commands update the desired visibility while a transition
is pending; the latest request is applied after acknowledgement, without a debounce.
A surface requests exclusive focus when created; a hide request immediately
withdraws keyboard interactivity even while opening is still pending.
Closing the header or using the close-window action hides; `quit` intentionally
terminates the daemon and its shells. Closing tabs still closes those terminals.
After the final shell exits the daemon hides; showing it starts a new shell.

This requires a Wayland compositor implementing layer-shell. It has no X11 or
ordinary-window fallback. Monitor choice is deterministic, not pointer-active,
because this client does not receive global pointer position. Overlay placement
and focus requests remain subject to compositor and session-lock policy.

The unit tests in `src/control.rs` cover owner exclusion, restart/stale-socket
recovery, permissions, symlink rejection, command parsing, and bounded quit
synchronization without waiting for a replacement owner. `src/dropdown.rs` covers
initial hidden state, height rounding, hiding during creation, rapid toggles during
destruction, stale acknowledgements, and unexpected compositor closure. Integration
needs a real or nested layer-shell compositor to exercise PTY survival and focus.

The Cargo package retains the upstream `cosmic-term` name so the embedded Fluent
translation domain remains compatible; the only binary is
`cosmic-dropdown-terminal`. Use the root project's installer for integration.

The `wgpu` feature, enabled by default, supports GPU rendering using `glyphon`
and `wgpu`. If `wgpu` is not enabled or fails to initialize, then rendering falls
back to using `softbuffer` and `tiny-skia`.

## Color Schemes

Custom color schemes can be imported from the `View -> Color schemes...` menu item.
You can find templates for color schemes in the [color-schemes](color-schemes) folder.
