# Cosmos Dropdown Terminal

A COSMIC panel applet with a persistent dropdown terminal. The included backend is adapted from [COSMIC Terminal](https://github.com/pop-os/cosmic-term). You do not need an external `cosmic-term` executable.

Click the panel icon to show or hide the terminal. It uses the full output width and 40% of its logical height. It respects reserved panel space and does not move other windows. Hiding preserves shells, jobs, tabs, panes and scrollback. Closing the window hides it. Exiting the last shell ends that shell; the next show creates another.

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

## Enable and use

In **COSMIC Settings → Desktop → Panel → Applets**, add **Cosmos Dropdown Terminal**. Reopen Settings or log in again if necessary. Close Settings, then explicitly enable **Super+§**:

```sh
just configure-shortcut
```

Only one dropdown implementation can own this shortcut. Registration preserves other actions, saves a `custom.before-dropdown` backup and refuses conflicts. `§` means the XKB `section` symbol, not a fixed physical key. For another layout, set the shortcut manually to `/home/YOUR_USER/.local/bin/cosmos-applet-dropdown-terminal-backend toggle`.

```sh
~/.local/bin/cosmos-applet-dropdown-terminal-backend toggle
~/.local/bin/cosmos-applet-dropdown-terminal-backend status
# WARNING: quit terminates shells and running jobs.
~/.local/bin/cosmos-applet-dropdown-terminal-backend quit
```

Removing the applet or installing an upgrade does not stop the backend. Finish jobs and use `quit` before starting an upgraded backend. Logout also ends sessions. Preferences use `~/.config/cosmic/dev.jp.CosmosDropdownTerminal`, separate from stock COSMIC Terminal. Logs use `$XDG_RUNTIME_DIR/cosmos-applet-dropdown-terminal-backend/backend.log`; the control directory and socket are private to your user.

The backend selects the first usable compositor output, not the pointer's output. Autohiding panels can overlap it. There is no animation or session restoration after logout. Existing evidence includes repeated hide/show with preserved shell state and Super+§ in an isolated COSMIC session, not every GPU or suspend path.

## Migration and removal

For an old `cosmic-dropdown-terminal` installation, remove its panel entry and shortcut. Finish its jobs, then run `cosmic-dropdown-terminal quit`. Before starting the new backend, run `just migrate`. This copies old preferences to the new namespace, retains the source and refuses existing destinations or unsafe files. Running sessions cannot be migrated. Replace the panel entry and explicitly configure the new shortcut. There are no legacy aliases. Remove old executable and desktop registrations with the old version's uninstaller, if available.

To remove this version, remove its panel entry and shortcut, finish jobs, quit the backend, then run `just uninstall`. Saved preferences remain.

## License

Independent project; not an official System76 application. GPL-3.0-only; see `LICENSE` and `terminal/LICENSE`. Upstream COSMIC Terminal source, translations, color schemes and notices remain under `terminal/`. Native dependency guidance: [libcosmic](https://github.com/pop-os/libcosmic#dependencies).
