// SPDX-License-Identifier: GPL-3.0-only

use cosmic::app::{Core, Task};
use cosmic::{Element, action};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

#[derive(Default)]
struct Applet {
    core: Core,
    busy: bool,
    error: Option<String>,
}

#[derive(Clone, Debug)]
enum Message {
    Toggle,
    Completed(Result<(), String>),
    Surface(cosmic::surface::Action<Message>),
}

// Resolve the installed sibling first; a panel's PATH need not include ~/.local/bin.
fn backend() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.parent()
                .map(|dir| dir.join("cosmos-applet-dropdown-terminal-backend"))
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("cosmos-applet-dropdown-terminal-backend"))
}

async fn control(command: &'static str) -> Result<(), String> {
    let mut child = tokio::process::Command::new(backend());
    child.arg(command).stdin(Stdio::null()).kill_on_drop(true);
    // Panel applets inherit a private Wayland connection. The terminal must connect
    // to the session compositor, not take over the panel's applet surface.
    child.env_remove("WAYLAND_SOCKET");
    match tokio::time::timeout(Duration::from_secs(40), child.output()).await {
        Ok(Ok(output)) if output.status.success() => Ok(()),
        Ok(Ok(output)) => Err(format!(
            "Terminal control failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Ok(Err(error)) => Err(format!("Cannot start cosmos-applet-dropdown-terminal-backend: {error}")),
        Err(_) => Err("Terminal control timed out after 40 seconds".into()),
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
        (
            Self {
                core,
                busy: true,
                error: None,
            },
            cosmic::iced::Task::perform(control("start"), |result| {
                action::app(Message::Completed(result))
            }),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle if !self.busy => {
                self.busy = true;
                return cosmic::iced::Task::perform(control("toggle"), |result| {
                    action::app(Message::Completed(result))
                });
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
            Message::Toggle => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let icon = if self.error.is_some() {
            "dialog-warning-symbolic"
        } else {
            "utilities-terminal-symbolic"
        };
        let button = self.core.applet.icon_button(icon).on_press(Message::Toggle);
        self.core
            .applet
            .applet_tooltip::<Message>(
                button,
                self.error
                    .clone()
                    .unwrap_or_else(|| "Cosmos Dropdown Terminal · Super+§".into()),
                false,
                Message::Surface,
                None,
            )
            .into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}
