// SPDX-License-Identifier: GPL-3.0-only
//! Surface state is independent of terminal state. Replacing either surface role
//! leaves terminal models, tabs and PTY subscriptions in the application.
use cosmic::{
    cctk::sctk::{
        output::OutputInfo,
        reexports::client::protocol::wl_output::{Transform, WlOutput},
    },
    iced::{
        Limits, Size,
        platform_specific::{
            runtime::wayland::layer_surface::{IcedMargin, IcedOutput, SctkLayerSurfaceSettings},
            shell::commands::layer_surface::{Anchor, KeyboardInteractivity, Layer},
        },
        window,
    },
};

pub const MIN_WIDTH: u32 = 96;
pub const MIN_HEIGHT: u32 = 96;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Absent,
    Creating,
    Ready,
    Closing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceRole {
    #[default]
    Overlay,
    Window,
}

pub enum SurfaceSettings {
    Overlay(SctkLayerSurfaceSettings),
    Window(window::Settings),
}

#[derive(Clone, Copy, Debug)]
struct ResizeDrag {
    start_y: f32,
    last_y: f32,
    start_height: f32,
    output_height: u32,
    percent: u8,
}

pub struct Dropdown {
    pub visible: bool,
    pub surface: Option<window::Id>,
    phase: Phase,
    role: SurfaceRole,
    output_invalidated: bool,
    outputs: Vec<(WlOutput, OutputInfo)>,
    selected: Option<WlOutput>,
    height_percent: u8,
    width_percent: u8,
    always_on_top: bool,
    actual_size: Option<Size>,
    available_height: Option<u32>,
    drag: Option<ResizeDrag>,
    resize_cancelled: bool,
}

impl Default for Dropdown {
    fn default() -> Self {
        Self {
            visible: false,
            surface: None,
            phase: Phase::Absent,
            role: SurfaceRole::Overlay,
            output_invalidated: false,
            outputs: Vec::new(),
            selected: None,
            height_percent: 40,
            width_percent: 100,
            always_on_top: true,
            actual_size: None,
            available_height: None,
            drag: None,
            resize_cancelled: false,
        }
    }
}

impl Dropdown {
    pub fn configure(&mut self, height: u8, width: u8, always_on_top: bool) -> bool {
        let height = height.clamp(1, 100);
        let width = width.clamp(1, 100);
        let changed = (height, width, always_on_top)
            != (self.height_percent, self.width_percent, self.always_on_top)
            || self.drag.is_some();
        if changed {
            self.height_percent = height;
            self.width_percent = width;
            self.always_on_top = always_on_top;
            // Keep capturing until release, but never commit the interrupted preview.
            self.resize_cancelled |= self.drag.take().is_some();
        }
        changed
    }

    pub fn role(&self) -> SurfaceRole {
        self.role
    }

    fn requested_role(&self) -> SurfaceRole {
        if self.always_on_top {
            SurfaceRole::Overlay
        } else {
            SurfaceRole::Window
        }
    }

    pub fn begin_close(&mut self) -> Option<window::Id> {
        if self.phase != Phase::Ready
            || (self.visible && !self.output_invalidated && self.role == self.requested_role())
        {
            return None;
        }
        self.drag = None;
        self.resize_cancelled = false;
        self.phase = Phase::Closing;
        self.surface
    }

    pub fn begin_create(&mut self, id: window::Id, role: SurfaceRole) {
        self.surface = Some(id);
        self.role = role;
        self.phase = Phase::Creating;
        self.actual_size = None;
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

    pub fn surface_closed(&mut self, id: window::Id) -> bool {
        if self.surface != Some(id) {
            return false;
        }
        if self.phase != Phase::Closing && !self.output_invalidated {
            self.visible = false;
        }
        self.surface = None;
        self.phase = Phase::Absent;
        self.output_invalidated = false;
        self.actual_size = None;
        self.drag = None;
        self.resize_cancelled = false;
        true
    }

    pub fn update_output(
        &mut self,
        output: WlOutput,
        info: Option<OutputInfo>,
        removed: bool,
    ) -> bool {
        let selected = self.selected.as_ref() == Some(&output);
        let before = self.output_size();
        let previous_output = self.selected.clone();
        if removed {
            self.outputs.retain(|(id, _)| id != &output);
            if selected {
                self.output_invalidated |=
                    self.surface.is_some() && self.role == SurfaceRole::Overlay;
                self.selected = None;
            }
        } else if let Some(info) = info {
            if let Some((_, current)) = self.outputs.iter_mut().find(|(id, _)| id == &output) {
                *current = info;
            } else {
                self.outputs.push((output, info));
            }
        }
        self.select_output();
        if before != self.output_size() || self.selected != previous_output {
            self.available_height = None;
            self.output_invalidated |= self.surface.is_some() && self.role == SurfaceRole::Overlay;
        }
        if before != self.output_size()
            && let Some((_, height)) = self.output_size()
            && let Some(drag) = self.drag.as_mut()
        {
            drag.start_height = percent_size(height, drag.percent) as f32;
            drag.start_y = drag.last_y;
            drag.output_height = height;
        }
        before != self.output_size() || (removed && selected)
    }

    fn select_output(&mut self) {
        if self.output_size().is_none() {
            self.selected = self
                .outputs
                .iter()
                .find(|(_, info)| logical_size(info).is_some())
                .map(|(output, _)| output.clone());
        }
    }

    fn output_size(&self) -> Option<(u32, u32)> {
        let selected = self.selected.as_ref()?;
        self.outputs
            .iter()
            .find(|(output, _)| output == selected)
            .and_then(|(_, info)| logical_size(info))
    }

    pub fn dimensions(&self) -> Option<(u32, u32)> {
        let (width, height) = self.output_size()?;
        Some((
            surface_dimension(width, self.width_percent, MIN_WIDTH, width),
            surface_dimension(
                height,
                self.height_percent(),
                MIN_HEIGHT,
                self.available_height.unwrap_or(height),
            ),
        ))
    }

    pub fn bottom_margin(&self, height: u32) -> i32 {
        self.available_height
            .unwrap_or(height)
            .saturating_sub(height)
            .min(i32::MAX as u32) as i32
    }

    pub fn settings(&mut self) -> Option<SurfaceSettings> {
        if self.phase != Phase::Absent || !self.visible {
            return None;
        }
        self.select_output();
        let (width, height) = self.dimensions()?;
        if self.always_on_top {
            Some(SurfaceSettings::Overlay(SctkLayerSurfaceSettings {
                id: window::Id::unique(),
                // Above application windows without covering panel menus.
                layer: Layer::Top,
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                // Omitting horizontal anchors centers the requested width.
                anchor: Anchor::TOP | Anchor::BOTTOM,
                output: IcedOutput::Output(self.selected.as_ref()?.clone()),
                namespace: "cosmos-applet-dropdown-terminal-backend".into(),
                // Respect reserved panel space so the applet remains clickable.
                exclusive_zone: 0,
                // Measure usable height once, then reserve the unused space below.
                // Opposing anchors keep the grip on-screen even at 100%.
                size: Some((Some(width), None)),
                margin: IcedMargin {
                    bottom: self.bottom_margin(height),
                    ..Default::default()
                },
                size_limits: Limits::NONE.min_width(1.0).min_height(1.0),
                ..Default::default()
            }))
        } else {
            let mut settings = window::Settings {
                size: Size::new(width as f32, height as f32),
                min_size: Some(Size::new(
                    MIN_WIDTH.min(width) as f32,
                    MIN_HEIGHT.min(height) as f32,
                )),
                decorations: false,
                transparent: true,
                exit_on_close_request: false,
                ..Default::default()
            };
            settings.platform_specific.application_id = "dev.jp.CosmosDropdownTerminal".into();
            Some(SurfaceSettings::Window(settings))
        }
    }

    pub fn resized(&mut self, id: window::Id, size: Size) -> bool {
        if self.surface != Some(id) {
            return false;
        }
        self.actual_size = Some(size);
        if self.role == SurfaceRole::Window
            && let Some(drag) = &mut self.drag
        {
            drag.percent = resize_percent(size.height, drag.output_height);
        }
        if self.role == SurfaceRole::Overlay && self.available_height.is_none() {
            self.available_height = Some(size.height.max(1.0).round() as u32);
            return true;
        }
        false
    }

    pub fn height_percent(&self) -> u8 {
        self.drag.map_or(self.height_percent, |drag| drag.percent)
    }

    pub fn displayed_height_percent(&self) -> u8 {
        match (self.actual_size, self.output_size()) {
            (Some(size), Some((_, height))) => resize_percent(size.height, height),
            _ => self.height_percent(),
        }
    }

    pub fn resizing(&self) -> bool {
        self.drag.is_some() || self.resize_cancelled
    }

    pub fn start_resize(&mut self, id: window::Id, y: f32) {
        if self.surface != Some(id) || !self.ready() || !self.visible {
            return;
        }
        let Some((_, output_height)) = self.output_size() else {
            return;
        };
        let start_height = self.actual_size.map_or_else(
            || percent_size(output_height, self.height_percent) as f32,
            |size| size.height,
        );
        self.resize_cancelled = false;
        self.drag = Some(ResizeDrag {
            start_y: y,
            last_y: y,
            start_height,
            output_height,
            percent: self.height_percent,
        });
    }

    pub fn drag_resize(&mut self, id: window::Id, y: f32) -> bool {
        if self.surface != Some(id) || self.role == SurfaceRole::Window {
            return false;
        }
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let percent = resize_percent(drag.start_height + y - drag.start_y, drag.output_height);
        let changed = percent != drag.percent;
        drag.percent = percent;
        drag.last_y = y;
        changed
    }

    pub fn finish_resize(&mut self, id: window::Id) -> Option<u8> {
        if self.surface != Some(id) {
            return None;
        }
        self.resize_cancelled = false;
        let drag = self.drag.take()?;
        if drag.percent == self.height_percent {
            return None;
        }
        self.height_percent = drag.percent;
        Some(drag.percent)
    }
}

fn percent_size(size: u32, percent: u8) -> u32 {
    ((u64::from(size) * u64::from(percent.clamp(1, 100)) + 50) / 100).max(1) as u32
}

fn surface_dimension(size: u32, percent: u8, minimum: u32, available: u32) -> u32 {
    percent_size(size, percent)
        .max(minimum)
        .min(size.min(available))
}

fn resize_percent(height: f32, output_height: u32) -> u8 {
    (height.max(0.0) * 100.0 / output_height.max(1) as f32)
        .round()
        .clamp(1.0, 100.0) as u8
}

fn logical_size(info: &OutputInfo) -> Option<(u32, u32)> {
    if let Some((width, height)) = info.logical_size
        && width > 0
        && height > 0
    {
        return Some((width as u32, height as u32));
    }
    let mode = info.modes.iter().find(|mode| mode.current)?;
    logical_mode_size(mode.dimensions, info.transform, info.scale_factor)
}

fn logical_mode_size(
    (mut width, mut height): (i32, i32),
    transform: Transform,
    scale: i32,
) -> Option<(u32, u32)> {
    if matches!(
        transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    ) {
        std::mem::swap(&mut width, &mut height);
    }
    let scale = scale.max(1);
    (width > 0 && height > 0).then_some((
        (width / scale).max(1) as u32,
        (height / scale).max(1) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_round_and_clamp_without_zero_sized_surfaces() {
        assert_eq!(
            [1, 1081, 2160].map(|size| percent_size(size, 33)),
            [1, 357, 713]
        );
        assert_eq!(percent_size(1000, 0), 10);
        assert_eq!(percent_size(1000, 255), 1000);
    }

    #[test]
    fn reserved_panel_space_keeps_full_height_and_drag_grip_on_screen() {
        let dropdown = Dropdown {
            available_height: Some(768),
            ..Default::default()
        };
        assert_eq!(dropdown.bottom_margin(320), 448);
        assert_eq!(dropdown.bottom_margin(768), 0);
        assert_eq!(dropdown.bottom_margin(800), 0);
    }

    #[test]
    fn config_defaults_and_invalid_percentages_are_safe() {
        let mut dropdown = Dropdown::default();
        assert_eq!(
            (
                dropdown.height_percent,
                dropdown.width_percent,
                dropdown.requested_role()
            ),
            (40, 100, SurfaceRole::Overlay)
        );
        dropdown.configure(0, 255, false);
        assert_eq!(
            (
                dropdown.height_percent,
                dropdown.width_percent,
                dropdown.requested_role()
            ),
            (1, 100, SurfaceRole::Window)
        );
    }

    #[test]
    fn hidden_startup_has_no_surface_and_waits_for_output() {
        let mut dropdown = Dropdown::default();
        assert!(!dropdown.visible);
        assert!(dropdown.surface.is_none());
        dropdown.visible = true;
        assert!(dropdown.settings().is_none());
    }

    #[test]
    fn hide_during_creation_waits_for_open_acknowledgement() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Overlay);
        dropdown.visible = false;
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), Some(id));
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.surface_closed(id));
        assert!(!dropdown.visible);
    }

    #[test]
    fn rapid_toggle_preserves_latest_show_until_close_acknowledgement() {
        let old = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(old, SurfaceRole::Overlay);
        assert!(dropdown.opened(old));
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
        let new = window::Id::unique();
        dropdown.begin_create(new, SurfaceRole::Overlay);
        assert!(!dropdown.surface_closed(old));
        assert!(!dropdown.opened(old));
        assert!(dropdown.opened(new));
        assert_eq!(dropdown.begin_close(), None);
    }

    #[test]
    fn role_change_during_creation_replaces_only_after_acknowledgement() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Overlay);
        dropdown.configure(60, 75, false);
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), Some(id));
        dropdown.configure(30, 50, true);
        assert!(dropdown.surface_closed(id));
        assert!(dropdown.visible);
        assert_eq!(dropdown.requested_role(), SurfaceRole::Overlay);
    }

    #[test]
    fn show_during_creation_does_not_remap_for_keyboard_capture() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Overlay);
        dropdown.visible = false;
        dropdown.visible = true;
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), None);
    }

    #[test]
    fn unexpected_compositor_close_hides_instead_of_reopening_forever() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Window);
        assert!(dropdown.opened(id));
        assert!(dropdown.surface_closed(id));
        assert!(!dropdown.visible);
    }

    #[test]
    fn drag_previews_clamp_and_commit_only_on_matching_release() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Overlay);
        assert!(dropdown.opened(id));
        dropdown.drag = Some(ResizeDrag {
            start_y: 400.0,
            last_y: 400.0,
            start_height: 400.0,
            output_height: 1000,
            percent: 40,
        });
        assert!(dropdown.drag_resize(id, 750.0));
        assert_eq!(
            (dropdown.height_percent(), dropdown.height_percent),
            (75, 40)
        );
        assert_eq!(dropdown.finish_resize(window::Id::unique()), None);
        assert!(dropdown.resizing());
        assert!(dropdown.drag_resize(id, -50.0));
        assert_eq!(dropdown.height_percent(), 1);
        assert!(dropdown.drag_resize(id, 2000.0));
        assert_eq!(dropdown.finish_resize(id), Some(100));
        assert_eq!(dropdown.finish_resize(id), None);
        assert_eq!(dropdown.height_percent, 100);
    }

    #[test]
    fn mode_fallback_applies_output_rotation_and_scale() {
        assert_eq!(
            logical_mode_size((3840, 2160), Transform::_90, 2),
            Some((1080, 1920))
        );
        assert_eq!(
            logical_mode_size((3840, 2160), Transform::Flipped270, 2),
            Some((1080, 1920))
        );
        assert_eq!(
            logical_mode_size((1920, 1080), Transform::Normal, 0),
            Some((1920, 1080))
        );
        assert_eq!(logical_mode_size((0, 1080), Transform::Normal, 1), None);
    }

    #[test]
    fn saved_geometry_cancels_drag_and_cannot_be_overwritten_by_its_release() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id, SurfaceRole::Overlay);
        assert!(dropdown.opened(id));
        dropdown.drag = Some(ResizeDrag {
            start_y: 400.0,
            last_y: 400.0,
            start_height: 400.0,
            output_height: 1000,
            percent: 40,
        });
        assert!(dropdown.drag_resize(id, 600.0));
        assert!(dropdown.configure(50, 80, true));
        assert!(dropdown.resizing());
        assert_eq!(dropdown.height_percent(), 50);
        assert!(!dropdown.drag_resize(id, 800.0));
        assert_eq!(dropdown.finish_resize(id), None);
        assert!(!dropdown.resizing());
        assert_eq!(dropdown.width_percent, 80);
    }

    #[test]
    fn minimum_dimensions_keep_grip_accessible_without_exceeding_reserved_space() {
        assert_eq!(surface_dimension(1920, 1, MIN_WIDTH, 1920), MIN_WIDTH);
        assert_eq!(surface_dimension(800, 1, MIN_HEIGHT, 768), MIN_HEIGHT);
        assert_eq!(surface_dimension(800, 100, MIN_HEIGHT, 768), 768);
        assert_eq!(surface_dimension(80, 1, MIN_HEIGHT, 64), 64);
    }

    #[test]
    fn saving_existing_geometry_also_cancels_preview() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            surface: Some(id),
            drag: Some(ResizeDrag {
                start_y: 400.0,
                last_y: 600.0,
                start_height: 400.0,
                output_height: 1000,
                percent: 60,
            }),
            ..Default::default()
        };
        assert!(dropdown.configure(40, 100, true));
        assert_eq!(dropdown.finish_resize(id), None);
        assert_eq!(dropdown.height_percent(), 40);
    }

    #[test]
    fn hidden_settings_change_all_dropdown_keys_without_creating_a_surface() {
        let mut dropdown = Dropdown::default();
        assert!(dropdown.configure(65, 55, false));
        assert!(!dropdown.visible);
        assert!(dropdown.surface.is_none());
        assert_eq!(
            (
                dropdown.height_percent(),
                dropdown.width_percent,
                dropdown.requested_role()
            ),
            (65, 55, SurfaceRole::Window)
        );
        assert!(dropdown.settings().is_none());
    }

    #[test]
    fn native_resize_commits_compositor_size_not_requested_pointer_delta() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            surface: Some(id),
            role: SurfaceRole::Window,
            drag: Some(ResizeDrag {
                start_y: 400.0,
                last_y: 400.0,
                start_height: 400.0,
                output_height: 1000,
                percent: 40,
            }),
            ..Default::default()
        };
        assert!(!dropdown.drag_resize(id, 900.0));
        dropdown.resized(id, Size::new(800.0, 600.0));
        assert_eq!(dropdown.finish_resize(id), Some(60));
    }
}
