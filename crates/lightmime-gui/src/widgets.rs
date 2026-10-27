//! The few controls the window needs that gpui does not provide.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle, KeyDownEvent, MouseButton,
    Pixels, SharedString, Stateful, Window, canvas, div, prelude::*, px, relative,
};

use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonKind {
    Primary,
    Plain,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    enabled: bool,
    theme: Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (bg, fg, border) = match kind {
        ButtonKind::Primary => (theme.accent, theme.accent_text, theme.accent),
        ButtonKind::Plain => (theme.surface, theme.text, theme.border),
    };
    let base = div()
        .id(id)
        .px_3()
        .py_1p5()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .flex()
        .justify_center()
        .child(label.into());
    if enabled {
        base.cursor_pointer()
            .hover(|style| style.opacity(0.85))
            .on_click(on_click)
    } else {
        base.opacity(0.45)
    }
}

/// A single-line text field. Enough for a host name: typing, backspace and
/// paste; no cursor movement or selection.
pub struct TextInput {
    focus: FocusHandle,
    text: String,
    placeholder: SharedString,
}

impl TextInput {
    pub fn new(text: String, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text,
            placeholder: placeholder.into(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.text = text;
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.control || keystroke.modifiers.platform {
            if keystroke.key == "v"
                && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
            {
                self.text.push_str(text.trim());
            }
        } else if keystroke.key == "backspace" {
            self.text.pop();
        } else if let Some(typed) = &keystroke.key_char
            && !typed.chars().any(char::is_control)
        {
            self.text.push_str(typed);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window, cx);
        let focused = self.focus.is_focused(window);
        let empty = self.text.is_empty();
        div()
            .id("text-input")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .flex_1()
            .px_2()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .bg(theme.surface)
            .text_sm()
            .cursor_text()
            .flex()
            .child(if empty {
                div()
                    .text_color(theme.muted)
                    .child(self.placeholder.clone())
            } else {
                div().text_color(theme.text).child(self.text.clone())
            })
            .when(focused, |this| {
                this.child(
                    div()
                        .w(px(1.5))
                        .h(px(16.))
                        .bg(theme.accent)
                        .when(empty, |caret| caret.ml(px(-1.))),
                )
            })
    }
}

/// Half the slider knob's width.
const KNOB: Pixels = px(8.);

/// A horizontal slider over 0..1. The caller keeps `bounds` between renders so
/// a click can be turned into a value.
pub fn slider(
    id: impl Into<ElementId>,
    fraction: f32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    theme: Theme,
    on_change: impl Fn(&f32, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let fraction = fraction.clamp(0.0, 1.0);
    let on_change = Rc::new(on_change);
    let value_at = {
        let bounds = bounds.clone();
        move |x: Pixels| {
            // The knob's centre travels between the insets, so the ends of
            // the track stay reachable and the knob never overhangs it.
            let b = bounds.get();
            let travel = b.size.width - KNOB * 2.;
            if travel <= px(0.) {
                return None;
            }
            Some(((x - b.origin.x - KNOB) / travel).clamp(0.0, 1.0))
        }
    };
    let value_at_move = value_at.clone();
    let on_change_move = on_change.clone();

    div()
        .id(id)
        .relative()
        .h(px(22.))
        .w_full()
        .flex()
        .items_center()
        .cursor_pointer()
        .child(
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .child(
            div()
                .h(px(6.))
                .w_full()
                .rounded_full()
                .bg(theme.border)
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction))
                        .rounded_full()
                        .bg(theme.accent),
                ),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(KNOB)
                .right(KNOB)
                .flex()
                .items_center()
                .child(
                    div()
                        .absolute()
                        .left(relative(fraction))
                        .ml(-KNOB)
                        .size(KNOB * 2.)
                        .rounded_full()
                        .border_2()
                        .border_color(theme.accent)
                        .bg(theme.surface),
                ),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            if let Some(value) = value_at(event.position.x) {
                on_change(&value, window, cx);
            }
        })
        .on_mouse_move(move |event, window, cx| {
            if event.pressed_button == Some(MouseButton::Left)
                && let Some(value) = value_at_move(event.position.x)
            {
                on_change_move(&value, window, cx);
            }
        })
}

/// An on/off switch.
pub fn switch(
    id: impl Into<ElementId>,
    on: bool,
    theme: Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(36.))
        .h(px(20.))
        .p(px(2.))
        .rounded_full()
        .flex()
        .when(on, |this| this.justify_end())
        .bg(if on { theme.accent } else { theme.border })
        .cursor_pointer()
        .on_click(on_click)
        .child(div().size(px(16.)).rounded_full().bg(theme.surface))
}
