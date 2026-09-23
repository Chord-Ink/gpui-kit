use std::ops::Range;

use gpui::{
    AccessibleAction, Along, AnyElement, App, AppContext as _, Axis, Bounds, Context,
    DispatchPhase, Div, ElementId, Entity, EventEmitter, FocusHandle, GlobalElementId,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Orientation, ParentElement, Pixels, Point, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, canvas, div,
    prelude::FluentBuilder as _, px,
};

use crate::{element_ext::ElementExt, geometry::AxisExt};

/// Events emitted by the [`SliderState`].
pub enum SliderEvent {
    /// Emitted continuously while the slider value is being changed by the user.
    Change(SliderValue),
    /// Emitted once when the user releases the slider after a drag or click.
    Release(SliderValue),
}

/// The value of the slider, can be a single value or a range of values.
///
/// - Can from a f32 value, which will be treated as a single value.
/// - Or from a (f32, f32) tuple, which will be treated as a range of values.
///
/// The default value is `SliderValue::Single(0.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliderValue {
    Single(f32),
    Range(f32, f32),
}

impl std::fmt::Display for SliderValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SliderValue::Single(value) => write!(f, "{}", value),
            SliderValue::Range(start, end) => write!(f, "{}..{}", start, end),
        }
    }
}

impl From<f32> for SliderValue {
    fn from(value: f32) -> Self {
        SliderValue::Single(value)
    }
}

impl From<(f32, f32)> for SliderValue {
    fn from(value: (f32, f32)) -> Self {
        SliderValue::Range(value.0, value.1)
    }
}

impl From<Range<f32>> for SliderValue {
    fn from(value: Range<f32>) -> Self {
        SliderValue::Range(value.start, value.end)
    }
}

impl Default for SliderValue {
    fn default() -> Self {
        SliderValue::Single(0.)
    }
}

impl SliderValue {
    /// Clamp the value to the given range.
    pub fn clamp(self, min: f32, max: f32) -> Self {
        match self {
            SliderValue::Single(value) => SliderValue::Single(value.clamp(min, max)),
            SliderValue::Range(start, end) => SliderValue::Range(
                start.min(end).clamp(min, max),
                start.max(end).clamp(min, max),
            ),
        }
    }

    /// Check if the value is a single value.
    #[inline]
    pub fn is_single(&self) -> bool {
        matches!(self, SliderValue::Single(_))
    }

    /// Check if the value is a range of values.
    #[inline]
    pub fn is_range(&self) -> bool {
        matches!(self, SliderValue::Range(_, _))
    }

    /// Get the start value.
    pub fn start(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(start, _) => *start,
        }
    }

    /// Get the end value.
    pub fn end(&self) -> f32 {
        match self {
            SliderValue::Single(value) => *value,
            SliderValue::Range(_, end) => *end,
        }
    }

    fn set_start(&mut self, value: f32) {
        if let SliderValue::Range(_, end) = self {
            *self = SliderValue::Range(value.min(*end), *end);
        } else {
            *self = SliderValue::Single(value);
        }
    }

    fn set_end(&mut self, value: f32) {
        if let SliderValue::Range(start, _) = self {
            *self = SliderValue::Range(*start, value.max(*start));
        } else {
            *self = SliderValue::Single(value);
        }
    }
}

/// The scale mode of the slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliderScale {
    /// Linear scale where values change uniformly across the slider range.
    /// This is the default mode.
    #[default]
    Linear,
    /// Logarithmic scale where the distance between values increases exponentially.
    ///
    /// This is useful for parameters that have a large range of values where smaller
    /// changes are more significant at lower values. Common examples include:
    ///
    /// - Volume controls (human hearing perception is logarithmic)
    /// - Frequency controls (musical notes follow a logarithmic scale)
    /// - Zoom levels
    /// - Any parameter where you want finer control at lower values
    ///
    /// # For example
    ///
    /// ```
    /// use gpui_base::slider::{SliderScale, SliderState};
    ///
    /// let slider = SliderState::new()
    ///     .min(1.0)    // Must be > 0 for logarithmic scale
    ///     .max(1000.0)
    ///     .scale(SliderScale::Logarithmic);
    /// ```
    ///
    /// - Moving the slider 1/3 of the way will yield ~10
    /// - Moving it 2/3 of the way will yield ~100
    /// - The full range covers 3 orders of magnitude evenly
    Logarithmic,
}

impl SliderScale {
    #[inline]
    pub fn is_linear(&self) -> bool {
        matches!(self, SliderScale::Linear)
    }

    #[inline]
    pub fn is_logarithmic(&self) -> bool {
        matches!(self, SliderScale::Logarithmic)
    }
}

/// State of the [`Slider`].
pub struct SliderState {
    min: f32,
    max: f32,
    step: f32,
    value: SliderValue,
    /// When is single value mode, only `end` is used, the start is always 0.0.
    percentage: Range<f32>,
    /// The bounds of the slider after rendered.
    bounds: Bounds<Pixels>,
    scale: SliderScale,
    /// Tracks whether the user is currently interacting with the slider so we
    /// only emit [`SliderEvent::Release`] after a real press/drag.
    dragging: bool,
}

impl SliderState {
    /// Create a new [`SliderState`].
    pub fn new() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
            value: SliderValue::default(),
            percentage: (0.0..0.0),
            bounds: Bounds::default(),
            scale: SliderScale::default(),
            dragging: false,
        }
    }

    /// Set the minimum value of the slider, default: 0.0
    pub fn min(mut self, min: f32) -> Self {
        if self.scale.is_logarithmic() {
            assert!(
                min > 0.0,
                "`min` must be greater than 0 for SliderScale::Logarithmic"
            );
            assert!(
                min < self.max,
                "`min` must be less than `max` for Logarithmic scale"
            );
        }
        self.min = min;
        self.update_thumb_pos();
        self
    }

    /// Set the maximum value of the slider, default: 100.0
    pub fn max(mut self, max: f32) -> Self {
        if self.scale.is_logarithmic() {
            assert!(
                max > self.min,
                "`max` must be greater than `min` for Logarithmic scale"
            );
        }
        self.max = max;
        self.update_thumb_pos();
        self
    }

    /// Set the step value of the slider, default: 1.0
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Set the scale of the slider, default: [`SliderScale::Linear`].
    pub fn scale(mut self, scale: SliderScale) -> Self {
        if scale.is_logarithmic() {
            assert!(
                self.min > 0.0,
                "`min` must be greater than 0 for Logarithmic scale"
            );
            assert!(
                self.max > self.min,
                "`max` must be greater than `min` for Logarithmic scale"
            );
        }
        self.scale = scale;
        self.update_thumb_pos();
        self
    }

    /// Set the default value of the slider, default: 0.0
    pub fn default_value(mut self, value: impl Into<SliderValue>) -> Self {
        self.value = value.into();
        self.update_thumb_pos();
        self
    }

    /// Set the value of the slider.
    pub fn set_value(
        &mut self,
        value: impl Into<SliderValue>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.value = value.into();
        self.update_thumb_pos();
        cx.notify();
    }

    /// Get the value of the slider.
    pub fn value(&self) -> SliderValue {
        self.value
    }

    /// Get the minimum value.
    pub fn min_value(&self) -> f32 {
        self.min
    }

    /// Get the maximum value.
    pub fn max_value(&self) -> f32 {
        self.max
    }

    /// Get the step value.
    pub fn step_value(&self) -> f32 {
        self.step
    }

    /// Converts a value between 0.0 and 1.0 to a value between the minimum and maximum value,
    /// depending on the chosen scale.
    fn percentage_to_value(&self, percentage: f32) -> f32 {
        match self.scale {
            SliderScale::Linear => self.min + (self.max - self.min) * percentage,
            SliderScale::Logarithmic => {
                // when percentage is 0, this simplifies to (max/min)^0 * min = 1 * min = min
                // when percentage is 1, this simplifies to (max/min)^1 * min = (max*min)/min = max
                // we clamp just to make sure we don't have issue with floating point precision
                let base = self.max / self.min;
                (base.powf(percentage) * self.min).clamp(self.min, self.max)
            }
        }
    }

    /// Converts a value between the minimum and maximum value to a value between 0.0 and 1.0,
    /// depending on the chosen scale.
    fn value_to_percentage(&self, value: f32) -> f32 {
        match self.scale {
            SliderScale::Linear => {
                let range = self.max - self.min;
                if range <= 0.0 {
                    0.0
                } else {
                    (value - self.min) / range
                }
            }
            SliderScale::Logarithmic => {
                let base = self.max / self.min;
                (value / self.min).log(base).clamp(0.0, 1.0)
            }
        }
    }

    fn update_thumb_pos(&mut self) {
        self.value = self.value.clamp(self.min, self.max);
        match self.value {
            SliderValue::Single(value) => {
                let percentage = self.value_to_percentage(value.clamp(self.min, self.max));
                self.percentage = 0.0..percentage;
            }
            SliderValue::Range(start, end) => {
                let clamped_start = start.clamp(self.min, self.max);
                let clamped_end = end.clamp(self.min, self.max);
                self.percentage =
                    self.value_to_percentage(clamped_start)..self.value_to_percentage(clamped_end);
            }
        }
    }

    /// Update value by mouse position
    #[doc(hidden)]
    pub fn update_value_by_position(
        &mut self,
        axis: Axis,
        position: Point<Pixels>,
        is_start: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dragging = true;
        let bounds = self.bounds;
        let step = self.step;

        let inner_pos = if axis.is_horizontal() {
            position.x - bounds.left()
        } else {
            bounds.bottom() - position.y
        };
        let total_size = bounds.size.along(axis);
        if total_size <= px(0.) {
            return;
        }
        let percentage = inner_pos.clamp(px(0.), total_size) / total_size;

        let percentage = if is_start {
            percentage.clamp(0.0, self.percentage.end)
        } else {
            percentage.clamp(self.percentage.start, 1.0)
        };

        let value = self.percentage_to_value(percentage);
        let max_steps = (self.max - self.min) / step;
        let last_step = if (max_steps - max_steps.round()).abs() <= f32::EPSILON * max_steps.abs() {
            max_steps.round()
        } else {
            max_steps.floor()
        };
        let steps = ((value - self.min) / step).round().clamp(0., last_step);
        let value = (self.min + steps * step).min(self.max);

        let before = self.value;
        if is_start {
            self.value.set_start(value);
        } else {
            self.value.set_end(value);
        }
        self.update_thumb_pos();
        if self.value != before {
            cx.emit(SliderEvent::Change(self.value));
            cx.notify();
        }
    }

    fn step_thumb(&mut self, start: bool, forward: bool, cx: &mut Context<Self>) {
        let before = self.value;
        let current = if start { before.start() } else { before.end() };
        let step = if forward { self.step } else { -self.step };
        let next = (current + step).clamp(self.min, self.max);
        if start {
            self.value.set_start(next);
        } else {
            self.value.set_end(next);
        }
        self.update_thumb_pos();
        if self.value != before {
            cx.emit(SliderEvent::Change(self.value));
            cx.notify();
        }
        cx.emit(SliderEvent::Release(self.value));
    }

    /// Emit [`SliderEvent::Release`] if the user was actively interacting
    /// with the slider. Called on mouse-up both inside and outside the slider.
    #[doc(hidden)]
    pub fn handle_release(&mut self, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        cx.emit(SliderEvent::Release(self.value));
    }
}

/// An unstyled slider behavior root.
///
/// Applications provide the track, range, and thumb presentation as children.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    axis: Axis,
    disabled: bool,
    label: Option<SharedString>,
    base: Div,
    children: Vec<AnyElement>,
}

impl Slider {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            disabled: false,
            label: None,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.axis = Axis::Vertical;
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl ParentElement for Slider {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let entity_id = self.state.entity_id();
        if self.disabled {
            self.state.update(cx, |state, _| state.dragging = false);
        }

        self.base
            .id(("slider", entity_id))
            .role(Role::Group)
            .when_some(self.label, |this, label| this.aria_label(label))
            .when(!self.disabled, |this| {
                this.on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&self.state, |state, _, _, cx| state.handle_release(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    window.listener_for(&self.state, |state, _, _, cx| state.handle_release(cx)),
                )
            })
            .children(self.children)
    }
}

/// An unstyled track that records the geometry used to map pointer positions.
#[derive(IntoElement)]
pub struct SliderTrack {
    state: Entity<SliderState>,
    axis: Axis,
    disabled: bool,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderTrack {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            disabled: false,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl ParentElement for SliderTrack {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderTrack {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderTrack {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for SliderTrack {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let id = self
            .base
            .interactivity()
            .element_id
            .clone()
            .unwrap_or_else(|| "slider-bar-container".into());
        let click_bounds =
            window.with_global_id(id.clone(), |id, window| geometry_at(id, window, cx));
        let pointer = pointer_events(self.state.clone(), click_bounds.clone(), axis);
        click_bounds.update(cx, |track, _| {
            track.focus = [None, None];
            if self.disabled {
                track.pressed = None;
            }
        });
        self.base
            .id(id)
            .children(self.children)
            .when(!self.disabled, |this| {
                this.child(pointer).on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(
                        &self.state,
                        move |state, event: &MouseDownEvent, window, cx| {
                            state.set_bounds(click_bounds.read(cx).bounds);
                            let is_start = if state.value().is_range() {
                                let size = state.bounds().size.along(axis);
                                let position = if axis.is_horizontal() {
                                    event.position.x - state.bounds().left()
                                } else {
                                    state.bounds().bottom() - event.position.y
                                };
                                let percentage = state.percentage();
                                let center = ((percentage.end - percentage.start) / 2.
                                    + percentage.start)
                                    * size;
                                position < center
                            } else {
                                false
                            };
                            let focus = click_bounds.update(cx, |track, _| {
                                track.pressed = Some(ThumbPress {
                                    start: is_start,
                                    offset: px(0.),
                                });
                                track.focus[usize::from(!is_start)].clone()
                            });
                            if let Some(focus) = focus {
                                focus.focus(window, cx);
                                window.prevent_default();
                            }
                            state.update_value_by_position(
                                axis,
                                event.position,
                                is_start,
                                window,
                                cx,
                            );
                        },
                    ),
                )
            })
    }
}

/// An unstyled slider indicator that records the value-mapping bounds.
#[derive(IntoElement)]
pub struct SliderIndicator {
    state: Entity<SliderState>,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderIndicator {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            base: div(),
            children: Vec::new(),
        }
    }
}

impl ParentElement for SliderIndicator {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderIndicator {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderIndicator {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for SliderIndicator {}

#[derive(Default)]
struct TrackState {
    bounds: Bounds<Pixels>,
    focus: [Option<FocusHandle>; 2],
    pressed: Option<ThumbPress>,
}

#[derive(Clone, Copy)]
struct ThumbPress {
    start: bool,
    offset: Pixels,
}

fn pointer_events(
    state: Entity<SliderState>,
    geometry: Entity<TrackState>,
    axis: Axis,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let move_state = state.clone();
            let move_geometry = geometry.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.pressed_button != Some(MouseButton::Left)
                {
                    return;
                }
                let track = move_geometry.read(cx);
                let Some(press) = track.pressed else { return };
                let bounds = track.bounds;
                let mut position = event.position;
                if axis.is_horizontal() {
                    position.x -= press.offset;
                } else {
                    position.y -= press.offset;
                }
                move_state.update(cx, |state, cx| {
                    if state.dragging {
                        state.set_bounds(bounds);
                        state.update_value_by_position(axis, position, press.start, window, cx);
                    }
                });
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    let held = geometry.update(cx, |track, _| track.pressed.take().is_some());
                    if held {
                        state.update(cx, |state, cx| state.handle_release(cx));
                    }
                }
            });
        },
    )
    .absolute()
    .size_full()
}

fn geometry_at(id: &GlobalElementId, window: &mut Window, cx: &mut App) -> Entity<TrackState> {
    window.with_element_state(id, |state: Option<Entity<TrackState>>, _| {
        let state = state.unwrap_or_else(|| cx.new(|_| TrackState::default()));
        (state.clone(), state)
    })
}

fn track_geometry(window: &mut Window, cx: &mut App) -> Option<Entity<TrackState>> {
    window.with_global_id("slider-geometry".into(), |path, window| {
        let track_type = std::any::type_name::<SliderTrack>();
        let index = path
            .iter()
            .rposition(|id| matches!(id, ElementId::Name(name) if name.as_ref() == track_type))?;
        let mut id = path.clone();
        *id = (&path[..index + 2]).into();
        Some(geometry_at(&id, window, cx))
    })
}

impl RenderOnce for SliderIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let geometry = track_geometry(window, cx);
        self.base
            .id("slider-bar")
            .children(self.children)
            .on_prepaint({
                let state = self.state;
                move |bounds, _, cx| {
                    if let Some(geometry) = geometry {
                        geometry.update(cx, |geometry, _| geometry.bounds = bounds);
                    }
                    state.update(cx, |state, _| state.set_bounds(bounds));
                }
            })
    }
}

/// An unstyled draggable slider thumb.
#[derive(IntoElement)]
pub struct SliderThumb {
    state: Entity<SliderState>,
    axis: Axis,
    start: bool,
    disabled: bool,
    focus: Option<FocusHandle>,
    base: Div,
    children: Vec<AnyElement>,
}

impl SliderThumb {
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            state: state.clone(),
            axis: Axis::Horizontal,
            start: false,
            disabled: false,
            focus: None,
            base: div(),
            children: Vec::new(),
        }
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }
    pub fn start(mut self, start: bool) -> Self {
        self.start = start;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
}

impl ParentElement for SliderThumb {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for SliderThumb {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for SliderThumb {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for SliderThumb {}

impl RenderOnce for SliderThumb {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let start = self.start;
        let drag_bounds = track_geometry(window, cx);
        let standalone = drag_bounds.is_none();
        let drag_bounds = drag_bounds.unwrap_or_else(|| {
            window.use_keyed_state(("slider-thumb-drag", u32::from(start)), cx, |_, _| {
                TrackState::default()
            })
        });
        let focus = self.focus.unwrap_or_else(|| {
            window
                .use_keyed_state(("slider-thumb-focus", u32::from(start)), cx, |_, cx| {
                    cx.focus_handle().tab_stop(true)
                })
                .read(cx)
                .clone()
        });
        crate::button::blur_when_disabled(&focus, self.disabled, window, cx);
        let fallback_bounds = self.state.read(cx).bounds();
        drag_bounds.update(cx, |track, _| {
            track.focus[usize::from(!start)] = (!self.disabled).then(|| focus.clone());
            if standalone {
                track.bounds = fallback_bounds;
            }
            if self.disabled && track.pressed.is_some_and(|press| press.start == start) {
                track.pressed = None;
            }
        });
        let pointer = pointer_events(self.state.clone(), drag_bounds.clone(), axis);
        let state = self.state.read(cx);
        let value = state.value();
        let low = if value.is_range() && !start {
            value.start()
        } else {
            state.min
        };
        let high = if value.is_range() && start {
            value.end()
        } else {
            state.max
        };
        self.base
            .id(("slider-thumb", start as u32))
            .role(Role::Slider)
            .aria_numeric_value(if start { value.start() } else { value.end() } as f64)
            .aria_min_numeric_value(low as f64)
            .aria_max_numeric_value(high as f64)
            .aria_numeric_value_step(state.step as f64)
            .aria_orientation(if axis.is_vertical() {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            })
            .children(self.children)
            .when(!self.disabled, |this| {
                this.when(standalone, |this| this.child(pointer))
                    .track_focus(&focus)
                    .on_a11y_action(AccessibleAction::Increment, {
                        let state = self.state.clone();
                        move |_, _, cx| {
                            state.update(cx, |state, cx| state.step_thumb(start, true, cx))
                        }
                    })
                    .on_a11y_action(AccessibleAction::Decrement, {
                        let state = self.state.clone();
                        move |_, _, cx| {
                            state.update(cx, |state, cx| state.step_thumb(start, false, cx))
                        }
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        window.listener_for(
                            &self.state,
                            move |state, event: &MouseDownEvent, window, cx| {
                                focus.focus(window, cx);
                                state.dragging = true;
                                let percentage = if start {
                                    state.percentage.start
                                } else {
                                    state.percentage.end
                                };
                                drag_bounds.update(cx, |track, _| {
                                    let center = if axis.is_horizontal() {
                                        track.bounds.left() + track.bounds.size.width * percentage
                                    } else {
                                        track.bounds.bottom()
                                            - track.bounds.size.height * percentage
                                    };
                                    track.pressed = Some(ThumbPress {
                                        start,
                                        offset: event.position.along(axis) - center,
                                    });
                                });
                                cx.stop_propagation();
                            },
                        ),
                    )
            })
    }
}

impl EventEmitter<SliderEvent> for SliderState {}

impl SliderState {
    #[doc(hidden)]
    pub fn percentage(&self) -> Range<f32> {
        self.percentage.clone()
    }

    #[doc(hidden)]
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    #[doc(hidden)]
    pub fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_value_conversions_and_clamping_are_preserved() {
        assert_eq!(SliderValue::from(5.), SliderValue::Single(5.));
        assert_eq!(SliderValue::from((2., 8.)), SliderValue::Range(2., 8.));
        assert_eq!(SliderValue::from(2.0..8.0), SliderValue::Range(2., 8.));
        assert_eq!(
            SliderValue::Range(-1., 12.).clamp(0., 10.),
            SliderValue::Range(0., 10.)
        );
    }

    #[test]
    fn legacy_linear_state_keeps_percentage_and_range_ordering() {
        let state = SliderState::new()
            .min(0.)
            .max(200.)
            .default_value((50., 150.));
        assert_eq!(state.value(), SliderValue::Range(50., 150.));
        assert_eq!(state.percentage(), 0.25..0.75);
    }

    #[test]
    fn legacy_logarithmic_state_keeps_mapping() {
        let state = SliderState::new()
            .min(1.)
            .max(1000.)
            .scale(SliderScale::Logarithmic)
            .default_value(10.);
        let percentage = state.percentage().end;
        assert!((percentage - (1. / 3.)).abs() < 0.0001);
    }

    #[test]
    #[should_panic(expected = "`min` must be greater than 0")]
    fn legacy_logarithmic_validation_is_preserved() {
        let _ = SliderState::new().scale(SliderScale::Logarithmic);
    }
}
