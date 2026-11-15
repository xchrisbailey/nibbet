//! The Markdown writer behind the serializer: delimiter-flanking aware, hoists
//! whitespace out of marked groups, escapes literal punctuation and writes
//! highlight as inline `<mark>`.
//!
//! This is the editor's own copy of the writer gpui-kit's TextView uses for
//! copy-as-source, with the options the editor needs always on. It is kept
//! here so the host API of gpui-kit stays small.

use gpui::Hsla;
use gpui_kit::base::text::editor_host::TextMark;

/// Delimiter-safe shared writer seam for links, references and images.
pub(crate) fn markdown_destination(value: &str) -> String {
    let angled = value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '<' | '>'));
    let mut escaped = String::new();
    for (ix, ch) in value.char_indices() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '&' if entity_like(&value[ix + 1..]) => escaped.push_str("\\&"),
            '<' => escaped.push_str("\\<"),
            '>' => escaped.push_str("\\>"),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            _ => escaped.push(ch),
        }
    }
    if angled {
        format!("<{escaped}>")
    } else {
        escaped
    }
}
pub(crate) fn markdown_title(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for (ix, ch) in value.char_indices() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '&' if entity_like(&value[ix + 1..]) => escaped.push_str("\\&"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("&#10;"),
            '\r' => escaped.push_str("&#13;"),
            _ => escaped.push(ch),
        }
    }
    format!("\"{escaped}\"")
}

/// How [`MarkdownSource`] writes its output; the serializer turns all three on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]

pub(crate) struct MarkdownSourceOptions {
    /// Write highlight as inline `<mark>` (with a `color` attribute unless it
    /// is the default colour) instead of `==x==`, which the parser reads back
    /// as literal text.
    pub(crate) highlight_as_html: bool,
    /// Move leading and trailing whitespace of a marked group outside its
    /// delimiters: `**a **b` is literal text, `**a** b` is emphasis.
    pub(crate) hoist_whitespace: bool,
    /// Backslash-escape Markdown punctuation in text pushed with
    /// [`MarkdownSource::push_literal`], so literal `*` stays literal.
    pub(crate) escape_literals: bool,
    /// FORK: the text is a table cell, where an unescaped `|` would split the
    /// cell even inside a wiki-link.
    pub(crate) table_cell: bool,
}

/// The highlight colour `<mark>` gets when it carries no colour of its own
/// (`format::html::default_mark_color`).
fn default_highlight_color() -> Hsla {
    gpui::rgb(0xfef08a).into()
}

/// `#rrggbb`, or `#rrggbbaa` when the colour is not opaque.
pub(crate) fn hex_color(color: Hsla) -> String {
    let rgba = gpui::Rgba::from(color);
    let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    let (r, g, b, a) = (byte(rgba.r), byte(rgba.g), byte(rgba.b), byte(rgba.a));
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Keep syntax and marks separate until all selected runs and atomic objects
/// have been collected. Wrapping every run independently creates adjacent `*`
/// delimiters that can turn italic formulas into bold ones when pasted.
#[derive(Default)]
pub(crate) struct MarkdownSource {
    pieces: Vec<(String, Vec<TextMark>)>,
    options: MarkdownSourceOptions,
}

impl MarkdownSource {
    pub(crate) fn with_options(options: MarkdownSourceOptions) -> Self {
        Self {
            pieces: Vec::new(),
            options,
        }
    }

    /// Push plain text (not Markdown source) carrying `mark`. With
    /// `escape_literals`, Markdown punctuation is backslash-escaped, except
    /// inside code, where escapes are not interpreted.
    pub(crate) fn push_literal(&mut self, text: &str, mark: &TextMark) {
        if !self.options.escape_literals || mark.code {
            self.push_marked(text, mark);
            return;
        }
        let mut escaped = String::with_capacity(text.len());
        let mut urls = url_spans(text).into_iter().peekable();
        let mut wiki_links = wiki_link_spans(text).into_iter().peekable();
        let mut prev: Option<char> = None;
        let mut ix = 0;
        // FORK: a `[!NOTE]` callout marker opening the text is written as
        // is; escaped, other tools no longer see the callout.
        if self.pieces.is_empty()
            && let Some(len) = callout_marker_len(text)
        {
            escaped.push_str(&text[..len]);
            prev = Some(']');
            ix = len;
        }
        while ix < text.len() {
            // A bare URL is written as is: the GFM autolink literal it parses
            // back to is the same text, and escapes would be pasted into it.
            if let Some(url) = urls.next_if(|url| url.range.start == ix) {
                let source = &text[url.range.clone()];
                if url.angled {
                    escaped.push('<');
                    escaped.push_str(source);
                    escaped.push('>');
                    prev = Some('>');
                } else {
                    escaped.push_str(source);
                    prev = source.chars().last();
                }
                ix = url.range.end;
                continue;
            }
            // FORK: `[[wiki-links]]` are written as is, so other tools still
            // see the link after a save.
            if let Some(link) = wiki_links.next_if(|link| link.start == ix) {
                let link_text = &text[link.clone()];
                if self.options.table_cell {
                    escaped.push_str(&link_text.replace('|', "\\|"));
                } else {
                    escaped.push_str(link_text);
                }
                prev = Some(']');
                ix = link.end;
                continue;
            }
            let Some(ch) = text[ix..].chars().next() else {
                break;
            };
            let rest = &text[ix + ch.len_utf8()..];
            let next = rest.chars().next();
            match ch {
                // Hard breaks, including terminal/table/styled breaks, need
                // an explicit semantic form. Backslash-newline at EOF reparses
                // as a literal backslash; whitespace hoisting can break marks.
                '\n' => escaped.push_str("<br>"),
                // An underscore between alphanumerics can neither open nor
                // close emphasis (`snake_case`), so it needs no escape.
                '_' if prev.is_some_and(char::is_alphanumeric)
                    && next.is_some_and(char::is_alphanumeric) =>
                {
                    escaped.push(ch)
                }
                // `&` is only special when it starts a character reference.
                '&' if !entity_like(rest) => escaped.push(ch),
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '$' | '&' | '|' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
            prev = Some(ch);
            ix += ch.len_utf8();
        }
        self.push_marked(&escaped, mark);
    }

    pub(crate) fn push_marked(&mut self, source: &str, mark: &TextMark) {
        if source.is_empty() {
            return;
        }
        // Outer-to-inner order for equally extensive marks. When a mark spans
        // more neighboring pieces, serialize it outside the shorter marks.
        let mut layers = Vec::new();
        if let Some(link) = &mark.link {
            layers.push(TextMark {
                link: Some(link.clone()),
                ..Default::default()
            });
        }
        if let Some(highlight) = mark.highlight {
            layers.push(TextMark {
                highlight: Some(highlight),
                ..Default::default()
            });
        }
        if mark.underline {
            layers.push(TextMark::default().underline());
        }
        if mark.strikethrough {
            layers.push(TextMark::default().strikethrough());
        }
        if mark.bold {
            layers.push(TextMark::default().bold());
        }
        if mark.italic {
            layers.push(TextMark::default().italic());
        }
        if mark.code {
            layers.push(TextMark::default().code());
        }
        self.pieces.push((source.to_string(), layers));
    }

    pub(crate) fn finish(mut self) -> String {
        /// `before` / `after`: the characters just outside `pieces`, for the
        /// delimiter flanking check (`None` = start or end of the text).
        fn write(
            pieces: &mut [(String, Vec<TextMark>)],
            options: MarkdownSourceOptions,
            before: Option<char>,
            after: Option<char>,
        ) -> String {
            let mut out = String::new();
            let mut offset = 0;
            let mut prev_marked = false;
            while offset < pieces.len() {
                // Nothing inside a code span is parsed, so in the editor's
                // mode code is always the innermost layer.
                let only_code = |mark: &&TextMark| {
                    !options.hoist_whitespace || !mark.code || pieces[offset].1.len() == 1
                };
                let Some((mark, count)) = pieces[offset]
                    .1
                    .iter()
                    .filter(only_code)
                    .map(|mark| {
                        let count = pieces[offset..]
                            .iter()
                            .take_while(|(_, layers)| {
                                layers.contains(mark)
                                    && (!options.hoist_whitespace
                                        || !mark.code
                                        || layers.len() == 1)
                            })
                            .count();
                        (mark.clone(), count)
                    })
                    .reduce(|best, candidate| {
                        if candidate.1 > best.1 {
                            candidate
                        } else {
                            best
                        }
                    })
                else {
                    out.push_str(&pieces[offset].0);
                    offset += 1;
                    prev_marked = false;
                    continue;
                };
                // A directly adjacent marked neighbour has delimiters of its
                // own that could merge with ours into one run (`***`), so
                // count it as a word character: only a non-punctuation edge
                // is then safe.
                let outside_before = if prev_marked {
                    Some('a')
                } else {
                    out.chars().last().or(before)
                };
                let outside_after = match pieces.get(offset + count) {
                    Some((_, layers)) if !layers.is_empty() => Some('a'),
                    Some((text, _)) => text.chars().next(),
                    None => after,
                };
                let group = &mut pieces[offset..offset + count];
                for (_, layers) in group.iter_mut() {
                    layers.retain(|layer| layer != &mark);
                }
                let content = write(group, options, Some('*'), Some('*'));
                out.push_str(&wrap_layer(
                    &content,
                    &mark,
                    options,
                    outside_before,
                    outside_after,
                ));
                offset += count;
                prev_marked = true;
            }
            out
        }
        let options = self.options;
        write(&mut self.pieces, options, None, None)
    }
}

/// Wrap one mark layer around already-written content, per `options`.
///
/// With `hoist_whitespace`, delimiters are also kept *flanking* in the
/// CommonMark sense: when `*`, `**` or `~~` would sit between a word
/// character outside and punctuation inside (`a*`code`*`), it could not open
/// or close, so the HTML form the parser reads back (`<em>`, `<strong>`,
/// `<s>`) is used instead.
fn wrap_layer(
    content: &str,
    mark: &TextMark,
    options: MarkdownSourceOptions,
    before: Option<char>,
    after: Option<char>,
) -> String {
    if mark.code && options.escape_literals {
        // Code delimiters must not collide with literal backtick runs. Code
        // whitespace belongs inside the span, never hoisted into prose.
        let longest = content
            .split(|ch| ch != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let fence = "`".repeat(longest + 1);
        let pad = content.starts_with('`')
            || content.ends_with('`')
            || (content.starts_with(' ')
                && content.ends_with(' ')
                && content.chars().any(|ch| ch != ' '));
        return if pad {
            format!("{fence} {content} {fence}")
        } else {
            format!("{fence}{content}{fence}")
        };
    }
    let (lead, core, trail) = if options.hoist_whitespace {
        let core = content.trim_matches(char::is_whitespace);
        if core.is_empty() {
            return content.to_string();
        }
        let start = content.len() - content.trim_start_matches(char::is_whitespace).len();
        (&content[..start], core, &content[start + core.len()..])
    } else {
        ("", content, "")
    };
    let wrapped = match mark.highlight {
        Some(color) if options.highlight_as_html && !core.is_empty() => {
            if color == default_highlight_color() {
                format!("<mark>{core}</mark>")
            } else {
                format!("<mark color=\"{}\">{core}</mark>", hex_color(color))
            }
        }
        _ if options.hoist_whitespace
            && (mark.bold || mark.italic || mark.strikethrough)
            && !delimiters_flank(
                lead.chars().last().or(before),
                core,
                trail.chars().next().or(after),
            ) =>
        {
            let tag = if mark.bold {
                "strong"
            } else if mark.italic {
                "em"
            } else {
                "s"
            };
            format!("<{tag}>{core}</{tag}>")
        }
        _ => wrap_with_mark(core, mark),
    };
    format!("{lead}{wrapped}{trail}")
}

/// Whether delimiters around `core` would be left-flanking at the start and
/// right-flanking at the end, given the characters outside.
fn delimiters_flank(before: Option<char>, core: &str, after: Option<char>) -> bool {
    let weak =
        |ch: Option<char>| ch.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation());
    let (Some(first), Some(last)) = (core.chars().next(), core.chars().last()) else {
        return true;
    };
    let opens = !first.is_whitespace() && (!first.is_ascii_punctuation() || weak(before));
    let closes = !last.is_whitespace() && (!last.is_ascii_punctuation() || weak(after));
    opens && closes
}

/// Wrap `text` with the Markdown syntax implied by `mark`, innermost first.
fn wrap_with_mark(text: &str, mark: &TextMark) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut out = text.to_string();
    if mark.code {
        out = format!("`{}`", out);
    }
    if mark.italic {
        out = format!("*{}*", out);
    }
    if mark.bold {
        out = format!("**{}**", out);
    }
    if mark.strikethrough {
        out = format!("~~{}~~", out);
    }
    if mark.underline {
        // Markdown has no underline syntax, and `__` reads as bold to most
        // parsers, so fall back to the inline HTML `<u>` parses from.
        out = format!("<u>{}</u>", out);
    }
    if mark.highlight.is_some() {
        out = format!("=={}==", out);
    }
    if let Some(link) = &mark.link {
        // A link whose text is its own URL is written as an autolink, which
        // keeps the URL verbatim: autolinks interpret neither escapes nor
        // character references.
        if link.title.is_none() && out == link.url.as_ref() && autolink_safe(&link.url) {
            return format!("<{}>", link.url);
        }
        out = match &link.title {
            Some(title) => format!(
                "[{}]({} {})",
                out,
                markdown_destination(&link.url),
                markdown_title(title)
            ),
            None => format!("[{}]({})", out, markdown_destination(&link.url)),
        };
    }
    out
}

/// Whether `rest`, the text after an `&`, would make it a character reference
/// (`&amp;`, `&#38;`, `&#x26;`) that a Markdown parser decodes. Any other `&`
/// is literal and needs no escape.
fn entity_like(rest: &str) -> bool {
    let (body, digit): (&str, fn(&char) -> bool) = match rest.strip_prefix('#') {
        Some(numeric) => match numeric.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, char::is_ascii_hexdigit),
            None => (numeric, char::is_ascii_digit),
        },
        None if rest.starts_with(|ch: char| ch.is_ascii_alphabetic()) => {
            (rest, char::is_ascii_alphanumeric)
        }
        None => return false,
    };
    let len = body.chars().take_while(digit).count();
    len > 0 && body[len..].starts_with(';')
}

/// FORK: the length of a `[!TYPE]` callout marker at the start of `text`, if
/// it can be written bare: nothing after it turns the brackets into a link.
fn callout_marker_len(text: &str) -> Option<usize> {
    let kind = text.strip_prefix("[!")?;
    let kind_len = kind
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-')))
        .filter(|len| *len > 0)?;
    let rest = kind[kind_len..].strip_prefix(']')?;
    (!rest.starts_with(['(', '[', ':'])).then_some(2 + kind_len + 1)
}

/// FORK: `[[target]]`, `[[target|label]]` and `[[target#heading]]` spans in
/// literal text that can be written without escapes. Only spans whose inner
/// text needs no escaping qualify (bar the `|` alias separator, which a table
/// cell still escapes): written bare they parse back to the same literal
/// text, as long as nothing after them turns the brackets into a link (`(`
/// or `[`).
// FORK: `pub(crate)` so the chip projection shares this decision.
pub(crate) fn wiki_link_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("[[").map(|at| from + at) {
        let inner_start = open + 2;
        let Some(close) = text[inner_start..].find("]]").map(|at| inner_start + at) else {
            break;
        };
        let inner = &text[inner_start..close];
        let end = close + 2;
        let followed_by_link = matches!(text[end..].chars().next(), Some('(' | '['));
        let safe = !inner.trim().is_empty()
            && !followed_by_link
            && inner.char_indices().all(|(ix, ch)| {
                let prev = inner[..ix].chars().next_back();
                let next = inner[ix + ch.len_utf8()..].chars().next();
                match ch {
                    '_' => {
                        prev.is_some_and(char::is_alphanumeric)
                            && next.is_some_and(char::is_alphanumeric)
                    }
                    '&' => !entity_like(&inner[ix + 1..]),
                    '\\' | '`' | '*' | '[' | ']' | '<' | '>' | '~' | '$' | '\n' | '\r' => false,
                    _ => true,
                }
            });
        if safe {
            spans.push(open..end);
            from = end;
        } else {
            from = open + 1;
        }
    }
    spans
}

/// A URL found in literal text, and how to write it so that it parses back to
/// exactly the same text.
struct UrlSpan {
    range: std::ops::Range<usize>,
    /// Write it as `<url>`. A bare GFM autolink literal runs to the next
    /// whitespace or `<` and takes any backslash along as text, so a URL that
    /// is followed by something that needs escaping must be closed explicitly.
    angled: bool,
}

/// `http(s)://` URLs in literal text. A URL must start the text or follow
/// whitespace or `(`, as GFM autolink literals do; it is written bare when it
/// runs to whitespace or the end of the text, and as an autolink otherwise.
/// Autolinks interpret neither escapes nor character references, and bare
/// autolink literals leave trailing punctuation as literal text that can only
/// close emphasis that nothing opens.
fn url_spans(text: &str) -> Vec<UrlSpan> {
    let mut spans = Vec::new();
    let mut prev: Option<char> = None;
    let mut ix = 0;
    while ix < text.len() {
        let rest = &text[ix..];
        let scheme = ["https://", "http://"].into_iter().find(|scheme| {
            rest.get(..scheme.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(scheme))
        });
        let boundary = prev.is_none_or(|ch| ch.is_whitespace() || ch == '(');
        if let (Some(scheme), true) = (scheme, boundary)
            && rest[scheme.len()..].starts_with(|ch: char| ch.is_ascii_alphanumeric())
        {
            // `|` would split a table cell, `<`/`>` cannot be inside `<url>`.
            let len = rest
                .find(|ch: char| {
                    ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '|')
                })
                .unwrap_or(rest.len());
            let url = &rest[..len];
            let followed_by_space = rest[len..].chars().next().is_none_or(char::is_whitespace);
            let has_reference = url
                .match_indices('&')
                .any(|(at, _)| entity_like(&url[at + 1..]));
            spans.push(UrlSpan {
                range: ix..ix + len,
                angled: !followed_by_space || has_reference,
            });
            prev = url.chars().last();
            ix += len;
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        prev = Some(ch);
        ix += ch.len_utf8();
    }
    spans
}

/// Whether `url` can be written as `<url>`: an absolute URI autolink, which
/// has a scheme and no whitespace, `<`, `>` or control characters.
fn autolink_safe(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    (2..=32).contains(&scheme.len())
        && scheme.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
        && !rest.is_empty()
        && !url
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>'))
}

/// FORK: rewrite the destinations of inline images and links. `f` gets each
/// destination as the parser reads it (unescaped, without `<…>`); `None`
/// keeps it, `Some` replaces it, written with [`markdown_destination`].
/// Only those byte ranges change: reference definitions, reference-style
/// links, autolinks, Code Blocks, code spans and all other text stay as
/// written.
pub fn map_destinations(markdown: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    use ::markdown::{ParseOptions, mdast::Node, to_mdast};

    fn collect(node: &Node, source: &str, out: &mut Vec<(usize, usize, String)>) {
        let url = match node {
            Node::Link(link) => Some(&link.url),
            Node::Image(image) => Some(&image.url),
            _ => None,
        };
        if let (Some(url), Some(position)) = (url, node.position())
            && let Some((start, end)) =
                destination_range(source, position.start.offset, position.end.offset)
        {
            out.push((start, end, url.clone()));
        }
        for child in node.children().into_iter().flatten() {
            collect(child, source, out);
        }
    }

    // A parse failure (it can't happen without MDX) leaves the text as is.
    let Ok(root) = to_mdast(markdown, &ParseOptions::gfm()) else {
        return markdown.to_string();
    };
    let mut found = Vec::new();
    collect(&root, markdown, &mut found);
    found.sort_by_key(|(start, ..)| *start);

    let mut out = String::with_capacity(markdown.len());
    let mut at = 0;
    for (start, end, url) in found {
        let Some(new) = f(&url) else { continue };
        out.push_str(&markdown[at..start]);
        out.push_str(&markdown_destination(&new));
        at = end;
    }
    out.push_str(&markdown[at..]);
    out
}

/// The byte range of the destination (with its `<>`, if any) of the inline
/// link or image written at `source[start..end]`. `None` for autolinks, bare
/// URLs and anything else that isn't `[label](destination …)`. mdast gives a
/// node's range but not its parts, so this walks the label to find where the
/// destination starts.
fn destination_range(source: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let text = source.get(start..end)?;
    let bytes = text.as_bytes();
    let mut i = match bytes {
        [b'!', b'[', ..] => 2,
        [b'[', ..] => 1,
        _ => return None,
    };
    // The label ends at its matching `]`; escapes and code spans can hide
    // brackets inside it.
    let mut depth = 1;
    while depth > 0 {
        match *bytes.get(i)? {
            b'\\' => i += 1,
            b'`' => {
                let run = bytes[i..].iter().take_while(|b| **b == b'`').count();
                let closing = text[i + run..]
                    .match_indices(&"`".repeat(run))
                    .find(|(at, _)| {
                        let after = i + run + at + run;
                        bytes.get(after) != Some(&b'`')
                            && (*at == 0 || bytes[i + run + at - 1] != b'`')
                    });
                match closing {
                    Some((at, _)) => i += run + at + run - 1,
                    None => i += run - 1,
                }
            }
            b'[' => depth += 1,
            b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    i += 1;
    while bytes.get(i)?.is_ascii_whitespace() {
        i += 1;
    }
    let dest = i;
    if bytes[i] == b'<' {
        i += 1;
        loop {
            match *bytes.get(i)? {
                b'\\' => i += 1,
                b'>' => break,
                _ => {}
            }
            i += 1;
        }
        i += 1;
    } else {
        let mut parens = 0;
        while let Some(&b) = bytes.get(i) {
            match b {
                b'\\' => i += 1,
                b'(' => parens += 1,
                b')' if parens == 0 => break,
                b')' => parens -= 1,
                b if b.is_ascii_whitespace() => break,
                _ => {}
            }
            i += 1;
        }
    }
    Some((start + dest, start + i))
}

#[cfg(test)]
mod map_destinations_tests {
    use super::map_destinations;

    fn upper(url: &str) -> Option<String> {
        Some(url.to_uppercase())
    }

    #[test]
    fn rewrites_images_and_links() {
        let src = "see [a *b*](x.md \"t\") and ![alt](pic.png) ok\n\n[](empty.md)";
        assert_eq!(
            map_destinations(src, upper),
            "see [a *b*](X.MD \"t\") and ![alt](PIC.PNG) ok\n\n[](EMPTY.MD)"
        );
    }

    #[test]
    fn reads_angle_destinations_and_writes_them_back_when_needed() {
        let src = "![a](<my pic.png>) [b](<x(1).md> 'T')";
        let mut seen = Vec::new();
        let out = map_destinations(src, |url| {
            seen.push(url.to_string());
            Some(format!("{url}2"))
        });
        assert_eq!(seen, ["my pic.png", "x(1).md"]);
        assert_eq!(out, "![a](<my pic.png2>) [b](<x(1).md2> 'T')");
    }

    #[test]
    fn none_keeps_a_destination_and_identity_is_a_no_op() {
        let src = "![a](keep.png) and [b](<odd one>)  \n\n    [x](y)\n";
        let out = map_destinations(src, |url| {
            (url != "keep.png").then(|| "new.png".to_string())
        });
        assert_eq!(out, "![a](keep.png) and [b](new.png)  \n\n    [x](y)\n");
        assert_eq!(map_destinations(src, |_| None), src);
    }

    #[test]
    fn leaves_code_definitions_and_references_alone() {
        let src = "`![a](x.png)`\n\n```md\n[b](y.md)\n```\n\n    ![c](z.png)\n\n[r][ref] ![i][ref] <https://e.com>\n\n[ref]: def.png\n\n![ok](a.png)";
        let out = map_destinations(src, upper);
        assert_eq!(out, src.replace("![ok](a.png)", "![ok](A.PNG)"));
    }

    #[test]
    fn handles_brackets_in_labels_and_multibyte_text() {
        let src = "é [a [b] `]` c](x.md) ![ü\\]](y.png)";
        assert_eq!(
            map_destinations(src, upper),
            "é [a [b] `]` c](X.MD) ![ü\\]](Y.PNG)"
        );
    }
}
