// SPDX-License-Identifier: GPL-3.0-only
//! Hiding the layer surface leaves terminal models, tabs and PTYs running.
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

pub struct Dropdown {
    pub visible: bool,
    pub surface: Option<window::Id>,
    phase: Phase,
    surface_invalidated: bool,
    outputs: Vec<(WlOutput, OutputInfo)>,
    selected: Option<WlOutput>,
    height_percent: u8,
    width_percent: u8,
    always_on_top: bool,
    // Show raises even an unpinned dropdown; losing focus lets other windows cover it.
    raised: bool,
    available_height: Option<u32>,
    configured_size: Option<Size>,
    measurement_margin: Option<u32>,
    panel_guards: usize,
    focus_loss_pending: bool,
}

impl Default for Dropdown {
    fn default() -> Self {
        Self {
            visible: false,
            surface: None,
            phase: Phase::Absent,
            surface_invalidated: false,
            outputs: Vec::new(),
            selected: None,
            height_percent: 40,
            width_percent: 100,
            always_on_top: true,
            raised: true,
            available_height: None,
            configured_size: None,
            measurement_margin: None,
            panel_guards: 0,
            focus_loss_pending: false,
        }
    }
}

impl Dropdown {
    pub fn configure(&mut self, height: u8, width: u8, always_on_top: bool) -> bool {
        let height = height.clamp(1, 100);
        let width = width.clamp(1, 100);
        let changed = (height, width, always_on_top)
            != (self.height_percent, self.width_percent, self.always_on_top);
        if changed {
            self.height_percent = height;
            self.width_percent = width;
            self.always_on_top = always_on_top;
        }
        changed
    }

    pub fn layer(&self) -> Layer {
        if self.always_on_top || self.raised {
            Layer::Top
        } else {
            Layer::Bottom
        }
    }

    pub fn set_raised(&mut self, raised: bool) -> bool {
        let previous = self.layer();
        self.raised = raised;
        self.layer() != previous
    }

    pub fn clear_focus_loss(&mut self) {
        self.focus_loss_pending = false;
    }

    pub fn focus_lost(&mut self) {
        self.focus_loss_pending = true;
    }

    pub fn panel_guard_acquired(&mut self) {
        self.panel_guards += 1;
    }

    pub fn panel_guard_released(&mut self) {
        debug_assert!(self.panel_guards > 0);
        self.panel_guards = self.panel_guards.saturating_sub(1);
    }

    pub fn take_focus_loss(&mut self) -> bool {
        self.panel_guards == 0 && std::mem::take(&mut self.focus_loss_pending)
    }

    pub fn begin_close(&mut self) -> Option<window::Id> {
        if self.phase != Phase::Ready || (self.visible && !self.surface_invalidated) {
            return None;
        }
        self.phase = Phase::Closing;
        self.surface
    }

    pub fn begin_create(&mut self, id: window::Id) {
        self.surface = Some(id);
        self.phase = Phase::Creating;
        self.configured_size = None;
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

    /// Only expose terminal content after this surface reaches its requested geometry.
    pub fn content_size(&self) -> Option<Size> {
        if !self.ready() || !self.visible || self.surface_invalidated {
            return None;
        }
        let size = self.configured_size?;
        matches_dimensions(size, self.dimensions()?).then_some(size)
    }

    pub fn needs_width_update(&self, width: u32) -> bool {
        self.configured_size
            .is_none_or(|size| size.width as u32 != width)
    }

    pub fn creating(&self) -> bool {
        self.phase == Phase::Creating
    }

    pub fn surface_closed(&mut self, id: window::Id) -> bool {
        if self.surface != Some(id) {
            return false;
        }
        if self.phase != Phase::Closing && !self.surface_invalidated {
            self.visible = false;
        }
        self.surface = None;
        self.phase = Phase::Absent;
        self.surface_invalidated = false;
        self.configured_size = None;
        self.measurement_margin = None;
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
                self.surface_invalidated |= self.surface.is_some();
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
            self.surface_invalidated |= self.surface.is_some();
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
                self.height_percent,
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

    pub fn settings(&mut self) -> Option<SctkLayerSurfaceSettings> {
        if self.phase != Phase::Absent || !self.visible {
            return None;
        }
        self.select_output();
        let (width, height) = self.dimensions()?;
        let bottom_margin = self.bottom_margin(height);
        self.measurement_margin = Some(bottom_margin as u32);
        Some(SctkLayerSurfaceSettings {
            id: window::Id::unique(),
            // Stacking does not change the dropdown into a managed window.
            layer: self.layer(),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            // Omitting horizontal anchors centers the requested width.
            anchor: Anchor::TOP | Anchor::BOTTOM,
            output: IcedOutput::Output(self.selected.as_ref()?.clone()),
            namespace: "cosmos-applet-dropdown-terminal-backend".into(),
            // Respect reserved panel space so the applet remains clickable.
            exclusive_zone: 0,
            // The initial configure measures usable height without resizing any PTY.
            // Reuse its margin on remap, then check for changed panel reservations.
            size: Some((Some(width), None)),
            margin: IcedMargin {
                bottom: bottom_margin,
                ..Default::default()
            },
            size_limits: Limits::NONE.min_width(1.0).min_height(1.0),
            ..Default::default()
        })
    }

    pub fn resized(&mut self, id: window::Id, size: Size) -> bool {
        if self.surface != Some(id)
            || !size.width.is_finite()
            || !size.height.is_finite()
            || size.width < 1.0
            || size.height < 1.0
        {
            return false;
        }
        self.configured_size = Some(size);
        let margin = self
            .measurement_margin
            .take()
            .or_else(|| self.available_height.is_none().then_some(0));
        if let Some(margin) = margin {
            let available_height = Some((size.height as u32).saturating_add(margin));
            let changed = self.available_height != available_height;
            self.available_height = available_height;
            return changed;
        }
        false
    }
}

fn matches_dimensions(size: Size, (width, height): (u32, u32)) -> bool {
    // Fractional scaling rounds physical buffers up before reporting logical size.
    size.width as u32 == width && size.height as u32 == height
}

fn percent_size(size: u32, percent: u8) -> u32 {
    ((u64::from(size) * u64::from(percent.clamp(1, 100)) + 50) / 100).max(1) as u32
}

fn surface_dimension(size: u32, percent: u8, minimum: u32, available: u32) -> u32 {
    percent_size(size, percent)
        .max(minimum)
        .min(size.min(available))
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
    fn measurement_and_stale_bounds_do_not_match_requested_content_geometry() {
        assert_eq!(
            [
                Size::new(1280.0, 768.0),
                Size::new(1280.0, 1280.0),
                Size::new(640.0, 320.0),
                Size::new(1280.0, 320.0),
            ]
            .map(|size| matches_dimensions(size, (1280, 320))),
            [false, false, false, true]
        );
    }

    #[test]
    fn fractional_scale_rounding_accepts_only_the_requested_logical_dimensions() {
        assert_eq!(
            [
                Size::new(1280.8, 320.8),
                Size::new(1281.0, 320.0),
                Size::new(1280.0, 319.8),
            ]
            .map(|size| matches_dimensions(size, (1280, 320))),
            [true, false, false]
        );
    }

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
    fn reserved_panel_space_keeps_full_height_on_screen() {
        let dropdown = Dropdown {
            available_height: Some(768),
            ..Default::default()
        };
        assert_eq!(dropdown.bottom_margin(320), 448);
        assert_eq!(dropdown.bottom_margin(768), 0);
        assert_eq!(dropdown.bottom_margin(800), 0);
    }

    #[test]
    fn configured_height_is_not_remeasured_as_screen_height_after_reopening() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id);
        assert!(dropdown.resized(id, Size::new(1280.0, 768.0)));
        assert!(dropdown.opened(id));
        assert!(!dropdown.resized(id, Size::new(1280.0, 320.0)));
        dropdown.visible = false;
        assert_eq!(dropdown.begin_close(), Some(id));
        assert!(dropdown.surface_closed(id));
        dropdown.visible = true;
        let next = window::Id::unique();
        dropdown.begin_create(next);
        assert_eq!(dropdown.configured_size, None);
        assert!(dropdown.needs_width_update(1280));
        assert!(!dropdown.resized(id, Size::new(1.0, 1.0)));
        assert_eq!(dropdown.configured_size, None);
        assert!(!dropdown.resized(next, Size::new(1280.0, 320.0)));
        assert_eq!(dropdown.configured_size, Some(Size::new(1280.0, 320.0)));
        assert!(!dropdown.needs_width_update(1280));
        assert!(dropdown.needs_width_update(640));
        assert_eq!(dropdown.bottom_margin(320), 448);
    }

    #[test]
    fn remap_rechecks_panel_space_without_measuring_terminal_height_as_usable_height() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            available_height: Some(768),
            measurement_margin: Some(448),
            ..Default::default()
        };
        dropdown.begin_create(id);
        assert!(dropdown.resized(id, Size::new(1280.0, 300.0)));
        assert_eq!(dropdown.available_height, Some(748));
        assert!(!dropdown.resized(id, Size::new(1280.0, 320.0)));
        assert_eq!(dropdown.available_height, Some(748));
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
        dropdown.begin_create(id);
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
        dropdown.begin_create(old);
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
        dropdown.begin_create(new);
        assert!(!dropdown.surface_closed(old));
        assert!(!dropdown.opened(old));
        assert!(dropdown.opened(new));
        assert_eq!(dropdown.begin_close(), None);
    }

    #[test]
    fn stacking_changes_keep_the_same_surface_and_visibility() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id);
        dropdown.configure(60, 75, false);
        assert_eq!(dropdown.begin_close(), None);
        assert!(dropdown.opened(id));
        assert_eq!(dropdown.begin_close(), None);
        assert_eq!(dropdown.layer(), Layer::Top);
        assert!(dropdown.set_raised(false));
        assert_eq!(dropdown.layer(), Layer::Bottom);
        dropdown.configure(30, 50, true);
        assert_eq!(dropdown.begin_close(), None);
        assert_eq!(dropdown.surface, Some(id));
        assert!(dropdown.visible);
        assert_eq!(dropdown.layer(), Layer::Top);
        assert!(!dropdown.set_raised(false));
        assert_eq!(dropdown.layer(), Layer::Top);
        dropdown.configure(30, 50, false);
        assert_eq!(dropdown.layer(), Layer::Bottom);
        assert!(dropdown.set_raised(true));
        assert_eq!(dropdown.layer(), Layer::Top);
        assert_eq!(dropdown.begin_close(), None);
    }

    #[test]
    fn show_during_creation_does_not_remap_for_keyboard_capture() {
        let id = window::Id::unique();
        let mut dropdown = Dropdown {
            visible: true,
            ..Default::default()
        };
        dropdown.begin_create(id);
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
        dropdown.begin_create(id);
        assert!(dropdown.opened(id));
        assert!(dropdown.surface_closed(id));
        assert!(!dropdown.visible);
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
    fn minimum_dimensions_do_not_exceed_reserved_space() {
        assert_eq!(surface_dimension(1920, 1, MIN_WIDTH, 1920), MIN_WIDTH);
        assert_eq!(surface_dimension(800, 1, MIN_HEIGHT, 768), MIN_HEIGHT);
        assert_eq!(surface_dimension(800, 100, MIN_HEIGHT, 768), 768);
        assert_eq!(surface_dimension(80, 1, MIN_HEIGHT, 64), 64);
    }

    #[test]
    fn hidden_settings_change_all_dropdown_keys_without_creating_a_surface() {
        let mut dropdown = Dropdown::default();
        assert!(dropdown.configure(65, 55, false));
        assert!(!dropdown.visible);
        assert!(dropdown.surface.is_none());
        assert_eq!(
            (
                dropdown.height_percent,
                dropdown.width_percent,
                dropdown.always_on_top
            ),
            (65, 55, false)
        );
        assert!(dropdown.settings().is_none());
    }

    #[test]
    fn panel_controls_defer_focus_loss_until_the_last_guard_disconnects() {
        let mut dropdown = Dropdown::default();
        dropdown.panel_guard_acquired();
        dropdown.panel_guard_acquired();
        dropdown.focus_lost();
        assert!(!dropdown.take_focus_loss());
        dropdown.panel_guard_released();
        assert!(!dropdown.take_focus_loss());
        dropdown.panel_guard_released();
        assert!(dropdown.take_focus_loss());
        assert!(!dropdown.take_focus_loss());
    }

    #[test]
    fn regained_focus_or_explicit_visibility_change_cancels_deferred_focus_loss() {
        let mut dropdown = Dropdown::default();
        dropdown.panel_guard_acquired();
        dropdown.focus_lost();
        dropdown.clear_focus_loss();
        dropdown.panel_guard_released();
        assert!(!dropdown.take_focus_loss());
    }

    #[test]
    fn unguarded_focus_loss_is_immediate_and_not_repeated() {
        let mut dropdown = Dropdown::default();
        dropdown.focus_lost();
        assert!(dropdown.take_focus_loss());
        assert!(!dropdown.take_focus_loss());
    }
}
