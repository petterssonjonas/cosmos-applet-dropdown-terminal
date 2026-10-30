// SPDX-License-Identifier: GPL-3.0-only
use cosmic::cctk::sctk::{
    delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::client::{
        Connection, EventQueue, QueueHandle, backend::WaylandError, globals::registry_queue_init,
        protocol::wl_output::WlOutput,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use cosmic::iced::{Subscription, futures::SinkExt, stream, window};
use std::{
    net::Shutdown,
    os::{
        fd::{AsRawFd, RawFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::io::unix::AsyncFd;

#[derive(Clone, Debug)]
pub struct Display {
    pub name: String,
    pub label: String,
}

impl AsRef<str> for Display {
    fn as_ref(&self) -> &str {
        &self.label
    }
}

pub fn choices(mut displays: Vec<Display>, preferred: &str) -> Arc<[Display]> {
    displays.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    displays.dedup_by(|a, b| a.name == b.name);
    if !preferred.is_empty() && !displays.iter().any(|display| display.name == preferred) {
        displays.push(Display {
            name: preferred.into(),
            label: format!("{preferred} (disconnected)"),
        });
    }
    displays.insert(
        0,
        Display {
            name: String::new(),
            label: "Automatic".into(),
        },
    );
    displays.into()
}

type Event = (window::Id, Result<Vec<Display>, String>);

pub fn subscription(id: window::Id) -> Subscription<Event> {
    Subscription::run_with(id, |&id| {
        stream::channel(2, move |mut sender| async move {
            if let Err(error) = watch(id, &mut sender).await {
                let _ = sender.send((id, Err(error))).await;
            }
        })
    })
}

// Closing settings cancels discovery, including a blocked initialization roundtrip.
struct Interrupt(UnixStream);
impl AsRawFd for Interrupt {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}
impl Drop for Interrupt {
    fn drop(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

async fn watch(
    id: window::Id,
    sender: &mut cosmic::iced::futures::channel::mpsc::Sender<Event>,
) -> Result<(), String> {
    // The panel's private WAYLAND_SOCKET may expose only its configured outputs.
    // Connect to the desktop socket without consuming or changing that environment FD.
    let display =
        PathBuf::from(std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into()));
    let path = if display.is_absolute() {
        display
    } else {
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is unset")?)
            .join(display)
    };
    let socket = UnixStream::connect(path).map_err(|error| error.to_string())?;
    let ready = AsyncFd::new(Interrupt(
        socket.try_clone().map_err(|error| error.to_string())?,
    ))
    .map_err(|error| error.to_string())?;
    let initialization = tokio::task::spawn_blocking(move || initialize(socket));
    let (connection, mut queue, mut state) =
        tokio::time::timeout(Duration::from_secs(5), initialization)
            .await
            .map_err(|_| "Display discovery timed out".to_owned())?
            .map_err(|error| error.to_string())??;
    state.changed = true;
    loop {
        queue
            .dispatch_pending(&mut state)
            .map_err(|error| error.to_string())?;
        if std::mem::take(&mut state.changed) {
            let displays = state
                .outputs
                .outputs()
                .filter_map(|output| {
                    let info = state.outputs.info(&output)?;
                    let name = info.name.filter(|name| !name.is_empty())?;
                    let model = info.model.trim();
                    let label = if model.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} — {model}")
                    };
                    Some(Display { name, label })
                })
                .collect();
            if sender.send((id, Ok(displays))).await.is_err() {
                return Ok(());
            }
        }
        let Some(read) = queue.prepare_read() else {
            continue;
        };
        connection.flush().map_err(|error| error.to_string())?;
        let mut readiness = ready.readable().await.map_err(|error| error.to_string())?;
        match read.read() {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
        readiness.clear_ready();
    }
}

fn initialize(socket: UnixStream) -> Result<(Connection, EventQueue<State>, State), String> {
    let connection = Connection::from_socket(socket).map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init(&connection).map_err(|error| error.to_string())?;
    let mut state = State {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &queue.handle()),
        changed: false,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|error| error.to_string())?;
    Ok((connection, queue, state))
}

struct State {
    registry: RegistryState,
    outputs: OutputState,
    changed: bool,
}
impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.changed = true;
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.changed = true;
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {
        self.changed = true;
    }
}
impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}
delegate_output!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_selection_survives_hotplug_without_becoming_automatic() {
        let connected = || {
            vec![Display {
                name: "DP-1".into(),
                label: "DP-1 — Display".into(),
            }]
        };
        let choices = choices(connected(), "DP-2");
        assert_eq!(
            choices
                .iter()
                .map(|display| display.name.as_str())
                .collect::<Vec<_>>(),
            ["", "DP-1", "DP-2"]
        );
        assert_eq!(choices[2].label, "DP-2 (disconnected)");
        let mut displays = connected();
        displays.push(Display {
            name: "DP-2".into(),
            label: "DP-2 — Second Display".into(),
        });
        let reconnected = super::choices(displays, "DP-2");
        assert_eq!(reconnected[2].label, "DP-2 — Second Display");
        assert_eq!(super::choices(Vec::new(), "DP-2")[1].name, "DP-2");
    }
}
