# Cosmos Dropdown Terminal

A COSMIC panel applet with a persistent dropdown terminal. The included backend is adapted from [COSMIC Terminal](https://github.com/pop-os/cosmic-term). You do not need an external `cosmic-term` executable.

Click the panel icon to show or hide the terminal. Right-click the icon to open **Settings**.

The terminal starts at 40% of the screen height and 100% of its width. It stays above other windows by default. Click the terminal to type. Click another window to give that window keyboard input. The terminal stays visible.

Hiding keeps shells, jobs, tabs, panes and scrollback. Closing the window hides it. If you exit the last shell, the next show starts a new shell.

## Requirements

Runtime: a COSMIC Wayland session with layer-shell support, working graphics drivers and terminal fonts. There is no X11 fallback. No Python process is needed at runtime.

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

Height and width accept 1% to 100%. Changes apply to an open terminal. Release a slider to save its value. Reserved panel space limits the maximum visible height. The bottom bar shows the actual percentage.

Drag the bottom edge of the terminal to change its height. The bottom bar shows the current percentage. Release the mouse button to save the height.

To change the shortcut, select its modifiers: **Super**, **Ctrl**, **Alt** or **Shift**. Enter a character or XKB key name, such as `§`, `F12` or `grave`. Select **Apply keybinding**. Do not press the whole combination in the key field. Close COSMIC Settings before you apply a shortcut. If the combination is already in use, the applet shows an error and keeps the old shortcut. You can also record a combination in COSMIC Settings, as described above.

Turn **Always on top** off to use a normal window. COSMIC then controls its position, size, tiling and stacking. It cannot be fixed to the top edge in this mode. Switching this setting keeps the same shells and tabs.

### Backend controls

```sh
~/.local/bin/cosmos-applet-dropdown-terminal-backend toggle
~/.local/bin/cosmos-applet-dropdown-terminal-backend status
# WARNING: quit terminates shells and running jobs.
~/.local/bin/cosmos-applet-dropdown-terminal-backend quit
```

Removing the applet or installing an upgrade does not stop the backend. Finish jobs and use `quit` before starting an upgraded backend. After an upgrade, remove and add the panel applet again to load its new menu. Logout also ends sessions. Preferences use `~/.config/cosmic/dev.jp.CosmosDropdownTerminal`, separate from stock COSMIC Terminal. Logs use `$XDG_RUNTIME_DIR/cosmos-applet-dropdown-terminal-backend/backend.log`; the control directory and socket are private to your user.

The backend selects the first usable compositor output, not the pointer's output. Autohiding panels can overlap it. There is no animation or session restoration after logout.

Checks in an isolated COSMIC session covered settings, shortcut changes and conflicts, minimum and maximum sizes, live dragging, keyboard focus, both stacking modes and output resizing. The shell PID and environment stayed unchanged across surface changes. These checks do not cover every GPU or suspend path.

## Migration and removal

For an old `cosmic-dropdown-terminal` installation, remove its panel entry and shortcut. Finish its jobs, then run `cosmic-dropdown-terminal quit`. Before starting the new backend, run `just migrate`. This copies old preferences to the new namespace, retains the source and refuses existing destinations or unsafe files. Running sessions cannot be migrated. Replace the panel entry and explicitly configure the new shortcut. There are no legacy aliases. Remove old executable and desktop registrations with the old version's uninstaller, if available.

To remove this version, remove its panel entry and shortcut, finish jobs, quit the backend, then run `just uninstall`. Saved preferences remain.

## License

Independent project; not an official System76 application. GPL-3.0-only; see `LICENSE` and `terminal/LICENSE`. Upstream COSMIC Terminal source, translations, color schemes and notices remain under `terminal/`. Native dependency guidance: [libcosmic](https://github.com/pop-os/libcosmic#dependencies).
