//! The list-beside-editor split shared by the Notes and Templates screens.

use gpui_kit::{Div, Styled as _, div, px};

/// The list's width when there's room for it.
const LIST_WIDTH: f32 = 256.;
/// The narrowest the list gets; its titles truncate below their width.
const LIST_MIN: f32 = 160.;
/// The editor width the list gives way to before the editor shrinks itself.
const EDITOR_ROOM: f32 = 328.;
/// Flex shrinks in proportion to `shrink × basis`, so a list factor far above
/// the editor's makes the list take nearly all of a shortfall until it
/// reaches `LIST_MIN`. Only then does the editor shrink (#49).
const LIST_SHRINK: f32 = 100.;

/// The list column: full width when there's room, narrower first when
/// there isn't.
pub(crate) fn list_column() -> Div {
    div()
        .flex()
        .flex_col()
        .flex_basis(px(LIST_WIDTH))
        .flex_shrink(LIST_SHRINK)
        .min_w(px(LIST_MIN))
}

/// The editor column: takes all the room past the list.
pub(crate) fn editor_column() -> Div {
    div()
        .flex_grow(1.)
        .flex_shrink(1.)
        .flex_basis(px(EDITOR_ROOM))
        .min_w_0()
}
