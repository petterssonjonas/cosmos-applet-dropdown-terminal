// SPDX-License-Identifier: GPL-3.0-only
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const CONFIG: &str = "cosmic/com.system76.CosmicSettings.Shortcuts/v1";
const COMMAND: &str = "'/tmp/dropdown terminal' toggle";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "cosmos-shortcut-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn directory(&self) -> PathBuf {
        self.0.join("config").join(CONFIG)
    }

    fn custom(&self, text: &str) {
        fs::create_dir_all(self.directory()).unwrap();
        fs::write(self.directory().join("custom"), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!(
            "CARGO_BIN_EXE_cosmos-applet-dropdown-terminal-shortcut"
        ))
        .args(args)
        .env("HOME", &self.0)
        .env("XDG_CONFIG_HOME", self.0.join("config"))
        .env("XDG_DATA_HOME", self.0.join("data"))
        .env("XDG_DATA_DIRS", self.0.join("system-data"))
        .output()
        .unwrap()
    }

    fn success(&self, args: &[&str], label: &str) {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{label}\n")
        );
        assert!(output.stderr.is_empty());
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn getter_never_creates_directories_or_files() {
    let sandbox = Sandbox::new();
    sandbox.success(&["--get", COMMAND], "Unassigned");
    assert!(!sandbox.0.join("config").exists());
    let original = format!("{{(modifiers: [Super], key: \"§\"): Spawn({COMMAND:?})}}");
    sandbox.custom(&original);
    sandbox.success(&["--get", COMMAND], "Super+section");
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(sandbox.directory()).unwrap().count(), 1);
}

#[test]
fn rebinds_exact_command_preserves_unrelated_entries_and_is_idempotent() {
    let sandbox = Sandbox::new();
    let unrelated =
        "(modifiers: [Ctrl], key: \"a\", future: Some(2)): FutureAction(nested: [3, 4])";
    let original = format!("{{{unrelated}}}");
    sandbox.custom(&original);
    sandbox.success(&[COMMAND], "Super+section");
    sandbox.success(&["--get", COMMAND], "Super+section");
    sandbox.success(&["--set", "Alt,Ctrl", "F12", COMMAND], "Ctrl+Alt+F12");
    sandbox.success(&["--get", COMMAND], "Ctrl+Alt+F12");
    let path = sandbox.directory().join("custom");
    let changed = fs::read_to_string(&path).unwrap();
    #[derive(Debug, serde::Deserialize, PartialEq, Eq, Hash)]
    enum Modifier {
        Super,
        Ctrl,
        Alt,
        Shift,
    }
    #[derive(Debug, serde::Deserialize, PartialEq, Eq, Hash)]
    struct Binding {
        modifiers: Vec<Modifier>,
        key: String,
        #[serde(default)]
        future: Option<u8>,
    }
    #[derive(Debug, serde::Deserialize, PartialEq, Eq)]
    enum Action {
        Spawn(String),
        FutureAction { nested: Vec<u8> },
    }
    let before: std::collections::HashMap<Binding, Action> = ron::from_str(&original).unwrap();
    let after: std::collections::HashMap<Binding, Action> = ron::from_str(&changed).unwrap();
    let (binding, action) = before.iter().next().unwrap();
    assert_eq!(after.get(binding), Some(action));
    assert_eq!(after.len(), 2);
    assert!(!after.keys().any(|binding| binding.key == "section"));
    sandbox.success(&["--set", "Ctrl,Alt", "F12", COMMAND], "Ctrl+Alt+F12");
    sandbox.success(&[COMMAND], "Ctrl+Alt+F12");
    assert_eq!(fs::read_to_string(path).unwrap(), changed);
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom.before-dropdown")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(sandbox.directory()).unwrap().count(), 3);
}

#[test]
fn conflicts_and_invalid_keys_leave_custom_and_backup_unchanged() {
    let sandbox = Sandbox::new();
    let original = format!(
        "{{
        (modifiers: [Super], key: \"section\"): Spawn({COMMAND:?}),
        (modifiers: [Alt, Ctrl], key: \"F12\"): FutureAction(nested: [1]),
    }}"
    );
    sandbox.custom(&original);
    for args in [
        ["--set", "Ctrl,Alt", "F12", COMMAND],
        ["--set", "Super", "NotAnXkbKeysym", COMMAND],
    ] {
        let output = sandbox.run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
            original
        );
        assert!(!sandbox.directory().join("custom.before-dropdown").exists());
    }
}

#[test]
fn respects_effective_defaults_without_overwriting_custom() {
    let sandbox = Sandbox::new();
    sandbox.custom("{}");
    let defaults = sandbox.0.join("system-data").join(CONFIG);
    fs::create_dir_all(&defaults).unwrap();
    fs::write(
        defaults.join("defaults"),
        "{(modifiers: [Alt], key: \"F12\"): Close}",
    )
    .unwrap();
    let output = sandbox.run(&["--set", "Alt", "F12", COMMAND]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Default shortcut conflict"));
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
        "{}"
    );
    assert!(!sandbox.directory().join("custom.before-dropdown").exists());
}

#[test]
fn rejects_physical_key_ambiguity_without_replacing_shortcuts() {
    let sandbox = Sandbox::new();
    let original = "{(modifiers: [Super], keycode: Some(49)): Close}";
    sandbox.custom(original);
    let output = sandbox.run(&[COMMAND]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("physical-key"));
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
        original
    );
    assert!(!sandbox.directory().join("custom.before-dropdown").exists());
}
