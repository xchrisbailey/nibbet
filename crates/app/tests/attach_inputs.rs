//! UI integration tests for adding Attachments: paste, drop and the Insert
//! Menu picker, in the block view and in raw mode.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Fixture, find_label, has_label, input, open_shell, press, write};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, ClipboardEntry, ClipboardItem, ExternalPaths, FileDropEvent, Image,
    ImageFormat, TestAppContext, point, px,
};
use sett::Selection;
use sett_store::Change;

/// Longer than the autosave delay.
const AFTER_AUTOSAVE: Duration = Duration::from_secs(2);

/// Opens the Note `plan` with this body and puts the caret at its end.
fn open_plan(cx: &mut TestAppContext, body: &str) -> (Fixture, PathBuf) {
    let f = open_shell(cx);
    let path = f.data_root.join("personal/notes/plan.md");
    write(
        cx,
        &f,
        "notes/plan.md",
        &format!("---\ntitle: Plan\n---\n{body}"),
        Change::Note {
            project: None,
            slug: "plan".into(),
        },
    );
    let model = f.model(cx);
    cx.update(|cx| model.update(cx, |w, cx| w.select(Selection::Notes, cx)));
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("note-plan", cx);
        window.click("note-body", cx);
    })
    .unwrap();
    cx.run_until_parked();
    (f, path)
}

fn saved_body(cx: &mut TestAppContext, path: &Path) -> String {
    cx.executor().advance_clock(AFTER_AUTOSAVE);
    cx.run_until_parked();
    let text = fs::read_to_string(path).unwrap();
    text.split_once("---\n")
        .and_then(|(_, rest)| rest.split_once("---\n"))
        .map(|(_, body)| body.to_string())
        .unwrap_or(text)
}

/// A folder of files to attach, outside the data root.
fn sources(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = files
        .iter()
        .map(|(name, bytes)| {
            let path = dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            path
        })
        .collect();
    (dir, paths)
}

/// What Finder puts on the clipboard: the files, and their names as text.
fn finder_copy(paths: &[PathBuf]) -> ClipboardItem {
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let mut item = ClipboardItem::new_string(names.join(" "));
    item.entries
        .push(ClipboardEntry::ExternalPaths(ExternalPaths(
            paths.iter().cloned().collect(),
        )));
    item
}

fn paste(cx: &mut TestAppContext, f: &Fixture, item: ClipboardItem) {
    cx.write_to_clipboard(item);
    press(cx, f, "secondary-v");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn pasting_files_copies_them_and_inserts_an_image_and_a_link_as_one_undo_step(
    cx: &mut TestAppContext,
) {
    let (f, path) = open_plan(cx, "First line\n");
    let (_dir, files) = sources(&[("Shot 1.png", b"png"), ("report.pdf", b"%PDF")]);
    paste(cx, &f, finder_copy(&files));

    let folder = f.data_root.join("personal/notes/plan");
    assert_eq!(fs::read(folder.join("shot-1.png")).unwrap(), b"png");
    assert_eq!(fs::read(folder.join("report.pdf")).unwrap(), b"%PDF");
    assert_eq!(
        saved_body(cx, &path),
        "First line\n\n![Shot 1](plan/shot-1.png)\n\n[report.pdf](plan/report.pdf)\n"
    );

    press(cx, &f, "secondary-z");
    assert_eq!(saved_body(cx, &path), "First line\n");
}
