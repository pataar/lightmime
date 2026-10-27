//! Window decorations for compositors that leave them to the app.
//!
//! GNOME (Mutter) on Wayland has no server-side decorations and gpui draws
//! none itself, so without this the window has no titlebar: it cannot be
//! moved, resized or closed with the mouse. Where the window manager draws
//! the titlebar (Plasma, most X11 window managers) this adds nothing.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, BoxShadow, CursorStyle, Decorations, FontWeight, HitboxBehavior, MouseButton,
    PathBuilder, Pixels, Point, ResizeEdge, SharedString, Size, Tiling, Window, canvas, div, hsla,
    prelude::*, px,
};

use crate::theme::Theme;

/// Invisible margin around the window: the shadow is drawn there and the
/// pointer resizes the window from it.
const INSET: Pixels = px(10.);
const RADIUS: Pixels = px(12.);
const TITLEBAR_HEIGHT: Pixels = px(40.);
const BUTTON_SIZE: Pixels = px(24.);

/// A press on the titlebar, kept until the pointer moves. Starting the move
/// on the press itself would swallow the second click of a double click.
#[derive(Clone, Default)]
pub struct Drag(Rc<Cell<bool>>);

/// Wraps `content` in a titlebar, border, shadow and resize margin when the
/// window has client-side decorations; returns it unchanged otherwise.
pub fn frame(
    window: &mut Window,
    theme: Theme,
    title: impl Into<SharedString>,
    drag: &Drag,
    content: AnyElement,
) -> AnyElement {
    let Decorations::Client { tiling } = window.window_decorations() else {
        return content;
    };
    window.set_client_inset(INSET);
    let rounded = !tiling.is_tiled();

    let body = div()
        .id("window-body")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(theme.bg)
        .border_color(theme.border)
        .cursor(CursorStyle::Arrow)
        .when(!tiling.top, |this| this.border_t_1())
        .when(!tiling.bottom, |this| this.border_b_1())
        .when(!tiling.left, |this| this.border_l_1())
        .when(!tiling.right, |this| this.border_r_1())
        .when(rounded, |this| {
            this.rounded(RADIUS).shadow(vec![
                BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.35)).blur_radius(INSET / 2.),
            ])
        })
        // Pointer moves over the content need no resize cursor update.
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .child(titlebar(window, theme, title.into(), drag))
        .child(div().flex_1().min_h(px(0.)).child(content));

    div()
        .id("window-frame")
        .size_full()
        .when(!tiling.top, |this| this.pt(INSET))
        .when(!tiling.bottom, |this| this.pb(INSET))
        .when(!tiling.left, |this| this.pl(INSET))
        .when(!tiling.right, |this| this.pr(INSET))
        .child(
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(window.mouse_position(), size, tiling) {
                        window.set_cursor_style(resize_cursor(edge), &hitbox);
                    }
                },
            )
            .size_full()
            .absolute(),
        )
        .on_mouse_move(|_, window, _| window.refresh())
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(event.position, size, tiling) {
                window.start_window_resize(edge);
            }
        })
        .child(body)
        .into_any_element()
}

fn titlebar(window: &Window, theme: Theme, title: SharedString, drag: &Drag) -> impl IntoElement {
    let controls = window.window_controls();
    let pressed = drag.0.clone();
    let released = drag.0.clone();
    let moved = drag.0.clone();

    div()
        .id("titlebar")
        .flex_none()
        .h(TITLEBAR_HEIGHT)
        .px_2()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        // Balances the close button so the title stays centred.
        .child(div().flex_none().size(BUTTON_SIZE))
        .child(
            div()
                .flex_1()
                .flex()
                .justify_center()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(close_button(theme))
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            if event.click_count == 2 && controls.maximize {
                pressed.set(false);
                window.zoom_window();
            } else {
                pressed.set(true);
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| released.set(false))
        .on_mouse_move(move |event, window, _| {
            if moved.replace(false) && event.pressed_button == Some(MouseButton::Left) {
                window.start_window_move();
            }
        })
        .when(controls.window_menu, |this| {
            this.on_mouse_down(MouseButton::Right, |event, window, _| {
                window.show_window_menu(event.position);
            })
        })
}

fn close_button(theme: Theme) -> impl IntoElement {
    let colour = theme.text;
    div()
        .id("close")
        .flex_none()
        .size(BUTTON_SIZE)
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.border)
        .hover(|style| style.opacity(0.8))
        .cursor_pointer()
        // Keep the titlebar from starting a move under the click.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, window, _| window.remove_window())
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let mut cross = PathBuilder::stroke(px(1.5));
                    cross.move_to(bounds.origin);
                    cross.line_to(bounds.bottom_right());
                    cross.move_to(bounds.top_right());
                    cross.line_to(bounds.bottom_left());
                    if let Ok(path) = cross.build() {
                        window.paint_path(path, colour);
                    }
                },
            )
            .size(px(8.)),
        )
}

/// The edge or corner under `pos`, if it is in the resize margin. Tiled
/// edges sit against the screen or another window and do not resize.
fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, tiling: Tiling) -> Option<ResizeEdge> {
    let top = pos.y < INSET && !tiling.top;
    let bottom = pos.y > size.height - INSET && !tiling.bottom;
    let left = pos.x < INSET && !tiling.left;
    let right = pos.x > size.width - INSET && !tiling.right;
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

fn resize_cursor(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}
