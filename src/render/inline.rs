//! Collect inline-level content from a tag's children: produces a flat
//! string plus a list of `InlineRange`s describing where bold/italic/
//! strikethrough apply.

use markdoc::types::{RenderableTreeNode, Scalar};

#[derive(Debug, Clone, Copy)]
pub enum InlineProp {
    Bold,
    Italic,
    Strikethrough,
    /// Override the foreground colour for this range — used by the
    /// code-block syntax highlighter so that keywords/strings/comments
    /// can be tinted independently of the body text colour.
    Color(krilla::color::rgb::Color),
}

#[derive(Debug, Clone)]
pub struct InlineRange {
    pub start: usize,
    pub end: usize,
    pub prop: InlineProp,
}

/// A hyperlink covering bytes `[start, end)` in the collected text.
/// `href` may be a relative URL, an absolute URL, an `#anchor`, etc. —
/// the renderer passes it to krilla as a URI action verbatim.
#[derive(Debug, Clone)]
pub struct LinkRange {
    pub start: usize,
    pub end: usize,
    pub href: String,
    /// Optional title attribute on the link, used as alt text on the PDF
    /// annotation when present.
    pub title: Option<String>,
    /// When `Some`, the renderer strokes a horizontal rule under each
    /// line of this link's text at the configured colour and width.
    /// Set by the paragraph layout from `Style::link` — the inline
    /// collector itself never populates it.
    pub underline: Option<UnderlineStroke>,
}

/// Stroke parameters for the optional underline drawn beneath a link.
#[derive(Debug, Clone)]
pub struct UnderlineStroke {
    pub color: krilla::color::rgb::Color,
    pub thickness: f32,
}

/// A mid-paragraph anchor declaration `{% tag id="X" %}` — the byte
/// offset records where in the collected text the tag appeared, used
/// later to compute its (line, y) for the PDF destination.
#[derive(Debug, Clone)]
pub struct MidAnchor {
    pub byte_offset: usize,
    pub id: String,
}

/// One `{% footnote %}…{% /footnote %}` call site recorded during
/// inline collection. `byte_offset` points at the FIRST byte of the
/// superscript call mark inside `Inlines::text`; `number` is the
/// 1-based footnote number assigned at collection time so the body
/// in the global registry can be matched at pagination time.
#[derive(Debug, Clone)]
pub struct FootnoteCall {
    pub byte_offset: usize,
    pub number: u32,
}

/// Collected inline content from a tag's children: text plus the style
/// ranges, link ranges, mid-paragraph anchor declarations, and
/// footnote call sites found within it.
pub struct Inlines {
    pub text: String,
    pub style_ranges: Vec<InlineRange>,
    pub links: Vec<LinkRange>,
    pub mid_anchors: Vec<MidAnchor>,
    pub footnote_calls: Vec<FootnoteCall>,
}

impl Inlines {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            style_ranges: Vec::new(),
            links: Vec::new(),
            mid_anchors: Vec::new(),
            footnote_calls: Vec::new(),
        }
    }

    /// Collect inline content from `children`. `footnotes` is the
    /// document-wide footnote-body registry: every `{% footnote %}`
    /// encountered appends its body and the call mark gets the new
    /// 1-based index. Pass a throwaway buffer at sites where footnotes
    /// shouldn't survive (captions, table cells, headings inside ToC,
    /// or measurement-only passes).
    pub fn from(children: &[RenderableTreeNode], footnotes: &mut Vec<String>) -> Self {
        let mut i = Self::new();
        collect_into(&mut i, children, footnotes);
        i
    }
}

/// Render an unsigned integer as the corresponding Unicode
/// superscript characters (e.g. `12 → "¹²"`). Used as the inline call
/// mark for a footnote.
pub fn superscript_number(n: u32) -> String {
    const MAP: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    let s = n.to_string();
    s.chars()
        .map(|c| c.to_digit(10).map(|d| MAP[d as usize]).unwrap_or(c))
        .collect()
}

/// Extract the anchor id from a `<tag>` or `<tagref>` Tag node.
///
/// Two accepted spellings:
///   - `{% tag id="foo" /%}`    — keyed attribute, the canonical form
///   - `{% tag "foo" /%}`       — primary attribute (Markdoc shorthand)
///
/// The earlier `{% tag="foo" /%}` typo-form was dropped — it's just a
/// noisier spelling of the primary shorthand and the parser now handles
/// `"foo"` cleanly without renderer post-processing.
pub fn anchor_id_attr(tag: &markdoc::types::Tag) -> Option<String> {
    if let Some(Scalar::String(s)) = tag.attributes.get("id")
        && !s.is_empty()
    {
        return Some(s.clone());
    }
    if let Some(Scalar::String(s)) = tag.attributes.get("primary")
        && !s.is_empty()
    {
        return Some(s.clone());
    }
    None
}

/// Extract the cross-document target id from a `<tagref>` Tag node.
/// `doc="<id>"` selects which Adeptus document the `id="<anchor>"`
/// lives in. Empty / absent attribute means "intra-document reference"
/// and the caller falls back to plain `anchor_id_attr` resolution.
pub fn doc_attr(tag: &markdoc::types::Tag) -> Option<String> {
    if let Some(Scalar::String(s)) = tag.attributes.get("doc")
        && !s.is_empty()
    {
        return Some(s.clone());
    }
    None
}

/// Backwards-compatible helper for callers that only want the text +
/// style ranges (no link tracking). `footnotes` is the registry —
/// see [`Inlines::from`] for the same caveats.
pub fn collect_inlines(
    text: &mut String,
    ranges: &mut Vec<InlineRange>,
    children: &[RenderableTreeNode],
    footnotes: &mut Vec<String>,
) {
    let mut tmp = Inlines {
        text: std::mem::take(text),
        style_ranges: std::mem::take(ranges),
        links: Vec::new(),
        mid_anchors: Vec::new(),
        footnote_calls: Vec::new(),
    };
    collect_into(&mut tmp, children, footnotes);
    *text = tmp.text;
    *ranges = tmp.style_ranges;
}

fn collect_into(out: &mut Inlines, children: &[RenderableTreeNode], footnotes: &mut Vec<String>) {
    for child in children {
        match child {
            RenderableTreeNode::Scalar(Scalar::String(s)) => out.text.push_str(s),
            RenderableTreeNode::Scalar(Scalar::Array(arr)) => {
                for item in arr {
                    if let Scalar::String(s) = item {
                        out.text.push_str(s);
                    }
                }
            }
            RenderableTreeNode::Scalar(_) => {}
            RenderableTreeNode::Tag(t) => {
                let prop = match t.name.as_str() {
                    "strong" => Some(InlineProp::Bold),
                    "em" => Some(InlineProp::Italic),
                    "s" | "strikethrough" => Some(InlineProp::Strikethrough),
                    "softbreak" => {
                        // CommonMark soft break (a single newline
                        // within a paragraph). Emit a space so words
                        // either side stay separated; let parley pick
                        // the wrap point.
                        out.text.push(' ');
                        collect_into(out, &t.children, footnotes);
                        continue;
                    }
                    "br" | "hardbreak" => {
                        // CommonMark hard break (two trailing spaces or
                        // backslash). Force a line break — parley treats
                        // U+000A as a mandatory break.
                        out.text.push('\n');
                        collect_into(out, &t.children, footnotes);
                        continue;
                    }
                    "a" => {
                        // Capture the link's byte range and href.
                        let href = match t.attributes.get("href") {
                            Some(Scalar::String(s)) => Some(s.clone()),
                            _ => None,
                        };
                        let title = match t.attributes.get("title") {
                            Some(Scalar::String(s)) if !s.is_empty() => Some(s.clone()),
                            _ => None,
                        };
                        let start = out.text.len();
                        collect_into(out, &t.children, footnotes);
                        let end = out.text.len();
                        if let Some(href) = href
                            && end > start
                        {
                            out.links.push(LinkRange {
                                start,
                                end,
                                href,
                                title,
                                underline: None,
                            });
                        }
                        continue;
                    }
                    "footnote" => {
                        // Allocate the next sequential number, capture
                        // the body as plain text into the registry,
                        // and inject a Unicode superscript call mark
                        // at the current position. Nested footnotes
                        // are flattened (their bodies merge into the
                        // outer footnote text).
                        let number = (footnotes.len() as u32) + 1;
                        // Reserve the slot up front so any inner
                        // footnotes get higher numbers; then fill in
                        // the body once collected.
                        footnotes.push(String::new());
                        let mut body = Inlines::new();
                        collect_into(&mut body, &t.children, footnotes);
                        let body_text = body.text.trim().to_string();
                        if let Some(slot) = footnotes.get_mut((number - 1) as usize) {
                            *slot = body_text;
                        }
                        let mark = superscript_number(number);
                        let call_offset = out.text.len();
                        out.text.push_str(&mark);
                        out.footnote_calls.push(FootnoteCall {
                            byte_offset: call_offset,
                            number,
                        });
                        continue;
                    }
                    "tagref" => {
                        // Cross-reference. Two flavours:
                        //
                        //   {% tagref id="X" /%}            — intra-doc.
                        //     Resolved to a PDF GoTo destination at emit
                        //     time via the anchor map. Renders the id as
                        //     link text.
                        //
                        //   {% tagref doc="D" id="X" /%}    — cross-doc.
                        //     Adeptus is meant to rewrite this BEFORE
                        //     handing the source to Scriptor (into either
                        //     a normal `[text](https://…)` external URL
                        //     or an intra-bundle `{% tagref id="…" /%}`).
                        //     During local iteration nothing rewrites it,
                        //     so we render a visibly-degraded placeholder
                        //     `[doc#anchor]` with no annotation — loud
                        //     enough that writers spot unresolved refs in
                        //     a draft preview without anything failing.
                        //     See CROSS_DOC.md for the full contract.
                        let doc = doc_attr(t);
                        let id = anchor_id_attr(t);
                        match (doc, id) {
                            (Some(doc), Some(id)) => {
                                let placeholder = format!("[{doc}#{id}]");
                                out.text.push_str(&placeholder);
                            }
                            (Some(doc), None) => {
                                let placeholder = format!("[{doc}#?]");
                                out.text.push_str(&placeholder);
                            }
                            (None, Some(id)) => {
                                let start = out.text.len();
                                out.text.push_str(&id);
                                let end = out.text.len();
                                out.links.push(LinkRange {
                                    start,
                                    end,
                                    href: format!("#{id}"),
                                    title: None,
                                    underline: None,
                                });
                            }
                            (None, None) => {
                                // No id, no doc — drop silently.
                            }
                        }
                        continue;
                    }
                    "tag" => {
                        // `{% tag id="X" %}` is an anchor declaration.
                        // Headings strip these before reaching the
                        // inline collector. When a tag survives to here
                        // it's a mid-paragraph anchor — record its
                        // byte offset so the renderer can resolve the
                        // (page, y) by walking the parley layout.
                        if let Some(id) = anchor_id_attr(t) {
                            out.mid_anchors.push(MidAnchor {
                                byte_offset: out.text.len(),
                                id,
                            });
                        }
                        continue;
                    }
                    _ => None,
                };
                let start = out.text.len();
                collect_into(out, &t.children, footnotes);
                let end = out.text.len();
                if let Some(p) = prop
                    && end > start
                {
                    out.style_ranges.push(InlineRange {
                        start,
                        end,
                        prop: p,
                    });
                }
            }
        }
    }
}
