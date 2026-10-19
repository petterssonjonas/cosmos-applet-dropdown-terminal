import 'recipes/common.just'

name := 'cosmos-applet-dropdown-terminal'
appid := 'dev.jp.CosmosAppletDropdownTerminal'
backend := name + '-backend'
shortcut := name + '-shortcut'
profile := 'release'
prefix := env('HOME') / '.local'
rootdir := ''
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
base-dir := absolute_path(clean(rootdir / prefix))

[default]
default: build-release

build-debug:
    cargo build --locked --target-dir {{quote(cargo-target-dir)}}
    cargo build --locked --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}
    cargo build --locked --manifest-path shortcut/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}

build-release:
    cargo build --release --locked --target-dir {{quote(cargo-target-dir)}}
    cargo build --release --locked --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}
    cargo build --release --locked --manifest-path shortcut/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}

check:
    cargo clippy --all-targets --locked --target-dir {{quote(cargo-target-dir)}} -- -D warnings
    cargo clippy --all-targets --locked --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}} -- -D warnings
    cargo clippy --all-targets --locked --manifest-path shortcut/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}} -- -D warnings

test:
    cargo test --locked --target-dir {{quote(cargo-target-dir)}}
    cargo test --locked --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}
    cargo test --locked --manifest-path shortcut/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}}
    python3 -m unittest discover -s tests -v

# Run backend controls; the applet itself must be hosted by the COSMIC panel.
[positional-arguments]
run *args:
    if [[ {{quote(profile)}} == debug ]]; then cargo run --locked --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}} -- "$@"; else cargo run --locked --profile {{quote(profile)}} --manifest-path terminal/Cargo.toml --target-dir {{quote(absolute_path(cargo-target-dir))}} -- "$@"; fi

# Build separately. Install files only, without changing shortcuts or panel layout.
install:
    @just --justfile {{quote(justfile())}} _install-file {{quote(cargo-target-dir / profile / name)}} {{quote(base-dir / 'bin' / name)}} 0755
    @just --justfile {{quote(justfile())}} _install-file {{quote(cargo-target-dir / profile / backend)}} {{quote(base-dir / 'bin' / backend)}} 0755
    @just --justfile {{quote(justfile())}} _install-file {{quote(cargo-target-dir / profile / shortcut)}} {{quote(base-dir / 'bin' / shortcut)}} 0755
    @just --justfile {{quote(justfile())}} _install-desktop resources/app.desktop {{quote(base-dir / 'share/applications' / appid + '.desktop')}} {{quote(absolute_path(prefix / 'bin' / name))}}

# Saved preferences, running sessions and user shortcuts are retained.
uninstall:
    rm -f -- {{quote(base-dir / 'bin' / name)}} {{quote(base-dir / 'bin' / backend)}} {{quote(base-dir / 'bin' / shortcut)}} {{quote(base-dir / 'share/applications' / appid + '.desktop')}}

# Explicit opt-in. Close COSMIC Settings first; conflicting bindings are refused.
configure-shortcut:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -z {{quote(rootdir)}} && $EUID != 0 ]] || { printf '%s\n' 'Configure shortcuts as the desktop user, without staging.' >&2; exit 1; }
    {{quote(absolute_path(prefix / 'bin' / shortcut))}} {{quote(quote(absolute_path(prefix / 'bin' / backend)) + ' toggle')}}

# End the old backend yourself before migrating; quit terminates its running jobs.
migrate:
    #!/usr/bin/env bash
    set -euo pipefail
    config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
    just --justfile {{quote(justfile())}} _migrate-directory "$config_home/cosmic/io.github.cosmic_dropdown.Terminal" "$config_home/cosmic/dev.jp.CosmosDropdownTerminal"
    printf '%s\n' 'Old sessions and shortcuts are not migrated. Replace the panel entry and old binding in COSMIC Settings, then run just configure-shortcut.'
