# Cosmos Dropdown Terminal

A COSMIC panel applet with a persistent dropdown terminal. The included backend is adapted from [COSMIC Terminal](https://github.com/pop-os/cosmic-term). You do not need an external `cosmic-term` executable.

Click the panel icon to show or hide the terminal. Right-click the icon to open **Settings**.

The terminal starts at 40% of the screen height and 100% of its width. It stays above other windows by default. Click the terminal to type. Click another window to give that window keyboard input. The terminal stays visible.

Hiding keeps shells, jobs, tabs, panes and scrollback. Closing the window hides it. If you exit the last shell, the next show starts a new shell.

## Requirements

Runtime: a COSMIC Wayland session with layer-shell and data-control clipboard support, working graphics drivers and terminal fonts. There is no X11 fallback. No Python process or external clipboard command is needed at runtime.

Build/install: current stable Rust from [rustup](https://rustup.rs), `just`, Bash, GNU coreutils/findutils, Git and the native development packages below. Distribution Rust packages can be too old. These are package examples, not a claim that all distributions were tested.

Fedora:

```sh
sudo dnf install git just gcc-c++ cmake pkgconf-pkg-config desktop-file-utils \
  expat-devel freetype-devel fontconfig-devel wayland-devel libxkbcommon-devel \
  libxkbcommon-x11-devel libinput-devel mesa-libEGL-devel libX11-devel \
  libXcursor-devel libXi-devel libXrandr-devel dbus-devel openssl-devel
```

Debian 13 or newer:

```sh
sudo apt update
sudo apt install git just build-essential cmake pkgconf desktop-file-utils \
  libexpat1-dev libfreetype-dev libfontconfig-dev libwayland-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libinput-dev libegl1-mesa-dev libx11-dev libxcursor-dev \
  libxi-dev libxrandr-dev libdbus-1-dev libssl-dev
```

Arch Linux:

```sh
sudo pacman -S --needed git just base-devel cmake pkgconf desktop-file-utils \
  expat freetype2 fontconfig wayland libxkbcommon libxkbcommon-x11 libinput \
  mesa libglvnd libx11 libxcursor libxi libxrandr dbus openssl
```

Optional installation tests use Python 3 and `gio`: Fedora `sudo dnf install python3 glib2`; Debian `sudo apt install python3 libglib2.0-bin`; Arch `sudo pacman -S --needed python glib2`. These are not native terminal runtime dependencies.

## Build and install

Install rustup from its linked site first. Run these commands as your desktop user:

```sh
git clone https://github.com/petterssonjonas/cosmos-applet-dropdown-terminal
cd cosmos-applet-dropdown-terminal
rustup toolchain install stable
rustup override set stable
just build-release
just install
```

Use `sudo` only for system package prerequisites, never for `just build-release` or `just install`. Installation uses `~/.local/bin` and `~/.local/share`. It does not change panel entries or shortcuts. Use your mapped desktop user in a build container, not root.

### Install a prebuilt archive

Extract the archive and open its directory. Run `just install` as your desktop user. Do not run `just build-release` unless you want to rebuild. The archive includes the release executables in `target/release`.

You need `just`, Bash, GNU coreutils and `desktop-file-utils` to install. Rust and development packages are not needed for a prebuilt install. The laptop must use the archive's CPU architecture and compatible system libraries. These executables are not statically linked. If a required library or GLIBC version is unavailable, build from source on the laptop.

## Enable and use

In **COSMIC Settings → Desktop → Panel → Applets**, add **Cosmos Dropdown Terminal**. Reopen Settings or log in again if necessary. Close Settings, then explicitly enable **Super+§**:

```sh
just configure-shortcut
```

`just install` does not register a keyboard shortcut. Run `just configure-shortcut` in your desktop session, as your normal user, with COSMIC Settings closed. This uses **Super+§** when no shortcut is assigned. It keeps an existing shortcut for this terminal. COSMIC watches shortcut changes; a session restart is normally not needed. Reopen COSMIC Settings to refresh its list.

To set the shortcut manually, open **Settings → Input devices → Keyboard → Keyboard shortcuts → View and customize shortcuts → Custom → Add shortcut**. Use **Cosmos Dropdown Terminal** as the name. Use this command, with your actual home directory:

```text
'/home/YOUR_USER/.local/bin/cosmos-applet-dropdown-terminal-backend' toggle
```

Keep the single quotes around the absolute executable path. The applet identifies its shortcut by this exact command. Do not use `~` or change the quoting. Focus **Type key combination**, press Super and the desired key, then release.

Search for the action name in Keyboard shortcuts. COSMIC can display the binding as **Super+section**. Settings' global search does not list every custom binding. The `section` symbol is `§`; the printed keycap can differ from the symbol produced by your keyboard layout. Recording the combination in Settings uses your actual layout.

Only one action can use a shortcut. Registration keeps other actions, saves a `custom.before-dropdown` backup and refuses conflicts. `§` means the XKB `section` symbol, not a fixed physical key.

### Dropdown settings

Right-click the panel icon. Select **Settings**.

| Setting | Default | Use |
| --- | --- | --- |
| Height | 40% | Set the height as a percentage of the screen. |
| Width | 100% | Set the width as a percentage of the screen. A narrow dropdown is centered. |
| Always on top | On | Keep the dropdown above other windows without blocking their keyboard input. |

Height and width accept 1% to 100%. Select **Save** to apply edited settings to the open terminal. Select **Cancel**, press Escape or close the popup to discard unsaved edits. If Save reports an error, read the message: some fields may already be saved.

Use **Settings** to change the height and width. Mouse resizing is not supported. There is no resize handle or bottom size bar. The terminal has a minimum size of 96 × 96 logical pixels. The screen and reserved panel space limit its size.

To change the shortcut, select its modifiers: **Super**, **Ctrl**, **Alt** or **Shift**. Enter a character or XKB key name, such as `§`, `F12` or `grave`. Select **Save**. Do not press the whole combination in the key field. Close COSMIC Settings before you save a shortcut. If the combination is already in use, the applet shows an error and keeps the old shortcut. You can also record a combination in COSMIC Settings, as described above.

The terminal always opens at the top edge, above application windows and below reserved panel space. Turn **Always on top** off to let application windows cover it when it loses focus. Showing the dropdown raises it again. This changes its stacking layer, not its position or size. It does not become a normal window. Changing this setting keeps the same shells and tabs.

The terminal's close button hides the dropdown. It does not stop the backend or close its shells. Click the panel icon or use the shortcut to show the same session again. Use **Restart** or **Quit** in the panel menu only when you want to close all sessions.

### Restart or quit

Right-click the panel icon. The menu has **Settings**, **Restart** and **Quit**.

- **Restart** stops the terminal backend, then opens a new terminal with the installed version.
- **Quit** stops the terminal backend. The panel icon stays in place. Click the icon or use the shortcut to start a new terminal.

**Warning:** Restart and Quit close all terminal shells and running jobs. Finish your work first. Both actions ask for confirmation. Select **Cancel** to keep the terminal running. To keep your shells and jobs, hide the dropdown instead.

### Copy and paste

Select terminal text. Use **Copy** in the right-click menu or press **Ctrl+Shift+C**. Paste into another application with that application's paste command. The backend keeps copied text available when you hide the dropdown or change **Always on top**. Another application can replace the clipboard contents normally.

Use **Ctrl+Shift+V** to paste into the terminal. If copying fails, a **Copy failed** message appears below the terminal content. The backend log contains the error.

### Backend controls

```sh
~/.local/bin/cosmos-applet-dropdown-terminal-backend toggle
~/.local/bin/cosmos-applet-dropdown-terminal-backend status
~/.local/bin/cosmos-applet-dropdown-terminal-backend reload
# WARNING: quit terminates shells and running jobs.
~/.local/bin/cosmos-applet-dropdown-terminal-backend quit
```

Removing the applet or installing an upgrade does not stop the backend. An old backend can keep running with old resize or settings behavior. Finish its jobs, then select **Restart** from the panel icon menu. Remove and add the panel applet again to load a new applet version. If the installed applet has no Restart action, run `~/.local/bin/cosmos-applet-dropdown-terminal-backend quit` before adding it again. Restart and Quit close all terminal shells. Logout also ends sessions. Do not restart the backend just to change settings: **Save** reloads preferences without closing shells.

Preferences use `~/.config/cosmic/dev.jp.CosmosDropdownTerminal`, separate from stock COSMIC Terminal. Logs use `$XDG_RUNTIME_DIR/cosmos-applet-dropdown-terminal-backend/backend.log`; the control directory and socket are private to your user.

The backend selects the first usable compositor output, not the pointer's output. Autohiding panels can overlap it. A compositor disconnect stops the backend and closes its terminal sessions. There is no animation or session restoration after logout or a compositor restart.

Checks in an isolated COSMIC session covered Save, Cancel, shortcut conflicts, minimum and maximum sizes, keyboard focus, stacking and output resizing. The dropdown opened above a maximized application with **Always on top** off and on. With the setting off, focus transfer let the other application cover it; showing the dropdown raised it again. With the setting on, it stayed above the other application while that application received keyboard input. The close button hid it without changing the shell process or its variables. Settings changed its height and width; dragging its bottom edge did not. Copying from the context menu remained available after hiding the terminal and pasted into another application. A forced compositor crash stopped the backend without continued log growth. Menu checks confirmed that Cancel kept the same backend, Restart replaced it, and Quit stopped it. The panel icon then started a new terminal. These checks do not cover every GPU or suspend path.

## Migration and removal

For an old `cosmic-dropdown-terminal` installation, remove its panel entry and shortcut. Finish its jobs, then run `cosmic-dropdown-terminal quit`. Before starting the new backend, run `just migrate`. This copies old preferences to the new namespace, retains the source and refuses existing destinations or unsafe files. Running sessions cannot be migrated. Replace the panel entry and explicitly configure the new shortcut. There are no legacy aliases. Remove old executable and desktop registrations with the old version's uninstaller, if available.

To remove this version, remove its panel entry and shortcut, finish jobs, quit the backend, then run `just uninstall`. Saved preferences remain.

## License

Independent project; not an official System76 application. GPL-3.0-only; see `LICENSE` and `terminal/LICENSE`. Upstream COSMIC Terminal source, translations, color schemes and notices remain under `terminal/`. Native dependency guidance: [libcosmic](https://github.com/pop-os/libcosmic#dependencies).
