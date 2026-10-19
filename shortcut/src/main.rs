// SPDX-License-Identifier: GPL-3.0-only
use ron::value::RawValue;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const CONFIG: &str = "cosmic/com.system76.CosmicSettings.Shortcuts/v1";
const BINDING: &str =
    "(modifiers: [Super], key: \"section\", description: Some(\"Cosmos Dropdown Terminal\"))";

// Preserve unknown actions and binding fields verbatim; never deserialize them
// into a reduced enum that could silently erase another application's shortcuts.
struct Entries(Vec<(Box<RawValue>, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;
        impl<'de> Visitor<'de> for MapVisitor {
            type Value = Entries;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a COSMIC shortcut map")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Entries, M::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_map(MapVisitor)
    }
}

#[derive(Deserialize, PartialEq)]
enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}
#[derive(Deserialize)]
struct Binding {
    modifiers: Vec<Modifier>,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: Option<u32>,
}

fn matching_action(text: &str) -> Result<Option<String>, Box<dyn Error>> {
    let entries: Entries = ron::from_str(text)?;
    let mut found = None;
    for (key, action) in entries.0 {
        let binding: Binding = key.into_rust()?;
        if binding.modifiers == [Modifier::Super] {
            if binding.keycode.is_some() {
                return Err("A Super+physical-key shortcut exists. Assign Super+§ in COSMIC Settings to check layout conflicts safely.".into());
            }
            if binding.key.eq_ignore_ascii_case("section") || binding.key == "§" {
                if found.is_some() {
                    return Err(
                        "Duplicate Super+§ bindings; resolve in COSMIC Settings first".into(),
                    );
                }
                found = Some(action.get_ron().to_owned());
            }
        }
    }
    Ok(found)
}

fn merge(text: &str, command: &str) -> Result<String, Box<dyn Error>> {
    let action = format!("Spawn({})", ron::to_string(command)?);
    if let Some(existing) = matching_action(text)? {
        #[derive(Deserialize)]
        enum SpawnAction {
            Spawn(String),
        }
        if matches!(&ron::from_str::<SpawnAction>(&existing), Ok(SpawnAction::Spawn(current)) if current == command)
        {
            return Ok(text.to_owned());
        }
        return Err("Super+§ already has an action; no shortcuts were changed".into());
    }
    let entries: Entries = ron::from_str(text)?;
    let mut result = String::from("{\n");
    for (key, value) in entries.0 {
        result.push_str(&format!("    {}: {},\n", key.get_ron(), value.get_ron()));
    }
    result.push_str(&format!("    {BINDING}: {action},\n}}\n"));
    Ok(result)
}

fn config_home() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(path.into());
    }
    Ok(PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?).join(".config"))
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 1 || args[0] == "--help" {
        println!(
            "Usage: cosmos-applet-dropdown-terminal-shortcut 'ABSOLUTE-COMMAND toggle'\nRegisters Super+§ without replacing existing bindings. Close COSMIC Settings before running.\nUses XDG_CONFIG_HOME; keeps custom.before-dropdown backup. Conflicts are never overwritten."
        );
        if args.as_slice() == ["--help"] {
            return Ok(());
        }
        return Err("Exactly one command argument is required".into());
    }
    let directory = config_home()?.join(CONFIG);
    fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("dropdown.lock"))?;
    lock.lock()?;
    let path = directory.join("custom");
    let original = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(error) => return Err(error.into()),
    };
    let updated = merge(&original, &args[0])?;
    if updated == original {
        println!("Super+§ is already configured");
        return Ok(());
    }
    // Check effective defaults as well, so a system binding is not overridden.
    let user_defaults = directory.join("defaults");
    let mut defaults = vec![user_defaults];
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        defaults.push(PathBuf::from(home).join(CONFIG).join("defaults"));
    } else if let Some(home) = std::env::var_os("HOME") {
        defaults.push(
            PathBuf::from(home)
                .join(".local/share")
                .join(CONFIG)
                .join("defaults"),
        );
    }
    for base in std::env::split_paths(
        &std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
    ) {
        defaults.push(base.join(CONFIG).join("defaults"));
    }
    for default in defaults {
        match fs::read_to_string(default) {
            Ok(text) => {
                if matching_action(&text)?.is_some() {
                    return Err(
                        "Super+§ conflicts with a default shortcut; use COSMIC Settings".into(),
                    );
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let backup = directory.join("custom.before-dropdown");
    match OpenOptions::new().write(true).create_new(true).open(backup) {
        Ok(mut file) => {
            file.write_all(original.as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let temporary = directory.join(format!("custom.dropdown-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(updated.as_bytes())?;
    file.sync_all()?;
    // Settings does not participate in our lock; reject changes observed before commit.
    let current = fs::read_to_string(&path).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok("{}".into())
        } else {
            Err(error)
        }
    })?;
    if current != original {
        fs::remove_file(temporary)?;
        return Err("Shortcuts changed concurrently; retry with COSMIC Settings closed".into());
    }
    fs::rename(temporary, path)?;
    println!("Registered Super+§: {}", args[0]);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unknown_actions_and_is_idempotent() {
        let original = "{(modifiers: [Ctrl], key: \"a\"): FutureAction(nested: [1, 2])}";
        let merged = merge(original, "/tmp/terminal toggle").unwrap();
        let entries: Entries = ron::from_str(&merged).unwrap();
        #[derive(Deserialize)]
        enum Future {
            FutureAction { nested: Vec<u32> },
        }
        let Future::FutureAction { nested } = entries.0[0].1.into_rust::<Future>().unwrap();
        assert_eq!(nested, vec![1, 2]);
        assert_eq!(merge(&merged, "/tmp/terminal toggle").unwrap(), merged);
    }
    #[test]
    fn rejects_conflicting_shortcuts_and_malformed_input() {
        assert!(
            merge(
                "{(modifiers: [Super], key: \"section\"): Close}",
                "terminal toggle"
            )
            .is_err()
        );
        assert!(merge("{broken", "terminal toggle").is_err());
        assert!(
            merge(
                "{(modifiers: [Super], keycode: Some(49)): Close}",
                "terminal toggle"
            )
            .is_err()
        );
    }
}
