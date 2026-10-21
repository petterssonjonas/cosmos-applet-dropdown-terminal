// SPDX-License-Identifier: GPL-3.0-only
mod settings;

use cosmic::app::{Core, Task};
use cosmic::cosmic_config::{self, ConfigSet};
use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::{self, Alignment, Length, Limits, Subscription, keyboard::Modifiers, window};
use cosmic::{Element, action, widget};
use settings::Preferences;
use std::any::TypeId;
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
}

struct Applet {
    core: Core,
    busy: bool,
    error: Option<String>,
    popup: Option<window::Id>,
    page: Page,
    preferences: Preferences,
    height_dirty: bool,
    width_dirty: bool,
    settings_error: Option<String>,
    shortcut_error: Option<String>,
    shortcut: String,
    shortcut_busy: bool,
    shortcut_modifiers: Modifiers,
    shortcut_key: String,
}

#[derive(Clone, Debug)]
enum Message {
    Toggle,
    Completed(Result<(), String>),
    Surface(cosmic::surface::Action<Message>),
    Menu,
    Settings,
    ClosePopup(window::Id),
    Preferences(Preferences),
    Height(u8),
    SaveHeight,
    Width(u8),
    SaveWidth,
    AlwaysOnTop(bool),
    ShortcutModifier(Modifiers, bool),
    ShortcutKey(String),
    SaveShortcut,
    ShortcutCompleted(Result<String, String>),
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

impl Applet {
    fn close_popup(&mut self) -> Task<Message> {
        self.height_dirty = false;
        self.width_dirty = false;
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
            match settings::load() {
                Ok(preferences) => self.receive_preferences(preferences),
                Err(error) => self.settings_error = Some(error),
            }
            if !self.shortcut_busy {
                self.shortcut_busy = true;
                return Task::batch([
                    open,
                    iced::Task::perform(shortcut(None), |result| {
                        action::app(Message::ShortcutCompleted(result))
                    }),
                ]);
            }
        }
        open
    }

    fn receive_preferences(&mut self, preferences: Preferences) {
        if !self.height_dirty {
            self.preferences.dropdown_height_percent =
                preferences.dropdown_height_percent.clamp(1, 100);
        }
        if !self.width_dirty {
            self.preferences.dropdown_width_percent =
                preferences.dropdown_width_percent.clamp(1, 100);
        }
        self.preferences.dropdown_always_on_top = preferences.dropdown_always_on_top;
    }

    fn save_percent(&mut self, height: bool) {
        let (key, value) = if height {
            (
                "dropdown_height_percent",
                self.preferences.dropdown_height_percent,
            )
        } else {
            (
                "dropdown_width_percent",
                self.preferences.dropdown_width_percent,
            )
        };
        let result = cosmic_config::Config::new(settings::CONFIG_ID, settings::CONFIG_VERSION)
            .and_then(|config| config.set(key, value));
        match result {
            Ok(()) => {
                self.settings_error = None;
                if height {
                    self.height_dirty = false;
                } else {
                    self.width_dirty = false;
                }
            }
            Err(error) => self.settings_error = Some(format!("Cannot save settings: {error}")),
        }
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let height = self.preferences.dropdown_height_percent;
        let width = self.preferences.dropdown_width_percent;
        let shortcut_label = if self.shortcut_busy {
            "Reading or saving shortcut…"
        } else {
            &self.shortcut
        };
        let modifiers = widget::row(MODIFIERS.iter().map(|&(modifier, label)| {
            widget::checkbox(self.shortcut_modifiers.contains(modifier))
                .label(label)
                .size(16)
                .text_size(14)
                .on_toggle_maybe(
                    (!self.shortcut_busy)
                        .then_some(move |enabled| Message::ShortcutModifier(modifier, enabled)),
                )
                .into()
        }))
        .spacing(12);
        let mut key = widget::text_input("§, F12, grave…", &self.shortcut_key);
        if !self.shortcut_busy {
            key = key.on_input(Message::ShortcutKey);
        }
        let save = widget::button::text("Apply keybinding")
            .on_press_maybe((!self.shortcut_busy).then_some(Message::SaveShortcut));
        let mut shortcut_controls = widget::column([
            widget::text("Global keybinding").into(),
            widget::text(shortcut_label).size(14).into(),
            modifiers.into(),
            widget::row([key.into(), save.into()])
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            widget::text("Enter a character or XKB key name. Do not press the whole combination.")
                .size(12)
                .into(),
        ])
        .spacing(12);
        if let Some(error) = &self.shortcut_error {
            shortcut_controls =
                shortcut_controls.push(widget::text(format!("Error: {error}")).size(14));
        }
        let mut content = widget::column([
            widget::text::title3("Dropdown settings").into(),
            shortcut_controls.into(),
            widget::divider::horizontal::default().into(),
            widget::text(format!("Height: {height}% of screen")).into(),
            widget::slider(1..=100, height, Message::Height).on_release(Message::SaveHeight).into(),
            widget::text(format!("Width: {width}% of screen")).into(),
            widget::slider(1..=100, width, Message::Width).on_release(Message::SaveWidth).into(),
            widget::row([
                widget::text("Always on top").width(Length::Fill).into(),
                widget::toggler(self.preferences.dropdown_always_on_top).on_toggle(Message::AlwaysOnTop).into(),
            ]).align_y(Alignment::Center).spacing(12).into(),
            widget::text("Off uses a normal window. COSMIC controls its position and stacking.").size(12).into(),
            widget::text("Drag the terminal's bottom edge to change its height. Changes are saved automatically.").size(12).into(),
        ]).spacing(12).width(Length::Fill);
        if let Some(error) = &self.settings_error {
            content = content.push(widget::text(format!("Error: {error}")).size(14));
        }
        widget::container(widget::scrollable(content))
            .padding(16)
            .width(380)
            .max_height(560)
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
                preferences,
                height_dirty: false,
                width_dirty: false,
                settings_error,
                shortcut_error: None,
                shortcut: "Unassigned".into(),
                shortcut_busy: false,
                shortcut_modifiers: Modifiers::LOGO,
                shortcut_key: "§".into(),
            },
            iced::Task::perform(control("start"), |result| {
                action::app(Message::Completed(result))
            }),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle if !self.busy => {
                self.busy = true;
                return self
                    .close_popup()
                    .chain(iced::Task::perform(control("toggle"), |result| {
                        action::app(Message::Completed(result))
                    }));
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
            Message::Settings => return self.open_popup(Page::Settings),
            Message::ClosePopup(id) if self.popup == Some(id) => return self.close_popup(),
            Message::Preferences(preferences) => self.receive_preferences(preferences),
            Message::Height(value) => {
                self.preferences.dropdown_height_percent = value.clamp(1, 100);
                self.height_dirty = true;
            }
            Message::SaveHeight => self.save_percent(true),
            Message::Width(value) => {
                self.preferences.dropdown_width_percent = value.clamp(1, 100);
                self.width_dirty = true;
            }
            Message::SaveWidth => self.save_percent(false),
            Message::AlwaysOnTop(value) => {
                match cosmic_config::Config::new(settings::CONFIG_ID, settings::CONFIG_VERSION)
                    .and_then(|config| config.set("dropdown_always_on_top", value))
                {
                    Ok(()) => {
                        self.preferences.dropdown_always_on_top = value;
                        self.settings_error = None;
                    }
                    Err(error) => {
                        self.settings_error = Some(format!("Cannot save settings: {error}"))
                    }
                }
            }
            Message::ShortcutModifier(modifier, enabled) if !self.shortcut_busy => {
                self.shortcut_modifiers.set(modifier, enabled);
            }
            Message::ShortcutKey(key) if !self.shortcut_busy => self.shortcut_key = key,
            Message::SaveShortcut if !self.shortcut_busy => {
                let mut modifiers = String::new();
                for (modifier, name) in MODIFIERS {
                    if self.shortcut_modifiers.contains(modifier) {
                        if !modifiers.is_empty() {
                            modifiers.push(',');
                        }
                        modifiers.push_str(name);
                    }
                }
                self.shortcut_busy = true;
                self.shortcut_error = None;
                return iced::Task::perform(
                    shortcut(Some((modifiers, self.shortcut_key.clone()))),
                    |result| action::app(Message::ShortcutCompleted(result)),
                );
            }
            Message::ShortcutCompleted(result) => {
                self.shortcut_busy = false;
                match result {
                    Ok(label) => {
                        if label != "Unassigned" {
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
                        self.shortcut_error = None;
                    }
                    Err(error) => self.shortcut_error = Some(error),
                }
            }
            Message::Toggle
            | Message::ClosePopup(_)
            | Message::SaveShortcut
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
                widget::button::text("Settings")
                    .on_press(Message::Settings)
                    .width(Length::Fill),
            )
            .padding(8)
            .width(200)
            .into(),
            Page::Settings => self.settings_view(),
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
        struct PreferencesSubscription;
        cosmic_config::config_subscription::<_, Preferences>(
            TypeId::of::<PreferencesSubscription>(),
            settings::CONFIG_ID.into(),
            settings::CONFIG_VERSION,
        )
        .map(|update| Message::Preferences(update.config))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}
