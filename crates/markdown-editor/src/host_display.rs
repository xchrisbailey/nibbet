//! FORK: how the host decides to show each image and link.
//!
//! The editor never resolves a destination itself. The host gives a callback
//! that turns an image's raw destination into an [`ImageDisplay`], and one
//! that marks a link's destination as an Attachment (or a missing one). The
//! model and the Markdown never see either: only the projection and the
//! renderer do.

use std::sync::Arc;

use ::markdown::mdast::Node;
use gpui::{
    AnyElement, App, Image, ImageFormat, ImageSource, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, SharedString, SharedUri, StatefulInteractiveElement as _,
    Styled as _, Window, div, rems,
};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::text::{
    InlineElement, InlineRenderContext, MarkdownNode, MarkdownParseContext, MarkdownPlugin,
};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};

/// How the host wants one image shown.
#[derive(Clone)]
pub enum ImageDisplay {
    /// Load it from this source.
    Load(ImageSource),
    /// Draw a small muted chip instead: a Lucide icon (`image-off`, `globe`)
    /// and a text. The image atom stays in the document, and so does its
    /// Markdown.
    Placeholder {
        icon: SharedString,
        text: SharedString,
    },
}

/// How the host wants one link decorated. The Markdown is the same for all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkDecoration {
    None,
    /// A paperclip before the link text.
    Attachment,
    /// A muted, struck-through link, with a muted paperclip before it.
    MissingAttachment,
}

/// Decides how the image with this raw destination shows. It runs whenever a
/// leaf with the image is projected, and again for every loaded image on every
/// frame, so it must be cheap and must not touch the disk. After the answers
/// change, call `BlockEditorState::set_image_display` again.
///
/// A `data:` destination never reaches it: the editor always decodes those.
pub type ImageDisplayFn = dyn Fn(&str) -> ImageDisplay + Send + Sync;

/// Decides how the link with this raw destination is decorated. It runs when
/// a leaf with the link is projected, not every frame; after the answers
/// change, call `BlockEditorState::set_link_decoration` again.
pub type LinkDecorationFn = dyn Fn(&str) -> LinkDecoration + Send + Sync;

pub(crate) const IMAGE_PLACEHOLDER: &str = "image-placeholder";
pub(crate) const ATTACHMENT_ICON: &str = "sett-attachment-icon";

const ICON_IMAGE_OFF: &[u8] = include_bytes!("../assets/icons/image-off.svg");
const ICON_GLOBE: &[u8] = include_bytes!("../assets/icons/globe.svg");
const ICON_PAPERCLIP: &[u8] = include_bytes!("../assets/icons/paperclip.svg");

/// The SVG for a Lucide icon name the host may ask for. Anything else shows
/// as `image-off`.
fn icon_data(name: &str) -> &'static [u8] {
    match name {
        "globe" => ICON_GLOBE,
        "paperclip" => ICON_PAPERCLIP,
        _ => ICON_IMAGE_OFF,
    }
}

/// What the placeholder chip draws.
pub(crate) struct PlaceholderData {
    pub(crate) icon: SharedString,
    pub(crate) text: SharedString,
}

/// What the paperclip before a link draws.
pub(crate) struct AttachmentIconData {
    pub(crate) missing: bool,
}

/// The paperclip object. One `U+FFFC`, like a Link chip.
pub(crate) fn attachment_icon_node(missing: bool) -> gpui_kit::base::text::editor_host::InlineNode {
    gpui_kit::base::text::editor_host::InlineNode::custom(
        MarkdownNode::new(ATTACHMENT_ICON, AttachmentIconData { missing })
            .text("")
            .accessibility_label("Attachment"),
    )
}

/// How to show an image atom, or `None` to leave it to the renderer.
pub(crate) fn display_for(display: &ImageDisplayFn, url: &str) -> Option<ImageDisplay> {
    if url.starts_with("data:") {
        return None;
    }
    Some(display(url))
}

/// The source the renderer loads for `uri` when the host decides. A `data:`
/// URL is decoded here, because gpui-base stops doing it once a resolver is
/// set and the URL would otherwise go to the HTTP loader.
pub(crate) fn source_for(display: &ImageDisplayFn, uri: &SharedUri) -> ImageSource {
    if let Some(image) = data_url_image(uri) {
        return ImageSource::Image(image);
    }
    match display(uri) {
        ImageDisplay::Load(source) => source,
        // The projection decided on a placeholder for this image already.
        // Stay "still loading" rather than let gpui fetch the destination.
        ImageDisplay::Placeholder { .. } => ImageSource::Custom(Arc::new(|_, _| None)),
    }
}

/// Decodes an image `data:` URL, base64 or percent-encoded. `None` for
/// anything else.
pub(crate) fn data_url_image(url: &str) -> Option<Arc<Image>> {
    let rest = url.strip_prefix("data:")?;
    let (head, body) = rest.split_once(',')?;
    let mut parts = head.split(';');
    let mime = parts.next()?.trim().to_ascii_lowercase();
    let base64 = parts.any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let format = ImageFormat::from_mime_type(&mime)?;
    let bytes = if base64 {
        decode_base64(body)?
    } else {
        percent_decode(body)
    };
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub(crate) fn percent_decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[at],
            bytes.get(at + 1).and_then(|b| hex(*b)),
            bytes.get(at + 2).and_then(|b| hex(*b)),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                at += 3;
            }
            (byte, ..) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    out
}

/// Draws `image-placeholder` nodes the host asked for. The editor's own
/// `ImagePolicy::Placeholder` nodes carry no data and keep drawing as text.
pub(crate) struct ImagePlaceholderPlugin;

impl MarkdownPlugin for ImagePlaceholderPlugin {
    fn name(&self) -> &str {
        IMAGE_PLACEHOLDER
    }

    fn parse(&self, _: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        None
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        _: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let data = node.data::<PlaceholderData>()?;
        Some(InlineElement::new(placeholder_chip(data, cx)))
    }
}

/// The widest a placeholder grows; a longer text ends in an ellipsis.
const MAX_WIDTH_REMS: f32 = 20.;

pub(crate) fn placeholder_chip(data: &PlaceholderData, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id("image-placeholder")
        .role(Role::Image)
        .aria_label(data.text.clone())
        .test_support()
        .items_center()
        .gap_1()
        .px_1()
        .max_w(rems(MAX_WIDTH_REMS))
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded(theme.radius)
        .bg(theme.muted)
        .text_color(theme.muted_foreground)
        .child(Icon::default().data(icon_data(&data.icon)).xsmall())
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(data.text.clone()),
        )
        .into_any_element()
}

/// Draws the paperclip in front of an Attachment link.
pub(crate) struct AttachmentIconPlugin;

impl MarkdownPlugin for AttachmentIconPlugin {
    fn name(&self) -> &str {
        ATTACHMENT_ICON
    }

    fn parse(&self, _: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        None
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        _: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let data = node.data::<AttachmentIconData>()?;
        let theme = cx.theme();
        let color = if data.missing {
            theme.muted_foreground
        } else {
            theme.link
        };
        Some(InlineElement::new(
            h_flex()
                .id("attachment-icon")
                .role(Role::Image)
                .aria_label("Attachment")
                .test_support()
                .items_center()
                .pr_1()
                .text_color(color)
                .child(Icon::default().data(ICON_PAPERCLIP).xsmall()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_and_percent_encoded_data_urls() {
        let png = data_url_image("data:image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(png.format(), ImageFormat::Png);
        assert_eq!(png.bytes(), b"\x89PNG\r\n\x1a\n");
        let svg = data_url_image("data:image/svg+xml,%3Csvg%3E%3C/svg%3E").unwrap();
        assert_eq!(svg.format(), ImageFormat::Svg);
        assert_eq!(svg.bytes(), b"<svg></svg>");
        assert!(data_url_image("https://example.com/a.png").is_none());
        assert!(data_url_image("data:text/plain;base64,aGVsbG8=").is_none());
        assert!(data_url_image("data:image/png;base64,not*base64").is_none());
    }
}
