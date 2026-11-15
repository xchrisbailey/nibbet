//! How the block view shows and opens what an Item's body links to as an
//! Attachment (ADR 0004): images, files, and the Items' own folders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use crate::attach::file_name;
use gpui_kit::{App, ImageSource, Resource};
use markdown_editor::{ImageDisplay, LinkDecoration};
use sett_store::{AttachmentOwner, Resolved, Space, Store, is_inline_image};

/// How a Cmd-clicked Attachment reaches the system's default app. Tests set
/// one to see what would open; without it the platform opener runs.
#[derive(Clone)]
pub struct AttachmentOpener(pub Rc<OpenFn>);

type OpenFn = dyn Fn(&Path, &mut App);

impl gpui_kit::Global for AttachmentOpener {}

pub(crate) fn open_attachment(path: &Path, cx: &mut App) {
    match cx.try_global::<AttachmentOpener>().cloned() {
        Some(opener) => (opener.0)(path, cx),
        None => cx.open_with_system(path),
    }
}

/// Answers for one open Item. The editor's callbacks run on every frame, so
/// each destination is resolved against the disk once and remembered; a
/// fresh `Attachments` replaces this one when the Item changes, its body
/// reloads, or a file in its folder changes.
pub(crate) struct Attachments {
    store: Store,
    space: Space,
    item: AttachmentOwner,
    resolved: Mutex<HashMap<String, Resolved>>,
}

/// What a Cmd-click on a link or image should do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Open {
    /// Hand this file to the system.
    File(PathBuf),
    /// The file is gone; say so, by this name.
    Missing(String),
    /// Not an Attachment: leave it to the web and mail handling.
    Ignore,
}

impl Attachments {
    pub(crate) fn new(store: Store, space: Space, item: AttachmentOwner) -> Attachments {
        Attachments {
            store,
            space,
            item,
            resolved: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn item(&self) -> &AttachmentOwner {
        &self.item
    }

    fn resolve(&self, destination: &str) -> Resolved {
        // A panic while holding the lock can't leave the map inconsistent.
        let mut resolved = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        resolved
            .entry(destination.to_string())
            .or_insert_with(|| {
                self.store
                    .resolve_attachment(self.space, &self.item, destination)
            })
            .clone()
    }

    pub(crate) fn image_display(&self, destination: &str) -> ImageDisplay {
        match self.resolve(destination) {
            Resolved::Found(path) if is_image_file(&path) => {
                ImageDisplay::Load(ImageSource::Resource(Resource::Path(path.into())))
            }
            Resolved::Found(path) => chip(
                "image-off",
                format!("Can\u{2019}t show {} as an image", file_name(&path)),
            ),
            Resolved::Missing(path) => {
                chip("image-off", format!("Missing image: {}", file_name(&path)))
            }
            Resolved::Refused => match web_host(destination) {
                Some(host) => chip("globe", format!("Image not loaded: {host}")),
                None => chip("globe", "Image not loaded".to_string()),
            },
        }
    }

    pub(crate) fn link_decoration(&self, destination: &str) -> LinkDecoration {
        if is_fragment(destination) {
            return LinkDecoration::None;
        }
        match self.resolve(destination) {
            Resolved::Found(_) => LinkDecoration::Attachment,
            Resolved::Missing(_) => LinkDecoration::MissingAttachment,
            Resolved::Refused => LinkDecoration::None,
        }
    }

    pub(crate) fn open(&self, destination: &str) -> Open {
        if is_fragment(destination) {
            return Open::Ignore;
        }
        match self.resolve(destination) {
            Resolved::Found(path) => Open::File(path),
            Resolved::Missing(path) => Open::Missing(file_name(&path)),
            Resolved::Refused => Open::Ignore,
        }
    }
}

fn chip(icon: &str, text: String) -> ImageDisplay {
    ImageDisplay::Placeholder {
        icon: icon.to_string().into(),
        text: text.into(),
    }
}

/// `#heading` points inside the page, not at a file.
fn is_fragment(destination: &str) -> bool {
    destination.trim().starts_with('#')
}

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_inline_image)
}

/// The host of an `http(s)` (or protocol-relative) URL, without credentials
/// or port. Anything else has none worth showing.
fn web_host(destination: &str) -> Option<&str> {
    let rest = destination.trim();
    let rest = match rest.split_once("://") {
        Some((scheme, rest))
            if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") =>
        {
            rest
        }
        Some(_) => return None,
        None => rest.strip_prefix("//")?,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => host.split(':').next()?,
    };
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn attachments() -> (tempfile::TempDir, Attachments) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let notes = dir.path().join("personal/notes");
        fs::create_dir_all(notes.join("plan")).unwrap();
        fs::write(notes.join("plan/shot.png"), b"x").unwrap();
        fs::write(notes.join("plan/report.pdf"), b"x").unwrap();
        let item = AttachmentOwner::Note {
            project: None,
            slug: "plan".into(),
        };
        (dir, Attachments::new(store, Space::Personal, item))
    }

    fn text(display: ImageDisplay) -> (String, String) {
        match display {
            ImageDisplay::Placeholder { icon, text } => (icon.to_string(), text.to_string()),
            ImageDisplay::Load(_) => panic!("expected a placeholder"),
        }
    }

    #[test]
    fn a_found_image_loads_from_inside_the_items_folder() {
        let (dir, a) = attachments();
        let ImageDisplay::Load(ImageSource::Resource(Resource::Path(path))) =
            a.image_display("plan/shot.png")
        else {
            panic!("expected a path");
        };
        assert!(path.starts_with(dir.path().join("personal/notes/plan")));
        assert!(path.ends_with("shot.png"));
    }

    #[test]
    fn images_that_cannot_load_get_a_chip() {
        let (_dir, a) = attachments();
        let cases = [
            ("plan/gone.png", "image-off", "Missing image: gone.png"),
            (
                "plan/report.pdf",
                "image-off",
                "Can\u{2019}t show report.pdf as an image",
            ),
            (
                "https://user@example.com:8080/a.png",
                "globe",
                "Image not loaded: example.com",
            ),
            ("../../../../etc/passwd", "globe", "Image not loaded"),
            ("/etc/passwd", "globe", "Image not loaded"),
        ];
        for (destination, icon, label) in cases {
            assert_eq!(
                text(a.image_display(destination)),
                (icon.to_string(), label.to_string()),
                "{destination}"
            );
        }
    }

    #[test]
    fn links_are_decorated_only_when_they_point_into_the_space() {
        let (_dir, a) = attachments();
        let cases = [
            ("plan/report.pdf", LinkDecoration::Attachment),
            ("plan/gone.pdf", LinkDecoration::MissingAttachment),
            ("https://example.com", LinkDecoration::None),
            ("mailto:a@b.co", LinkDecoration::None),
            ("#heading", LinkDecoration::None),
            ("../../../x.pdf", LinkDecoration::None),
        ];
        for (destination, want) in cases {
            assert_eq!(a.link_decoration(destination), want, "{destination}");
        }
    }

    #[test]
    fn opening_depends_on_the_file() {
        let (dir, a) = attachments();
        assert_eq!(
            a.open("plan/report.pdf"),
            Open::File(dir.path().join("personal/notes/plan/report.pdf"))
        );
        assert_eq!(
            a.open("plan/gone.pdf"),
            Open::Missing("gone.pdf".to_string())
        );
        assert_eq!(a.open("../../../../etc/passwd"), Open::Ignore);
        assert_eq!(a.open("https://example.com"), Open::Ignore);
    }
}
