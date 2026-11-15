use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sett_store::{AttachmentItem, Date, Resolved, Space, Store, is_inline_image};

const SPACE: Space = Space::Work;

fn new_store() -> (tempfile::TempDir, Store, Arc<Mutex<Vec<PathBuf>>>) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("sett")).unwrap();
    let trashed = Arc::new(Mutex::new(Vec::new()));
    let log = trashed.clone();
    store.set_trash(move |path| {
        log.lock().unwrap().push(path.to_path_buf());
        if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    });
    (dir, store, trashed)
}

fn source(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn note(slug: &str) -> AttachmentItem {
    AttachmentItem::Note {
        project: None,
        slug: slug.into(),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn names_are_slugified_with_a_lowercase_extension() {
    let (dir, store, _) = new_store();
    let item = note("meeting");
    let cases = [
        ("Login Error.PNG", "login-error.png", "Login Error", true),
        ("Café Menü.jpg", "café-menü.jpg", "Café Menü", true),
        (".png", "attachment.png", "", true),
        ("Q3 Report.pdf", "q3-report.pdf", "Q3 Report", false),
        ("README", "readme", "README", false),
    ];
    for (original, file, name, inline) in cases {
        let src = source(dir.path(), original, original.as_bytes());
        let attached = store.attach_file(SPACE, &item, &src).unwrap();
        assert_eq!(
            attached.destination,
            format!("meeting/{file}"),
            "{original}"
        );
        assert_eq!(attached.name, name, "{original}");
        assert_eq!(attached.inline, inline, "{original}");
        assert!(
            store
                .attachment_dir(SPACE, &item)
                .unwrap()
                .join(file)
                .is_file()
        );
    }
}

#[test]
fn a_clash_gets_a_number_unless_the_bytes_match() {
    let (dir, store, _) = new_store();
    let item = note("meeting");
    let first = source(dir.path(), "shot.png", b"one");
    let a = store.attach_file(SPACE, &item, &first).unwrap();
    assert_eq!(a.destination, "meeting/shot.png");

    let other = dir.path().join("elsewhere");
    fs::create_dir(&other).unwrap();
    let different = source(&other, "shot.png", b"two");
    let b = store.attach_file(SPACE, &item, &different).unwrap();
    assert_eq!(b.destination, "meeting/shot-2.png");
    let third = dir.path().join("third");
    fs::create_dir(&third).unwrap();
    let c = store
        .attach_file(SPACE, &item, &source(&third, "shot.png", b"three"))
        .unwrap();
    assert_eq!(c.destination, "meeting/shot-3.png");

    let again = store.attach_file(SPACE, &item, &different).unwrap();
    assert_eq!(again.destination, "meeting/shot-2.png");
    assert_eq!(
        names(&store.attachment_dir(SPACE, &item).unwrap()),
        ["shot-2.png", "shot-3.png", "shot.png"]
    );
}

#[test]
fn a_source_in_the_items_own_folder_is_linked_not_copied() {
    let (_dir, store, _) = new_store();
    let item = note("meeting");
    let folder = store.attachment_dir(SPACE, &item).unwrap();
    fs::create_dir_all(&folder).unwrap();
    let own = source(&folder, "Already Here.png", b"x");

    let attached = store.attach_file(SPACE, &item, &own).unwrap();

    assert_eq!(attached.destination, "meeting/Already Here.png");
    assert_eq!(names(&folder), ["Already Here.png"]);
}

#[test]
fn another_items_folder_is_copied_from() {
    let (_dir, store, _) = new_store();
    let theirs = store.attachment_dir(SPACE, &note("other")).unwrap();
    fs::create_dir_all(&theirs).unwrap();
    let src = source(&theirs, "x.png", b"x");

    let attached = store.attach_file(SPACE, &note("meeting"), &src).unwrap();

    assert_eq!(attached.destination, "meeting/x.png");
    assert!(src.exists());
}

#[test]
fn directories_and_missing_files_are_refused() {
    let (dir, store, _) = new_store();
    let item = note("meeting");

    assert!(store.attach_file(SPACE, &item, dir.path()).is_err());
    assert!(
        store
            .attach_file(SPACE, &item, &dir.path().join("nope.png"))
            .is_err()
    );
    assert!(!store.attachment_dir(SPACE, &item).unwrap().exists());
}

#[test]
fn a_journal_entry_with_no_file_yet_still_gets_its_folder() {
    let (_dir, store, _) = new_store();
    let item = AttachmentItem::Journal(Date::parse("2026-11-14").unwrap());

    let attached = store.attach_bytes(SPACE, &item, "PNG", b"pixels").unwrap();

    assert_eq!(attached.destination, "2026-11-14/pasted-image.png");
    assert!(attached.inline);
    let path = store
        .root()
        .join("work/journal/2026-11-14/pasted-image.png");
    assert_eq!(fs::read(path).unwrap(), b"pixels");
    // No half-written temp files are left behind.
    assert_eq!(
        names(&store.root().join("work/journal/2026-11-14")),
        ["pasted-image.png"]
    );
}

#[test]
fn folders_sit_beside_each_kind_of_item() {
    let (_dir, store, _) = new_store();
    let root = store.root().join("work");
    let task = AttachmentItem::Task {
        project: "inbox".into(),
        slug: "fix-login".into(),
    };
    let in_project = AttachmentItem::Note {
        project: Some("inbox".into()),
        slug: "n".into(),
    };
    let template = AttachmentItem::Template("bug".into());
    assert_eq!(
        store.attachment_dir(SPACE, &task).unwrap(),
        root.join("projects/inbox/tasks/fix-login")
    );
    assert_eq!(
        store.attachment_dir(SPACE, &in_project).unwrap(),
        root.join("projects/inbox/notes/n")
    );
    assert_eq!(
        store.attachment_dir(SPACE, &note("m")).unwrap(),
        root.join("notes/m")
    );
    assert_eq!(
        store.attachment_dir(SPACE, &template).unwrap(),
        root.join("templates/bug")
    );
    assert!(store.attachment_dir(SPACE, &note("../x")).is_err());
}

#[test]
fn only_pictures_show_inline() {
    for ext in [
        "png", "JPG", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff", "ico",
    ] {
        assert!(is_inline_image(ext), "{ext}");
    }
    for ext in ["pdf", "heic", "txt", ""] {
        assert!(!is_inline_image(ext), "{ext}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn heic_becomes_a_jpeg_and_a_failed_conversion_keeps_the_original() {
    let (dir, store, _) = new_store();
    let item = note("meeting");
    // No HEIC fixture is committed: sips can make one from a PNG.
    let png = dir.path().join("src.png");
    let tiny = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let status = std::process::Command::new("/usr/bin/base64")
        .args(["-D", "-o"])
        .arg(&png)
        .arg("-i")
        .arg({
            let b64 = dir.path().join("b64");
            fs::write(&b64, tiny).unwrap();
            b64
        })
        .status()
        .unwrap();
    assert!(status.success());
    let heic = dir.path().join("IMG 0042.heic");
    let made = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "heic"])
        .arg(&png)
        .arg("--out")
        .arg(&heic)
        .output()
        .unwrap();
    if !made.status.success() {
        return;
    }

    let attached = store.attach_file(SPACE, &item, &heic).unwrap();
    assert_eq!(attached.destination, "meeting/img-0042.jpg");
    assert!(attached.inline);

    let fake = source(dir.path(), "broken.heic", b"not an image");
    let attached = store.attach_file(SPACE, &item, &fake).unwrap();
    assert_eq!(attached.destination, "meeting/broken.heic");
    assert!(!attached.inline);
    assert_eq!(
        fs::read(
            store
                .attachment_dir(SPACE, &item)
                .unwrap()
                .join("broken.heic")
        )
        .unwrap(),
        b"not an image"
    );
}

fn resolve(store: &Store, item: &AttachmentItem, destination: &str) -> Resolved {
    store.resolve_attachment(SPACE, item, destination)
}

#[test]
fn links_resolve_inside_the_space_only() {
    let (_dir, store, _) = new_store();
    let space = store.root().join("work");
    let item = note("meeting");
    fs::create_dir_all(space.join("notes/meeting")).unwrap();
    fs::write(space.join("notes/meeting/a b.png"), "x").unwrap();
    fs::create_dir_all(space.join("notes/other-note")).unwrap();
    fs::write(space.join("notes/other-note/x.png"), "x").unwrap();
    let found = |rel: &str| Resolved::Found(space.join(rel));

    assert_eq!(
        resolve(&store, &item, "meeting/a b.png"),
        found("notes/meeting/a b.png")
    );
    assert_eq!(
        resolve(&store, &item, "<meeting/a b.png>"),
        found("notes/meeting/a b.png")
    );
    assert_eq!(
        resolve(&store, &item, "meeting/a%20b.png"),
        found("notes/meeting/a b.png")
    );
    assert_eq!(
        resolve(&store, &item, "./meeting/a b.png"),
        found("notes/meeting/a b.png")
    );
    assert_eq!(
        resolve(&store, &item, "../notes/other-note/x.png"),
        found("notes/other-note/x.png")
    );
    assert_eq!(
        resolve(&store, &item, "meeting/gone.png"),
        Resolved::Missing(space.join("notes/meeting/gone.png"))
    );
    for refused in [
        "../../../etc/passwd",
        "../../personal/notes/x.png",
        "/etc/passwd",
        "https://example.com/x.png",
        "http://example.com/x.png",
        "file:///etc/passwd",
        "mailto:a@b.c",
        "",
        "%2e%2e/%2e%2e/%2e%2e/x",
    ] {
        assert_eq!(
            resolve(&store, &item, refused),
            Resolved::Refused,
            "{refused}"
        );
    }
}

#[test]
fn trashing_an_item_takes_its_folder_too() {
    let (_dir, store, trashed) = new_store();
    let today = Date::parse("2026-03-04").unwrap();
    let task = store
        .create_task(SPACE, "inbox", "Fix login", today)
        .unwrap();
    let in_note = store.create_note(SPACE, None, "Meeting", today).unwrap();
    let template = store.create_template(SPACE, "Bug").unwrap();
    let mut entry = sett_store::JournalEntry::new(SPACE, today);
    entry.body = "hi".into();
    store.save_journal_entry(&mut entry).unwrap();

    for item in [
        AttachmentItem::from(&task),
        AttachmentItem::from(&in_note),
        AttachmentItem::from(&template),
        AttachmentItem::from(&entry),
    ] {
        store.attach_bytes(SPACE, &item, "png", b"x").unwrap();
    }
    store.trash_task(&task).unwrap();
    store.trash_note(&in_note).unwrap();
    store.trash_template(&template).unwrap();
    store.trash_journal_entry(&entry).unwrap();

    for item in [
        AttachmentItem::from(&task),
        AttachmentItem::from(&in_note),
        AttachmentItem::from(&template),
        AttachmentItem::from(&entry),
    ] {
        assert!(!store.attachment_dir(SPACE, &item).unwrap().exists());
    }
    assert_eq!(trashed.lock().unwrap().len(), 8);
}

#[test]
fn trashing_an_item_with_no_folder_trashes_just_the_file() {
    let (_dir, store, trashed) = new_store();
    let today = Date::parse("2026-03-04").unwrap();
    let n = store.create_note(SPACE, None, "Plain", today).unwrap();
    store.trash_note(&n).unwrap();
    assert_eq!(trashed.lock().unwrap().len(), 1);
}

#[test]
fn a_leftover_folder_makes_its_slug_taken() {
    let (_dir, store, _) = new_store();
    let space = store.root().join("work");
    let today = Date::parse("2026-03-04").unwrap();
    fs::create_dir_all(space.join("notes/meeting")).unwrap();
    fs::create_dir_all(space.join("projects/inbox/tasks/fix-login")).unwrap();
    fs::create_dir_all(space.join("templates/bug")).unwrap();

    let n = store.create_note(SPACE, None, "Meeting", today).unwrap();
    let t = store
        .create_task(SPACE, "inbox", "Fix login", today)
        .unwrap();
    let t2 = store
        .create_task_in(SPACE, "inbox", "Todo", "Fix login", today, None)
        .unwrap();
    let tpl = store.create_template(SPACE, "Bug").unwrap();

    assert_eq!(n.slug, "meeting-2");
    assert_eq!(t.slug, "fix-login-2");
    assert_eq!(t2.slug, "fix-login-3");
    assert_eq!(tpl.slug, "bug-2");
    let other = store.create_template(SPACE, "Other").unwrap();
    fs::create_dir_all(space.join("templates/renamed")).unwrap();
    assert_eq!(
        store
            .rename_template(SPACE, &other.slug, "Renamed")
            .unwrap(),
        "renamed-2"
    );
}
