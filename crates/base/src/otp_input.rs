use std::ops::Range;

use crate::{
    StyledExt as _,
    button::blur_when_disabled,
    input::{Backspace, Delete, MoveLeft, MoveRight, Paste, blink_cursor::BlinkCursor},
};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, ElementInputHandler, Empty,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Pixels, Point, Render, RenderOnce, SharedString,
    StyleRefinement, Styled, Subscription, UTF16Selection, Window, canvas, div,
    prelude::FluentBuilder as _,
};

/// A semantic notification from a one-time-code state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OtpEvent {
    /// A text edit committed a value.
    Change,
    /// A text edit filled the code.
    Complete,
    Focus,
    Blur,
}

/// Stateful input and focus behavior for a fixed-length numeric one-time code.
pub struct OtpState {
    focus_handle: FocusHandle,
    value: SharedString,
    blink_cursor: Entity<BlinkCursor>,
    masked: bool,
    length: usize,
    active_index: usize,
    disabled: bool,
    composition: Option<Composition>,
    input_bounds: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

struct Composition {
    text: String,
    marked: Range<usize>,
    selected: Range<usize>,
}

impl OtpState {
    pub fn new(length: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let blink_cursor = cx.new(|_| BlinkCursor::new());
        let subscriptions = vec![
            cx.observe(&blink_cursor, |_, _, cx| cx.notify()),
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() && this.focus_handle.is_focused(window) {
                    this.blink_cursor.update(cx, |cursor, cx| cursor.start(cx));
                }
            }),
            cx.on_focus(&focus_handle, window, Self::on_focus),
            cx.on_blur(&focus_handle, window, Self::on_blur),
        ];
        Self {
            focus_handle,
            value: SharedString::default(),
            blink_cursor,
            masked: false,
            length,
            active_index: 0,
            disabled: false,
            composition: None,
            input_bounds: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn default_value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = self.clean(&value.into()).into();
        self
    }

    pub fn set_value(
        &mut self,
        value: impl Into<SharedString>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.clean(&value.into());
        let composing = self.composition.take().is_some();
        if self.value.as_ref() != value || composing {
            self.value = value.into();
            cx.notify();
        }
    }

    pub fn value(&self) -> &SharedString {
        &self.value
    }
    pub fn len(&self) -> usize {
        self.length
    }
    /// The box selected for editing, kept when the value changes by code.
    pub fn active_index(&self) -> usize {
        self.active_index
    }
    /// The visible character for a box, including masking.
    pub fn display_char(&self, index: usize) -> Option<char> {
        self.value.as_bytes().get(index).map(|digit| {
            if self.masked {
                crate::input::MASK_CHAR
            } else {
                char::from(*digit)
            }
        })
    }
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }
    pub fn is_masked(&self) -> bool {
        self.masked
    }
    pub fn cursor_visible(&self, cx: &App) -> bool {
        self.blink_cursor.read(cx).visible()
    }
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }
    pub fn set_masked(&mut self, masked: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.masked = masked;
        cx.notify();
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
    }

    fn clean(&self, text: &str) -> String {
        text.chars()
            .filter(char::is_ascii_digit)
            .take(self.length)
            .collect()
    }

    fn select_box(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.length == 0 {
            return;
        }
        self.active_index = index.min(self.length - 1);
        self.composition = None;
        self.focus_handle.focus(window, cx);
        self.blink_cursor.update(cx, |cursor, cx| cursor.pause(cx));
        cx.notify();
    }

    fn commit(&mut self, value: String, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| cursor.pause(cx));
        self.value = value.into();
        cx.emit(OtpEvent::Change);
        if self.length > 0 && self.value.len() == self.length {
            cx.emit(OtpEvent::Complete);
        }
        cx.notify();
    }

    fn write_from(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let clean = self.clean(text);
        if clean.is_empty() || !self.accepts_text_input(window, cx) {
            return;
        }
        let mut chars: Vec<char> = self.value.chars().collect();
        chars.resize(self.length, ' ');
        for (slot, digit) in chars[self.active_index..].iter_mut().zip(clean.chars()) {
            *slot = digit;
        }
        let value = chars.into_iter().filter(char::is_ascii_digit).collect();
        let next = self.active_index.saturating_add(clean.len());
        self.select_box(next, window, cx);
        self.commit(value, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        let index = if self.active_index < self.value.len() {
            self.active_index
        } else {
            self.active_index.saturating_sub(1)
        };
        self.select_box(index, window, cx);
        self.delete(&Delete, window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_text_input(window, cx) {
            return;
        }
        self.composition = None;
        let mut value = self.value.to_string();
        if self.active_index < value.len() {
            value.remove(self.active_index);
        }
        self.commit(value, cx);
    }

    fn move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_box(self.active_index.saturating_sub(1), window, cx);
    }

    fn move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_box(self.active_index.saturating_add(1), window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            EntityInputHandler::paste(self, item, window, cx);
        }
    }

    fn input_text(&self) -> String {
        self.composition.as_ref().map_or_else(
            || {
                self.value
                    .as_bytes()
                    .get(self.active_index)
                    .map(|digit| char::from(*digit).to_string())
                    .unwrap_or_default()
            },
            |composition| composition.text.clone(),
        )
    }

    fn input_selection(&self) -> Range<usize> {
        self.composition.as_ref().map_or_else(
            || 0..usize::from(self.active_index < self.value.len()),
            |composition| composition.selected.clone(),
        )
    }

    fn replacement_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .or_else(|| {
                self.composition
                    .as_ref()
                    .map(|composition| composition.marked.clone())
            })
            .unwrap_or_else(|| self.input_selection())
    }

    fn on_focus(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| cursor.start(cx));
        cx.emit(OtpEvent::Focus);
    }
    fn on_blur(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition = None;
        self.blink_cursor.update(cx, |cursor, cx| cursor.stop(cx));
        cx.emit(OtpEvent::Blur);
    }
}

fn byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    let mut offset = 0;
    let mut start = text.len();
    let mut end = text.len();
    for (ix, ch) in text.char_indices() {
        if offset <= range.start {
            start = ix;
        }
        if offset >= range.end.max(range.start) {
            end = ix;
            break;
        }
        offset += ch.len_utf16();
        if offset <= range.start {
            start = ix + ch.len_utf8();
        }
    }
    start.min(end)..end
}

impl EntityInputHandler for OtpState {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.input_text();
        let range = byte_range(&text, range);
        *adjusted_range = Some(
            text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count(),
        );
        Some(text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        (ignore_disabled_input || self.accepts_text_input(window, cx)).then(|| UTF16Selection {
            range: self.input_selection(),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.composition
            .as_ref()
            .map(|composition| composition.marked.clone())
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(composition) = self.composition.take() {
            self.write_from(&composition.text, window, cx);
            cx.notify();
        }
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.composition = None;
            self.write_from(&text, window, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_text_input(window, cx) {
            return;
        }
        let mut value = self.input_text();
        let range = byte_range(&value, self.replacement_range(range));
        value.replace_range(range, text);
        let composing = self.composition.take().is_some();
        if value.is_empty() && !composing {
            self.delete(&Delete, window, cx);
        } else {
            self.write_from(&value, window, cx);
        }
        if composing {
            cx.notify();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_text_input(window, cx) {
            return;
        }
        let mut value = self.input_text();
        let range = byte_range(&value, self.replacement_range(range));
        let start = value[..range.start].encode_utf16().count();
        value.replace_range(range, text);
        let end = start + text.encode_utf16().count();
        let selected = selected.map_or(end..end, |range| {
            start + range.start.min(end - start)
                ..start + range.end.max(range.start).min(end - start)
        });
        self.composition = Some(Composition {
            text: value,
            marked: start..end,
            selected,
        });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.input_bounds?;
        bounds.contains(&point).then(|| {
            if point.x < bounds.center().x {
                0
            } else {
                self.input_text().encode_utf16().count()
            }
        })
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.input_text().encode_utf16().count())
    }

    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        !self.disabled && self.length > 0 && self.focus_handle.is_focused(window)
    }
}

impl Focusable for OtpState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl EventEmitter<OtpEvent> for OtpState {}
impl Render for OtpState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Unstyled OTP interaction root. Applications provide the visual cells as children.
#[derive(IntoElement)]
pub struct OtpInput {
    state: Entity<OtpState>,
    disabled: bool,
    style: StyleRefinement,
    children: Vec<AnyElement>,
}

impl OtpInput {
    pub fn new(state: &Entity<OtpState>) -> Self {
        Self {
            state: state.clone(),
            disabled: false,
            style: StyleRefinement::default(),
            children: vec![],
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Wires a styled box to pointer focus and native text input.
    pub fn cell<E>(&self, index: usize, cell: E) -> E
    where
        E: InteractiveElement + ParentElement + Styled,
    {
        let state = self.state.clone();
        let disabled = self.disabled;
        cell.relative()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !disabled {
                    state.update(cx, |state, cx| state.select_box(index, window, cx));
                }
            })
            .child(self.input_handler(Some(index)))
    }

    fn input_handler(&self, index: Option<usize>) -> impl IntoElement + use<> {
        let state = self.state.clone();
        let disabled = self.disabled;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let current = state.read(cx);
                if disabled
                    || current.length == 0
                    || index.is_some_and(|index| index != current.active_index)
                    || !current.focus_handle.is_focused(window)
                {
                    return;
                }
                let focus = current.focus_handle.clone();
                state.update(cx, |state, _| state.input_bounds = Some(bounds));
                window.handle_input(&focus, ElementInputHandler::new(bounds, state.clone()), cx);
            },
        )
        .absolute()
        .inset_0()
    }
}
impl Styled for OtpInput {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl ParentElement for OtpInput {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}
impl RenderOnce for OtpInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let input_handler = self.input_handler(None);
        let state = self.state;
        let focus_handle = state.read(cx).focus_handle.clone();
        state.update(cx, |state, _| {
            state.disabled = self.disabled;
            if self.disabled {
                state.composition = None;
            }
        });
        blur_when_disabled(&focus_handle, self.disabled, window, cx);
        div()
            .id(("base-otp-input", state.entity_id()))
            .relative()
            .when(!self.disabled && state.read(cx).length > 0, |this| {
                this.track_focus(&focus_handle.tab_stop(true))
                    .key_context("Input")
                    .on_action(window.listener_for(&state, OtpState::backspace))
                    .on_action(window.listener_for(&state, OtpState::delete))
                    .on_action(window.listener_for(&state, OtpState::move_left))
                    .on_action(window.listener_for(&state, OtpState::move_right))
                    .on_action(window.listener_for(&state, OtpState::paste))
            })
            .child(input_handler)
            .children(self.children)
            .refine_style(&self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::OtpState;
    use gpui::{AppContext as _, EntityInputHandler as _, TestAppContext};

    #[gpui::test]
    fn values_are_ascii_and_bounded_before_any_presentation(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            let state = cx.new(|cx| OtpState::new(4, window, cx).default_value("a1３2-345"));
            assert_eq!(state.read(cx).value().as_ref(), "1234");
            state.update(cx, |state, cx| {
                state.set_value("🙂６98a7", window, cx);
                assert_eq!(state.value().as_ref(), "987");
            });
        });
    }

    #[gpui::test]
    fn native_replacements_respect_cell_ranges_and_zero_length(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            for length in [0, 4] {
                let state = cx.new(|cx| OtpState::new(length, window, cx).default_value("12"));
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                    state.replace_text_in_range(Some(0..1), "83", window, cx);
                    assert_eq!(state.value().as_ref(), if length == 0 { "" } else { "83" });
                    if length > 0 {
                        state.replace_and_mark_text_in_range(None, "🙂9", Some(1..3), window, cx);
                        let mut adjusted = None;
                        assert_eq!(
                            state
                                .text_for_range(1..2, &mut adjusted, window, cx)
                                .as_deref(),
                            Some("🙂")
                        );
                        assert_eq!(adjusted, Some(0..2), "UTF-16 ranges keep whole characters");
                        state.set_value("4567", window, cx);
                        state.unmark_text(window, cx);
                        assert_eq!(
                            state.value().as_ref(),
                            "4567",
                            "external sync cancels the old composition"
                        );
                    }
                });
            }
        });
    }
}
