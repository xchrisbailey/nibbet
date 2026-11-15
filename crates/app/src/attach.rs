//! Adding Attachments (ADR 0004): paste, drop and the Insert Menu picker all
//! gather sources, copy them into the Item's folder, then insert Markdown.
//! This is the copying and the Markdown; `ItemEditor` runs it off the UI
//! thread and puts the result in the body.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::{ClipboardEntry, ClipboardItem};
use sett_store::{AttachmentOwner, Resolved, Space, Store};

/// Something to attach.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// A file from disk (a drop, a pick, or files copied in Finder).
    Path(PathBuf),
    /// Image data from the clipboard.
    Image { ext: String, bytes: Vec<u8> },
}

impl Source {
    /// What to call it in a notice.
    fn label(&self) -> String {
        match self {
            Source::Path(path) => file_name(path),
            Source::Image { .. } => "the pasted image".to_string(),
        }
    }

    /// Finder files, or else image data. Finder's own file-name text is
    /// ignored. Empty when the clipboard holds neither.
    pub(crate) fn from_clipboard(item: &ClipboardItem) -> Vec<Source> {
        let paths: Vec<Source> = item
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                _ => None,
            })
            .flatten()
            .map(|path| Source::Path(path.clone()))
            .collect();
        if !paths.is_empty() {
            return paths;
        }
        item.entries()
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(Source::Image {
                    ext: image.format.extension().to_string(),
                    bytes: image.bytes.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The file names, for pasting into a Code Block as text.
    pub(crate) fn names(sources: &[Source]) -> String {
        sources
            .iter()
            .filter_map(|source| match source {
                Source::Path(path) => Some(file_name(path)),
                Source::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// One attached file's Markdown.
#[derive(Debug)]
struct Piece {
    markdown: String,
    image: bool,
}

/// What copying the sources produced: Markdown for the ones that worked, and
/// the ones that didn't.
#[derive(Debug, Default)]
pub(crate) struct Attached {
    pieces: Vec<Piece>,
    failed: Vec<(String, io::Error)>,
}

/// Copies each source into the Item's folder, in order. Blocking. A folder
/// or a failed copy is recorded and the rest carry on.
pub(crate) fn copy(
    store: &Store,
    space: Space,
    item: &AttachmentOwner,
    sources: Vec<Source>,
) -> Attached {
    let mut out = Attached::default();
    for source in sources {
        let label = source.label();
        match copy_one(store, space, item, &source) {
            Ok(piece) => out.pieces.push(piece),
            Err(err) => out.failed.push((label, err)),
        }
    }
    out
}

fn copy_one(
    store: &Store,
    space: Space,
    item: &AttachmentOwner,
    source: &Source,
) -> io::Result<Piece> {
    let (attached, link_text, alt) = match source {
        Source::Path(path) => {
            if path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "It\u{2019}s a folder",
                ));
            }
            let attached = store.attach_file(space, item, path)?;
            let alt = attached.name.clone();
            (attached, file_name(path), alt)
        }
        Source::Image { ext, bytes } => {
            let attached = store.attach_bytes(space, item, ext, bytes)?;
            let name = attached
                .destination
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string();
            (attached, name, String::new())
        }
    };
    let destination = markdown_editor::markdown_destination(&attached.destination);
    Ok(if attached.inline {
        Piece {
            markdown: format!("![{}]({destination})", escape(&alt)),
            image: true,
        }
    } else {
        Piece {
            markdown: format!("[{}]({destination})", escape(&link_text)),
            image: false,
        }
    })
}

impl Attached {
    /// Whether anything went in.
    pub(crate) fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// Images, and any batch of several files, get a paragraph each. One
    /// other file is an inline link at the caret.
    pub(crate) fn as_blocks(&self) -> bool {
        self.pieces.len() > 1 || self.pieces.iter().any(|piece| piece.image)
    }

    /// The Markdown to insert: paragraphs in order when [`Self::as_blocks`],
    /// otherwise the one link.
    pub(crate) fn markdown(&self) -> String {
        self.pieces
            .iter()
            .map(|piece| piece.markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// The error notice for what didn't go in, or `None` when all did.
    pub(crate) fn notice(&self) -> Option<String> {
        let (_, first) = self.failed.first()?;
        let names = self
            .failed
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let reason = first.to_string();
        let reason = reason.trim_end_matches('.');
        Some(format!("Couldn\u{2019}t attach {names}. {reason}."))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Backslash-escapes what would end a link's label early.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Markdown whose Attachments were copied into another Item's folder.
#[derive(Debug, Default)]
pub(crate) struct Carried {
    /// The Markdown with its destinations pointed at the copies.
    pub(crate) markdown: String,
    failed: Vec<(String, io::Error)>,
}

impl Carried {
    /// The error notice for what couldn't be copied, or `None` when all was.
    pub(crate) fn notice(&self) -> Option<String> {
        let (_, first) = self.failed.first()?;
        let names = self
            .failed
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let reason = first.to_string();
        let reason = reason.trim_end_matches('.');
        Some(format!("Couldn\u{2019}t copy {names}. {reason}."))
    }
}

/// Copies the files `markdown` links to relative to the Item `from` into the
/// Item `to`, and points those links at the copies. Blocking.
///
/// A destination that isn't a file there (a web link, a missing file, a path
/// outside the Space) stays as written, so the text still reads as it did. A
/// file that can't be copied also stays as written, and is reported.
pub(crate) fn carry(
    store: &Store,
    from: (Space, &AttachmentOwner),
    to: (Space, &AttachmentOwner),
    markdown: &str,
) -> Carried {
    let mut failed = Vec::new();
    // One copy per file however often the text links to it.
    let mut done: HashMap<String, Option<String>> = HashMap::new();
    let markdown = markdown_editor::map_destinations(markdown, |destination| {
        done.entry(destination.to_string())
            .or_insert_with(|| {
                let Resolved::Found(path) = store.resolve_attachment(from.0, from.1, destination)
                else {
                    return None;
                };
                if !path.is_file() {
                    return None;
                }
                match store.attach_file(to.0, to.1, &path) {
                    Ok(attached) => Some(attached.destination),
                    Err(err) => {
                        failed.push((file_name(&path), err));
                        None
                    }
                }
            })
            .clone()
    });
    Carried { markdown, failed }
}

/// `markdown` with its links into the folder `old/` pointed at `new/`, for
/// when an Item's folder is renamed with it.
pub(crate) fn rename_folder(markdown: &str, old: &str, new: &str) -> String {
    let prefix = format!("{old}/");
    markdown_editor::map_destinations(markdown, |destination| {
        destination
            .strip_prefix(&prefix)
            .map(|rest| format!("{new}/{rest}"))
    })
}

/// What goes into the raw Markdown at `cursor` in `text`: `markdown`, set
/// off by blank lines when it is paragraphs.
pub(crate) fn raw_insertion(text: &str, cursor: usize, markdown: &str, as_blocks: bool) -> String {
    if !as_blocks {
        return markdown.to_string();
    }
    let before = &text[..cursor.min(text.len())];
    let after = &text[cursor.min(text.len())..];
    let lead = blank_lines_needed(before.chars().rev());
    let trail = blank_lines_needed(after.chars());
    format!("{lead}{markdown}{trail}")
}

/// The newlines to add so a blank line separates `markdown` from the text on
/// one side; nothing when that side is empty (the edge of the file).
fn blank_lines_needed(mut near: impl Iterator<Item = char>) -> &'static str {
    let Some(first) = near.next() else { return "" };
    let newlines = std::iter::once(first)
        .chain(near)
        .take_while(|&c| c == '\n')
        .count();
    match newlines {
        0 => "\n\n",
        1 => "\n",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_to_files_already_in_the_folder_are_escaped_at_any_depth() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let item = AttachmentOwner::Note {
            project: None,
            slug: "plan".into(),
        };
        let folder = store.attachment_dir(Space::Personal, &item).unwrap();
        std::fs::create_dir_all(folder.join("sub dir")).unwrap();
        let top = folder.join("my (draft).pdf");
        let deep = folder.join("sub dir/shot 1.png");
        std::fs::write(&top, b"x").unwrap();
        std::fs::write(&deep, b"x").unwrap();

        let attached = copy(
            &store,
            Space::Personal,
            &item,
            vec![Source::Path(top), Source::Path(deep)],
        );

        assert_eq!(
            attached.markdown(),
            "[my (draft).pdf](<plan/my (draft).pdf>)\n\n![shot 1](<plan/sub dir/shot 1.png>)"
        );
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    }

    #[test]
    fn the_notice_names_every_failure() {
        let attached = Attached {
            pieces: Vec::new(),
            failed: vec![
                ("a folder".into(), io::Error::other("It\u{2019}s a folder")),
                ("report.pdf".into(), io::Error::other("disk full")),
            ],
        };
        assert_eq!(
            attached.notice().as_deref(),
            Some("Couldn\u{2019}t attach a folder, report.pdf. It\u{2019}s a folder.")
        );
    }

    #[test]
    fn raw_images_get_blank_lines_only_where_text_is_adjacent() {
        let md = "![a](a.png)";
        assert_eq!(raw_insertion("", 0, md, true), md);
        assert_eq!(
            raw_insertion("one\n\ntwo", 5, md, true),
            md.to_string() + "\n\n"
        );
        assert_eq!(raw_insertion("one\ntwo", 3, md, true), "\n\n![a](a.png)\n");
        assert_eq!(raw_insertion("one two", 3, md, true), "\n\n![a](a.png)\n\n");
        assert_eq!(raw_insertion("one two", 3, "[f](f)", false), "[f](f)");
    }

    #[test]
    fn link_text_escapes_brackets() {
        assert_eq!(escape("a [1] b\\c"), "a \\[1\\] b\\\\c");
    }
}
