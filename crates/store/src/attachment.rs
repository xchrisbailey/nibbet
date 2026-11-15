//! Attachments (ADR 0004): files kept in a folder named after their Item's
//! slug, beside the Item's file, and linked from its body with relative paths.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Date, JournalEntry, Note, Space, Store, Task, Template, check_slug, invalid, slug};

/// What can own Attachments: an Item or a Template. Names it within a Space, so the
/// Store methods take the `Space` beside it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttachmentOwner {
    Task {
        project: String,
        slug: String,
    },
    Note {
        /// `None` for a Space-level Note.
        project: Option<String>,
        slug: String,
    },
    Journal(Date),
    Template(String),
}

impl AttachmentOwner {
    fn slug(&self) -> String {
        match self {
            AttachmentOwner::Task { slug, .. } | AttachmentOwner::Note { slug, .. } => slug.clone(),
            AttachmentOwner::Journal(date) => date.to_string(),
            AttachmentOwner::Template(slug) => slug.clone(),
        }
    }
}

impl From<&Task> for AttachmentOwner {
    fn from(task: &Task) -> Self {
        AttachmentOwner::Task {
            project: task.project.clone(),
            slug: task.slug.clone(),
        }
    }
}

impl From<&Note> for AttachmentOwner {
    fn from(note: &Note) -> Self {
        AttachmentOwner::Note {
            project: note.project.clone(),
            slug: note.slug.clone(),
        }
    }
}

impl From<&JournalEntry> for AttachmentOwner {
    fn from(entry: &JournalEntry) -> Self {
        AttachmentOwner::Journal(entry.date)
    }
}

impl From<&Template> for AttachmentOwner {
    fn from(template: &Template) -> Self {
        AttachmentOwner::Template(template.slug.clone())
    }
}

/// A file that now sits in an Item's Attachment folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    /// The link destination, relative to the Item's directory: `fix-login/name.png`.
    pub destination: String,
    /// The original name without its extension, for alt or link text.
    pub name: String,
    /// Whether the body shows it as an image (`![name](dest)`) rather than a
    /// link (`[name.ext](dest)`).
    pub inline: bool,
}

/// What a link destination in a body points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Found(PathBuf),
    Missing(PathBuf),
    /// Absolute, a URL, or outside the Item's Space (ADR 0002).
    Refused,
}

const INLINE: [&str; 10] = [
    "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff", "ico",
];
/// Formats WebKit can't draw, converted to JPEG on the way in.
const CONVERT: [&str; 3] = ["heic", "heif", "avif"];

/// Whether a file with this extension (no dot) shows inline as an image.
pub fn is_inline_image(ext: &str) -> bool {
    INLINE.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

impl Store {
    /// The folder holding `item`'s Attachments. It may not exist yet: it is
    /// created with the first Attachment.
    pub fn attachment_dir(&self, space: Space, item: &AttachmentOwner) -> io::Result<PathBuf> {
        let slug = item.slug();
        check_slug(&slug)?;
        Ok(self.item_dir(space, item)?.join(slug))
    }

    /// The directory holding the Item's own file.
    fn item_dir(&self, space: Space, item: &AttachmentOwner) -> io::Result<PathBuf> {
        match item {
            AttachmentOwner::Task { project, .. } => {
                check_slug(project)?;
                Ok(self.project_dir(space, project).join("tasks"))
            }
            AttachmentOwner::Note { project, .. } => {
                if let Some(p) = project {
                    check_slug(p)?;
                }
                Ok(self.notes_dir(space, project.as_deref()))
            }
            AttachmentOwner::Journal(_) => Ok(self.journal_dir(space)),
            AttachmentOwner::Template(_) => Ok(self.templates_dir(space)),
        }
    }

    /// Copies `source` into the Item's folder, or links it if it is already
    /// there. Blocking: call it off the UI thread.
    pub fn attach_file(
        &self,
        space: Space,
        item: &AttachmentOwner,
        source: &Path,
    ) -> io::Result<Attached> {
        if source.is_dir() {
            return Err(invalid(format!("{} is a folder", source.display())));
        }
        let folder = self.attachment_dir(space, item)?;
        let slug = item.slug();
        let file_name = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid(format!("invalid path {source:?}")))?;
        let (stem, ext) = split_name(file_name);

        // Already in this Item's folder, at any depth: nothing to copy.
        if let (Ok(have), Ok(dir)) = (fs::canonicalize(source), fs::canonicalize(&folder))
            && let Ok(inside) = have.strip_prefix(&dir)
        {
            let relative = inside
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            return Ok(Attached {
                destination: format!("{slug}/{relative}"),
                name: stem.to_string(),
                inline: is_inline_image(&ext),
            });
        }

        let (ext, bytes) = if CONVERT.contains(&ext.as_str()) {
            match convert_to_jpeg(source) {
                Some(jpeg) => ("jpg".to_string(), jpeg),
                None => (ext, fs::read(source)?),
            }
        } else {
            (ext, fs::read(source)?)
        };
        self.store_bytes(&folder, &slug, stem, &ext, &bytes)
    }

    /// Writes clipboard image data as `pasted-image.<ext>`. Blocking.
    pub fn attach_bytes(
        &self,
        space: Space,
        item: &AttachmentOwner,
        ext: &str,
        bytes: &[u8],
    ) -> io::Result<Attached> {
        let folder = self.attachment_dir(space, item)?;
        self.store_bytes(
            &folder,
            &item.slug(),
            "pasted-image",
            &clean_ext(ext),
            bytes,
        )
    }

    fn store_bytes(
        &self,
        folder: &Path,
        slug: &str,
        stem: &str,
        ext: &str,
        bytes: &[u8],
    ) -> io::Result<Attached> {
        let base = if stem.chars().any(char::is_alphanumeric) {
            slug::slugify(stem)
        } else {
            "attachment".to_string()
        };
        let file = |n: u32| match (n, ext.is_empty()) {
            (1, true) => base.clone(),
            (1, false) => format!("{base}.{ext}"),
            (n, true) => format!("{base}-{n}"),
            (n, false) => format!("{base}-{n}.{ext}"),
        };
        self.create_dirs(folder)?;
        let mut chosen = None;
        for n in 1.. {
            let name = file(n);
            let path = folder.join(&name);
            match fs::read(&path) {
                Ok(existing) if existing == bytes => {
                    chosen = Some((name, false));
                    break;
                }
                Ok(_) => continue,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    chosen = Some((name, true));
                    break;
                }
                // A folder or unreadable file in the way counts as a clash.
                Err(_) if path.exists() => continue,
                Err(e) => return Err(e),
            }
        }
        let (name, write) = chosen.expect("an unbounded range always yields a free name");
        if write {
            let path = folder.join(&name);
            // Hidden temp then rename, so the watcher never sees a half file.
            let tmp = folder.join(format!(".{name}.sett-tmp"));
            fs::write(&tmp, bytes)?;
            self.log.file(self.rel(&path), bytes);
            fs::rename(&tmp, &path)?;
        }
        Ok(Attached {
            destination: format!("{slug}/{name}"),
            name: stem.to_string(),
            inline: is_inline_image(&clean_ext(ext)),
        })
    }

    /// Where a body's link destination leads for `item`. Purely lexical apart
    /// from the final existence check, so it is cheap enough to call while
    /// rendering; callers cache the answer.
    pub fn resolve_attachment(
        &self,
        space: Space,
        item: &AttachmentOwner,
        destination: &str,
    ) -> Resolved {
        let Ok(dir) = self.item_dir(space, item) else {
            return Resolved::Refused;
        };
        let destination = destination.trim();
        let destination = destination
            .strip_prefix('<')
            .and_then(|d| d.strip_suffix('>'))
            .unwrap_or(destination);
        let Some(decoded) = percent_decode(destination) else {
            return Resolved::Refused;
        };
        if decoded.is_empty() || decoded.starts_with(['/', '\\']) || has_scheme(&decoded) {
            return Resolved::Refused;
        }
        let mut path = dir;
        for part in Path::new(&decoded).components() {
            match part {
                Component::Normal(p) => path.push(p),
                Component::CurDir => {}
                Component::ParentDir => {
                    path.pop();
                }
                Component::RootDir | Component::Prefix(_) => return Resolved::Refused,
            }
        }
        if !path.starts_with(self.space_dir(space)) {
            return Resolved::Refused;
        }
        if path.exists() {
            Resolved::Found(path)
        } else {
            Resolved::Missing(path)
        }
    }

    /// Trashes the Item's Attachment folder, if it has one. Called after the
    /// Item's file went; a failure here doesn't bring the file back.
    pub(crate) fn trash_attachments(&self, space: Space, item: &AttachmentOwner) -> io::Result<()> {
        let folder = self.attachment_dir(space, item)?;
        if folder.is_dir() {
            self.trash(&folder)?;
        }
        Ok(())
    }
}

/// `(stem, lowercase extension)`; a leading dot is part of the stem only when
/// nothing follows it.
fn split_name(file_name: &str) -> (&str, String) {
    let path = Path::new(file_name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(clean_ext)
        .unwrap_or_default();
    // `.png` has the stem `.png` and no extension: treat it as an empty stem.
    if ext.is_empty() && file_name.starts_with('.') && file_name.matches('.').count() == 1 {
        return ("", clean_ext(&file_name[1..]));
    }
    (stem, ext)
}

fn clean_ext(ext: &str) -> String {
    ext.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

static TEMP: AtomicU32 = AtomicU32::new(0);

/// macOS's own converter, so no image crate is needed. `None` on any failure,
/// and the caller then keeps the original.
fn convert_to_jpeg(source: &Path) -> Option<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!(
        "sett-convert-{}-{}.jpg",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let ok = Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg"])
        .arg(source)
        .arg("--out")
        .arg(&tmp)
        .output()
        .is_ok_and(|o| o.status.success());
    let bytes = ok.then(|| fs::read(&tmp).ok()).flatten();
    let _ = fs::remove_file(&tmp);
    bytes.filter(|b| !b.is_empty())
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `https:`, `file:`, `mailto:` and so on.
fn has_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}
