// SPDX-License-Identifier: GPL-3.0-only
//! Surface state is independent of terminal state. Hiding destroys only the
//! layer surface; the application, terminal models and PTY subscriptions survive.
use cosmic::{
    cctk::sctk::{
        output::OutputInfo,
        reexports::client::protocol::wl_output::{Transform, WlOutput},
    },
    iced::{
        Limits,
        platform_specific::{
            runtime::wayland::layer_surface::{IcedOutput, SctkLayerSurfaceSettings},
            shell::commands::layer_surface::{Anchor, KeyboardInteractivity, Layer},
        },
        window,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Absent,
    Creating,
    Ready,
    Closing,
}

#[derive(Default)]
pub struct Dropdown {
    pub visible: bool,
    pub surface: Option<window::Id>,
    phase: Phase,
    output_invalidated: bool,
    keyboard_withdrawn: bool,
    outputs: Vec<(WlOutput, OutputInfo)>,
    selected: Option<WlOutput>,
}
impl Dropdown {
    pub fn begin_close(&mut self) -> Option<window::Id> {
        if self.phase != Phase::Ready
            || (self.visible && !self.output_invalidated && !self.keyboard_withdrawn)
        {
            return None;
        }
        self.phase = Phase::Closing;
        self.surface
    }
    pub fn opened(&mut self, id: window::Id) -> bool {
        if self.surface != Some(id) || self.phase != Phase::Creating {
            return false;
        }
        self.phase = Phase::Ready;
        true
    }
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }
    pub fn creating(&self) -> bool {
        self.phase == Phase::Creating
    }
    pub fn withdraw_keyboard(&mut self) {
        if self.creating() {
            self.keyboard_withdrawn = true;
        }
    }
    pub fn surface_closed(&mut self, id: window::Id) -> bool {
        if self.surface != Some(id) {
            return false;
        }
        if self.phase != Phase::Closing && !self.output_invalidated && !self.keyboard_withdrawn {
            self.visible = false;
        }
        self.surface = None;
        self.phase = Phase::Absent;
        self.output_invalidated = false;
        self.keyboard_withdrawn = false;
        true
    }
    pub fn update_output(
        &mut self,
        output: WlOutput,
        info: Option<OutputInfo>,
        removed: bool,
    ) -> bool {
        let selected_removed = removed && self.selected.as_ref() == Some(&output);
        if removed {
            self.outputs.retain(|(id, _)| id != &output);
            self.output_invalidated |= selected_removed && self.surface.is_some();
            if selected_removed {
                self.selected = None;
            }
        } else if let Some(info) = info {
            if let Some((_, current)) = self.outputs.iter_mut().find(|(id, _)| id == &output) {
                *current = info;
            } else {
                self.outputs.push((output, info));
            }
        }
        selected_removed
    }
    pub fn settings(&mut self) -> Option<SctkLayerSurfaceSettings> {
        if self.phase != Phase::Absent || !self.visible {
            return None;
        }
        let (output, info) = self.outputs.iter().find(|(id, info)| {
            self.selected.as_ref().is_none_or(|selected| selected == id)
                && logical_size(info).is_some()
        })?;
        let (_, height) = logical_size(info)?;
        self.selected = Some(output.clone());
        let id = window::Id::unique();
        self.surface = Some(id);
        self.phase = Phase::Creating;
        Some(SctkLayerSurfaceSettings {
            // COSMIC grants exclusive focus on initial mapping; promoting an
            // already-mapped noninteractive layer does not acquire keyboard focus.
            id,
            layer: Layer::Overlay,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            output: IcedOutput::Output(output.clone()),
            namespace: "cosmos-applet-dropdown-terminal-backend".into(),
            // Respect reserved panel space so the applet remains clickable.
            exclusive_zone: 0,
            size: Some((None, Some(dropdown_height(height)))),
            size_limits: Limits::NONE.min_width(1.0).min_height(1.0),
            ..Default::default()
        })
    }
    pub fn height(&self) -> Option<u32> {
        let selected = self.selected.as_ref()?;
        self.outputs
            .iter()
            .find(|(output, _)| output == selected)
            .and_then(|(_, info)| logical_size(info))
            .map(|(_, height)| dropdown_height(height))
    }
}
fn dropdown_height(height: u32) -> u32 {
    ((u64::from(height) * 2 + 2) / 5).max(1) as u32
}
fn logical_size(info: &OutputInfo) -> Option<(u32, u32)> {
    if let Some((width, height)) = info.logical_size
        && width > 0
        && height > 0
    {
        return Some((width as u32, height as u32));
    }
    let mode = info.modes.iter().find(|mode| mode.current)?;
    let (mut width, mut height) = mode.dimensions;
    if matches!(
        info.transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    let scale = info.scale_factor.max(1);
    (width > 0 && height > 0).then_some((
        (width / scale).max(1) as u32,
        (height / scale).max(1) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn height_is_forty_percent_rounded_to_nearest_logical_pixel() {
        assert_eq!(
            [1080, 1440, 2160, 1, 1081].map(dropdown_height),
            [432, 576, 864, 1, 432]
        );
    }
    #[test]
    fn hidden_startup_has_no_surface_and_waits_for_output() {
        let mut dropdown = Dropdown::default();
        assert!(!dropdown.visible);
        assert!(dropdown.surface.is_none());
        dropdown.visible = true;
        assert!(dropdown.settings().is_none());
        dropdown.visible = false;
        assert!(dropdown.surface.is_none());
    }
    #[test]
    fn hide_during_creation_waits_for_open_acknowledgement() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            surface: Some(id),
            phase: Phase::Creating,
            ..Default::default()
        };
        dropdown.visible = false;
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), Some(id));
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.surface_closed(id));
        assert!(dropdown.surface.is_none());
        assert!(!dropdown.visible);
    }
    #[test]
    fn rapid_toggle_preserves_latest_show_until_close_acknowledgement() {
        let old = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            surface: Some(old),
            phase: Phase::Ready,
            ..Default::default()
        };
        dropdown.visible = false;
        assert_eq!(dropdown.begin_close(), Some(old));
        for _ in 0..19 {
            dropdown.visible = !dropdown.visible;
        }
        assert!(dropdown.visible);
        assert!(dropdown.settings().is_none());
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.surface_closed(old));
        assert!(dropdown.visible);
        assert!(dropdown.surface.is_none());
        let new = window::Id::unique();
        dropdown.surface = Some(new);
        dropdown.phase = Phase::Creating;
        assert!(!dropdown.surface_closed(old));
        assert!(dropdown.opened(new));
        assert!(dropdown.ready());
        assert_eq!(dropdown.begin_close(), None);
    }
    #[test]
    fn unexpected_compositor_close_hides_instead_of_reopening_forever() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            surface: Some(id),
            phase: Phase::Ready,
            ..Default::default()
        };
        assert!(dropdown.surface_closed(id));
        assert!(!dropdown.visible);
    }
    #[test]
    fn hide_then_show_during_creation_remaps_for_exclusive_focus() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            surface: Some(id),
            phase: Phase::Creating,
            ..Default::default()
        };
        dropdown.visible = false;
        dropdown.withdraw_keyboard();
        dropdown.visible = true;
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), Some(id));
        assert!(dropdown.surface_closed(id));
        assert!(dropdown.visible);
        assert!(!dropdown.keyboard_withdrawn);
    }
}
