// SPDX-License-Identifier: GPL-3.0-only
use cosmic::iced::{Subscription, futures::SinkExt, stream};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Clone, Debug)]
pub enum Event {
    Ready(u64),
    Ended(u64, Option<String>),
}

#[derive(Default)]
pub struct Guard {
    generation: u64,
    active: Option<u64>,
    ready: bool,
    ended: bool,
}

impl Guard {
    pub fn set_needed(&mut self, needed: bool) {
        if !needed {
            self.active = None;
            self.ready = false;
            self.ended = false;
        } else if self.active.is_none() {
            self.generation = self.generation.wrapping_add(1);
            self.active = Some(self.generation);
        }
    }

    pub fn acknowledged(&self) -> Option<u64> {
        self.active.filter(|_| self.ready)
    }

    pub fn receive(&mut self, event: &Event) -> bool {
        let generation = match event {
            Event::Ready(generation) | Event::Ended(generation, _) => *generation,
        };
        if self.active != Some(generation) || self.ended {
            return false;
        }
        match event {
            Event::Ready(_) => self.ready = true,
            Event::Ended(..) => {
                self.ready = false;
                // Keep this attempt's identity until controls are abandoned. A failed
                // connection must not turn subscription updates into a retry loop.
                self.ended = true;
            }
        }
        true
    }

    pub fn subscription(&self) -> Subscription<Event> {
        match self.active {
            Some(generation) => Subscription::run_with(generation, |&generation| {
                stream::channel(2, move |mut output| async move {
                    let result = hold(generation, &mut output).await;
                    let _ = output.send(Event::Ended(generation, result.err())).await;
                })
            }),
            None => Subscription::none(),
        }
    }
}

async fn hold(
    generation: u64,
    output: &mut cosmic::iced::futures::channel::mpsc::Sender<Event>,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(super::backend());
    command
        .arg("panel-guard")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .env_remove("WAYLAND_SOCKET");
    let mut child = command
        .spawn()
        .map_err(|error| format!("Cannot protect terminal panel controls: {error}"))?;
    // The subscription owns both the process and its lease pipe. Cancellation
    // closes stdin and kills the helper, including before its acknowledgement.
    let _stdin = child.stdin.take().ok_or("Panel guard has no input pipe.")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Panel guard has no output pipe.")?;
    let mut reader = BufReader::new(stdout);
    let mut acknowledgement = String::new();
    tokio::time::timeout(
        Duration::from_secs(40),
        reader.read_line(&mut acknowledgement),
    )
    .await
    .map_err(|_| "Terminal panel guard timed out after 40 seconds.".to_owned())?
    .map_err(|error| format!("Cannot read terminal panel guard acknowledgement: {error}"))?;
    if acknowledgement != "guarded\n" {
        return Err("Terminal panel controls are unavailable. Right-click the icon and select Restart to load the installed backend. Restart closes all shells and jobs.".into());
    }
    if output.send(Event::Ready(generation)).await.is_err() {
        return Ok(());
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("Cannot monitor terminal panel controls: {error}"))?;
    Err(format!(
        "Terminal panel controls disconnected ({status}). Leave the icon and try again, or select Restart from its right-click menu."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_acknowledgement_and_release_revokes_it() {
        let mut guard = Guard::default();
        guard.set_needed(true);
        let generation = guard.active.unwrap();
        assert_eq!(guard.acknowledged(), None);
        assert!(guard.receive(&Event::Ready(generation)));
        assert_eq!(guard.acknowledged(), Some(generation));
        guard.set_needed(false);
        assert_eq!(guard.acknowledged(), None);
        assert!(guard.active.is_none());
        assert!(!guard.receive(&Event::Ready(generation)));
    }

    #[test]
    fn repeated_demand_keeps_one_lease_and_old_events_cannot_change_replacement() {
        let mut guard = Guard::default();
        guard.set_needed(true);
        let first = guard.active.unwrap();
        guard.receive(&Event::Ready(first));
        guard.set_needed(true);
        assert_eq!(guard.acknowledged(), Some(first));
        guard.set_needed(false);
        guard.set_needed(true);
        let replacement = guard.active.unwrap();
        assert_ne!(first, replacement);
        assert!(!guard.receive(&Event::Ready(first)));
        assert_eq!(guard.acknowledged(), None);
        guard.receive(&Event::Ready(replacement));
        assert!(!guard.receive(&Event::Ended(first, None)));
        assert_eq!(guard.acknowledged(), Some(replacement));
    }

    #[test]
    fn disconnect_stays_disabled_without_retrying_or_accepting_late_ack() {
        let mut guard = Guard::default();
        guard.set_needed(true);
        let generation = guard.active.unwrap();
        guard.receive(&Event::Ready(generation));
        guard.receive(&Event::Ended(generation, None));
        guard.set_needed(true);
        assert_eq!(guard.active, Some(generation));
        assert_eq!(guard.acknowledged(), None);
        assert!(!guard.receive(&Event::Ready(generation)));
        guard.set_needed(false);
        guard.set_needed(true);
        assert_ne!(guard.active, Some(generation));
    }
}
