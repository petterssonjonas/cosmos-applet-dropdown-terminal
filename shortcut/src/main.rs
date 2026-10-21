// SPDX-License-Identifier: GPL-3.0-only
use ron::value::RawValue;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use xkbcommon::xkb;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const CONFIG: &str = "cosmic/com.system76.CosmicSettings.Shortcuts/v1";
const HELP: &str = "Usage: cosmos-applet-dropdown-terminal-shortcut COMMAND
       cosmos-applet-dropdown-terminal-shortcut --get COMMAND
       cosmos-applet-dropdown-terminal-shortcut --set MODIFIERS KEY COMMAND

COMMAND must exactly match the registered Spawn command, including shell quoting.
Positional COMMAND registers Super+section only when unassigned.
MODIFIERS is a comma-separated list of Super,Ctrl,Alt,Shift; use '' for none.
KEY is an XKB keysym name or one Unicode character (including §).
Success prints the canonical chord; --get prints Unassigned when absent.
Close COSMIC Settings before changing shortcuts. Uses XDG_CONFIG_HOME and keeps
custom.before-dropdown. Conflicts, duplicates and physical-key ambiguity fail
without replacing shortcuts. --get never creates directories or writes files.";

// Keep unknown actions and binding fields as RON, not a reduced schema.
struct Entries(Vec<(Box<RawValue>, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct MapVisitor;
        impl<'de> Visitor<'de> for MapVisitor {
            type Value = Entries;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a COSMIC shortcut map")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Entries, M::Error> {
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

struct Fields(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct FieldVisitor;
        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = Fields;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a shortcut binding")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Fields, M::Error> {
                let mut fields = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    fields.push(entry);
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_struct("Binding", &[], FieldVisitor)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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
#[derive(Debug, PartialEq)]
struct Chord {
    modifiers: Vec<Modifier>,
    key: String,
}

fn normalize_modifiers(mut modifiers: Vec<Modifier>) -> Result<Vec<Modifier>> {
    modifiers.sort_unstable();
    if modifiers.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("Duplicate modifiers; no shortcuts were changed".into());
    }
    Ok(modifiers)
}

fn canonical_key(key: &str) -> Result<String> {
    // xkbcommon's Rust wrapper panics on embedded NULs.
    if key.is_empty() || key.contains('\0') || key.chars().any(char::is_control) {
        return Err("KEY must be an XKB keysym name or one printable Unicode character".into());
    }
    let mut characters = key.chars();
    let first = characters.next().ok_or("KEY is empty")?;
    let symbol = if characters.next().is_none() {
        xkb::utf32_to_keysym(u32::from(first))
    } else {
        xkb::keysym_from_name(key, xkb::KEYSYM_NO_FLAGS)
    };
    let name = xkb::keysym_get_name(symbol);
    if symbol.raw() == 0 || name.is_empty() || name == "VoidSymbol" || name.starts_with("0x") {
        return Err(format!("Invalid XKB keysym: {key}").into());
    }
    Ok(name)
}

impl Chord {
    fn parse(modifiers: &str, key: &str) -> Result<Self> {
        let modifiers = if modifiers.is_empty() {
            Vec::new()
        } else {
            modifiers
                .split(',')
                .map(|name| match name {
                    "Super" => Ok(Modifier::Super),
                    "Ctrl" => Ok(Modifier::Ctrl),
                    "Alt" => Ok(Modifier::Alt),
                    "Shift" => Ok(Modifier::Shift),
                    _ => Err(format!("Unknown modifier: {name}").into()),
                })
                .collect::<Result<Vec<_>>>()?
        };
        Ok(Self {
            modifiers: normalize_modifiers(modifiers)?,
            key: canonical_key(key)?,
        })
    }

    fn from_binding(binding: Binding) -> Result<Self> {
        if binding.keycode.is_some() {
            return Err(
                "Physical-key binding is ambiguous; resolve it in COSMIC Settings first".into(),
            );
        }
        Ok(Self {
            modifiers: normalize_modifiers(binding.modifiers)?,
            key: canonical_key(&binding.key)?,
        })
    }

    fn label(&self) -> String {
        let mut label = String::new();
        for modifier in &self.modifiers {
            label.push_str(match modifier {
                Modifier::Super => "Super+",
                Modifier::Ctrl => "Ctrl+",
                Modifier::Alt => "Alt+",
                Modifier::Shift => "Shift+",
            });
        }
        label.push_str(&self.key);
        label
    }

    fn binding(&self, original: Option<&RawValue>) -> Result<String> {
        let modifiers = ron::to_string(&self.modifiers)?;
        let key = ron::to_string(&self.key)?;
        let mut result = format!("(modifiers: {modifiers}, key: {key}");
        if let Some(original) = original {
            for (name, value) in original.into_rust::<Fields>()?.0 {
                if name != "modifiers" && name != "key" {
                    result.push_str(&format!(", {name}: {}", value.get_ron().trim_start()));
                }
            }
        } else {
            result.push_str(", description: Some(\"Cosmos Dropdown Terminal\")");
        }
        result.push(')');
        Ok(result)
    }
}

fn owns(action: &RawValue, command: &str) -> bool {
    #[derive(Deserialize)]
    enum SpawnAction {
        Spawn(String),
    }
    matches!(action.into_rust::<SpawnAction>(), Ok(SpawnAction::Spawn(current)) if current == command)
}

fn owned_binding(entries: &Entries, command: &str) -> Result<Option<(usize, Chord)>> {
    let mut found = None;
    for (index, (key, action)) in entries.0.iter().enumerate() {
        if owns(action, command) {
            if found.is_some() {
                return Err("Multiple bindings own this command; resolve duplicates in COSMIC Settings first".into());
            }
            found = Some((index, Chord::from_binding(key.into_rust()?)?));
        }
    }
    Ok(found)
}

fn check_conflicts(entries: &Entries, target: &Chord, owner: Option<usize>) -> Result<()> {
    for (index, (key, _)) in entries.0.iter().enumerate() {
        let binding: Binding = key.into_rust()?;
        if normalize_modifiers(binding.modifiers)? != target.modifiers {
            continue;
        }
        // A physical shortcut with the same modifiers might map to this keysym
        // in the active layout. Do not guess from a hard-coded keycode table.
        if binding.keycode.is_some() {
            return Err(format!(
                "{} may conflict with a physical-key shortcut; use COSMIC Settings",
                target.label()
            )
            .into());
        }
        // COSMIC also supports modifier-only actions, such as Super for Launcher.
        // Those do not collide with a chord that includes a non-modifier key.
        if binding.key.is_empty() {
            continue;
        }
        if canonical_key(&binding.key)? == target.key && owner != Some(index) {
            return Err(format!(
                "{} already has an action; no shortcuts were changed",
                target.label()
            )
            .into());
        }
    }
    Ok(())
}

fn merge(text: &str, defaults: &str, command: &str, target: &Chord) -> Result<String> {
    let entries: Entries = ron::from_str(text)?;
    let owner = owned_binding(&entries, command)?;
    check_conflicts(&entries, target, owner.as_ref().map(|(index, _)| *index))?;
    check_conflicts(&ron::from_str(defaults)?, target, None)
        .map_err(|error| format!("Default shortcut conflict: {error}"))?;
    if owner.as_ref().is_some_and(|(_, current)| current == target) {
        return Ok(text.to_owned());
    }
    let owned_index = owner.map(|(index, _)| index);
    let mut result = String::from("{\n");
    for (index, (key, action)) in entries.0.iter().enumerate() {
        if owned_index == Some(index) {
            result.push_str(&format!(
                "    {}: {},\n",
                target.binding(Some(key))?,
                action.get_ron().trim_start()
            ));
        } else {
            result.push_str(&format!(
                "    {}: {},\n",
                key.get_ron().trim_start(),
                action.get_ron().trim_start()
            ));
        }
    }
    if owned_index.is_none() {
        result.push_str(&format!(
            "    {}: Spawn({}),\n",
            target.binding(None)?,
            ron::to_string(command)?
        ));
    }
    result.push_str("}\n");
    Ok(result)
}

fn config_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(path.into());
    }
    Ok(PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?).join(".config"))
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn defaults(directory: &Path) -> Result<String> {
    let mut paths = vec![directory.join("defaults")];
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(PathBuf::from(home).join(CONFIG).join("defaults"));
    } else if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".local/share")
                .join(CONFIG)
                .join("defaults"),
        );
    }
    for base in std::env::split_paths(
        &std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
    ) {
        paths.push(base.join(CONFIG).join("defaults"));
    }
    for path in paths {
        if let Some(text) = read_optional(&path)? {
            return Ok(text);
        }
    }
    Ok("{}".into())
}

fn commit(directory: &Path, original: &str, updated: &str) -> Result<()> {
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
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        file.write_all(updated.as_bytes())?;
        file.sync_all()?;
        // Settings does not participate in our lock. Recheck before replacing.
        let current = read_optional(&directory.join("custom"))?.unwrap_or_else(|| "{}".into());
        if current != original {
            return Err("Shortcuts changed concurrently; retry with COSMIC Settings closed".into());
        }
        fs::rename(&temporary, directory.join("custom"))?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--help"] {
        println!("{HELP}");
        return Ok(());
    }
    let (command, get, requested) = match args.as_slice() {
        [command] if !command.starts_with('-') => (command, false, None),
        [mode, command] if mode == "--get" => (command, true, None),
        [mode, modifiers, key, command] if mode == "--set" => {
            (command, false, Some(Chord::parse(modifiers, key)?))
        }
        _ => return Err(HELP.into()),
    };
    if command.trim().is_empty() {
        return Err("COMMAND must not be empty".into());
    }
    let directory = config_home()?.join(CONFIG);
    if get {
        let text = read_optional(&directory.join("custom"))?.unwrap_or_else(|| "{}".into());
        let entries = ron::from_str(&text)?;
        if let Some((index, chord)) = owned_binding(&entries, command)? {
            check_conflicts(&entries, &chord, Some(index))?;
            check_conflicts(&ron::from_str(&defaults(&directory)?)?, &chord, None)?;
            println!("{}", chord.label());
        } else {
            println!("Unassigned");
        }
        return Ok(());
    }
    fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("dropdown.lock"))?;
    lock.lock()?;
    let original = read_optional(&directory.join("custom"))?.unwrap_or_else(|| "{}".into());
    let target = match requested {
        Some(chord) => chord,
        None => match owned_binding(&ron::from_str(&original)?, command)? {
            Some((_, chord)) => chord,
            None => Chord::parse("Super", "section")?,
        },
    };
    let updated = merge(&original, &defaults(&directory)?, command, &target)?;
    if updated != original {
        commit(&directory, &original, &updated)?;
    }
    println!("{}", target.label());
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
    const COMMAND: &str = "'/tmp/terminal app' toggle";

    #[test]
    fn canonicalizes_unicode_and_rejects_invalid_key_names() {
        assert_eq!(
            Chord::parse("Super", "§").unwrap(),
            Chord::parse("Super", "section").unwrap()
        );
        for key in [
            "",
            "DefinitelyNotAnXkbKey",
            "NoSymbol",
            "VoidSymbol",
            "a\0b",
            "\n",
        ] {
            assert!(Chord::parse("Super", key).is_err(), "accepted {key:?}");
        }
        assert!(Chord::parse("Super,Super", "F12").is_err());
        assert!(Chord::parse("Meta", "F12").is_err());
    }

    #[test]
    fn changes_only_owned_binding_and_preserves_unknown_fields_and_actions() {
        let original = format!("{{
            (modifiers: [Super], key: \"section\", future: Future(nested: [1, 2]), description: Some(\"Mine\")): Spawn({}),
            (modifiers: [Ctrl], key: \"a\", extra: Some(7)): FutureAction(nested: [3, 4]),
        }}", ron::to_string(COMMAND).unwrap());
        let target = Chord::parse("Alt,Ctrl", "F12").unwrap();
        let merged = merge(&original, "{}", COMMAND, &target).unwrap();
        assert_eq!(
            owned_binding(&ron::from_str(&merged).unwrap(), COMMAND)
                .unwrap()
                .unwrap()
                .1
                .label(),
            "Ctrl+Alt+F12"
        );
        #[derive(Debug, Deserialize, PartialEq)]
        enum FutureValue {
            Future { nested: Vec<u32> },
        }
        let before: Entries = ron::from_str(&original).unwrap();
        let after: Entries = ron::from_str(&merged).unwrap();
        let fields = after.0[0].0.into_rust::<Fields>().unwrap().0;
        let field = |name: &str| &fields.iter().find(|(key, _)| key == name).unwrap().1;
        assert_eq!(
            field("future").into_rust::<FutureValue>().unwrap(),
            FutureValue::Future { nested: vec![1, 2] }
        );
        assert_eq!(
            field("description").into_rust::<Option<String>>().unwrap(),
            Some("Mine".into())
        );
        assert_eq!(after.0.len(), 2);
        // Raw unknown values retain their contents, not surrounding indentation.
        assert_eq!(
            after.0[1].0.get_ron().trim(),
            before.0[1].0.get_ron().trim()
        );
        assert_eq!(
            after.0[1].1.get_ron().trim(),
            before.0[1].1.get_ron().trim()
        );
        assert_eq!(merge(&merged, "{}", COMMAND, &target).unwrap(), merged);
        let changed = merge(
            &merged,
            "{}",
            COMMAND,
            &Chord::parse("Super", "F11").unwrap(),
        )
        .unwrap();
        assert_eq!(merge(&changed, "{}", COMMAND, &target).unwrap(), merged);
    }

    #[test]
    fn rejects_custom_default_duplicate_and_physical_conflicts() {
        let target = Chord::parse("Super,Ctrl", "section").unwrap();
        for conflict in [
            "{(modifiers: [Ctrl, Super], key: \"§\"): Close}",
            "{(modifiers: [Super, Ctrl], keycode: Some(49)): Close}",
            "{(modifiers: [Super, Ctrl], key: \"section\"): Close, (modifiers: [Ctrl, Super], key: \"§\"): Close}",
        ] {
            assert!(merge(conflict, "{}", COMMAND, &target).is_err());
            assert!(merge("{}", conflict, COMMAND, &target).is_err());
        }
        assert!(merge("{broken", "{}", COMMAND, &target).is_err());
    }

    #[test]
    fn modifier_only_launcher_does_not_block_super_key_chords() {
        let target = Chord::parse("Super", "§").unwrap();
        let defaults = "{(modifiers: [Super]): System(Launcher)}";
        let merged = merge("{}", defaults, COMMAND, &target).unwrap();
        let entries = ron::from_str(&merged).unwrap();
        assert_eq!(owned_binding(&entries, COMMAND).unwrap().unwrap().1, target);
        check_conflicts(&ron::from_str(defaults).unwrap(), &target, None).unwrap();
    }

    #[test]
    fn requires_exact_command_ownership() {
        let original = "{(modifiers: [Super], key: \"section\"): Spawn(\"other toggle\")}";
        assert!(
            merge(
                original,
                "{}",
                COMMAND,
                &Chord::parse("Super", "section").unwrap()
            )
            .is_err()
        );
        assert!(
            owned_binding(&ron::from_str(original).unwrap(), COMMAND)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_multiple_owned_bindings_instead_of_guessing() {
        let original = "{(modifiers: [Super], key: \"section\"): Spawn(\"terminal toggle\"), (modifiers: [Alt], key: \"F12\"): Spawn(\"terminal toggle\")}";
        assert!(owned_binding(&ron::from_str(original).unwrap(), "terminal toggle").is_err());
    }
}
