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

    fn defaults(&self, text: &str) {
        let directory = self.0.join("system-data").join(CONFIG);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("defaults"), text).unwrap();
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

#[test]
fn escape_spellings_save_the_canonical_chord() {
    for key in ["escape", "esc", "ESC", "Escape"] {
        let sandbox = Sandbox::new();
        sandbox.success(&["--set", "Super", key, COMMAND], "Super+Escape");
        sandbox.success(&["--get", COMMAND], "Super+Escape");
        let saved = fs::read_to_string(sandbox.directory().join("custom")).unwrap();
        assert!(saved.contains("key: \"Escape\""), "{saved}");
        sandbox.success(&["--set", "Super", "eScApE", COMMAND], "Super+Escape");
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
            saved
        );
    }
}

#[test]
fn equivalent_escape_conflicts_preserve_all_existing_files() {
    for existing in ["Escape", "escape", "ESC"] {
        let sandbox = Sandbox::new();
        let original = format!(
            "{{(modifiers: [Super], key: {existing:?}, future: Some(7)): Close,
            (modifiers: [Alt], key: \"F12\"): Spawn({COMMAND:?})}}"
        );
        sandbox.custom(&original);
        let backup = sandbox.directory().join("custom.before-dropdown");
        fs::write(&backup, "existing backup").unwrap();
        for requested in ["escape", "esc", "ESC", "Escape"] {
            let output = sandbox.run(&["--set", "Super", requested, COMMAND]);
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("already has an action"));
            assert_eq!(
                fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
                original
            );
            assert_eq!(fs::read_to_string(&backup).unwrap(), "existing backup");
        }
    }
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq, Hash)]
enum SavedModifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq, Hash)]
struct SavedBinding {
    modifiers: Vec<SavedModifier>,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: Option<u32>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    future: Option<u8>,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
enum SavedAction {
    Spawn(String),
    Disable,
    None,
    FutureAction { nested: Vec<u8> },
}

fn saved_shortcuts(sandbox: &Sandbox) -> std::collections::HashMap<SavedBinding, SavedAction> {
    ron::from_str(&fs::read_to_string(sandbox.directory().join("custom")).unwrap()).unwrap()
}

#[test]
fn removed_defaults_can_be_reassigned_without_duplicates_or_lost_fields() {
    for removed in ["Disable", "None"] {
        let sandbox = Sandbox::new();
        sandbox.defaults("{(modifiers: [Super], key: \"Escape\"): System(LockScreen)}");
        let original = format!(
            "{{
                (modifiers: [Super], key: \"Escape\", description: Some(\"Removed\"), future: Some(7)): {removed},
                (modifiers: [Ctrl], key: \"a\", future: Some(2)): FutureAction(nested: [3, 4]),
            }}"
        );
        sandbox.custom(&original);
        let before = saved_shortcuts(&sandbox);
        sandbox.success(&["--get", COMMAND], "Unassigned");
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
            original
        );
        sandbox.success(&["--set", "Super", "esc", COMMAND], "Super+Escape");
        sandbox.success(&["--get", COMMAND], "Super+Escape");
        let after = saved_shortcuts(&sandbox);
        for (binding, action) in before {
            let expected = if binding.key == "Escape" {
                SavedAction::Spawn(COMMAND.into())
            } else {
                action
            };
            assert_eq!(after.get(&binding), Some(&expected));
        }
        assert_eq!(after.len(), 2);
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom.before-dropdown")).unwrap(),
            original
        );
        let saved = fs::read_to_string(sandbox.directory().join("custom")).unwrap();
        sandbox.success(&["--set", "Super", "Escape", COMMAND], "Super+Escape");
        sandbox.success(&[COMMAND], "Super+Escape");
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
            saved
        );
    }
}

#[test]
fn owned_override_shadows_defaults_and_keeps_them_disabled_when_moved() {
    let sandbox = Sandbox::new();
    sandbox.defaults(
        "{(modifiers: [Ctrl, Super], key: \"Escape\", description: Some(\"Default\")): System(LockScreen)}",
    );
    let original = format!(
        "{{(modifiers: [Super, Ctrl], key: \"Escape\", description: Some(\"Mine\"), future: Some(9)): Spawn({COMMAND:?})}}"
    );
    sandbox.custom(&original);
    let backup = sandbox.directory().join("custom.before-dropdown");
    fs::write(&backup, "existing backup").unwrap();
    sandbox.success(&["--get", COMMAND], "Super+Ctrl+Escape");
    sandbox.success(
        &["--set", "Ctrl,Super", "esc", COMMAND],
        "Super+Ctrl+Escape",
    );
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
        original
    );
    sandbox.success(&["--set", "Alt", "F12", COMMAND], "Alt+F12");
    sandbox.success(&["--get", COMMAND], "Alt+F12");
    let moved = saved_shortcuts(&sandbox);
    let (old, old_action) = moved
        .iter()
        .find(|(binding, _)| binding.key == "Escape")
        .unwrap();
    assert_eq!(old_action, &SavedAction::Disable);
    assert_eq!(old.description.as_deref(), Some("Mine"));
    assert_eq!(old.future, Some(9));
    let (new, new_action) = moved
        .iter()
        .find(|(binding, _)| binding.key == "F12")
        .unwrap();
    assert_eq!(new_action, &SavedAction::Spawn(COMMAND.into()));
    assert_eq!(new.description.as_deref(), Some("Mine"));
    assert_eq!(new.future, Some(9));
    sandbox.success(
        &["--set", "Super,Ctrl", "Escape", COMMAND],
        "Super+Ctrl+Escape",
    );
    sandbox.success(&["--get", COMMAND], "Super+Ctrl+Escape");
    let restored = saved_shortcuts(&sandbox);
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored.values().next(),
        Some(&SavedAction::Spawn(COMMAND.into()))
    );
    assert_eq!(fs::read_to_string(backup).unwrap(), "existing backup");
}

#[test]
fn moving_to_a_removed_default_reuses_its_slot_instead_of_creating_another_binding() {
    let sandbox = Sandbox::new();
    sandbox.defaults("{(modifiers: [Super], key: \"Escape\"): System(LockScreen)}");
    let original = format!(
        "{{(modifiers: [Super], key: \"Escape\"): Disable,
        (modifiers: [Alt], key: \"F12\", future: Some(8)): Spawn({COMMAND:?})}}"
    );
    sandbox.custom(&original);
    sandbox.success(&["--set", "Super", "Escape", COMMAND], "Super+Escape");
    sandbox.success(&["--get", COMMAND], "Super+Escape");
    let saved = saved_shortcuts(&sandbox);
    assert_eq!(saved.len(), 1);
    let (binding, action) = saved.iter().next().unwrap();
    assert_eq!(binding.modifiers, vec![SavedModifier::Super]);
    assert_eq!(binding.key, "Escape");
    assert_eq!(binding.future, Some(8));
    assert_eq!(action, &SavedAction::Spawn(COMMAND.into()));
}

#[test]
fn active_custom_overrides_are_not_mistaken_for_removed_defaults() {
    for action in ["Close", "System(LockScreen)", "FutureAction(nested: [1])"] {
        let sandbox = Sandbox::new();
        sandbox.defaults("{(modifiers: [Super], key: \"Escape\"): Disable}");
        let original = format!(
            "{{(modifiers: [Super], key: \"Escape\"): {action},
            (modifiers: [Alt], key: \"F12\"): Spawn({COMMAND:?})}}"
        );
        sandbox.custom(&original);
        let backup = sandbox.directory().join("custom.before-dropdown");
        fs::write(&backup, "existing backup").unwrap();
        let output = sandbox.run(&["--set", "Super", "Escape", COMMAND]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("already has an action"));
        assert_eq!(
            fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
            original
        );
        assert_eq!(fs::read_to_string(backup).unwrap(), "existing backup");
        sandbox.success(&["--get", COMMAND], "Alt+F12");
    }
}

#[test]
fn removing_a_different_chord_does_not_disable_the_default_action_everywhere() {
    let sandbox = Sandbox::new();
    sandbox.defaults("{(modifiers: [Super], key: \"Escape\"): System(LockScreen)}");
    let original = "{(modifiers: [Alt], key: \"Escape\"): Disable}";
    sandbox.custom(original);
    let output = sandbox.run(&["--set", "Super", "Escape", COMMAND]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Default shortcut conflict"));
    assert_eq!(
        fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
        original
    );
    assert!(!sandbox.directory().join("custom.before-dropdown").exists());
}

#[test]
fn disabled_physical_defaults_are_shadowed_without_blocking_unrelated_keys() {
    let sandbox = Sandbox::new();
    sandbox.defaults("{(modifiers: [Super], keycode: Some(49)): Close}");
    sandbox.custom("{(modifiers: [Super], keycode: Some(49)): Disable}");
    sandbox.success(&["--set", "Super", "Escape", COMMAND], "Super+Escape");
    sandbox.success(&["--get", COMMAND], "Super+Escape");
    let saved = saved_shortcuts(&sandbox);
    let (physical, action) = saved
        .iter()
        .find(|(binding, _)| binding.keycode.is_some())
        .unwrap();
    assert_eq!(physical.keycode, Some(49));
    assert_eq!(action, &SavedAction::Disable);
}

#[test]
fn getters_and_idempotent_setters_reject_remaining_physical_and_duplicate_conflicts() {
    for conflict in [
        "(modifiers: [Super], keycode: Some(49)): Close",
        "(modifiers: [Super], key: \"Escape\"): Close",
        "(modifiers: [Super], key: \"Escape\"): Disable",
    ] {
        let sandbox = Sandbox::new();
        sandbox.defaults("{(modifiers: [Super], key: \"Escape\"): System(LockScreen)}");
        let original =
            format!("{{(modifiers: [Super], key: \"Escape\"): Spawn({COMMAND:?}), {conflict}}}");
        sandbox.custom(&original);
        let backup = sandbox.directory().join("custom.before-dropdown");
        fs::write(&backup, "existing backup").unwrap();
        for args in [
            vec!["--get", COMMAND],
            vec!["--set", "Super", "Escape", COMMAND],
        ] {
            let output = sandbox.run(&args);
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
            assert_eq!(
                fs::read_to_string(sandbox.directory().join("custom")).unwrap(),
                original
            );
            assert_eq!(fs::read_to_string(&backup).unwrap(), "existing backup");
        }
    }
}

#[test]
fn explicitly_inactive_defaults_allow_registration_and_readback() {
    for action in ["Disable", "None"] {
        let sandbox = Sandbox::new();
        sandbox.defaults(&format!(
            "{{(modifiers: [Super], key: \"Escape\"): {action}}}"
        ));
        sandbox.success(&["--get", COMMAND], "Unassigned");
        sandbox.success(&["--set", "Super", "Escape", COMMAND], "Super+Escape");
        sandbox.success(&["--get", COMMAND], "Super+Escape");
        let saved = saved_shortcuts(&sandbox);
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved.values().next(),
            Some(&SavedAction::Spawn(COMMAND.into()))
        );
    }
}
