//! FORK: images the host loads, drawn as our own inline object.
//!
//! gpui-base's `InlineFlow::image_element` is private and has no style hook,
//! so a loaded image can't get a border, a radius or a height cap. Instead the
//! projection swaps the image atom for an `sett-image` object (one `U+FFFC`,
//! like a Link chip) that draws `img(source)` itself:
//!
//! - natural size, one image pixel per point,
//! - shrunk to the width of the leaf it sits in, then to at most [`MAX_HEIGHT`]
//!   tall, with the aspect ratio kept,
//! - a 1px `border` outline and the theme's small radius.
//!
//! The renderer measures an inline object once, before it knows the wrap
//! width, so the leaf's width comes from the previous frame ([`ImageWidths`]):
//! the editor's right edge less the leaf's left edge (a list or quote indents
//! only on the left) and less the right padding of a Callout. A leaf's own
//! bounds are its text's, not the column's, so they can't give it.
//! Until a leaf has one, the editor's content width bounds the image, and
//! until that is known, the window's. The frame records the widths and asks
//! for one more redraw only when one the images use changed.
//!
//! The decoded size comes from measuring an `img` element, which loads
//! through the host's image cache and notifies the view when it is done.

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, RwLock},
};

use ::markdown::mdast::Node;
use gpui::{
    AnyElement, App, AvailableSpace, InteractiveElement as _, IntoElement as _, ObjectFit,
    ParentElement as _, Pixels, Role, SharedString, SharedUri, Size,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img, px, size,
};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::text::{
    InlineElement, InlineRenderContext, MarkdownNode, MarkdownParseContext, MarkdownPlugin,
    editor_host::InlineNode,
};
use gpui_kit::component::ActiveTheme as _;

use super::host_display::{
    ImageDisplayFn, PlaceholderData, percent_decode, placeholder_chip, source_for,
};

pub(crate) const IMAGE_OBJECT: &str = "sett-image";

/// The tallest an image is drawn, border included.
pub(crate) const MAX_HEIGHT: Pixels = px(480.);

/// Width changes smaller than this are measuring noise, not a reflow.
const WIDTH_EPSILON: f32 = 0.5;

/// What the plugin needs to draw one image.
pub(crate) struct ImageObjectData {
    /// The host's answer for a destination, asked each frame: a source can
    /// hold a closure that is not `Send`, which node data must be.
    display: Arc<ImageDisplayFn>,
    url: SharedUri,
    /// `BlockId::as_u64` of the leaf, to find the width it wraps at.
    leaf: u64,
    /// The accessibility label: the alt text, or "Image".
    label: SharedString,
    /// What a failed load shows: the file name.
    name: SharedString,
}

/// The object for an image the host loads. `url` is the raw destination.
pub(crate) fn image_node(
    display: Arc<ImageDisplayFn>,
    leaf: u64,
    url: &SharedUri,
    alt: Option<&SharedString>,
) -> InlineNode {
    let label: SharedString = alt
        .filter(|alt| !alt.is_empty())
        .cloned()
        .unwrap_or_else(|| "Image".into());
    InlineNode::custom(
        MarkdownNode::new(
            IMAGE_OBJECT,
            ImageObjectData {
                display,
                url: url.clone(),
                leaf,
                name: file_name(url).unwrap_or_else(|| label.clone()),
                label: label.clone(),
            },
        )
        .text(label.clone())
        .accessibility_label(label),
    )
}

/// The last path segment of a destination, decoded. `None` for `data:` URLs
/// and destinations that end in a slash.
fn file_name(url: &str) -> Option<SharedString> {
    if url.starts_with("data:") {
        return None;
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().filter(|name| !name.is_empty())?;
    Some(
        String::from_utf8_lossy(&percent_decode(last))
            .into_owned()
            .into(),
    )
}

/// `natural` shrunk to fit `max_w` by `max_h`, aspect ratio kept, never
/// enlarged.
pub(crate) fn fit(natural: Size<Pixels>, max_w: Pixels, max_h: Pixels) -> Size<Pixels> {
    let scale = (max_w / natural.width)
        .min(max_h / natural.height)
        .min(1.)
        .max(0.);
    size(natural.width * scale, natural.height * scale)
}

/// The widths the images wrap at, as the last frame laid them out.
#[derive(Clone, Default)]
pub(crate) struct ImageWidths(Arc<RwLock<Widths>>);

#[derive(Default)]
struct Widths {
    /// The editor's content width.
    editor: Option<Pixels>,
    /// The width of each leaf an image asked about.
    leaves: HashMap<u64, Pixels>,
    /// Leaves with an image object, which the frame records.
    asked: HashSet<u64>,
}

fn differs(a: Option<Pixels>, b: Pixels) -> bool {
    a.is_none_or(|a| (a - b).abs() > px(WIDTH_EPSILON))
}

impl ImageWidths {
    /// The most an image in `leaf` may be wide: its leaf's width, else the
    /// editor's, else `fallback`. Remembers that the leaf has an image.
    fn bound(&self, leaf: u64, fallback: Pixels) -> Pixels {
        {
            let widths = self.0.read().unwrap_or_else(|e| e.into_inner());
            if widths.asked.contains(&leaf) {
                return widths
                    .leaves
                    .get(&leaf)
                    .or(widths.editor.as_ref())
                    .copied()
                    .unwrap_or(fallback);
            }
        }
        let mut widths = self.0.write().unwrap_or_else(|e| e.into_inner());
        widths.asked.insert(leaf);
        widths
            .leaves
            .get(&leaf)
            .or(widths.editor.as_ref())
            .copied()
            .unwrap_or(fallback)
    }

    /// Takes this frame's widths. Returns whether one an image used changed,
    /// so the editor redraws once with it.
    pub(crate) fn record(
        &self,
        editor: Pixels,
        leaves: impl Iterator<Item = (u64, Pixels)>,
    ) -> bool {
        let mut widths = self.0.write().unwrap_or_else(|e| e.into_inner());
        if widths.asked.is_empty() {
            widths.editor = Some(editor);
            return false;
        }
        let mut changed = differs(widths.editor, editor)
            && widths
                .asked
                .iter()
                .any(|leaf| !widths.leaves.contains_key(leaf));
        widths.editor = Some(editor);
        for (leaf, width) in leaves {
            if !widths.asked.contains(&leaf) {
                continue;
            }
            if differs(widths.leaves.get(&leaf).copied(), width) {
                widths.leaves.insert(leaf, width);
                changed = true;
            }
        }
        changed
    }
}

/// Draws `sett-image` nodes.
pub(crate) struct ImageObjectPlugin {
    pub(crate) widths: ImageWidths,
}

impl MarkdownPlugin for ImageObjectPlugin {
    fn name(&self) -> &str {
        IMAGE_OBJECT
    }

    fn parse(&self, _: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        None
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        ctx: &InlineRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let data = node.data::<ImageObjectData>()?;
        let source = source_for(&*data.display, &data.url);
        let bound = self.widths.bound(data.leaf, window.viewport_size().width);

        // Measuring is what loads the image, and what finds out it failed.
        let failed = Rc::new(Cell::new(false));
        let flag = failed.clone();
        let natural = img(source.clone())
            .id("image-probe")
            .with_fallback(move || {
                flag.set(true);
                div().into_any_element()
            })
            .into_any_element()
            .layout_as_root(AvailableSpace::min_size(), window, cx);
        eprintln!(
            "PROBE natural={natural:?} failed={} bound={bound:?}",
            failed.get()
        );
        if failed.get() {
            let chip = PlaceholderData {
                icon: "image-off".into(),
                text: data.name.clone(),
            };
            return Some(InlineElement::new(placeholder_chip(&chip, cx)));
        }

        let theme = cx.theme();
        let border = px(1.);
        let frame = div()
            .id("image")
            .role(Role::Image)
            .aria_label(data.label.clone())
            .test_support()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .overflow_hidden();
        if natural.width <= px(0.) || natural.height <= px(0.) {
            // Still loading: the box the renderer reserves for an image of
            // no known size, three quarters of a line.
            let side = ctx.line_height() * 0.75;
            return Some(InlineElement::new(frame.w(side).h(side)));
        }
        let inner = fit(
            natural,
            (bound - border * 2.).max(px(1.)),
            MAX_HEIGHT - border * 2.,
        );
        let outer = size(inner.width + border * 2., inner.height + border * 2.);
        let element: AnyElement = frame
            .w(outer.width)
            .h(outer.height)
            .child(
                img(source.clone())
                    .w(inner.width)
                    .h(inner.height)
                    .rounded(theme.radius)
                    .object_fit(ObjectFit::Fill),
            )
            .into_any_element();
        // Centre the box on the text's x-height, as the renderer does for its
        // own images.
        let font = ctx.text_style().font();
        let font_id = window.text_system().resolve_font(&font);
        let ascent = window.text_system().ascent(font_id, ctx.font_size());
        let descent = window.text_system().descent(font_id, ctx.font_size());
        let line = ctx.line_height();
        let body_baseline = (line - (ascent + descent)) / 2. + ascent;
        let baseline = (outer.height / 2. + body_baseline - line / 2.)
            .max(px(0.))
            .min(outer.height);
        Some(InlineElement::new(element).with_baseline(baseline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shrinks_to_the_tighter_bound_and_never_enlarges() {
        let fit = |w, h, mw, mh| {
            let s = fit(size(px(w), px(h)), px(mw), px(mh));
            (f32::from(s.width).round(), f32::from(s.height).round())
        };
        assert_eq!(fit(100., 50., 600., 478.), (100., 50.));
        assert_eq!(fit(2000., 100., 600., 478.), (600., 30.));
        assert_eq!(fit(100., 1000., 600., 478.), (48., 478.));
    }

    #[test]
    fn a_redraw_is_asked_only_when_a_width_an_image_uses_changes() {
        let widths = ImageWidths::default();
        let frame = |editor: f32, leaf: f32| {
            widths.record(px(editor), [(1, px(leaf)), (2, px(50.))].into_iter())
        };
        // No image asked yet: nothing to redraw.
        assert!(!frame(600., 600.));
        assert_eq!(widths.bound(1, px(1920.)), px(600.));
        // The leaf's own width is new to the image: one redraw, then quiet.
        assert!(frame(600., 580.));
        assert!(!frame(600., 580.2));
        assert_eq!(widths.bound(1, px(1920.)), px(580.));
        // Leaf 2 holds no image, so its width never matters.
        assert!(!widths.record(px(600.), [(2, px(10.))].into_iter()));
        // The window shrinks.
        assert!(frame(400., 380.));
    }

    #[test]
    fn names_the_file_from_the_destination() {
        assert_eq!(
            file_name("fix/my%20shot.png?x=1").as_deref(),
            Some("my shot.png")
        );
        assert_eq!(file_name("data:image/png;base64,AA"), None);
        assert_eq!(file_name("https://example.com/dir/"), None);
    }
}
