//! Direct projection of the editing model into the shared renderer's
//! `BlockNode`s (design §7.3): no Markdown round trip, so no render-mode
//! placeholders and no re-parse of a whole list for an edit inside it.
//! Every text leaf (paragraph, heading, code block, table cell) gets its
//! `BlockId` as `editor_key`.

use std::{collections::HashMap, ops::Range, sync::Arc};

use super::host_display::{
    IMAGE_PLACEHOLDER, ImageDisplay, ImageDisplayFn, LinkDecoration, LinkDecorationFn,
    PlaceholderData, attachment_icon_node, display_for,
};
use super::{
    document::{AtomData, BlockContent, EditorDocument},
    id::BlockId,
    inline::{InlineAtom, InlineText},
    kind::BlockKindData,
};
use gpui::SharedString;
use gpui_kit::base::text::{
    MarkdownNode,
    editor_host::{BlockNode, BlockState, CodeBlock, InlineNode, Paragraph, Span, Table, TextMark},
};

/// A paragraph holding `inline`, keyed by `id`. `open` holds the Link spans
/// shown as source; every other Link span is drawn as a chip.
pub(crate) fn paragraph(
    inline: &InlineText,
    id: BlockId,
    display: &Display,
    open: &[Range<usize>],
) -> Paragraph {
    let mut paragraph = Paragraph::default();
    // FORK: where a link starts that the host decorates.
    let decorated = display.decorations(inline);
    // FORK: Links as chips. The text is cut at the Link spans.
    let spans = super::wiki_chip::chip_spans(inline);
    for (range, mark, atom) in inline.pieces() {
        match atom {
            Some(InlineAtom::Image(image)) => match display.image_placeholder(image) {
                Some(node) => {
                    let len = node.text().len();
                    paragraph.push(node.marks(vec![(0..len, mark.clone())]))
                }
                None => paragraph.push(InlineNode::image(image.clone())),
            },
            Some(InlineAtom::Custom(node)) => {
                let node = InlineNode::custom(node.clone());
                let len = node.text().len();
                paragraph.push(node.marks(vec![(0..len, mark.clone())]));
            }
            None => {
                let mut at = range.start;
                // FORK: host link decoration. The paperclip is an inline
                // object before the link's first run; a missing Attachment is
                // struck through and no longer drawn as a link (the model
                // keeps the link, so following it still works).
                let run = decorated
                    .iter()
                    .find(|d| d.start <= range.start && range.end <= d.end);
                let mark = &match run {
                    Some(d) if d.kind == LinkDecoration::MissingAttachment => TextMark {
                        strikethrough: true,
                        link: None,
                        ..mark.clone()
                    },
                    _ => mark.clone(),
                };
                if let Some(d) = run.filter(|d| d.start == range.start) {
                    let icon = attachment_icon_node(d.kind == LinkDecoration::MissingAttachment);
                    let len = icon.text().len();
                    paragraph.push(icon.marks(vec![(0..len, TextMark::default())]));
                }
                let push_text = |paragraph: &mut Paragraph, range: Range<usize>| {
                    let text = &inline.text()[range];
                    if !text.is_empty() {
                        paragraph.push(
                            InlineNode::new(text.to_string())
                                .marks(vec![(0..text.len(), mark.clone())]),
                        );
                    }
                };
                for span in spans
                    .iter()
                    .filter(|s| range.start <= s.start && s.end <= range.end)
                {
                    push_text(&mut paragraph, at..span.start);
                    let source = &inline.text()[span.clone()];
                    if open.contains(span) {
                        // FORK: an open Link is inline code, in the
                        // projection only.
                        let mark = TextMark {
                            code: true,
                            ..mark.clone()
                        };
                        paragraph.push(
                            InlineNode::new(source.to_string())
                                .marks(vec![(0..source.len(), mark)]),
                        );
                    } else {
                        let chip = super::wiki_chip::chip_node(
                            source,
                            super::wiki_chip::ChipKey::new(id, span.start),
                        );
                        let len = chip.text().len();
                        paragraph.push(chip.marks(vec![(0..len, mark.clone())]));
                    }
                    at = span.end;
                }
                push_text(&mut paragraph, at..range.end);
            }
        }
    }
    paragraph.with_leaf_key(id.as_u64())
}

/// An image atom shown as its alt text, never loading the image. It is a
/// custom inline object without a renderer, which draws its text fallback and
/// stays one atomic caret and selection unit.
fn image_placeholder(
    image: &gpui_kit::base::text::editor_host::ImageNode,
    // FORK: the host's chip, an icon and a text. `None` is the plain-text
    // placeholder of `ImagePolicy::Placeholder`.
    chip: Option<(SharedString, SharedString)>,
) -> InlineNode {
    let label: SharedString = match &chip {
        Some((_, text)) => text.clone(),
        None => image
            .alt
            .clone()
            .filter(|alt| !alt.is_empty())
            .unwrap_or_else(|| "Image".into()),
    };
    let node = match chip {
        Some((icon, text)) => MarkdownNode::new(IMAGE_PLACEHOLDER, PlaceholderData { icon, text }),
        None => MarkdownNode::new(IMAGE_PLACEHOLDER, ()),
    };
    InlineNode::custom(node.text(label.clone()).accessibility_label(label))
}

/// FORK: what the host decides about how images and links are shown, as the
/// projection sees it.
#[derive(Default)]
pub(crate) struct Display {
    /// Present image atoms as text placeholders: nothing is resolved,
    /// decoded or loaded (`ImagePolicy::Placeholder`).
    pub(crate) placeholders: bool,
    pub(crate) images: Option<Arc<ImageDisplayFn>>,
    pub(crate) links: Option<Arc<LinkDecorationFn>>,
}

/// A link run the host decorates, in model offsets.
pub(crate) struct DecoratedLink {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: LinkDecoration,
}

impl Display {
    /// The placeholder to draw for `image`, or `None` to let the renderer
    /// load it.
    fn image_placeholder(
        &self,
        image: &gpui_kit::base::text::editor_host::ImageNode,
    ) -> Option<InlineNode> {
        if self.placeholders {
            return Some(image_placeholder(image, None));
        }
        let images = self.images.as_deref()?;
        match display_for(images, &image.url)? {
            ImageDisplay::Placeholder { icon, text } => {
                Some(image_placeholder(image, Some((icon, text))))
            }
            ImageDisplay::Load(_) => None,
        }
    }

    /// The link runs of `inline` the host decorates. A run is a stretch of
    /// text with the same destination, whatever its other marks.
    pub(crate) fn decorations(&self, inline: &InlineText) -> Vec<DecoratedLink> {
        let Some(decorate) = self.links.as_deref() else {
            return Vec::new();
        };
        let mut runs: Vec<(DecoratedLink, SharedString)> = Vec::new();
        for (range, mark, atom) in inline.pieces() {
            let Some(link) = mark.link.as_ref().filter(|_| atom.is_none()) else {
                continue;
            };
            match runs.last_mut() {
                Some((run, url)) if run.end == range.start && *url == link.url => {
                    run.end = range.end;
                }
                _ => match decorate(&link.url) {
                    LinkDecoration::None => {}
                    kind => runs.push((
                        DecoratedLink {
                            start: range.start,
                            end: range.end,
                            kind,
                        },
                        link.url.clone(),
                    )),
                },
            }
        }
        runs.into_iter().map(|(run, _)| run).collect()
    }
}

/// Project one block (and its subtree), uncached.
#[cfg(test)]
pub(crate) fn project_block(doc: &EditorDocument, id: BlockId) -> Option<BlockNode> {
    NodeCache::default().node(doc, id)
}

/// Per-node projection cache (design §7.3). Every block and table row is
/// cached under its id with the block revision it was built at; `touch`
/// bubbles revisions to ancestors, so an edit rebuilds only the path from
/// the edited leaf to its top-level block and reuses every other subtree
/// (sharing its retained `InlineState`s).
#[derive(Default)]
pub(crate) struct NodeCache {
    /// FORK: how the host shows images and links.
    pub(crate) display: Display,
    /// FORK: the Link spans shown as source, by leaf.
    open: super::wiki_chip::OpenSpans,
    nodes: HashMap<BlockId, (u64, BlockNode)>,
    rows: HashMap<BlockId, (u64, Vec<Paragraph>)>,
    /// Nodes and rows built (not reused), for the perf tests (D9).
    pub(crate) built: usize,
}

impl NodeCache {
    /// Drop entries of blocks that left the document, once the cache has
    /// grown well past the document (amortized O(1) per pass).
    pub(crate) fn collect_garbage(&mut self, doc: &EditorDocument) {
        if self.nodes.len() + self.rows.len() > 2 * doc.len() + 64 {
            self.nodes.retain(|id, _| doc.get(*id).is_some());
            self.rows.retain(|id, _| doc.get(*id).is_some());
        }
    }

    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.rows.clear();
    }

    /// FORK: the open Link spans of `leaf`.
    pub(crate) fn open_of(&self, leaf: BlockId) -> Vec<Range<usize>> {
        self.open.of(leaf)
    }

    /// FORK: show these Link spans as source. Drops the cached projection of
    /// the leaves that opened or closed and their ancestors, and returns
    /// those blocks.
    pub(crate) fn set_open(
        &mut self,
        doc: &EditorDocument,
        open: super::wiki_chip::OpenSpans,
    ) -> Vec<BlockId> {
        if self.open == open {
            return Vec::new();
        }
        let old = std::mem::replace(&mut self.open, open);
        let mut dirty: Vec<BlockId> = old
            .leaves()
            .chain(self.open.leaves())
            .flat_map(|leaf| doc.path(leaf))
            .collect();
        dirty.sort_by_key(|id| id.as_u64());
        dirty.dedup();
        for id in &dirty {
            self.nodes.remove(id);
            self.rows.remove(id);
        }
        dirty
    }

    pub(crate) fn node(&mut self, doc: &EditorDocument, id: BlockId) -> Option<BlockNode> {
        let revision = doc.block_revision(id);
        let node = match self.nodes.get(&id) {
            Some((rev, node)) if *rev == revision => node.clone(),
            _ => {
                let node = self.build(doc, id)?;
                self.built += 1;
                self.nodes.insert(id, (revision, node.clone()));
                node
            }
        };
        Some(node)
    }

    fn row(&mut self, doc: &EditorDocument, id: BlockId) -> Vec<Paragraph> {
        let revision = doc.block_revision(id);
        match self.rows.get(&id) {
            Some((rev, row)) if *rev == revision => row.clone(),
            _ => {
                self.built += 1;
                let row: Vec<Paragraph> = doc
                    .children(id)
                    .iter()
                    .map(|cell| match doc.inline(*cell) {
                        Some(inline) => {
                            paragraph(inline, *cell, &self.display, &self.open.of(*cell))
                        }
                        None => paragraph(&InlineText::new(), *cell, &self.display, &[]),
                    })
                    .collect();
                self.rows.insert(id, (revision, row.clone()));
                row
            }
        }
    }

    fn build(&mut self, doc: &EditorDocument, id: BlockId) -> Option<BlockNode> {
        let entry = doc.get(id)?;
        let children = |this: &mut Self| -> Vec<BlockNode> {
            doc.children(id)
                .iter()
                .filter_map(|child| this.node(doc, *child))
                .collect()
        };
        let open = self.open.of(id);
        let inline = || match &entry.content {
            BlockContent::Inline(inline) => paragraph(inline, id, &self.display, &open),
            _ => paragraph(&InlineText::new(), id, &self.display, &[]),
        };
        Some(match &entry.kind {
            BlockKindData::Root => BlockNode::Root {
                children: children(self),
                span: None,
            },
            BlockKindData::Paragraph => BlockNode::Paragraph(inline()),
            BlockKindData::Heading { level } => BlockNode::Heading {
                level: *level,
                children: inline(),
                span: None,
            },
            // FORK: a Callout projects as a plain quote; `element.rs` draws
            // its box.
            BlockKindData::Blockquote { .. } => BlockNode::Blockquote {
                children: children(self),
                span: None,
            },
            BlockKindData::List { ordered, start, .. } => BlockNode::List {
                children: children(self),
                ordered: *ordered,
                start: *start,
                span: None,
            },
            BlockKindData::ListItem { checked, spread } => BlockNode::ListItem {
                children: children(self),
                spread: *spread,
                checked: *checked,
                // FORK PATCH: field added in gpui-kit 0.9.1; the projection has no source spans.
                task_marker: None,
                span: None,
            },
            BlockKindData::CodeBlock { lang, meta } => {
                let code = match &entry.content {
                    BlockContent::Plain(code) => code.clone(),
                    _ => String::new(),
                };
                let code = CodeBlock::new(code.into(), lang.clone(), None::<Span>)
                    .with_meta(meta.clone())
                    .with_leaf_key(id.as_u64());
                BlockNode::CodeBlock(code)
            }
            BlockKindData::Table { aligns, .. } => BlockNode::Table(Table::new(
                doc.children(id)
                    .iter()
                    .map(|row| self.row(doc, *row))
                    .collect(),
                aligns.clone(),
            )),
            BlockKindData::TableRow | BlockKindData::TableCell => return None,
            BlockKindData::HorizontalRule => BlockNode::HorizontalRule { span: None },
            BlockKindData::Break { html } => BlockNode::Break {
                html: *html,
                span: None,
            },
            BlockKindData::Custom => match &entry.content {
                BlockContent::Atom(AtomData::Custom(node)) => BlockNode::Custom {
                    node: node.clone(),
                    state: BlockState::default(),
                },
                _ => BlockNode::Unknown,
            },
            BlockKindData::Definition {
                identifier,
                url,
                title,
            } => BlockNode::Definition {
                identifier: identifier.clone(),
                url: url.clone(),
                title: title.clone(),
                span: None,
            },
            BlockKindData::Unknown => BlockNode::Unknown,
        })
    }
}
