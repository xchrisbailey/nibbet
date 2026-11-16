//! UI integration tests for adding Attachments: paste, drop and the Insert
//! Menu picker, in the block view and in raw mode.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Fixture, has_label, input, open_shell, press, write};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ClipboardEntry, ClipboardItem, ExternalPaths, FileDropEvent, Image,
    ImageFormat, InputEvent as _, TestAppContext, point, px,
};
use nibbet::Selection;
use nibbet_store::Change;

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

fn paste_text(cx: &mut TestAppContext, f: &Fixture, text: &str) {
    paste(cx, f, ClipboardItem::new_string(text.to_string()));
}

#[gpui_kit::test]
fn pasting_image_data_writes_pasted_image_png_with_no_alt(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    let image = Image::from_bytes(ImageFormat::Png, b"png-bytes".to_vec());
    paste(cx, &f, ClipboardItem::new_image(&image));

    let folder = f.data_root.join("personal/notes/plan");
    assert_eq!(
        fs::read(folder.join("pasted-image.png")).unwrap(),
        b"png-bytes"
    );
    assert_eq!(
        saved_body(cx, &path),
        "First line\n\n![](plan/pasted-image.png)\n"
    );
}

#[gpui_kit::test]
fn a_text_paste_is_unchanged(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    paste_text(cx, &f, " and more");
    assert_eq!(saved_body(cx, &path), "First line and more\n");
    assert!(!f.data_root.join("personal/notes/plan").exists());
}

#[gpui_kit::test]
fn a_code_block_copy_and_paste_still_works(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "```rust\nlet a = 1;\n```\n");
    press(cx, &f, "secondary-a secondary-c secondary-down");
    paste_text_from_clipboard(cx, &f);
    assert_eq!(
        saved_body(cx, &path),
        "```rust\nlet a = 1;let a = 1;\n```\n"
    );
}

fn paste_text_from_clipboard(cx: &mut TestAppContext, f: &Fixture) {
    press(cx, f, "secondary-v");
}

#[gpui_kit::test]
fn files_pasted_into_a_code_block_go_in_as_their_names(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "```\nx\n```\n");
    let (_dir, files) = sources(&[("report.pdf", b"%PDF")]);
    paste(cx, &f, finder_copy(&files));
    assert_eq!(saved_body(cx, &path), "```\nxreport.pdf\n```\n");
    assert!(!f.data_root.join("personal/notes/plan").exists());
}

fn drop_at(cx: &mut TestAppContext, f: &Fixture, paths: &[PathBuf], x: f32, y: f32) {
    let at = point(px(x), px(y));
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let paths = || ExternalPaths(paths.iter().cloned().collect());
        window.dispatch_event(
            FileDropEvent::Entered {
                position: at,
                paths: paths(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            FileDropEvent::Pending { position: at }.to_platform_input(),
            cx,
        );
        window.dispatch_event(
            FileDropEvent::Submit { position: at }.to_platform_input(),
            cx,
        );
        window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The top-right corner area of the first line of the body, and a point in
/// the empty space below it.
fn body_points(cx: &mut TestAppContext, f: &Fixture) -> ((f32, f32), (f32, f32)) {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("note-body").bounds();
        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        ((x + w - 20., y + 28.), (x + 40., y + h - 10.))
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_drop_inserts_at_the_point_and_below_the_text_at_the_end(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "One\n\nTwo\n");
    let (first_line, below) = body_points(cx, &f);
    let (_dir, files) = sources(&[("a.png", b"a"), ("b.png", b"b")]);

    drop_at(cx, &f, &files[..1], first_line.0, first_line.1);
    assert_eq!(saved_body(cx, &path), "One\n\n![a](plan/a.png)\n\nTwo\n");

    drop_at(cx, &f, &files[1..], below.0, below.1);
    assert_eq!(
        saved_body(cx, &path),
        "One\n\n![a](plan/a.png)\n\nTwo\n\n![b](plan/b.png)\n"
    );
}

#[gpui_kit::test]
fn a_folder_in_a_drop_is_skipped_with_one_notice(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    let (dir, mut files) = sources(&[("report.pdf", b"%PDF")]);
    let folder = dir.path().join("a folder");
    fs::create_dir(&folder).unwrap();
    files.insert(0, folder);
    let (_, below) = body_points(cx, &f);
    drop_at(cx, &f, &files, below.0, below.1);

    // One other file is a link inline at the caret.
    assert_eq!(
        saved_body(cx, &path),
        "First line[report.pdf](plan/report.pdf)\n"
    );
    let notices = cx
        .update_window(f.window, |_, window, cx| window.notifications(cx).len())
        .unwrap();
    assert_eq!(notices, 1);
}

#[gpui_kit::test]
fn the_insert_menu_attachment_option_attaches_the_picked_files(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    press(cx, &f, "secondary-down enter");
    input(cx, &f, "/att");
    assert!(has_label(cx, &f, "Attachment"));
    press(cx, &f, "enter");

    let (_dir, files) = sources(&[("a.png", b"a"), ("report.pdf", b"%PDF")]);
    let picked = files.clone();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.files && !options.directories && options.multiple);
        assert_eq!(options.prompt.as_deref(), Some("Attach"));
        Some(picked)
    });
    cx.run_until_parked();
    assert_eq!(
        saved_body(cx, &path),
        "First line\n\n![a](plan/a.png)\n\n[report.pdf](plan/report.pdf)\n"
    );
}

#[gpui_kit::test]
fn the_picked_files_go_in_where_the_menu_was_not_where_the_caret_went(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    press(cx, &f, "secondary-down enter");
    input(cx, &f, "/att");
    press(cx, &f, "enter");
    // The user clicks elsewhere while the picker is open.
    press(cx, &f, "secondary-up");

    let (_dir, files) = sources(&[("a.png", b"a")]);
    cx.simulate_path_prompt_response(move |_| Some(files));
    cx.run_until_parked();
    assert_eq!(saved_body(cx, &path), "First line\n\n![a](plan/a.png)\n");
}

#[gpui_kit::test]
fn cancelling_the_picker_leaves_the_empty_block(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    press(cx, &f, "secondary-down enter");
    input(cx, &f, "/file");
    press(cx, &f, "enter");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    input(cx, &f, "x");
    assert_eq!(saved_body(cx, &path), "First line\n\nx\n");
}

#[gpui_kit::test]
fn raw_mode_paste_inserts_markdown_with_blank_lines_around_images(cx: &mut TestAppContext) {
    let (f, path) = open_plan(cx, "First line\n");
    cx.update_window(f.window, |_, window, cx| window.press("cmd-/", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(has_label(cx, &f, "Raw Markdown"));
    let (_dir, files) = sources(&[("shot.png", b"png")]);
    paste(cx, &f, finder_copy(&files));
    assert_eq!(
        saved_body(cx, &path),
        "![shot](plan/shot.png)\n\nFirst line\n"
    );
}

#[gpui_kit::test]
fn a_journal_entry_without_a_file_is_saved_by_the_attachment(cx: &mut TestAppContext) {
    let f = open_shell(cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.within("nav").click("0-0", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("note-body", cx);
    })
    .unwrap();
    let (_dir, files) = sources(&[("report.pdf", b"%PDF")]);
    paste(cx, &f, finder_copy(&files));

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let journal = f.data_root.join("personal/journal");
    assert_eq!(
        saved_body(cx, &journal.join(format!("{today}.md"))).trim_end(),
        format!("[report.pdf]({today}/report.pdf)")
    );
    assert!(journal.join(&today).join("report.pdf").exists());
}
