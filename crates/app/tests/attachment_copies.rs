//! UI integration tests for Attachments that follow content moving around
//! inside nibbet: copy and paste between Items, and using a Template.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Fixture, has_label, input, open_shell, press, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ClipboardItem, TestAppContext};
use nibbet::Selection;
use nibbet_store::Change;

/// Longer than the autosave delay.
const AFTER_AUTOSAVE: Duration = Duration::from_secs(2);

fn note_change(slug: &str) -> Change {
    Change::Note {
        project: None,
        slug: slug.into(),
    }
}

/// Opens the Notes list with a Note `plan` holding an image and an empty
/// Note `other`, then opens `plan`.
fn two_notes(cx: &mut TestAppContext) -> Fixture {
    let f = open_shell(cx);
    let notes = f.data_root.join("personal/notes");
    write(
        cx,
        &f,
        "notes/plan.md",
        "---\ntitle: Plan\n---\n![shot](plan/shot.png)\n",
        note_change("plan"),
    );
    write(
        cx,
        &f,
        "notes/other.md",
        "---\ntitle: Other\n---\nOther text\n",
        note_change("other"),
    );
    fs::create_dir_all(notes.join("plan")).unwrap();
    fs::write(notes.join("plan/shot.png"), b"png").unwrap();
    let model = f.model(cx);
    cx.update(|cx| model.update(cx, |w, cx| w.select(Selection::Notes, cx)));
    open(cx, &f, "plan");
    f
}

fn open(cx: &mut TestAppContext, f: &Fixture, slug: &str) {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("note-{slug}"), cx);
        window.click("note-body", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn body(cx: &mut TestAppContext, f: &Fixture, slug: &str) -> String {
    cx.executor().advance_clock(AFTER_AUTOSAVE);
    cx.run_until_parked();
    let text = fs::read_to_string(f.data_root.join(format!("personal/notes/{slug}.md"))).unwrap();
    text.split_once("---\n")
        .and_then(|(_, rest)| rest.split_once("---\n"))
        .map(|(_, body)| body.to_string())
        .unwrap_or(text)
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Copies the whole body of the open Note.
fn copy_all(cx: &mut TestAppContext, f: &Fixture) {
    press(cx, f, "secondary-a secondary-c");
}

/// Pastes on a new empty line under the body.
fn paste_at_end(cx: &mut TestAppContext, f: &Fixture) {
    press(cx, f, "secondary-down enter secondary-v");
}

#[gpui_kit::test]
fn pasting_a_copied_image_into_another_item_copies_the_file_as_one_undo_step(
    cx: &mut TestAppContext,
) {
    let f = two_notes(cx);
    copy_all(cx, &f);
    open(cx, &f, "other");
    paste_at_end(cx, &f);

    let notes = f.data_root.join("personal/notes");
    assert_eq!(fs::read(notes.join("other/shot.png")).unwrap(), b"png");
    assert_eq!(
        body(cx, &f, "other"),
        "Other text\n\n![shot](other/shot.png)\n"
    );
    // The source keeps its file.
    assert_eq!(files(&notes.join("plan")), ["shot.png"]);

    // One undo takes the whole paste back; the new line stays.
    press(cx, &f, "secondary-z");
    assert_eq!(body(cx, &f, "other").trim_end(), "Other text");
}

#[gpui_kit::test]
fn raw_mode_copy_and_paste_carries_the_files_too(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    cx.update_window(f.window, |_, window, cx| window.press("cmd-/", cx))
        .unwrap();
    cx.run_until_parked();
    copy_all(cx, &f);
    open(cx, &f, "other");
    press(cx, &f, "secondary-down enter secondary-v");

    let notes = f.data_root.join("personal/notes");
    assert_eq!(fs::read(notes.join("other/shot.png")).unwrap(), b"png");
    assert!(body(cx, &f, "other").contains("![shot](other/shot.png)"));
}

#[gpui_kit::test]
fn cut_leaves_the_source_files_where_they_are(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    press(cx, &f, "secondary-a secondary-x");
    open(cx, &f, "other");
    paste_at_end(cx, &f);

    let notes = f.data_root.join("personal/notes");
    assert_eq!(files(&notes.join("plan")), ["shot.png"]);
    assert_eq!(files(&notes.join("other")), ["shot.png"]);
}

#[gpui_kit::test]
fn pasting_back_into_the_same_item_changes_nothing(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    copy_all(cx, &f);
    paste_at_end(cx, &f);

    let notes = f.data_root.join("personal/notes");
    assert_eq!(files(&notes.join("plan")), ["shot.png"]);
    assert!(!notes.join("other").exists());
    assert_eq!(
        body(cx, &f, "plan"),
        "![shot](plan/shot.png)\n\n![shot](plan/shot.png)\n"
    );
}

#[gpui_kit::test]
fn text_from_outside_nibbet_is_pasted_as_it_is(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    // Another Item's folder name in text that nibbet never copied.
    copy_all(cx, &f);
    open(cx, &f, "other");
    cx.write_to_clipboard(ClipboardItem::new_string(
        "![x](fix-login/x.png)".to_string(),
    ));
    paste_at_end(cx, &f);

    let notes = f.data_root.join("personal/notes");
    assert!(!notes.join("other").exists());
    assert_eq!(
        body(cx, &f, "other"),
        "Other text\n\n![x](fix-login/x.png)\n"
    );
}

#[gpui_kit::test]
fn deleting_an_image_block_leaves_its_file(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    press(cx, &f, "secondary-a backspace");
    assert_eq!(body(cx, &f, "plan").trim(), "");
    assert_eq!(
        files(&f.data_root.join("personal/notes/plan")),
        ["shot.png"]
    );
}

/// Writes a Template with an image in its own folder.
fn template_with_image(cx: &mut TestAppContext, f: &Fixture) -> PathBuf {
    let dir = f.data_root.join("personal/templates");
    fs::create_dir_all(dir.join("bug")).unwrap();
    fs::write(dir.join("bug/trace.png"), b"trace").unwrap();
    fs::write(
        dir.join("bug.md"),
        "---\ntitle: Bug\n---\n## Steps\n\n![trace](bug/trace.png)\n",
    )
    .unwrap();
    let model = f.model(cx);
    cx.update(|cx| {
        model.update(cx, |m, cx| {
            m.apply_change(Change::Template("bug".into()), cx)
        })
    });
    cx.run_until_parked();
    dir
}

#[gpui_kit::test]
fn using_a_template_copies_its_images_into_the_items_folder(cx: &mut TestAppContext) {
    let f = two_notes(cx);
    template_with_image(cx, &f);
    open(cx, &f, "other");
    press(cx, &f, "secondary-down enter");
    input(cx, &f, "/bug");
    assert!(has_label(cx, &f, "Bug"));
    press(cx, &f, "enter");

    let notes = f.data_root.join("personal/notes");
    assert_eq!(fs::read(notes.join("other/trace.png")).unwrap(), b"trace");
    assert_eq!(
        body(cx, &f, "other"),
        "Other text\n\n## Steps\n\n![trace](other/trace.png)\n"
    );
}
