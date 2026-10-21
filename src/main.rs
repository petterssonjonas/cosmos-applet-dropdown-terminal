// SPDX-License-Identifier: GPL-3.0-only
mod settings;

use cosmic::app::{Core, Task};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{self, Alignment, Length, Limits, Subscription, keyboard::Modifiers, window};
use cosmic::{Element, action, widget};
use settings::Preferences;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

const MODIFIERS: [(Modifiers, &str); 4] = [
    (Modifiers::LOGO, "Super"),
    (Modifiers::CTRL, "Ctrl"),
    (Modifiers::ALT, "Alt"),
    (Modifiers::SHIFT, "Shift"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Menu,
    Settings,
    ConfirmRestart,
}

struct Applet {
    core: Core,
    busy: bool,
    error: Option<String>,
    popup: Option<window::Id>,
    page: Page,
    preferences: Preferences,
    original_preferences: Option<Preferences>,
    settings_error: Option<String>,
    shortcut_error: Option<String>,
    shortcut: String,
    shortcut_busy: bool,
    shortcut_edited: bool,
    shortcut_modifiers: Modifiers,
    shortcut_key: String,
    saving: bool,
}

#[derive(Clone, Debug)]
enum Message {
    Toggle,
    Completed(Result<(), String>),
    Surface(cosmic::surface::Action<Message>),
    Menu,
    Settings,
    ConfirmRestart,
    Restart,
    ClosePopup(window::Id),
    Height(u8),
    Width(u8),
    AlwaysOnTop(bool),
    ShortcutModifier(Modifiers, bool),
    ShortcutKey(String),
    ShortcutLoaded(window::Id, Result<String, String>),
    Save,
    Cancel,
    Saved(window::Id, Result<(), String>, Option<String>),
}

// Resolve installed siblings first; a panel's PATH need not include ~/.local/bin.
fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join(name)))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn backend() -> PathBuf {
    sibling("cosmos-applet-dropdown-terminal-backend")
}

async fn control(command: &'static str) -> Result<(), String> {
    let mut child = tokio::process::Command::new(backend());
    child.arg(command).stdin(Stdio::null()).kill_on_drop(true);
    // The terminal uses the desktop Wayland connection, not the panel's private socket.
    child.env_remove("WAYLAND_SOCKET");
    match tokio::time::timeout(Duration::from_secs(40), child.output()).await {
        Ok(Ok(output)) if output.status.success() => Ok(()),
        Ok(Ok(output)) => Err(format!(
            "Terminal control failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Ok(Err(error)) => Err(format!("Cannot start the terminal backend: {error}")),
        Err(_) => Err("Terminal control timed out after 40 seconds".into()),
    }
}

async fn restart() -> Result<(), String> {
    // The quit command waits for the old backend to release its ownership lock.
    // Never start a replacement if stopping the old backend failed.
    control("quit").await?;
    control("show").await
}

async fn shortcut(binding: Option<(String, String)>) -> Result<String, String> {
    let backend = backend();
    if !backend.is_absolute() {
        return Err(
            "Install the terminal backend beside the applet before setting its shortcut.".into(),
        );
    }
    let path = backend
        .to_str()
        .ok_or("The backend path must be valid UTF-8.")?;
    let command = format!("'{}' toggle", path.replace('\'', "'\\''"));
    let mut child =
        tokio::process::Command::new(sibling("cosmos-applet-dropdown-terminal-shortcut"));
    match binding {
        Some((modifiers, key)) => {
            child.args(["--set", &modifiers, &key, &command]);
        }
        None => {
            child.args(["--get", &command]);
        }
    }
    child.stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(10), child.output())
        .await
        .map_err(|_| "Shortcut configuration timed out.".to_owned())?
        .map_err(|error| format!("Cannot start the shortcut helper: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

async fn save_settings(
    original: Preferences,
    draft: Preferences,
    binding: Option<(String, String, String)>,
) -> (Result<(), String>, Option<String>) {
    // Check for external shortcut edits before writing either configuration.
    if let Some((_, _, expected)) = &binding {
        let current = match shortcut(None).await {
            Ok(current) => current,
            Err(error) => {
                return (
                    Err(format!(
                        "Nothing saved. Cannot read current shortcut: {error}"
                    )),
                    None,
                );
            }
        };
        if current != *expected {
            return (Err("Nothing saved. The shortcut changed outside this dialog. Cancel and reopen settings before saving.".into()), None);
        }
    }
    let mut errors = Vec::new();
    let mut saved_shortcut = None;
    if let Err(error) = settings::save(&original, &draft) {
        errors.push(format!(
            "Cannot save preferences; some keys may have been saved: {error}"
        ));
    } else if let Some((modifiers, key, _)) = binding {
        match shortcut(Some((modifiers, key))).await {
            Ok(label) => saved_shortcut = Some(label),
            Err(error) => errors.push(format!(
                "Preferences saved, but the shortcut was not saved: {error}"
            )),
        }
    }
    // Explicit reload acknowledges visible application, rather than trusting file-watch timing.
    if let Err(error) = control("reload").await {
        errors.push(format!(
            "Cannot apply saved preferences to the terminal: {error}\n\
             If an older backend is still running, finish running shell jobs, then restart the backend. \
             Restarting closes its shells; this dialog does not stop them automatically."
        ));
    }
    if errors.is_empty() {
        (Ok(()), saved_shortcut)
    } else {
        (Err(errors.join("\n")), saved_shortcut)
    }
}

impl Applet {
    fn close_popup(&mut self) -> Task<Message> {
        if let Some(original) = &self.original_preferences {
            self.preferences = original.clone();
        }
        if self.shortcut_edited {
            self.shortcut_edited = false;
            self.shortcut_modifiers = Modifiers::LOGO;
            self.shortcut_key.clear();
            self.shortcut_key.push('§');
            self.receive_shortcut(self.shortcut.clone());
        }
        match self.popup.take() {
            Some(id) => destroy_popup(id),
            None => Task::none(),
        }
    }

    fn open_popup(&mut self, page: Page) -> Task<Message> {
        let Some(parent) = self.core.main_window_id() else {
            return Task::none();
        };
        let close = self.close_popup();
        let id = window::Id::unique();
        self.popup = Some(id);
        self.page = page;
        let width = if page == Page::Menu { 200.0 } else { 380.0 };
        let mut popup = self
            .core
            .applet
            .get_popup_settings(parent, id, None, None, None);
        popup.positioner.size_limits = Limits::NONE
            .min_width(width)
            .max_width(width)
            .max_height(560.0);
        let open = close.chain(get_popup(popup));
        if page == Page::Settings {
            self.settings_error = None;
            self.shortcut_error = None;
            self.original_preferences = match settings::load() {
                Ok(preferences) => {
                    self.preferences = preferences.clone();
                    Some(preferences)
                }
                Err(error) => {
                    self.settings_error = Some(format!("Cannot read settings: {error}"));
                    None
                }
            };
            self.shortcut = "Unassigned".into();
            self.shortcut_modifiers = Modifiers::LOGO;
            self.shortcut_key = "§".into();
            self.shortcut_busy = true;
            return Task::batch([
                open,
                iced::Task::perform(shortcut(None), move |result| {
                    action::app(Message::ShortcutLoaded(id, result))
                }),
            ]);
        }
        open
    }

    fn receive_shortcut(&mut self, label: String) {
        // A late read may update the saved label, never an edited draft.
        if !self.shortcut_edited && label != "Unassigned" {
            self.shortcut_modifiers = Modifiers::empty();
            let mut parts = label.split('+').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    self.shortcut_key = part.into();
                } else if let Some(&(modifier, _)) =
                    MODIFIERS.iter().find(|(_, name)| *name == part)
                {
                    self.shortcut_modifiers.insert(modifier);
                }
            }
        }
        self.shortcut = label;
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let height = self.preferences.dropdown_height_percent;
        let width = self.preferences.dropdown_width_percent;
        let shortcut_label = if self.shortcut_busy {
            "Reading shortcut…"
        } else {
            &self.shortcut
        };
        let modifiers = widget::row(MODIFIERS.iter().map(|&(modifier, label)| {
            widget::checkbox(self.shortcut_modifiers.contains(modifier))
                .label(label)
                .size(16)
                .text_size(14)
                .on_toggle_maybe(
                    (!self.saving)
                        .then_some(move |enabled| Message::ShortcutModifier(modifier, enabled)),
                )
                .into()
        }))
        .spacing(12);
        let mut key = widget::text_input("§, F12, grave…", &self.shortcut_key);
        if !self.saving {
            key = key.on_input(Message::ShortcutKey);
        }
        let mut shortcut_controls = widget::column([
            widget::text("Global keybinding").into(),
            widget::text(shortcut_label).size(14).into(),
            modifiers.into(),
            key.into(),
            widget::text("Enter a character or XKB key name. Do not press the whole combination.")
                .size(12)
                .into(),
        ])
        .spacing(12);
        if let Some(error) = &self.shortcut_error {
            shortcut_controls =
                shortcut_controls.push(widget::text(format!("Error: {error}")).size(14));
        }
        let content = widget::column([
            widget::text::title3("Dropdown settings").into(),
            shortcut_controls.into(),
            widget::divider::horizontal::default().into(),
            widget::text(format!("Height: {height}% of screen")).into(),
            widget::slider(1..=100, height, Message::Height).into(),
            widget::text(format!("Width: {width}% of screen")).into(),
            widget::slider(1..=100, width, Message::Width).into(),
            widget::row([
                widget::text("Always on top").width(Length::Fill).into(),
                widget::toggler(self.preferences.dropdown_always_on_top)
                    .on_toggle(Message::AlwaysOnTop)
                    .into(),
            ])
            .align_y(Alignment::Center)
            .spacing(12)
            .into(),
            widget::text(
                "Off lets other windows cover the dropdown when it loses focus. Opening it raises it.",
            )
            .size(12)
            .into(),
            widget::text(
                "Save applies these settings and the edited shortcut. Cancel discards edits.",
            )
            .size(12)
            .into(),
        ])
        .spacing(12)
        .width(Length::Fill);
        let actions = widget::row([
            widget::Space::new().width(Length::Fill).into(),
            widget::button::standard("Cancel")
                .on_press_maybe((!self.saving).then_some(Message::Cancel))
                .into(),
            widget::button::suggested(if self.saving { "Saving…" } else { "Save" })
                .on_press_maybe(
                    (!self.saving && !self.shortcut_busy && self.original_preferences.is_some())
                        .then_some(Message::Save),
                )
                .into(),
        ])
        .spacing(8)
        .align_y(Alignment::Center);
        let mut footer = widget::column::with_capacity(2).spacing(8);
        if let Some(error) = &self.settings_error {
            footer = footer.push(
                widget::scrollable(widget::text(format!("Error: {error}")).size(14)).height(108),
            );
        }
        let footer = footer.push(actions);
        // Only the fields scroll; actions remain reachable at the popup's height limit.
        widget::container(
            widget::column([
                widget::scrollable(content).height(Length::Fill).into(),
                widget::divider::horizontal::default().into(),
                footer.into(),
            ])
            .spacing(12),
        )
        .padding(16)
        .width(380)
        .height(560)
        .into()
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "dev.jp.CosmosAppletDropdownTerminal";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, (): ()) -> (Self, Task<Message>) {
        let (preferences, settings_error) = match settings::load() {
            Ok(preferences) => (preferences, None),
            Err(error) => (Preferences::default(), Some(error)),
        };
        (
            Self {
                core,
                busy: true,
                error: None,
                popup: None,
                page: Page::Menu,
                original_preferences: Some(preferences.clone()),
                preferences,
                settings_error,
                shortcut_error: None,
                shortcut: "Unassigned".into(),
                shortcut_busy: false,
                shortcut_edited: false,
                shortcut_modifiers: Modifiers::LOGO,
                shortcut_key: "§".into(),
                saving: false,
            },
            iced::Task::perform(control("start"), |result| {
                action::app(Message::Completed(result))
            }),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle if !self.busy && !self.saving => {
                self.busy = true;
                return self.close_popup().chain(iced::Task::perform(
                    control("toggle-visibility"),
                    |result| action::app(Message::Completed(result)),
                ));
            }
            Message::Completed(result) => {
                self.busy = false;
                self.error = result.err();
                if let Some(error) = &self.error {
                    eprintln!("{error}");
                }
            }
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Surface(action));
            }
            Message::Menu => {
                return if self.popup.is_some() {
                    self.close_popup()
                } else {
                    self.open_popup(Page::Menu)
                };
            }
            Message::Settings if !self.busy && !self.saving => {
                return self.open_popup(Page::Settings);
            }
            Message::ConfirmRestart if !self.busy && !self.saving => {
                return self.open_popup(Page::ConfirmRestart);
            }
            Message::Restart
                if !self.busy
                    && !self.saving
                    && self.popup.is_some()
                    && self.page == Page::ConfirmRestart =>
            {
                self.busy = true;
                self.error = None;
                return self
                    .close_popup()
                    .chain(iced::Task::perform(restart(), |result| {
                        action::app(Message::Completed(result))
                    }));
            }
            Message::ClosePopup(id) if self.popup == Some(id) => return self.close_popup(),
            Message::Height(value) if !self.saving => {
                self.preferences.dropdown_height_percent = value.clamp(1, 100);
            }
            Message::Width(value) if !self.saving => {
                self.preferences.dropdown_width_percent = value.clamp(1, 100);
            }
            Message::AlwaysOnTop(value) if !self.saving => {
                self.preferences.dropdown_always_on_top = value;
            }
            Message::ShortcutModifier(modifier, enabled) if !self.saving => {
                self.shortcut_modifiers.set(modifier, enabled);
                self.shortcut_edited = true;
            }
            Message::ShortcutKey(key) if !self.saving => {
                self.shortcut_key = key;
                self.shortcut_edited = true;
            }
            Message::ShortcutLoaded(id, result) if self.popup == Some(id) => {
                self.shortcut_busy = false;
                match result {
                    Ok(label) => self.receive_shortcut(label),
                    Err(error) => self.shortcut_error = Some(error),
                }
            }
            Message::Save if !self.saving && !self.shortcut_busy => {
                let (Some(id), Some(original)) = (self.popup, &self.original_preferences) else {
                    return Task::none();
                };
                if self.shortcut_edited && self.shortcut_error.is_some() {
                    self.settings_error = Some(
                        "Cannot safely replace an unread shortcut. Cancel and reopen settings to retry."
                            .into(),
                    );
                    return Task::none();
                }
                let binding = self.shortcut_edited.then(|| {
                    let mut modifiers = String::new();
                    for (modifier, name) in MODIFIERS {
                        if self.shortcut_modifiers.contains(modifier) {
                            if !modifiers.is_empty() {
                                modifiers.push(',');
                            }
                            modifiers.push_str(name);
                        }
                    }
                    (modifiers, self.shortcut_key.clone(), self.shortcut.clone())
                });
                self.saving = true;
                self.settings_error = None;
                return iced::Task::perform(
                    save_settings(original.clone(), self.preferences.clone(), binding),
                    move |(result, label)| action::app(Message::Saved(id, result, label)),
                );
            }
            Message::Saved(id, result, label) => {
                self.saving = false;
                if self.popup == Some(id) {
                    if let Some(label) = label {
                        self.shortcut = label;
                    }
                    match result {
                        Ok(()) => {
                            self.original_preferences = Some(self.preferences.clone());
                            self.error = None;
                            return self.close_popup();
                        }
                        Err(error) => self.settings_error = Some(error),
                    }
                } else if let Err(error) = result {
                    // Dismissing an in-flight Save must not silently hide its failure.
                    self.error = Some(error);
                }
            }
            Message::Cancel if !self.saving => return self.close_popup(),
            Message::Toggle
            | Message::Settings
            | Message::ConfirmRestart
            | Message::Restart
            | Message::ClosePopup(_)
            | Message::Height(_)
            | Message::Width(_)
            | Message::AlwaysOnTop(_)
            | Message::Save
            | Message::Cancel
            | Message::ShortcutLoaded(..)
            | Message::ShortcutModifier(..)
            | Message::ShortcutKey(_) => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let icon = if self.error.is_some() {
            "dialog-warning-symbolic"
        } else {
            "utilities-terminal-symbolic"
        };
        let button =
            widget::mouse_area(self.core.applet.icon_button(icon).on_press(Message::Toggle))
                .on_right_press(Message::Menu);
        self.core
            .applet
            .applet_tooltip::<Message>(
                button,
                self.error.clone().unwrap_or_else(|| {
                    "Cosmos Dropdown Terminal · Right-click for Settings".into()
                }),
                self.popup.is_some(),
                Message::Surface,
                None,
            )
            .into()
    }

    fn view_window(&self, id: window::Id) -> Element<'_, Message> {
        if self.popup != Some(id) {
            return widget::text("").into();
        }
        let content = match self.page {
            Page::Menu => widget::container(
                widget::column([
                    widget::button::text("Settings")
                        .on_press_maybe((!self.busy && !self.saving).then_some(Message::Settings))
                        .width(Length::Fill)
                        .into(),
                    widget::button::text("Restart")
                        .on_press_maybe(
                            (!self.busy && !self.saving).then_some(Message::ConfirmRestart),
                        )
                        .width(Length::Fill)
                        .into(),
                ])
                .spacing(4),
            )
            .padding(8)
            .width(200)
            .into(),
            Page::Settings => self.settings_view(),
            Page::ConfirmRestart => widget::container(
                widget::column([
                    widget::text::title3("Restart terminal?").into(),
                    widget::text("This closes all terminal shells and running jobs. Finish your work first.").into(),
                    widget::text("A new terminal opens with the installed backend. The panel icon stays in place.").size(14).into(),
                    widget::row([
                        widget::Space::new().width(Length::Fill).into(),
                        widget::button::standard("Cancel").on_press(Message::Cancel).into(),
                        widget::button::destructive("Restart")
                            .on_press_maybe((!self.busy && !self.saving).then_some(Message::Restart))
                            .into(),
                    ]).spacing(8).into(),
                ]).spacing(12),
            ).padding(16).width(380).into(),
        };
        self.core.applet.popup_container(content).into()
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::ClosePopup(id))
    }

    fn on_escape(&mut self) -> Task<Message> {
        self.close_popup()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.popup.is_none() {
            return Subscription::none();
        }
        // Focused controls can capture Escape before libcosmic's navigation handler sees it.
        iced::event::listen_raw(|event, _, id| {
            if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            }) = event
            {
                Some(Message::ClosePopup(id))
            } else {
                None
            }
        })
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::Application;

    fn applet() -> Applet {
        let preferences = Preferences::default();
        Applet {
            core: Core::default(),
            busy: false,
            error: None,
            popup: Some(window::Id::unique()),
            page: Page::Settings,
            original_preferences: Some(preferences.clone()),
            preferences,
            settings_error: None,
            shortcut_error: None,
            shortcut: "Super+F12".into(),
            shortcut_busy: false,
            shortcut_edited: false,
            shortcut_modifiers: Modifiers::LOGO,
            shortcut_key: "F12".into(),
            saving: false,
        }
    }

    #[test]
    fn cancel_discards_all_preference_edits() {
        let mut applet = applet();
        let original = applet.preferences.clone();
        let _ = applet.update(Message::Height(70));
        let _ = applet.update(Message::Width(60));
        let _ = applet.update(Message::AlwaysOnTop(false));
        assert_eq!(applet.original_preferences.as_ref(), Some(&original));
        let _ = applet.update(Message::Cancel);
        assert_eq!(applet.preferences, original);
        assert!(applet.popup.is_none());
    }

    #[test]
    fn escape_and_outside_dismiss_discard_draft() {
        for escape in [false, true] {
            let mut applet = applet();
            let original = applet.preferences.clone();
            let _ = applet.update(Message::Height(70));
            let _ = applet.update(Message::ShortcutKey("F10".into()));
            if escape {
                let _ = applet.on_escape();
            } else {
                let id = applet.popup.unwrap();
                let _ = applet.update(Message::ClosePopup(id));
            }
            assert_eq!(applet.preferences, original);
            assert!(!applet.shortcut_edited);
            assert!(applet.popup.is_none());
        }
    }

    #[test]
    fn late_shortcut_reads_preserve_edits_and_ignore_closed_sessions() {
        let mut applet = applet();
        let id = applet.popup.unwrap();
        let _ = applet.update(Message::ShortcutKey("F10".into()));
        let _ = applet.update(Message::ShortcutLoaded(id, Ok("Ctrl+F11".into())));
        assert_eq!(applet.shortcut_key, "F10");
        assert_eq!(applet.shortcut, "Ctrl+F11");
        applet.popup = Some(window::Id::unique());
        let _ = applet.update(Message::ShortcutLoaded(id, Ok("Alt+F9".into())));
        assert_eq!(applet.shortcut, "Ctrl+F11");
    }

    #[test]
    fn failed_save_keeps_draft_open_and_reports_partial_failure() {
        let mut applet = applet();
        let id = applet.popup.unwrap();
        let _ = applet.update(Message::Height(70));
        applet.saving = true;
        let error = "Preferences saved, but the shortcut was not saved: conflict";
        let _ = applet.update(Message::Saved(id, Err(error.into()), None));
        assert_eq!(applet.popup, Some(id));
        assert_eq!(applet.preferences.dropdown_height_percent, 70);
        assert_eq!(applet.settings_error.as_deref(), Some(error));
        assert!(!applet.saving);
    }

    #[test]
    fn successful_save_accepts_draft_and_closes_dialog() {
        let mut applet = applet();
        let id = applet.popup.unwrap();
        let _ = applet.update(Message::Height(70));
        let _ = applet.update(Message::Saved(id, Ok(()), None));
        assert_eq!(applet.preferences.dropdown_height_percent, 70);
        assert!(applet.popup.is_none());
    }

    #[test]
    fn cancel_cannot_undo_explicit_save_in_flight() {
        let mut applet = applet();
        applet.saving = true;
        let id = applet.popup;
        let _ = applet.update(Message::Cancel);
        assert_eq!(applet.popup, id);
    }

    #[test]
    fn restart_requires_an_open_confirmation_and_cancel_revokes_it() {
        let mut applet = applet();
        let _ = applet.update(Message::Restart);
        assert!(!applet.busy);
        applet.page = Page::ConfirmRestart;
        let _ = applet.update(Message::Cancel);
        let _ = applet.update(Message::Restart);
        assert!(!applet.busy);
        assert!(applet.popup.is_none());
    }

    #[test]
    fn confirmed_restart_is_single_flight_and_waits_for_settings_save() {
        let mut applet = applet();
        applet.page = Page::ConfirmRestart;
        applet.saving = true;
        let confirmation = applet.popup;
        let _ = applet.update(Message::Restart);
        assert!(!applet.busy);
        assert_eq!(applet.popup, confirmation);
        applet.saving = false;
        let _ = applet.update(Message::Restart);
        assert!(applet.busy);
        assert!(applet.popup.is_none());
        let _ = applet.update(Message::Toggle);
        let _ = applet.update(Message::Restart);
        assert!(applet.busy);
        assert!(applet.popup.is_none());
    }

    #[test]
    fn icon_toggle_is_single_flight_and_waits_for_settings_save() {
        let mut applet = applet();
        applet.saving = true;
        let popup = applet.popup;
        let _ = applet.update(Message::Toggle);
        assert!(!applet.busy);
        assert_eq!(applet.popup, popup);
        applet.saving = false;
        let _ = applet.update(Message::Toggle);
        assert!(applet.busy);
        assert!(applet.popup.is_none());
        let _ = applet.update(Message::Toggle);
        assert!(applet.busy);
        assert!(applet.popup.is_none());
    }
}
