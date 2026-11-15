//! Files are sacred (ADR 0001): every way of adding or moving Attachments
//! leaves an Item or Template whose frontmatter can't be read byte-identical,
//! and writes nothing beside it.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, note, open_shell, press, settle, write};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ClipboardEntry, ClipboardItem, ExternalPaths, FileDropEvent, Image,
    ImageFormat, InputEvent as _, TestAppContext, point, px,
};
use sett::Selection;
use sett_store::Change;

const BROKEN: &str = "---\ntitle: A\ntitle: B\n---\nBody\n";

fn click(cx: &mut TestAppContext, f: &Fixture, id: &'static str) {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn finder_copy(path: &Path) -> ClipboardItem {
    let mut item = ClipboardItem::new_string("x.png".to_string());
    item.entries
        .push(ClipboardEntry::ExternalPaths(ExternalPaths(
            [path.to_path_buf()].into_iter().collect(),
        )));
    item
}

fn paste(cx: &mut TestAppContext, f: &Fixture, item: ClipboardItem) {
    cx.write_to_clipboard(item);
    press(cx, f, "secondary-v");
}

fn drop_file(cx: &mut TestAppContext, f: &Fixture, path: &Path) {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("note-body").bounds();
        let at = point(
            bounds.origin.x + px(40.),
            bounds.origin.y + bounds.size.height - px(10.),
        );
        let paths = || ExternalPaths([path.to_path_buf()].into_iter().collect());
        window.dispatch_event(
            FileDropEvent::Entered {
                position: at,
                paths: paths(),
            }
            .to_platform_input(),
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

/// Everything under `dir`, by relative path, for comparing before and after.
fn tree(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
            if path.is_dir() {
                walk(&path, root, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[gpui_kit::test]
fn attaching_carrying_and_renaming_never_touch_a_file_that_cannot_be_read(cx: &mut TestAppContext) {
    let f = open_shell(cx);
    let personal = f.data_root.join("personal");
    let notes = personal.join("notes");
    let broken = notes.join("plan.md");
    write(cx, &f, "notes/plan.md", BROKEN, note(None, "plan"));
    write(
        cx,
        &f,
        "notes/other.md",
        "---\ntitle: Other\n---\n![shot](other/shot.png)\n",
        note(None, "other"),
    );
    fs::create_dir_all(notes.join("other")).unwrap();
    fs::write(notes.join("other/shot.png"), b"png").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file = outside.path().join("x.png");
    fs::write(&file, b"x").unwrap();
    let model = f.model(cx);
    cx.update(|cx| model.update(cx, |w, cx| w.select(Selection::Notes, cx)));

    // Text copied from another Item, so the paste would carry its files.
    click(cx, &f, "note-other");
    click(cx, &f, "note-body");
    press(cx, &f, "secondary-a secondary-c");
    click(cx, &f, "note-plan");
    click(cx, &f, "note-body");
    let before = tree(&notes);

    let image = Image::from_bytes(ImageFormat::Png, b"png-bytes".to_vec());
    for step in ["carried paste", "pasted files", "pasted image", "drop"] {
        match step {
            "carried paste" => press(cx, &f, "secondary-v"),
            "pasted files" => paste(cx, &f, finder_copy(&file)),
            "pasted image" => paste(cx, &f, ClipboardItem::new_image(&image)),
            _ => drop_file(cx, &f, &file),
        }
        settle(cx);
        assert_eq!(fs::read_to_string(&broken).unwrap(), BROKEN, "{step}");
        assert_eq!(tree(&notes), before, "{step}");
    }

    // A Template's rename rewrites its links, but not in a broken file.
    let templates = personal.join("templates");
    fs::create_dir_all(templates.join("bug")).unwrap();
    fs::write(templates.join("bug/shot.png"), b"png").unwrap();
    let template = "---\ntitle: A\ntitle: B\n---\n![shot](bug/shot.png)\n";
    fs::write(templates.join("bug.md"), template).unwrap();
    cx.update(|cx| {
        model.update(cx, |w, cx| {
            w.apply_change(Change::Template("bug".into()), cx);
            w.select(Selection::Templates, cx);
        })
    });
    cx.run_until_parked();
    let before = tree(&templates);
    click(cx, &f, "template-bug");
    click(cx, &f, "note-title");
    press(cx, &f, "secondary-a");
    common::input(cx, &f, "Defect");
    press(cx, &f, "enter");
    settle(cx);
    assert_eq!(
        fs::read_to_string(templates.join("bug.md")).unwrap(),
        template
    );
    assert_eq!(tree(&templates), before);
}
