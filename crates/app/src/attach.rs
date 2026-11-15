//! Adding Attachments (ADR 0004): paste, drop and the Insert Menu picker all
//! gather sources, copy them into the Item's folder, then insert Markdown.
//! This is the copying, the Markdown and where it lands; `ItemEditor` runs it
//! off the UI thread and tells it which editor and caret.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use gpui_kit::{App, AppContext as _, ClipboardEntry, ClipboardItem, Context, Window};
use markdown_editor::{
    BlockEditorState, BlockId, BlockKind, BlockPosition, BlockSelection, PasteRequest,
};
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

/// One attached file's Markdown: an image or a link.
#[derive(Debug)]
struct FileLink {
    markdown: String,
    image: bool,
}

/// What copying the sources produced: Markdown for the ones that worked, and
/// the ones that didn't.
#[derive(Debug, Default)]
pub(crate) struct Copied {
    links: Vec<FileLink>,
    failed: Vec<(String, io::Error)>,
}

/// Copies each source into the Item's folder, in order. Blocking. A folder
/// or a failed copy is recorded and the rest carry on.
pub(crate) fn copy(
    store: &Store,
    space: Space,
    owner: &AttachmentOwner,
    sources: Vec<Source>,
) -> Copied {
    let mut out = Copied::default();
    for source in sources {
        let label = source.label();
        match copy_one(store, space, owner, &source) {
            Ok(link) => out.links.push(link),
            Err(err) => out.failed.push((label, err)),
        }
    }
    out
}

fn copy_one(
    store: &Store,
    space: Space,
    owner: &AttachmentOwner,
    source: &Source,
) -> io::Result<FileLink> {
    let (attached, link_text, alt) = match source {
        Source::Path(path) => {
            if path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "It\u{2019}s a folder",
                ));
            }
            let attached = store.attach_file(space, owner, path)?;
            let alt = attached.name.clone();
            (attached, file_name(path), alt)
        }
        Source::Image { ext, bytes } => {
            let attached = store.attach_bytes(space, owner, ext, bytes)?;
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
        FileLink {
            markdown: format!("![{}]({destination})", escape(&alt)), // md-escaped
            image: true,
        }
    } else {
        FileLink {
            markdown: format!("[{}]({destination})", escape(&link_text)), // md-escaped
            image: false,
        }
    })
}

/// What is ready to insert, and what to tell the user about what wasn't.
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) markdown: String,
    /// Whether each block stays whole (images, batches) instead of merging
    /// into the text around the caret.
    pub(crate) as_blocks: bool,
    pub(crate) notice: Option<String>,
}

impl Copied {
    /// Images, and any batch of several files, get a paragraph each. One
    /// other file is an inline link at the caret.
    pub(crate) fn into_outcome(self) -> Outcome {
        Outcome {
            as_blocks: self.links.len() > 1 || self.links.iter().any(|link| link.image),
            markdown: self
                .links
                .iter()
                .map(|link| link.markdown.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
            notice: notice("attach", &self.failed),
        }
    }
}

/// The error notice for what failed (`verb` is "attach" or "copy"), or
/// `None` when nothing did.
fn notice(verb: &str, failed: &[(String, io::Error)]) -> Option<String> {
    let (_, first) = failed.first()?;
    let names = failed
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let reason = first.to_string();
    let reason = reason.trim_end_matches('.');
    Some(format!("Couldn\u{2019}t {verb} {names}. {reason}."))
}

pub(crate) fn file_name(path: &Path) -> String {
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

/// Where in the block view a drop of files goes.
#[derive(Clone, Copy)]
pub(crate) enum DropAt {
    /// The caret nearest the pointer.
    Pointer,
    /// The end of the body.
    End,
}

/// While files are dragged over the block view, and when they drop: puts the
/// caret under `pointer` or at the end of the body, so the Markdown lands
/// where the user is aiming.
pub(crate) fn place_caret(
    block: &mut BlockEditorState,
    at: DropAt,
    pointer: gpui_kit::Point<gpui_kit::Pixels>,
    window: &mut Window,
    cx: &mut Context<BlockEditorState>,
) {
    match at {
        DropAt::Pointer => {
            if let Some(position) = block.position_at(pointer)
                && block
                    .set_selection(BlockSelection::caret(position), cx)
                    .is_ok()
            {
                block.focus(window, cx);
            }
        }
        DropAt::End => {
            block.focus(window, cx);
            // Only fails for an empty document, where the caret is already
            // at the end.
            block.select_end(cx).ok();
        }
    }
}

/// The last text copied or cut from an editor, and the Item it came from,
/// app-wide. A paste of the same text into another Item brings the source's
/// Attachments along (ADR 0004).
struct CopiedFrom {
    text: String,
    space: Space,
    owner: AttachmentOwner,
}

impl gpui_kit::Global for CopiedFrom {}

/// Remembers a copy or cut from `from`, or forgets the last one when it was
/// raw code (`markdown` false), which is never rewritten. A cut leaves the
/// source's files where they are.
pub(crate) fn remember_copy(
    cx: &mut App,
    text: &str,
    markdown: bool,
    from: Option<(Space, AttachmentOwner)>,
) {
    match from {
        Some((space, owner)) if markdown => cx.set_global(CopiedFrom {
            text: text.to_string(),
            space,
            owner,
        }),
        _ => {
            if cx.has_global::<CopiedFrom>() {
                cx.remove_global::<CopiedFrom>();
            }
        }
    }
}

/// Where `text` was copied from, when that is another Item than `to`: the
/// paste then carries its Attachments. `None` for text from outside sett or
/// from the same Item.
pub(crate) fn copied_from(
    cx: &App,
    text: &str,
    to: (Space, &AttachmentOwner),
) -> Option<(Space, AttachmentOwner)> {
    let from = cx
        .try_global::<CopiedFrom>()
        .filter(|from| from.text == text)?;
    (from.space != to.0 || &from.owner != to.1).then(|| (from.space, from.owner.clone()))
}

/// Runs `work` off the UI thread, then `then` on the entity it was started
/// from, so a link is never drawn before its file exists.
pub(crate) fn off_thread<T: 'static, R: Send + 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
    work: impl FnOnce() -> R + Send + 'static,
    then: impl FnOnce(&mut T, R, &mut Window, &mut Context<T>) + 'static,
) {
    cx.spawn_in(window, async move |this, cx| {
        let result = cx.background_spawn(async move { work() }).await;
        this.update_in(cx, |this, window, cx| then(this, result, window, cx))
            .ok();
    })
    .detach();
}

/// Where in the block view finished Markdown goes.
#[derive(Debug, Clone)]
pub(crate) enum Landing {
    /// The caret as it is when the copying is done (a drop moved it first).
    Caret,
    /// The paste that started it: settled first, then the Markdown goes in at
    /// the caret as it is now even when the paste went stale.
    Paste(PasteRequest),
    /// Like [`Landing::Paste`], but the Markdown is pasted as the clipboard's
    /// text would be, so the editor's own paste rules apply.
    PasteText(PasteRequest),
    /// The empty block the Insert Menu left, however the document changed
    /// while the files were copied: it is filled if still empty, otherwise the
    /// Markdown goes in as blocks after its text. A block that is gone, or no
    /// longer at the top level, means the end of the body.
    Menu(BlockId),
}

impl Landing {
    /// The Insert Menu's block, from the caret it leaves there.
    pub(crate) fn menu(block: &BlockEditorState) -> Landing {
        match block.selection() {
            BlockSelection::Text { head, .. } => Landing::Menu(head.block),
            _ => Landing::Caret,
        }
    }
}

/// Puts `markdown` into the block view at `landing`, as one undo step.
pub(crate) fn land(
    block: &mut BlockEditorState,
    landing: &Landing,
    markdown: &str,
    as_blocks: bool,
    cx: &mut Context<BlockEditorState>,
) {
    let insert = |block: &mut BlockEditorState, as_blocks, cx: &mut Context<BlockEditorState>| {
        if as_blocks {
            block.insert_markdown_blocks(markdown, cx);
        } else {
            block.insert_markdown(markdown, cx);
        }
    };
    match landing {
        Landing::Caret => insert(block, as_blocks, cx),
        Landing::Paste(request) => {
            block.apply_paste(request, None, cx);
            if !markdown.is_empty() {
                insert(block, as_blocks, cx);
            }
        }
        Landing::PasteText(request) => {
            if !block.apply_paste(request, Some(markdown), cx) {
                insert(block, as_blocks, cx);
            }
        }
        Landing::Menu(id) => {
            let (at, empty) = match menu_caret(block, *id) {
                Some((at, empty)) => (Some(at), empty),
                None => (None, false),
            };
            let moved = match at {
                Some(at) => block.set_selection(BlockSelection::caret(at), cx),
                // Only fails for an empty document, where the caret is
                // already at the end.
                None => block.select_end(cx).or(Ok(())),
            };
            if moved.is_ok() {
                insert(block, as_blocks || !empty, cx);
            }
        }
    }
}

/// The end of the top-level text block `id` and whether it is an empty
/// paragraph; `None` when it is gone or no longer at the top level.
fn menu_caret(block: &BlockEditorState, id: BlockId) -> Option<(BlockPosition, bool)> {
    let doc = block.model().document();
    let leaf = doc.block(id)?;
    let text = leaf.text()?;
    (leaf.parent()?.id() == doc.root().id()).then(|| {
        (
            BlockPosition::new(id, text.len()),
            text.is_empty() && leaf.kind() == BlockKind::Paragraph,
        )
    })
}

/// Markdown whose Attachments were copied into another Item's folder.
#[derive(Debug, Default)]
pub(crate) struct Carried {
    /// The Markdown with its destinations pointed at the copies.
    markdown: String,
    failed: Vec<(String, io::Error)>,
}

impl Carried {
    pub(crate) fn into_outcome(self) -> Outcome {
        Outcome {
            markdown: self.markdown,
            as_blocks: false,
            notice: notice("copy", &self.failed),
        }
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

        let outcome = copy(
            &store,
            Space::Personal,
            &item,
            vec![Source::Path(top), Source::Path(deep)],
        );

        assert_eq!(
            outcome.into_outcome().markdown,
            "[my (draft).pdf](<plan/my (draft).pdf>)\n\n![shot 1](<plan/sub dir/shot 1.png>)"
        );
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    }

    #[test]
    fn the_notice_names_every_failure() {
        let copied = Copied {
            links: Vec::new(),
            failed: vec![
                ("a folder".into(), io::Error::other("It\u{2019}s a folder")),
                ("report.pdf".into(), io::Error::other("disk full")),
            ],
        };
        assert_eq!(
            copied.into_outcome().notice.as_deref(),
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
