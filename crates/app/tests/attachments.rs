//! UI integration tests for Attachments in an Item's body: images show or get
//! a chip, links to files open, and changes made outside sett show up.

mod common;

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use common::{Fixture, note, open_shell, write};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AppContext as _, InputEvent as _, Modifiers, ModifiersChangedEvent, MouseButton,
    TestAppContext, Window,
};
use sett::{AppModel, AttachmentOpener, Selection};
use sett_store::{AttachmentItem, Change};

fn open_plan(cx: &mut TestAppContext, body: &str) -> Fixture {
    let f = open_shell(cx);
    let attachments = f.data_root.join("personal/notes/plan");
    fs::create_dir_all(&attachments).unwrap();
    // A real file of the wrong type, and one that is a valid image name.
    fs::write(attachments.join("report.pdf"), b"%PDF").unwrap();
    fs::write(attachments.join("notes.txt"), b"hi").unwrap();
    write(
        cx,
        &f,
        "notes/plan.md",
        &format!("---\ntitle: Plan\n---\n{body}"),
        note(None, "plan"),
    );
    write(
        cx,
        &f,
        "notes/other.md",
        "---\ntitle: Other\n---\nhello\n",
        note(None, "other"),
    );
    let model = f.model(cx);
    cx.update(|cx| model.update(cx, |w, cx| w.select(Selection::Notes, cx)));
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("note-plan", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    f
}

fn has_label(cx: &mut TestAppContext, f: &Fixture, label: &str) -> bool {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        window.try_find_by_label(label).is_some()
    })
    .unwrap()
}

/// Records what the system would be asked to open.
fn stub_opener(cx: &mut TestAppContext) -> Rc<RefCell<Vec<PathBuf>>> {
    let opened = Rc::new(RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|cx| {
        cx.set_global(AttachmentOpener(Rc::new(move |path, _| {
            seen.borrow_mut().push(path.to_path_buf());
        })));
    });
    opened
}

fn with_cmd(window: &mut Window, held: bool, cx: &mut gpui_kit::App) {
    let modifiers = if held {
        Modifiers {
            platform: cfg!(target_os = "macos"),
            control: cfg!(not(target_os = "macos")),
            ..Modifiers::none()
        }
    } else {
        Modifiers::none()
    };
    window.dispatch_event(
        ModifiersChangedEvent {
            modifiers,
            capslock: window.capslock(),
        }
        .to_platform_input(),
        cx,
    );
}

/// Cmd-clicks `dx` points into the body's first line.
fn cmd_click_body(cx: &mut TestAppContext, f: &Fixture, dx: f32) {
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        let body = window.find("note-body").bounds();
        let at = body.origin + gpui_kit::point(gpui_kit::px(dx), gpui_kit::px(24.0));
        with_cmd(window, true, cx);
        window.pointer_move(at, None, cx);
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
        with_cmd(window, false, cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn images_that_cannot_load_show_a_chip_saying_why(cx: &mut TestAppContext) {
    let f = open_plan(
        cx,
        "![a](plan/gone.png)\n\n![b](https://example.com/x.png)\n\n\
         ![c](../../../../../etc/hosts)\n\n![d](plan/notes.txt)\n",
    );
    for label in [
        "Missing image: gone.png",
        "Image not loaded: example.com",
        "Image not loaded",
        "Can\u{2019}t show notes.txt as an image",
    ] {
        assert!(has_label(cx, &f, label), "{label}");
    }
}

#[gpui_kit::test]
fn a_cross_item_link_inside_the_space_is_an_attachment(cx: &mut TestAppContext) {
    let f = open_plan(cx, "[Other](other.md)\n");
    assert!(has_label(cx, &f, "Attachment"));
}

#[gpui_kit::test]
fn a_web_link_is_not_an_attachment(cx: &mut TestAppContext) {
    let f = open_plan(cx, "[docs](https://example.com)\n");
    assert!(!has_label(cx, &f, "Attachment"));
}

#[gpui_kit::test]
fn cmd_click_on_an_attachment_link_opens_the_file(cx: &mut TestAppContext) {
    let opened = stub_opener(cx);
    let f = open_plan(cx, "[report](plan/report.pdf)\n");
    cmd_click_body(cx, &f, 60.);
    assert_eq!(
        *opened.borrow(),
        [f.data_root.join("personal/notes/plan/report.pdf")]
    );
}

#[gpui_kit::test]
fn cmd_click_on_a_missing_attachment_posts_a_notice(cx: &mut TestAppContext) {
    let opened = stub_opener(cx);
    let f = open_plan(cx, "[report](plan/gone.pdf)\n");
    cmd_click_body(cx, &f, 60.);
    assert!(opened.borrow().is_empty());
    let notices = cx
        .update_window(f.window, |_, window, cx| window.notifications(cx).len())
        .unwrap();
    assert_eq!(notices, 1);
}

#[gpui_kit::test]
fn cmd_click_on_a_path_outside_the_space_does_nothing(cx: &mut TestAppContext) {
    let opened = stub_opener(cx);
    let f = open_plan(cx, "[passwd](../../../../../etc/passwd)\n");
    cmd_click_body(cx, &f, 30.);
    assert!(opened.borrow().is_empty());
    let notices = cx
        .update_window(f.window, |_, window, cx| window.notifications(cx).len())
        .unwrap();
    assert_eq!(notices, 0);
}

#[gpui_kit::test]
fn a_file_added_outside_sett_replaces_the_missing_chip(cx: &mut TestAppContext) {
    let f = open_plan(cx, "![late](plan/late.png)\n");
    assert!(has_label(cx, &f, "Missing image: late.png"));

    // A one-pixel PNG, written the way an agent would.
    let png = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0, 5, 0xfe, 2, 0xfe, 0xa7, 0x35,
        0x81, 0x84, 0, 0, 0, 0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    fs::write(f.data_root.join("personal/notes/plan/late.png"), png).unwrap();
    let model = f.model(cx);
    let change = Change::Attachment {
        item: AttachmentItem::Note {
            project: None,
            slug: "plan".into(),
        },
        file: Some("late.png".into()),
    };
    cx.update(|cx| model.update(cx, |m: &mut AppModel, cx| m.apply_change(change, cx)));
    cx.run_until_parked();
    assert!(!has_label(cx, &f, "Missing image: late.png"));
}
