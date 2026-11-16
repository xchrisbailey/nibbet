//! How a Note's body lays out when the window is narrow (#49).

mod common;

use common::{note, open_shell_sized, settle, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Pixels, TestAppContext, px};
use sett::Selection;

/// The Budget Note from the screenshot fixtures. The Link chip opens the
/// paragraph after the table, so its top tracks the table's bottom.
const BUDGET: &str = "---\ntitle: Budget\n---\n\
| Item     | Estimate |\n\
|----------|----------|\n\
| Cabinets | 6,000    |\n\
| Worktop  | 2,500    |\n\
\n\
[[budget]] tracks the quotes.\n";

/// How far below the top of the editor the paragraph after the Budget
/// table starts, in a window `width` points wide.
fn below_table(cx: &mut TestAppContext, width: f32) -> Pixels {
    let f = open_shell_sized(cx, width, 919.);
    write(cx, &f, "notes/budget.md", BUDGET, note(None, "budget"));
    let model = f.model(cx);
    cx.update(|cx| model.update(cx, |w, cx| w.select(Selection::Notes, cx)));
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("note-budget", cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let body = window.find("note-body").bounds();
        let chip = window.within("note-body").find_by_label("Budget").bounds();
        chip.top() - body.top()
    })
    .unwrap()
}

/// In a half-screen window the Note editor is narrow. A table whose cells
/// each fit on one line in a wide window must not wrap inside a word there
/// ("Estimat / e"): its rows keep their one-line height, so what follows the
/// table doesn't move down.
#[gpui_kit::test]
fn a_table_keeps_whole_words_in_a_half_screen_window(cx: &mut TestAppContext) {
    let wide = below_table(cx, 2000.);
    let half = below_table(cx, 743.);
    assert!(
        (half - wide).abs() < px(1.),
        "the table ends {half:?} down at 743pt but {wide:?} at 2000pt: its cells wrapped"
    );
}

/// Opens the fixtures' Kitchen reno Budget Note, which links to a Task, in a
/// window `width` points wide.
fn open_budget_in_project(cx: &mut TestAppContext, width: f32) -> common::Fixture {
    let f = open_shell_sized(cx, width, 919.);
    write(
        cx,
        &f,
        "projects/kitchen-reno/project.md",
        "---\nname: Kitchen reno\ncolumns: [Quotes]\n---\n",
        sett_store::Change::Project("kitchen-reno".into()),
    );
    write(
        cx,
        &f,
        "projects/kitchen-reno/tasks/get-three-quotes.md",
        "---\ntitle: Get three quotes\ncolumn: Quotes\n---\n",
        sett_store::Change::Task {
            project: "kitchen-reno".into(),
            slug: "get-three-quotes".into(),
        },
    );
    write(
        cx,
        &f,
        "projects/kitchen-reno/notes/budget.md",
        "---\ntitle: Budget\n---\nQuotes tracked in [[get-three-quotes]].\n",
        note(Some("kitchen-reno"), "budget"),
    );
    let model = f.model(cx);
    let item = sett_store::ItemId::Note {
        project: Some("kitchen-reno".into()),
        slug: "budget".into(),
    };
    cx.update(|cx| model.update(cx, |w, cx| w.open_item(item, cx)));
    settle(cx);
    f
}

/// A Link row under the body stays inside the panel in a half-screen
/// window ("Get three quotes Task · Kitchen reno" ran off the edge).
#[gpui_kit::test]
fn a_link_row_fits_its_panel_in_a_half_screen_window(cx: &mut TestAppContext) {
    let f = open_budget_in_project(cx, 743.);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let panel = window.find("note-body").bounds();
        let row = window
            .within("outgoing-links")
            .find_by_label("Get three quotes")
            .bounds();
        assert!(
            row.right() <= panel.right(),
            "the Link row ends at {:?}, past the Note body's {:?}",
            row.right(),
            panel.right()
        );
    })
    .unwrap();
}

/// The Note body's width and its Notes list row's, in a window `width` points
/// wide.
fn body_and_list_widths(cx: &mut TestAppContext, width: f32) -> (Pixels, Pixels) {
    let f = open_budget_in_project(cx, width);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        (
            window.find("note-body").bounds().size.width,
            window.find("note-budget").bounds().size.width,
        )
    })
    .unwrap()
}

/// In a half-screen window the Notes list gives way before the Note body
/// does (the body was left 232pt); with room to spare it keeps its width.
#[gpui_kit::test]
fn the_notes_list_shrinks_first_in_a_half_screen_window(cx: &mut TestAppContext) {
    let (_, wide_list) = body_and_list_widths(cx, 1440.);
    let (half_body, half_list) = body_and_list_widths(cx, 743.);
    assert!(
        half_body >= px(320.),
        "the Note body is {half_body:?} at 743pt"
    );
    assert!(
        half_list < wide_list,
        "the Notes list kept {half_list:?} at 743pt"
    );
}
