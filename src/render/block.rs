//! `Block`: the unit of pagination. The layout pass walks a
//! `RenderableTreeNode` tree and produces a flat sequence of these,
//! each with its computed height and pre-laid-out content. Pagination
//! and emission then operate on `Block`s without re-touching the AST.

use std::sync::Arc;

use krilla::color::rgb;
use krilla::image::Image as KrillaImage;
use markdoc::types::{RenderableTreeNode, Scalar, Tag};
use parley::{FontContext, Layout, LayoutContext};
use usvg::Tree as SvgTree;

use crate::assets::{AssetResolver, MediaFormat, NullAssetResolver, sniff_format};

use super::inline::{InlineProp, InlineRange, Inlines, LinkRange, MidAnchor, collect_inlines};
use super::paginate::paginate as paginate_blocks;
use super::style::MarkerSequence;
use super::style::Style;
use super::style::TableColumnSizing;
use super::text::{TextStyle, build_layout, measure_first_line_width, monospace_families};

/// One unit of paginatable content.
#[derive(Clone)]
pub struct Block {
    /// The content's drawn height (does not include `space_after`).
    pub height: f32,
    /// Vertical gap to leave after this block.
    pub space_after: f32,
    pub draw: BlockDraw,
    /// If this block represents a heading, an entry to add to the
    /// document outline at emit time. The outline entry's destination
    /// points to the page+y where this block lands after pagination.
    pub outline: Option<OutlineEntry>,
    /// Optional anchor id for `{% tag id="X" %}` declarations. Recorded
    /// at emit time as `(id, page_idx, y)` so `{% tagref %}` internal
    /// links can resolve to a PDF destination.
    pub anchor_id: Option<String>,
    /// PDF/UA semantic role override. When set, the emit pass wraps
    /// this block's tagged content in the requested structure-tree
    /// tag instead of the default `P`. Used by the footnote pool to
    /// emit each entry as `Note`.
    pub tag_role: Option<TagRole>,
}

/// Override for the structure-tree tag emit assigns to a Text block.
/// Defaults (when `None`) are: `Hn` for headings, `P` for everything
/// else. Add variants here as PDF/UA wants more semantic flavours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagRole {
    /// `Note` — used for footnote/endnote bodies.
    Note,
}

/// Heading metadata captured at layout time so the emit pass can
/// place an outline entry at the resolved page+y.
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub level: u8,
    pub text: String,
}

/// Outcome of attempting to fit a block's content into a partial page.
///
/// Variants intentionally carry `Block` by value rather than `Box<Block>`:
/// each `SplitOutcome` is constructed, immediately destructured by the
/// paginator, and dropped within the same call frame. Boxing buys nothing
/// for this short-lived flow.
#[allow(clippy::large_enum_variant)]
pub enum SplitOutcome {
    /// Block fits whole into the remaining space (returned unchanged).
    Whole(Block),
    /// Block split at a line boundary: head fits in the remaining space,
    /// tail is the rest.
    Split(Block, Block),
    /// Block can't be split further (or even one line wouldn't fit in
    /// the remaining space). Caller should start a new page.
    NoFit(Block),
}

impl Block {
    /// Footnote numbers whose call mark sits inside this block (and its
    /// descendants). Used by pagination to figure out which footnote
    /// bodies must accompany this block on its destination page.
    pub fn collect_footnote_numbers(&self, out: &mut Vec<u32>) {
        match &self.draw {
            BlockDraw::Text(slice) => out.extend(slice.footnote_numbers()),
            BlockDraw::BoxedGroup { children, .. } => {
                for c in children {
                    c.collect_footnote_numbers(out);
                }
            }
            BlockDraw::ListItem { body, .. } => {
                for c in body {
                    c.collect_footnote_numbers(out);
                }
            }
            // Tables, images, SVG, rules carry no footnote calls in v1.
            _ => {}
        }
    }

    /// Try to fit at most `remaining` of vertical space. Currently only
    /// `BlockDraw::Text` is splittable; other block kinds return `Whole`
    /// (if they fit) or `NoFit` (if they don't).
    pub fn try_split(self, remaining: f32) -> SplitOutcome {
        let needed = self.height + self.space_after;
        if needed <= remaining {
            return SplitOutcome::Whole(self);
        }
        let anchor_id = self.anchor_id.clone();
        match self.draw {
            BlockDraw::Text(slice) => try_split_text(slice, self.space_after, remaining),
            BlockDraw::Table {
                x,
                column_widths,
                rows,
                cell_padding,
                header_bg,
                border_color,
                border_thickness,
                border_style,
                edge,
                caption,
            } => try_split_table(
                x,
                column_widths,
                rows,
                cell_padding,
                header_bg,
                border_color,
                border_thickness,
                border_style,
                edge,
                self.space_after,
                remaining,
                anchor_id,
                caption,
            ),
            other => SplitOutcome::NoFit(Block {
                height: self.height,
                space_after: self.space_after,
                draw: other,
                outline: None,
                anchor_id,

                tag_role: None,
            }),
        }
    }
}

fn try_split_text(slice: TextSlice, space_after: f32, remaining: f32) -> SplitOutcome {
    let start = slice.line_range.start;
    let end = slice.line_range.end;
    let mut acc = 0.0_f32;
    let mut split_at = start;
    for i in start..end {
        let h = slice.line_heights[i];
        if acc + h > remaining {
            break;
        }
        acc += h;
        split_at = i + 1;
    }
    if split_at == start {
        // Not even one line fits.
        return SplitOutcome::NoFit(Block {
            height: slice.height(),
            space_after,
            draw: BlockDraw::Text(slice),
            outline: None,
            anchor_id: None,

            tag_role: None,
        });
    }
    if split_at == end {
        return SplitOutcome::Whole(Block {
            height: slice.height(),
            space_after,
            draw: BlockDraw::Text(slice),
            outline: None,
            anchor_id: None,

            tag_role: None,
        });
    }
    let head_height: f32 = slice.line_heights[start..split_at].iter().sum();
    let tail_height: f32 = slice.line_heights[split_at..end].iter().sum();
    let head = Block {
        height: head_height,
        space_after: 0.0,
        draw: BlockDraw::Text(TextSlice {
            layout: slice.layout.clone(),
            text: slice.text.clone(),
            links: slice.links.clone(),
            mid_anchors: slice.mid_anchors.clone(),
            footnote_calls: slice.footnote_calls.clone(),
            line_heights: slice.line_heights.clone(),
            x: slice.x,
            line_range: start..split_at,
            skip_y: slice.skip_y,
        }),
        outline: None,
        anchor_id: None,

        tag_role: None,
    };
    let tail = Block {
        height: tail_height,
        space_after,
        draw: BlockDraw::Text(TextSlice {
            x: slice.x,
            line_range: split_at..end,
            skip_y: slice.skip_y + head_height,
            layout: slice.layout,
            text: slice.text,
            links: slice.links,
            mid_anchors: slice.mid_anchors,
            footnote_calls: slice.footnote_calls,
            line_heights: slice.line_heights,
        }),
        outline: None,
        anchor_id: None,

        tag_role: None,
    };
    SplitOutcome::Split(head, tail)
}

#[allow(clippy::too_many_arguments)]
fn try_split_table(
    x: f32,
    column_widths: Vec<f32>,
    rows: Vec<TableRow>,
    cell_padding: f32,
    header_bg: rgb::Color,
    border_color: rgb::Color,
    border_thickness: f32,
    border_style: super::style::TableBorders,
    edge: Option<(rgb::Color, f32)>,
    space_after: f32,
    remaining: f32,
    anchor_id: Option<String>,
    caption: Option<String>,
) -> SplitOutcome {
    // Header rows form a contiguous prefix (`is_header == true`); they
    // repeat on every continuation page.
    let header_count = rows.iter().take_while(|r| r.is_header).count();

    // Header footprint: header rows + their bordering lines (top edge,
    // each header row's bottom edge). When body follows, the same
    // footprint applies; we only add `row.height + border_thickness`
    // per body row.
    let header_height: f32 = rows[..header_count].iter().map(|r| r.height).sum::<f32>()
        + border_thickness * (header_count as f32 + 1.0);

    // Find the largest body-row prefix that fits.
    let mut included_body = 0;
    let mut footprint = header_height;
    for body_row in &rows[header_count..] {
        let increment = body_row.height + border_thickness;
        if footprint + increment > remaining {
            break;
        }
        footprint += increment;
        included_body += 1;
    }

    if included_body == 0 {
        if header_count < rows.len() {
            let row_available = remaining - header_height - border_thickness;
            if row_available > 0.0
                && let Some((head_row, tail_row)) =
                    try_split_first_body_row(&rows[header_count], row_available, cell_padding)
            {
                let mut head_rows: Vec<TableRow> = rows[..header_count].to_vec();
                head_rows.push(head_row);

                let mut tail_rows: Vec<TableRow> = rows[..header_count].to_vec();
                tail_rows.push(tail_row);
                let body_after_first: Vec<TableRow> =
                    rows.into_iter().skip(header_count + 1).collect();
                tail_rows.extend(body_after_first);

                let head_block = make_table_block(
                    x,
                    column_widths.clone(),
                    head_rows,
                    cell_padding,
                    header_bg,
                    border_color,
                    border_thickness,
                    border_style,
                    edge,
                    0.0,
                    anchor_id,
                    caption.clone(),
                );
                let tail_block = make_table_block(
                    x,
                    column_widths,
                    tail_rows,
                    cell_padding,
                    header_bg,
                    border_color,
                    border_thickness,
                    border_style,
                    edge,
                    space_after,
                    None,
                    None,
                );
                return SplitOutcome::Split(head_block, tail_block);
            }
        }
        return SplitOutcome::NoFit(make_table_block(
            x,
            column_widths,
            rows,
            cell_padding,
            header_bg,
            border_color,
            border_thickness,
            border_style,
            edge,
            space_after,
            anchor_id,
            caption,
        ));
    }
    if header_count + included_body == rows.len() {
        return SplitOutcome::Whole(make_table_block(
            x,
            column_widths,
            rows,
            cell_padding,
            header_bg,
            border_color,
            border_thickness,
            border_style,
            edge,
            space_after,
            anchor_id,
            caption,
        ));
    }

    // Split: head = headers + body[..included]; tail = headers (cloned)
    // + body[included..]. Head retains the anchor; tail has no anchor
    // (the table started on the head's page, and the LoF/LoT entry
    // should point there).
    let split_at = header_count + included_body;
    let header_clone: Vec<TableRow> = rows[..header_count].to_vec();
    let mut head_rows = rows;
    let mut tail_rows = head_rows.split_off(split_at);
    tail_rows.splice(0..0, header_clone);

    let head_block = make_table_block(
        x,
        column_widths.clone(),
        head_rows,
        cell_padding,
        header_bg,
        border_color,
        border_thickness,
        border_style,
        edge,
        0.0,
        anchor_id,
        caption.clone(),
    );
    let tail_block = make_table_block(
        x,
        column_widths,
        tail_rows,
        cell_padding,
        header_bg,
        border_color,
        border_thickness,
        border_style,
        edge,
        space_after,
        None,
        None,
    );
    SplitOutcome::Split(head_block, tail_block)
}

/// Split one row into a head + tail when even the row alone can't fit
/// in the remaining page space. Each cell is paginated independently
/// at the row's available inner height; cell-page-1 forms the head's
/// content for that cell, cell-page-2-onwards is concatenated as the
/// tail content.
///
/// Returns `None` when no split is possible (e.g. row content can't
/// be reduced — maybe a single image taller than `available`).
fn try_split_first_body_row(
    row: &TableRow,
    available: f32,
    cell_padding: f32,
) -> Option<(TableRow, TableRow)> {
    let inner = available - 2.0 * cell_padding;
    if inner <= 0.0 {
        return None;
    }

    let mut head_cells: Vec<Vec<Block>> = Vec::with_capacity(row.cells.len());
    let mut tail_cells: Vec<Vec<Block>> = Vec::with_capacity(row.cells.len());
    let mut any_split = false;
    let mut head_has_content = false;

    for cell in &row.cells {
        if cell.is_empty() {
            head_cells.push(Vec::new());
            tail_cells.push(Vec::new());
            continue;
        }
        let pages = paginate_blocks(cell.clone(), inner);
        let mut iter = pages.into_iter();
        let head = iter.next().unwrap_or_default();
        let rest: Vec<Block> = iter.flatten().collect();
        if !head.is_empty() {
            head_has_content = true;
        }
        if !rest.is_empty() {
            any_split = true;
        }
        head_cells.push(head);
        tail_cells.push(rest);
    }

    if !any_split || !head_has_content {
        return None;
    }

    let head_height = cells_max_height(&head_cells) + 2.0 * cell_padding;
    let tail_height = cells_max_height(&tail_cells) + 2.0 * cell_padding;

    Some((
        TableRow {
            is_header: false,
            height: head_height,
            cells: head_cells,
        },
        TableRow {
            is_header: false,
            height: tail_height,
            cells: tail_cells,
        },
    ))
}

fn cells_max_height(cells: &[Vec<Block>]) -> f32 {
    cells
        .iter()
        .map(|c| sum_block_height_slice(c))
        .fold(0.0_f32, f32::max)
}

fn sum_block_height_slice(blocks: &[Block]) -> f32 {
    let total: f32 = blocks.iter().map(|b| b.height + b.space_after).sum();
    total - blocks.last().map(|b| b.space_after).unwrap_or(0.0)
}

#[allow(clippy::too_many_arguments)]
fn make_table_block(
    x: f32,
    column_widths: Vec<f32>,
    rows: Vec<TableRow>,
    cell_padding: f32,
    header_bg: rgb::Color,
    border_color: rgb::Color,
    border_thickness: f32,
    border_style: super::style::TableBorders,
    edge: Option<(rgb::Color, f32)>,
    space_after: f32,
    anchor_id: Option<String>,
    caption: Option<String>,
) -> Block {
    let height: f32 =
        rows.iter().map(|r| r.height).sum::<f32>() + border_thickness * (rows.len() as f32 + 1.0);
    Block {
        height,
        space_after,
        draw: BlockDraw::Table {
            x,
            column_widths,
            rows,
            cell_padding,
            header_bg,
            border_color,
            border_thickness,
            border_style,
            edge,
            caption,
        },
        outline: None,
        anchor_id,

        tag_role: None,
    }
}

/// A decoded icon drawn at a [`BlockDraw::BoxedGroup`]'s top-left
/// content corner. Used by callouts; the box's children are laid out
/// indented past `size` so they clear it.
#[derive(Clone)]
pub struct BoxedGroupIcon {
    pub decoded: super::decoration::DecodedMedia,
    /// Absolute x of the icon's left edge (page-local).
    pub x: f32,
    /// Square draw size in points.
    pub size: f32,
}

/// Circle-badge geometry for an ordered-list marker, precomputed at
/// layout time so emit only has to stroke a circle and centre the
/// marker glyphs. Offsets are relative to the list item's
/// `(marker_x, block top)` origin.
#[derive(Clone, Copy)]
pub struct MarkerBadge {
    pub fill: rgb::Color,
    /// Circle diameter in points.
    pub diameter: f32,
    /// Circle-centre x, measured from `marker_x`.
    pub center_dx: f32,
    /// Circle-centre y, measured from the block's top y.
    pub center_dy: f32,
}

// `ListItem.marker` is a `Layout<rgb::Color>` (~333 bytes), much larger
// than other variants. Boxing it would force pagination/emit to chase a
// pointer for every list bullet — net loss for typical list-heavy docs.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum BlockDraw {
    /// A (possibly partial) parley text block. When `slice.line_range`
    /// covers all lines, this is the full paragraph; pagination may
    /// split into two `Text` blocks each pointing at the same
    /// underlying `Layout` via `Arc`, with disjoint line ranges.
    Text(TextSlice),
    /// A horizontal rule.
    Rule {
        x: f32,
        width: f32,
        thickness: f32,
        color: rgb::Color,
    },
    /// A box decoration: draws a background/border around the children,
    /// with optional left accent stripe, then renders the children inside
    /// (offset by `padding`). An optional `icon` is drawn at the box's
    /// top-left content corner (children are laid out indented past it).
    BoxedGroup {
        x: f32,
        width: f32,
        background: Option<rgb::Color>,
        border: Option<rgb::Color>,
        accent_left: Option<rgb::Color>,
        accent_width: f32,
        padding: f32,
        children: Vec<Block>,
        icon: Option<BoxedGroupIcon>,
        /// Optional horizontal rule `(colour, thickness)` drawn across
        /// the top / bottom edge of the box — the "bulletin" framing,
        /// used instead of a fill+border.
        top_rule: Option<(rgb::Color, f32)>,
        bottom_rule: Option<(rgb::Color, f32)>,
    },
    /// A list item: marker drawn at `marker_x`. Body blocks are
    /// pre-positioned at the indented content x (baked in at layout
    /// time), and share the marker's top y so the marker sits on the
    /// first body line's baseline. `ordered` controls the surrounding
    /// `L` tag's `/ListNumbering` attribute under PDF/UA: ordered
    /// items map to `Decimal`, unordered to `Disc`.
    ListItem {
        marker: Layout<rgb::Color>,
        marker_text: String,
        marker_x: f32,
        body: Vec<Block>,
        ordered: bool,
        /// When set, the marker is drawn centred inside a filled circle
        /// instead of as plain inline text.
        badge: Option<MarkerBadge>,
    },
    /// A pre-decoded raster image, drawn at `(x, y_top)` with the given
    /// display size. `caption` is used for the List of Figures.
    Image {
        image: KrillaImage,
        x: f32,
        width: f32,
        height: f32,
        caption: Option<String>,
    },
    /// A pre-parsed SVG tree. `caption` is used for the List of Figures.
    Svg {
        tree: Arc<SvgTree>,
        x: f32,
        width: f32,
        height: f32,
        caption: Option<String>,
    },
    /// Internal page-break marker. Carries no content; the paginator
    /// flushes the current page when it encounters one and drops the
    /// marker. Synthetic — sources can't produce one directly (the
    /// renderer materialises them from style-driven features like the
    /// cover page).
    PageBreak,
    /// A simple equal-width table with optional header row. `caption`
    /// is the caption text (used by the List of Tables when present).
    /// Visual rendering of the caption is a separate sibling block.
    Table {
        x: f32,
        column_widths: Vec<f32>,
        rows: Vec<TableRow>,
        cell_padding: f32,
        header_bg: rgb::Color,
        border_color: rgb::Color,
        border_thickness: f32,
        /// Which rules to draw (grid / horizontal-only / none).
        border_style: super::style::TableBorders,
        /// Optional outer-frame stroke `(color, thickness)` for the
        /// top/bottom (and grid left/right) rules; `None` draws edges in
        /// the internal border colour like the rest.
        edge: Option<(rgb::Color, f32)>,
        caption: Option<String>,
    },
}

#[derive(Clone)]
pub struct TableRow {
    pub is_header: bool,
    pub height: f32,
    /// One cell per column. Cell content blocks have their `x` already
    /// shifted into the column at layout time.
    pub cells: Vec<Vec<Block>>,
}

/// A reference into a parley `Layout` that may cover all of it or a
/// contiguous sub-range of its lines. Splittable at line boundaries by
/// the paginator, with both halves sharing the same underlying data via
/// `Arc`.
#[derive(Clone)]
pub struct TextSlice {
    pub layout: Arc<Layout<rgb::Color>>,
    pub text: Arc<String>,
    pub links: Arc<Vec<LinkRange>>,
    /// Mid-paragraph anchor declarations — `{% tag id="X" %}` placed
    /// inside running text. The renderer maps each byte offset to a
    /// (line, y) when building the anchor map.
    pub mid_anchors: Arc<Vec<MidAnchor>>,
    /// Footnote call sites in this text — same byte offsets as the
    /// raw collected `text`, paired with the assigned 1-based number.
    /// Pagination uses these to determine which footnote bodies belong
    /// in each page's pool.
    pub footnote_calls: Arc<Vec<super::inline::FootnoteCall>>,
    /// Drawn height per line (ascent + descent + leading), in line order.
    pub line_heights: Arc<Vec<f32>>,
    pub x: f32,
    /// Half-open range of lines this slice draws.
    pub line_range: std::ops::Range<usize>,
    /// Y offset to subtract from baselines so the slice's first drawn
    /// line appears at the block's top y. Equals the cumulative
    /// `line_heights[..line_range.start]`.
    pub skip_y: f32,
}

impl TextSlice {
    /// Build the canonical "whole paragraph" slice — covers every line.
    pub fn whole(layout: Layout<rgb::Color>, text: String, links: Vec<LinkRange>, x: f32) -> Self {
        Self::whole_with_anchors(layout, text, links, Vec::new(), x)
    }

    /// Same as `whole` but with mid-paragraph anchor declarations.
    pub fn whole_with_anchors(
        layout: Layout<rgb::Color>,
        text: String,
        links: Vec<LinkRange>,
        mid_anchors: Vec<MidAnchor>,
        x: f32,
    ) -> Self {
        Self::whole_with_extras(layout, text, links, mid_anchors, Vec::new(), x)
    }

    /// Full constructor — includes footnote call sites collected by
    /// `Inlines`.
    pub fn whole_with_extras(
        layout: Layout<rgb::Color>,
        text: String,
        links: Vec<LinkRange>,
        mid_anchors: Vec<MidAnchor>,
        footnote_calls: Vec<super::inline::FootnoteCall>,
        x: f32,
    ) -> Self {
        let line_heights: Vec<f32> = layout
            .lines()
            .map(|l| {
                let m = l.metrics();
                m.ascent + m.descent + m.leading
            })
            .collect();
        let n = line_heights.len();
        Self {
            layout: Arc::new(layout),
            text: Arc::new(text),
            links: Arc::new(links),
            mid_anchors: Arc::new(mid_anchors),
            footnote_calls: Arc::new(footnote_calls),
            line_heights: Arc::new(line_heights),
            x,
            line_range: 0..n,
            skip_y: 0.0,
        }
    }

    /// Drawn height of just this slice.
    pub fn height(&self) -> f32 {
        self.line_heights[self.line_range.clone()]
            .iter()
            .copied()
            .sum()
    }

    /// Footnote numbers whose call mark falls within this slice's
    /// drawn line range. A split paragraph thereby attributes each
    /// footnote to the slice that actually shows the call mark.
    pub fn footnote_numbers(&self) -> Vec<u32> {
        if self.footnote_calls.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for call in self.footnote_calls.iter() {
            if let Some(line) = find_line_for_byte(&self.layout, call.byte_offset)
                && line >= self.line_range.start
                && line < self.line_range.end
            {
                out.push(call.number);
            }
        }
        out
    }
}

fn find_line_for_byte(layout: &Layout<rgb::Color>, byte: usize) -> Option<usize> {
    let mut last = None;
    for (i, line) in layout.lines().enumerate() {
        last = Some(i);
        for run in line.runs() {
            for cluster in run.visual_clusters() {
                let r = cluster.text_range();
                if byte >= r.start && byte < r.end {
                    return Some(i);
                }
                if byte == r.end {
                    return Some(i);
                }
            }
        }
    }
    last
}

/// Build the block list for one page's footnote pool: a separator
/// rule followed by one Text block per `(number, body)` entry, in
/// number order. `inner_x` and `inner_w` are the body content's
/// horizontal extents; the rule is anchored at `inner_x` with the
/// width given by `style.footnote.rule_width_frac`.
///
/// Returns an empty `Vec` when `entries` is empty so the caller can
/// add zero pool height when no footnotes hit this page.
pub fn build_footnote_pool_blocks(
    entries: &[(u32, String)],
    style: &Style,
    body_families: &'static [&'static str],
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
    inner_x: f32,
    inner_w: f32,
) -> Vec<Block> {
    if entries.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();

    // 1. Separator rule. Implemented as a thin Rule block with leading
    //    `gap_above` baked into the previous block's `space_after` —
    //    here we render only what sits below the body content.
    out.push(Block {
        height: style.footnote.gap_above,
        space_after: 0.0,
        draw: BlockDraw::Rule {
            x: inner_x,
            width: 0.0,
            thickness: 0.0,
            color: style.footnote.rule_color.into(),
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    });
    let rule_w = (inner_w * style.footnote.rule_width_frac).max(0.0);
    out.push(Block {
        height: style.footnote.rule_thickness.max(0.5),
        space_after: style.footnote.gap_below_rule,
        draw: BlockDraw::Rule {
            x: inner_x,
            width: rule_w,
            thickness: style.footnote.rule_thickness,
            color: style.footnote.rule_color.into(),
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    });

    // 2. One paragraph per footnote: "ⁿ body text".
    let entry_style = TextStyle {
        font_size: style.footnote.font_size,
        font_weight: 400.0,
        line_height: style.footnote.line_height,
        color: style.footnote.text_color.into(),
        font_families: body_families,
        italic: false,
    };
    let last = entries.len().saturating_sub(1);
    for (i, (number, body)) in entries.iter().enumerate() {
        let mark = super::inline::superscript_number(*number);
        let text = format!("{mark} {}", body);
        let layout = build_layout(&text, &[], &entry_style, inner_w, font_cx, layout_cx);
        let slice = TextSlice::whole(layout, text, Vec::new(), inner_x);
        let h = slice.height();
        out.push(Block {
            height: h,
            space_after: if i == last {
                0.0
            } else {
                style.footnote.entry_space_after
            },
            draw: BlockDraw::Text(slice),
            outline: None,
            anchor_id: None,
            tag_role: Some(TagRole::Note),
        });
    }
    out
}

/// Total drawn height of a slice of pool blocks (sum of height +
/// space_after, minus the trailing block's space_after).
pub fn pool_height(pool: &[Block]) -> f32 {
    if pool.is_empty() {
        return 0.0;
    }
    let mut h = 0.0;
    for (i, b) in pool.iter().enumerate() {
        h += b.height;
        if i + 1 < pool.len() {
            h += b.space_after;
        }
    }
    h
}

/// Layout context passed through the recursion. Keeps it tidy.
pub struct LayoutCtx<'a> {
    pub style: &'a Style,
    pub font_cx: &'a mut FontContext,
    pub layout_cx: &'a mut LayoutContext<rgb::Color>,
    pub assets: &'a dyn AssetResolver,
    /// Monotonically incrementing counter used to assign synthetic
    /// anchor ids to headings that don't have an explicit `{% tag %}`.
    pub next_heading: usize,
    /// Counter for synthetic figure anchors (`__figure_<n>`).
    pub next_figure: usize,
    /// Counter for synthetic table anchors (`__table_<n>`).
    pub next_table: usize,
    /// When `layout_children` encounters `{% caption %}` immediately
    /// followed by a `<table>`, it stashes the caption text here so
    /// `layout_table` can pick it up and store it on the table block
    /// for the List of Tables.
    pub pending_table_caption: Option<String>,
    /// Same idea for figures — `{% caption %}...{% /caption %}` placed
    /// just before an image / media tag attaches as that figure's
    /// caption, overriding any alt-derived default.
    pub pending_figure_caption: Option<String>,
    /// Footnote registry — a counter for the next footnote number plus
    /// the body text for each registered footnote, indexed by number-1.
    /// Inline collection registers each `{% footnote %}` body and gets
    /// back a sequential number; pagination later pulls bodies by
    /// number to compose the per-page footnote pool.
    pub footnotes: Vec<String>,
    /// Effective body font family list, resolved from
    /// `Style::body_font_families` (or the bundled Noto defaults when
    /// empty). Layout sites use this in place of `default_families()`
    /// so caller-specified custom fonts apply to all body text.
    pub body_families: &'a [&'static str],
    /// Optional word hyphenator. When present, plain-text body
    /// paragraphs (no bold/italic/links/anchors/footnotes) get soft
    /// hyphens inserted before parley layout so wrapped lines can
    /// break inside long words. Paragraphs with inline markup are
    /// skipped to avoid drifting byte offsets in their ranges.
    pub hyphenator: Option<&'a super::hyphen::WordHyphenator>,
    /// Running per-level counters for automatic heading numbering,
    /// indexed by `level - 1` (so `[0]` is h1). Only meaningful when
    /// `style.heading_numbering.enabled`. A heading at level L bumps
    /// `counters[L-1]` and zeroes every deeper level.
    pub heading_counters: [u32; 6],
    /// Current ordered/unordered list nesting depth (0 at the outermost
    /// list). Drives depth-cycled ordered numbering (`1.` → `a.` → `i.`).
    pub list_depth: usize,
}

impl<'a> LayoutCtx<'a> {
    pub fn next_heading_id(&mut self) -> usize {
        let n = self.next_heading;
        self.next_heading += 1;
        n
    }
    pub fn next_figure_id(&mut self) -> usize {
        let n = self.next_figure;
        self.next_figure += 1;
        n
    }
    pub fn next_table_id(&mut self) -> usize {
        let n = self.next_table;
        self.next_table += 1;
        n
    }

    /// Advance the heading counters for a numbered heading at `level`
    /// and return the formatted prefix (e.g. `"1.2.1"`), without the
    /// trailing separator. Returns `None` when numbering is disabled or
    /// `level` is deeper than the configured `max_depth`.
    fn bump_heading_number(&mut self, level: u8) -> Option<String> {
        let cfg = &self.style.heading_numbering;
        if !cfg.enabled {
            return None;
        }
        bump_heading_counters(&mut self.heading_counters, level, cfg.max_depth)
    }
}

/// Advance `counters` for a heading at `level` (1-based) and format the
/// dotted prefix. Returns `None` when `level` is 0 or deeper than
/// `max_depth` (clamped to `1..=6`).
///
/// Bumping a level resets all deeper levels, so a fresh `h2` after
/// `1.3.4` yields `1.4` rather than `1.4.4`. Pulled out as a free
/// function so the counter arithmetic is unit-testable without building
/// a whole `LayoutCtx`.
fn bump_heading_counters(counters: &mut [u32; 6], level: u8, max_depth: u8) -> Option<String> {
    if level == 0 || level > max_depth.clamp(1, 6) {
        return None;
    }
    let idx = (level - 1) as usize;
    counters[idx] += 1;
    for deeper in &mut counters[idx + 1..] {
        *deeper = 0;
    }
    let prefix = counters[..=idx]
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".");
    Some(prefix)
}

/// Used as a stand-in resolver when the caller hasn't supplied one.
/// Documents that reference media will render placeholders.
pub fn null_resolver() -> &'static dyn AssetResolver {
    static NULL: NullAssetResolver = NullAssetResolver;
    &NULL
}

/// Top-level entry: lay out a transformed Markdoc document into blocks.
pub fn layout_document(root: &RenderableTreeNode, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    let column_x = ctx.style.margin_x;
    let column_w = ctx.style.page_width - 2.0 * ctx.style.margin_x;
    layout_node(root, column_x, column_w, ctx)
}

/// Lay out a single node into 0+ blocks.
///
/// `x` is the absolute left edge for any text in this subtree.
/// `width` is the available text-column width.
fn layout_node(
    node: &RenderableTreeNode,
    x: f32,
    width: f32,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<Block> {
    let RenderableTreeNode::Tag(tag) = node else {
        return Vec::new();
    };

    match tag.name.as_str() {
        // Document / generic structural wrappers: recurse children.
        "div" => layout_children(&tag.children, x, width, ctx),

        "p" => layout_paragraph(tag, x, width, ctx),
        h @ ("h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
            let level = h.as_bytes()[1] - b'0';
            layout_heading(tag, level, x, width, ctx)
        }

        "ul" => layout_list(tag, ListKind::Unordered, x, width, ctx),
        "ol" => layout_list(tag, ListKind::Ordered, x, width, ctx),

        "blockquote" => layout_blockquote(tag, x, width, ctx),

        "pre" => layout_code_block(tag, x, width, ctx),

        "callout" => layout_callout(tag, x, width, ctx),

        "table" => layout_table(tag, x, width, ctx),

        "img" | "media" => layout_media(tag, x, width, ctx),

        // `{% toc /%}` marks where a start-positioned table of contents
        // should be inserted (front matter before it, body after). The
        // marker block renders nothing; the renderer locates its page to
        // pick the split point. A trailing page break starts the body on
        // a fresh page after the ToC.
        "toc" => vec![toc_marker_block(x), page_break_block()],

        "hr" => vec![layout_rule(x, width, ctx.style)],

        // Inline-only tags should never appear at block level (`a`, `strong`
        // etc.); if they do, just recurse children defensively so content
        // isn't lost.
        _ => layout_children(&tag.children, x, width, ctx),
    }
}

fn layout_children(
    children: &[RenderableTreeNode],
    x: f32,
    width: f32,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < children.len() {
        // Detect `{% caption %}...{% /caption %}` directly followed by
        // either a `<table>` or an `<img>` / `<media>` (possibly
        // wrapped in a `<p>`). Render the caption as a small text
        // block above the figure/table, AND stash the caption text so
        // the corresponding layout (table / media) attaches it for
        // LoT / LoF entries.
        if let Some((cap_text, target, override_pos)) = caption_before_target(children, i) {
            let caption_block =
                build_caption_block(&caption_inlines_for_node(&children[i]), x, width, ctx);
            match target {
                CaptionTarget::Table => ctx.pending_table_caption = Some(cap_text.clone()),
                CaptionTarget::Figure => ctx.pending_figure_caption = Some(cap_text.clone()),
            }
            let target_blocks = layout_node(&children[i + 1], x, width, ctx);
            // Per-caption attribute wins over the global style setting.
            let pos = override_pos.unwrap_or(ctx.style.caption_position);
            match pos {
                super::style::CaptionPosition::Above => {
                    out.push(caption_block);
                    out.extend(target_blocks);
                }
                super::style::CaptionPosition::Below => {
                    out.extend(target_blocks);
                    out.push(caption_block);
                }
            }
            i += 2;
            continue;
        }
        out.extend(layout_node(&children[i], x, width, ctx));
        i += 1;
    }
    out
}

/// What follows a caption: either a table or a figure (image / media).
#[derive(Copy, Clone)]
enum CaptionTarget {
    Table,
    Figure,
}

/// Lay out the visible caption text as a small italic block at column x.
fn build_caption_block(inlines: &Inlines, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Block {
    let style = TextStyle {
        font_size: ctx.style.body_font_size * 0.92,
        font_weight: 400.0,
        line_height: ctx.style.body_line_height,
        color: ctx.style.blockquote_text_color.into(),
        font_families: ctx.body_families,
        italic: true,
    };
    let layout = build_layout(
        &inlines.text,
        &inlines.style_ranges,
        &style,
        width,
        ctx.font_cx,
        ctx.layout_cx,
    );
    let slice = TextSlice::whole_with_anchors(
        layout,
        inlines.text.clone(),
        inlines.links.clone(),
        inlines.mid_anchors.clone(),
        x,
    );
    let height = slice.height();
    Block {
        height,
        space_after: ctx.style.paragraph_space_after * 0.5,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,

        tag_role: None,
    }
}

/// If `children[i]` is a caption tag (or a paragraph wrapping one) and
/// `children[i+1]` is a captionable target (table or figure, possibly
/// wrapped in a `<p>`), return the caption's collected inline text,
/// the target kind, and an optional per-caption position override
/// supplied as `{% caption position="above"|"below" %}`.
fn caption_before_target(
    children: &[RenderableTreeNode],
    i: usize,
) -> Option<(String, CaptionTarget, Option<super::style::CaptionPosition>)> {
    let cap = find_caption_tag(children.get(i)?)?;
    let next = children.get(i + 1)?;
    let target = classify_caption_target(next)?;
    let mut text = String::new();
    let mut ranges = Vec::new();
    // Captions don't carry footnotes — pass a throwaway registry so
    // any stray `{% footnote %}` inside is silently dropped without
    // affecting the document's footnote numbering.
    collect_inlines(&mut text, &mut ranges, &cap.children, &mut Vec::new());
    if text.trim().is_empty() {
        return None;
    }
    let override_pos = caption_position_attr(cap);
    Some((text, target, override_pos))
}

/// Read an optional `position="above"|"below"` attribute from a
/// `{% caption %}` tag; unknown values are ignored.
fn caption_position_attr(cap: &Tag) -> Option<super::style::CaptionPosition> {
    match cap.attributes.get("position") {
        Some(Scalar::String(s)) => match s.as_str() {
            "above" => Some(super::style::CaptionPosition::Above),
            "below" => Some(super::style::CaptionPosition::Below),
            _ => None,
        },
        _ => None,
    }
}

fn classify_caption_target(node: &RenderableTreeNode) -> Option<CaptionTarget> {
    if contains_top_table(node) {
        return Some(CaptionTarget::Table);
    }
    if contains_top_figure(node) {
        return Some(CaptionTarget::Figure);
    }
    None
}

fn contains_top_figure(node: &RenderableTreeNode) -> bool {
    if let RenderableTreeNode::Tag(t) = node {
        if t.name == "img" || t.name == "media" {
            return true;
        }
        if t.name == "p" || t.name == "div" {
            return t.children.iter().any(contains_top_figure);
        }
    }
    false
}

/// Return the inner `<caption>` tag whether the node is the tag itself
/// or a `<p>` wrapping only that tag.
fn find_caption_tag(node: &RenderableTreeNode) -> Option<&Tag> {
    if let RenderableTreeNode::Tag(t) = node {
        if t.name == "caption" {
            return Some(t.as_ref());
        }
        if t.name == "p" {
            // Only accept paragraphs that contain a single caption tag
            // (plus possible softbreaks) — anything else is a real
            // paragraph that should not be repurposed.
            let mut found: Option<&Tag> = None;
            for c in &t.children {
                match c {
                    RenderableTreeNode::Tag(inner) => {
                        if inner.name == "caption" && found.is_none() {
                            found = Some(inner.as_ref());
                        } else if matches!(inner.name.as_str(), "softbreak" | "hardbreak" | "br") {
                            continue;
                        } else {
                            return None;
                        }
                    }
                    RenderableTreeNode::Scalar(_) => return None,
                }
            }
            return found;
        }
    }
    None
}

/// Check whether `node` is a `<table>` or a wrapper that contains one
/// directly.
fn contains_top_table(node: &RenderableTreeNode) -> bool {
    if let RenderableTreeNode::Tag(t) = node {
        if t.name == "table" {
            return true;
        }
        if t.name == "p" || t.name == "div" {
            return t.children.iter().any(contains_top_table);
        }
    }
    false
}

fn caption_inlines_for_node(node: &RenderableTreeNode) -> Inlines {
    if let Some(cap) = find_caption_tag(node) {
        // Captions don't carry document footnotes; throwaway registry.
        Inlines::from(&cap.children, &mut Vec::new())
    } else {
        Inlines::new()
    }
}

// ── Paragraph ───────────────────────────────────────────────────────────

fn layout_paragraph(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    // Markdown / Markdoc both wrap some block-level constructs inside
    // a `<p>` produced by pulldown-cmark:
    //   - `![alt](url)` becomes `<p><img></p>` / `<p><media></p>`.
    //   - `{% callout %}…{% /callout %}` written on its own line ends up
    //     inside a synthetic paragraph because the tokeniser sees it as
    //     a regular inline run.
    // Promote any such direct child to its block-level layout so it
    // actually renders as a box/image instead of being flattened into
    // the inline text by the `Inlines` collector. Remaining children
    // flow as a normal paragraph.
    let mut promoted: Vec<Block> = Vec::new();
    let mut text_children: Vec<RenderableTreeNode> = Vec::new();
    for child in &tag.children {
        if let RenderableTreeNode::Tag(t) = child {
            match t.name.as_str() {
                "img" | "media" => {
                    promoted.extend(layout_media(t, x, width, ctx));
                    continue;
                }
                "callout" => {
                    promoted.extend(layout_callout(t, x, width, ctx));
                    continue;
                }
                _ => {}
            }
        }
        text_children.push(child.clone());
    }

    let mut out = promoted;

    let mut inlines = Inlines::from(&text_children, &mut ctx.footnotes);
    if inlines.text.trim().is_empty() {
        return out;
    }
    // Style link text so readers spot it before they hover. The PDF
    // annotation is already created from `inlines.links`; this just
    // restyles the underlying glyphs. Done before hyphenation: links
    // pin the byte ranges anyway, so the soft-hyphen guard below
    // already skips paragraphs that contain any link.
    let link_style = &ctx.style.link;
    let link_color: krilla::color::rgb::Color = link_style.color.into();
    for link in &inlines.links {
        inlines.style_ranges.push(InlineRange {
            start: link.start,
            end: link.end,
            prop: InlineProp::Color(link_color),
        });
        if link_style.italic {
            inlines.style_ranges.push(InlineRange {
                start: link.start,
                end: link.end,
                prop: InlineProp::Italic,
            });
        }
        if link_style.bold {
            inlines.style_ranges.push(InlineRange {
                start: link.start,
                end: link.end,
                prop: InlineProp::Bold,
            });
        }
    }
    // Stamp the underline stroke on each link so emit can stroke a
    // rule under it. Stays None when the style disables underlining.
    if link_style.underline {
        let underline = super::inline::UnderlineStroke {
            color: link_color,
            thickness: link_style.underline_thickness,
        };
        for link in &mut inlines.links {
            link.underline = Some(underline.clone());
        }
    }
    // Hyphenate plain paragraphs only — inline ranges, links, anchors
    // and footnote calls all key on byte offsets, and inserting soft
    // hyphens shifts those, so we skip the pass when any are present.
    if let Some(h) = ctx.hyphenator
        && inlines.style_ranges.is_empty()
        && inlines.links.is_empty()
        && inlines.mid_anchors.is_empty()
        && inlines.footnote_calls.is_empty()
    {
        inlines.text = h.hyphenate(&inlines.text);
    }
    let style = TextStyle {
        font_size: ctx.style.body_font_size,
        font_weight: 400.0,
        line_height: ctx.style.body_line_height,
        color: ctx.style.text_color.into(),
        font_families: ctx.body_families,
        italic: false,
    };
    let layout = build_layout(
        &inlines.text,
        &inlines.style_ranges,
        &style,
        width,
        ctx.font_cx,
        ctx.layout_cx,
    );
    let slice = TextSlice::whole_with_extras(
        layout,
        inlines.text,
        inlines.links,
        inlines.mid_anchors,
        inlines.footnote_calls,
        x,
    );
    let height = slice.height();
    out.push(Block {
        height,
        space_after: ctx.style.paragraph_space_after,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,

        tag_role: None,
    });
    out
}

// ── Table of contents ───────────────────────────────────────────────────

/// One entry to render in the generated TOC. `text` is the heading
/// text; `level` is its heading level; `target_anchor_id` is the
/// auto-assigned anchor id of the heading (so the entry can hyperlink);
/// `page_number` is the 1-indexed PDF page number to display.
#[derive(Debug, Clone)]
pub struct TocEntry {
    pub level: u8,
    pub text: String,
    pub target_anchor_id: String,
    pub page_number: usize,
}

/// Format a single TOC entry with leader dots filling the gap between
/// the title and the page number:
///
///   `Title ......................... 12`
///
/// Falls back to a plain "Title  N" form when the title alone overflows
/// the available width (in which case the entry will wrap to multiple
/// lines and leader dots wouldn't make sense).
fn format_toc_entry_with_leaders(
    title: &str,
    page_number: usize,
    text_style: &TextStyle<'_>,
    available_width: f32,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
) -> String {
    let page_str = page_number.to_string();
    let title_w = measure_first_line_width(title, text_style, font_cx, layout_cx);
    let page_w = measure_first_line_width(&page_str, text_style, font_cx, layout_cx);
    // Reserve a small visual gap on each side of the dot run.
    let gap = measure_first_line_width(" ", text_style, font_cx, layout_cx) * 2.0;
    let dot_w = measure_first_line_width(".", text_style, font_cx, layout_cx);

    if dot_w <= 0.0 || title_w + page_w + gap >= available_width {
        // Title doesn't leave room for any dots — fall back to a plain
        // two-piece line; parley will wrap if it has to.
        return format!("{title}  {page_str}");
    }
    let dot_room = available_width - title_w - page_w - gap;
    let n_dots = (dot_room / dot_w).floor() as usize;
    if n_dots == 0 {
        return format!("{title}  {page_str}");
    }
    let dots: String = std::iter::repeat_n('.', n_dots).collect();
    format!("{title} {dots} {page_str}")
}

/// Build the TOC blocks: a title heading (h1-style), then one Block
/// per entry. Each entry is a single Text block with a clickable
/// internal link covering the entry text.
pub fn build_toc_blocks(
    entries: &[TocEntry],
    style: &Style,
    body_families: &'static [&'static str],
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let column_x = style.margin_x;
    let column_w = style.page_width - 2.0 * style.margin_x;

    // Title block (skipped when the title is empty).
    if !style.toc.title.is_empty() {
        let title_style = TextStyle {
            font_size: style.toc.title_font_size,
            font_weight: 700.0,
            line_height: style.body_line_height,
            color: style.text_color.into(),
            font_families: body_families,
            italic: false,
        };
        let title_layout = build_layout(
            &style.toc.title,
            &[],
            &title_style,
            column_w,
            font_cx,
            layout_cx,
        );
        let title_slice =
            TextSlice::whole(title_layout, style.toc.title.clone(), Vec::new(), column_x);
        blocks.push(Block {
            height: title_slice.height(),
            space_after: 18.0,
            draw: BlockDraw::Text(title_slice),
            // A top-level outline entry so the section's own title drives
            // the running header (and bookmark) on its page rather than
            // the previous chapter bleeding through.
            outline: Some(OutlineEntry {
                level: 1,
                text: style.toc.title.clone(),
            }),
            anchor_id: None,

            tag_role: None,
        });
    }

    // Per-entry blocks.
    let entry_text_style = TextStyle {
        font_size: style.toc.entry_font_size,
        font_weight: 400.0,
        line_height: style.body_line_height,
        color: style.text_color.into(),
        font_families: body_families,
        italic: false,
    };
    for entry in entries {
        if entry.level > style.toc.max_depth {
            continue;
        }
        let indent = (entry.level.saturating_sub(1) as f32) * style.toc.entry_indent_per_level;
        let entry_x = column_x + indent;
        let entry_w = column_w - indent;

        let body = format_toc_entry_with_leaders(
            &entry.text,
            entry.page_number,
            &entry_text_style,
            entry_w,
            font_cx,
            layout_cx,
        );

        let layout = build_layout(&body, &[], &entry_text_style, entry_w, font_cx, layout_cx);
        // Whole entry is clickable.
        let links = vec![LinkRange {
            start: 0,
            end: body.len(),
            href: format!("#{}", entry.target_anchor_id),
            title: None,
            underline: None,
        }];
        let slice = TextSlice::whole(layout, body, links, entry_x);
        blocks.push(Block {
            height: slice.height(),
            space_after: style.toc.entry_space_after,
            draw: BlockDraw::Text(slice),
            outline: None,
            anchor_id: None,

            tag_role: None,
        });
    }

    blocks
}

/// Like `build_toc_blocks` but for flat list sections (List of Figures,
/// List of Tables). Title and font sizes come from a `ListSectionStyle`
/// instead of `TocStyle`. Entries don't indent by level.
#[allow(clippy::too_many_arguments)]
pub fn build_list_section_blocks(
    title: &str,
    title_font_size: f32,
    entry_font_size: f32,
    entry_space_after: f32,
    entries: &[TocEntry],
    style: &Style,
    body_families: &'static [&'static str],
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let column_x = style.margin_x;
    let column_w = style.page_width - 2.0 * style.margin_x;

    if !title.is_empty() {
        let title_style = TextStyle {
            font_size: title_font_size,
            font_weight: 700.0,
            line_height: style.body_line_height,
            color: style.text_color.into(),
            font_families: body_families,
            italic: false,
        };
        let title_layout = build_layout(title, &[], &title_style, column_w, font_cx, layout_cx);
        let title_slice = TextSlice::whole(title_layout, title.to_string(), Vec::new(), column_x);
        blocks.push(Block {
            height: title_slice.height(),
            space_after: 18.0,
            draw: BlockDraw::Text(title_slice),
            // Top-level outline entry so this section's title drives its
            // own running header rather than inheriting the prior chapter.
            outline: Some(OutlineEntry {
                level: 1,
                text: title.to_string(),
            }),
            anchor_id: None,

            tag_role: None,
        });
    }

    let entry_text_style = TextStyle {
        font_size: entry_font_size,
        font_weight: 400.0,
        line_height: style.body_line_height,
        color: style.text_color.into(),
        font_families: body_families,
        italic: false,
    };
    for entry in entries {
        let body = format_toc_entry_with_leaders(
            &entry.text,
            entry.page_number,
            &entry_text_style,
            column_w,
            font_cx,
            layout_cx,
        );
        let layout = build_layout(&body, &[], &entry_text_style, column_w, font_cx, layout_cx);
        let links = vec![LinkRange {
            start: 0,
            end: body.len(),
            href: format!("#{}", entry.target_anchor_id),
            title: None,
            underline: None,
        }];
        let slice = TextSlice::whole(layout, body, links, column_x);
        blocks.push(Block {
            height: slice.height(),
            space_after: entry_space_after,
            draw: BlockDraw::Text(slice),
            outline: None,
            anchor_id: None,

            tag_role: None,
        });
    }
    blocks
}

// ── Heading ─────────────────────────────────────────────────────────────

/// True when tag `t` carries `key` set to a falsey scalar — `false`
/// (boolean), `"false"`, or `"0"`. Used to read `numbered="false"` off
/// a heading's anchor tag. Absent or any other value reads as "not
/// explicitly false".
fn attr_is_false(t: &Tag, key: &str) -> bool {
    match t.attributes.get(key) {
        Some(Scalar::Boolean(b)) => !b,
        Some(Scalar::String(s)) => {
            let s = s.trim();
            s.eq_ignore_ascii_case("false") || s == "0"
        }
        _ => false,
    }
}

fn layout_heading(tag: &Tag, level: u8, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    let h = ctx.style.heading.for_level(level).clone();

    // Pull anchor declarations (`{% tag id="X" %}`) out of the heading's
    // children so the id is recorded on the resulting block. Returns
    // the first id found; subsequent declarations on the same heading
    // are ignored (uncommon, but tolerated). The same scan reads an
    // optional `numbered="false"` attribute which opts the heading out
    // of automatic section numbering (used for front-matter headings).
    let mut heading_anchor: Option<String> = None;
    let mut opt_out_numbering = false;
    for child in &tag.children {
        if let RenderableTreeNode::Tag(t) = child
            && t.name == "tag"
        {
            if heading_anchor.is_none()
                && let Some(id) = super::inline::anchor_id_attr(t)
            {
                heading_anchor = Some(id);
            }
            if attr_is_false(t, "numbered") {
                opt_out_numbering = true;
            }
        }
    }

    let mut text = String::new();
    let mut ranges = Vec::new();
    collect_inlines(&mut text, &mut ranges, &tag.children, &mut ctx.footnotes);
    if text.trim().is_empty() {
        return Vec::new();
    }

    // Prepend the automatic section number when enabled and not opted
    // out. Baking it into `text` (and the outline text below) means it
    // flows to the visible heading, the running header, and the ToC
    // without any further plumbing — each consumes the heading string.
    if !opt_out_numbering && let Some(number) = ctx.bump_heading_number(level) {
        let prefix = format!("{number}{}", ctx.style.heading_numbering.separator);
        // Shift any inline style ranges right by the prefix length so
        // bold/italic spans keep covering the original words.
        let shift = prefix.len();
        for r in &mut ranges {
            r.start += shift;
            r.end += shift;
        }
        text.insert_str(0, &prefix);
    }

    // Emit space-before as an empty (zero-height) spacer block so the
    // paginator sees it; otherwise consecutive headings on a fresh page
    // would push the heading off the top edge.
    let mut blocks = Vec::new();
    if h.space_before > 0.0 {
        blocks.push(spacer_block(h.space_before));
    }

    let style = TextStyle {
        font_size: h.font_size,
        font_weight: h.font_weight,
        line_height: ctx.style.body_line_height,
        color: h.color.unwrap_or(ctx.style.text_color).into(),
        font_families: ctx.body_families,
        italic: false,
    };
    let layout = build_layout(&text, &ranges, &style, width, ctx.font_cx, ctx.layout_cx);
    let outline_text = text.clone();
    let slice = TextSlice::whole(layout, text, Vec::new(), x);
    let height = slice.height();
    // Auto-assign a synthetic anchor when the heading didn't have an
    // explicit `{% tag %}` declaration. ToC entries (and any future
    // "back to chapter" links) need every heading to be linkable.
    let heading_id = ctx.next_heading_id();
    let resolved_anchor = heading_anchor.unwrap_or_else(|| format!("__heading_{heading_id}"));

    blocks.push(Block {
        height,
        space_after: h.space_after,
        draw: BlockDraw::Text(slice),
        outline: Some(OutlineEntry {
            level,
            text: outline_text,
        }),
        anchor_id: Some(resolved_anchor),

        tag_role: None,
    });
    blocks
}

// ── Lists ───────────────────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum ListKind {
    Unordered,
    Ordered,
}

fn layout_list(
    tag: &Tag,
    kind: ListKind,
    x: f32,
    width: f32,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<Block> {
    let indent = ctx.style.list_indent;
    let content_x = x + indent;
    let content_w = width - indent;
    let marker_x = x;

    // Iterate <li> children; non-li children (rare) are skipped.
    let items: Vec<&RenderableTreeNode> = tag
        .children
        .iter()
        .filter(|c| matches!(c, RenderableTreeNode::Tag(t) if t.name == "li"))
        .collect();

    let mut out = Vec::new();
    for (idx, item_node) in items.iter().enumerate() {
        let RenderableTreeNode::Tag(item_tag) = item_node else {
            continue;
        };
        let ordered = matches!(kind, ListKind::Ordered);
        let badge_on = ordered && ctx.style.list_marker.badge;
        let marker_text = match kind {
            ListKind::Unordered => "•".to_string(),
            ListKind::Ordered => {
                let seq = ctx.style.list_marker.sequence_for_depth(ctx.list_depth);
                let label = format_ordered_marker(idx + 1, seq);
                // A badge delimits the marker visually, so the trailing
                // dot is dropped in that mode.
                if badge_on { label } else { format!("{label}.") }
            }
        };
        // Inside a badge the marker uses the badge text colour; otherwise
        // it inherits the body text colour.
        let marker_color = if badge_on {
            ctx.style
                .list_marker
                .badge_text_color
                .unwrap_or(ctx.style.text_color)
                .into()
        } else {
            ctx.style.text_color.into()
        };
        let marker_style = TextStyle {
            font_size: ctx.style.body_font_size,
            font_weight: 400.0,
            line_height: ctx.style.body_line_height,
            color: marker_color,
            font_families: ctx.body_families,
            italic: false,
        };
        let marker_layout = build_layout(
            &marker_text,
            &[],
            &marker_style,
            indent,
            ctx.font_cx,
            ctx.layout_cx,
        );

        // Badge geometry: a circle sized off the body font, centred on
        // the first line's text middle (baseline less ~half cap-height)
        // so the marker — drawn at its natural y — lands centred in it.
        let badge = if badge_on {
            let fs = ctx.style.body_font_size;
            let baseline = marker_layout
                .lines()
                .next()
                .map(|l| l.metrics().baseline)
                .unwrap_or(fs);
            let diameter = fs * ctx.style.list_marker.badge_scale;
            Some(MarkerBadge {
                fill: ctx.style.list_marker.badge_fill.into(),
                diameter,
                center_dx: diameter * 0.5,
                center_dy: baseline - 0.32 * fs,
            })
        } else {
            None
        };

        // Lay out the item body one nesting level deeper so a nested
        // ordered list picks the next numbering style. <li> children may
        // be paragraphs, nested lists, etc. — recurse normally.
        ctx.list_depth += 1;
        let body = layout_li_body(item_tag, content_x, content_w, ctx);
        ctx.list_depth -= 1;
        let body_height: f32 = body
            .iter()
            .map(|b| b.height + b.space_after)
            .sum::<f32>()
            // Trim trailing space_after of the last child — the item's
            // own list_item_space_after takes its place.
            .max(0.0);
        let body_height = body_height - body.last().map(|b| b.space_after).unwrap_or(0.0);

        out.push(Block {
            height: body_height.max(marker_layout.height()),
            space_after: ctx.style.list_item_space_after,
            draw: BlockDraw::ListItem {
                marker: marker_layout,
                marker_text,
                marker_x,
                body,
                ordered,
                badge,
            },
            outline: None,
            anchor_id: None,

            tag_role: None,
        });
    }

    // No extra space after the whole list (the last item carries its own
    // space_after; behaves consistently with other top-level blocks).
    out
}

/// Format a 1-based ordered-list position in the given sequence style.
fn format_ordered_marker(n: usize, seq: MarkerSequence) -> String {
    match seq {
        MarkerSequence::Decimal => n.to_string(),
        MarkerSequence::LowerAlpha => to_alpha(n, false),
        MarkerSequence::UpperAlpha => to_alpha(n, true),
        MarkerSequence::LowerRoman => to_roman(n, false),
        MarkerSequence::UpperRoman => to_roman(n, true),
    }
}

/// Spreadsheet-style bijective base-26: 1→a, 26→z, 27→aa, 28→ab, …
fn to_alpha(mut n: usize, upper: bool) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let base = if upper { b'A' } else { b'a' };
    let mut buf = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        buf.push(base + rem as u8);
        n = (n - 1) / 26;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap()
}

/// Roman numerals for 1..=3999; anything outside that classic range
/// falls back to decimal.
fn to_roman(mut n: usize, upper: bool) -> String {
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    const VALS: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, sym) in VALS {
        while n >= v {
            s.push_str(sym);
            n -= v;
        }
    }
    if upper { s.to_uppercase() } else { s }
}

/// Layout the contents of one `<li>`. Markdown list items often wrap
/// their contents in a paragraph for "loose" lists or place text
/// directly for "tight" lists; both cases reduce to "lay out children".
fn layout_li_body(item: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    // If the <li>'s direct children are all inline (text/strong/em/...) and
    // there's no block-level child, treat the whole thing as one paragraph.
    let any_block_child = item.children.iter().any(|c| {
        if let RenderableTreeNode::Tag(t) = c {
            matches!(
                t.name.as_str(),
                "p" | "ul"
                    | "ol"
                    | "blockquote"
                    | "pre"
                    | "callout"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "hr"
            )
        } else {
            false
        }
    });
    if !any_block_child {
        // Synthesize a paragraph wrapper to reuse layout_paragraph.
        return layout_paragraph(item, x, width, ctx);
    }
    layout_children(&item.children, x, width, ctx)
}

// ── Block quote ─────────────────────────────────────────────────────────

fn layout_blockquote(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    let indent = ctx.style.blockquote_indent;
    let inner_x = x + indent;
    let inner_w = width - indent;

    // Lay out children with shifted text colour to set them apart visually.
    // We do this by walking children manually with a tweaked TextStyle —
    // but for simplicity we just use the regular layout and rely on the
    // accent bar drawn by the BoxedGroup decoration.
    let children = layout_children(&tag.children, inner_x, inner_w, ctx);
    let inner_height: f32 = children
        .iter()
        .map(|b| b.height + b.space_after)
        .sum::<f32>()
        - children.last().map(|b| b.space_after).unwrap_or(0.0);

    vec![Block {
        height: inner_height,
        space_after: ctx.style.paragraph_space_after,
        draw: BlockDraw::BoxedGroup {
            x,
            width,
            background: None,
            border: None,
            accent_left: Some(ctx.style.blockquote_bar_color.into()),
            accent_width: ctx.style.blockquote_bar_width,
            padding: 0.0, // children already shifted via inner_x
            children,
            icon: None,
            top_rule: None,
            bottom_rule: None,
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    }]
}

// ── Code block ──────────────────────────────────────────────────────────

fn layout_code_block(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    // <pre> in our transformer typically wraps a Code node with the source
    // as its content scalar. Extract the raw text.
    let source = extract_code_text(&tag.children);
    if source.trim().is_empty() {
        return Vec::new();
    }

    let padding = ctx.style.code_padding;
    let inner_x = x + padding;
    let inner_w = width - 2.0 * padding;

    let mono_families = monospace_families(&ctx.style.code_font_family);
    // SAFETY: monospace_families returns &'static str entries.
    let leaked: Vec<&'static str> = mono_families.into_iter().collect();
    let style = TextStyle {
        font_size: ctx.style.code_font_size,
        font_weight: 400.0,
        line_height: ctx.style.body_line_height,
        color: ctx.style.code_text_color.into(),
        font_families: Box::leak(leaked.into_boxed_slice()),
        italic: false,
    };

    // Highlight: schema renders the fence's `language` attribute as
    // `data-language`. Map every token to an InlineRange::Color tinted
    // by the style palette. Unknown languages produce no spans, so the
    // block renders in plain `code_text_color`.
    let lang = match tag.attributes.get("data-language") {
        Some(Scalar::String(s)) => s.as_str(),
        _ => "",
    };
    let palette = &ctx.style.code_highlight;
    let ranges: Vec<super::inline::InlineRange> = if lang.is_empty() {
        Vec::new()
    } else {
        super::highlight::tokenize(lang, &source)
            .into_iter()
            .map(|t| {
                let color: krilla::color::rgb::Color = match t.class {
                    super::highlight::TokenClass::Keyword => palette.keyword.into(),
                    super::highlight::TokenClass::String => palette.string.into(),
                    super::highlight::TokenClass::Comment => palette.comment.into(),
                    super::highlight::TokenClass::Number => palette.number.into(),
                };
                super::inline::InlineRange {
                    start: t.start,
                    end: t.end,
                    prop: super::inline::InlineProp::Color(color),
                }
            })
            .collect()
    };

    let layout = build_layout(
        &source,
        &ranges,
        &style,
        inner_w,
        ctx.font_cx,
        ctx.layout_cx,
    );
    let inner_height = layout.height();

    let slice = TextSlice::whole(layout, source, Vec::new(), inner_x);
    let inner_block = Block {
        height: inner_height,
        space_after: 0.0,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,

        tag_role: None,
    };

    vec![Block {
        height: inner_height + 2.0 * padding,
        space_after: ctx.style.paragraph_space_after,
        draw: BlockDraw::BoxedGroup {
            x,
            width,
            background: Some(ctx.style.code_background.into()),
            border: None,
            accent_left: None,
            accent_width: 0.0,
            padding,
            children: vec![inner_block],
            icon: None,
            top_rule: None,
            bottom_rule: None,
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    }]
}

fn extract_code_text(children: &[RenderableTreeNode]) -> String {
    let mut s = String::new();
    for child in children {
        match child {
            RenderableTreeNode::Scalar(Scalar::String(text)) => s.push_str(text),
            RenderableTreeNode::Tag(t) => s.push_str(&extract_code_text(&t.children)),
            _ => {}
        }
    }
    s
}

/// True for tag names that are laid out as block-level constructs.
/// Anything not in this set (plain `Scalar`s, `strong`, `em`, `a`, …)
/// is treated as inline content and grouped into a paragraph wrapper
/// by [`wrap_inline_runs_in_paragraphs`].
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "blockquote"
            | "pre"
            | "callout"
            | "table"
            | "hr"
            | "div"
            | "img"
            | "media"
    )
}

/// Group consecutive inline children (bare `Scalar`s, inline `Tag`s like
/// `strong`/`em`/`a`/`code`) into synthetic `<p>` wrappers, leaving
/// block-level children untouched. Needed because markdoc's parser
/// leaves loose inline content directly under a block-level tag like
/// `{% callout %}…{% /callout %}` — without this wrapping
/// `layout_node` would drop the bare scalars.
fn wrap_inline_runs_in_paragraphs(children: &[RenderableTreeNode]) -> Vec<RenderableTreeNode> {
    let mut out: Vec<RenderableTreeNode> = Vec::new();
    let mut run: Vec<RenderableTreeNode> = Vec::new();
    let flush = |run: &mut Vec<RenderableTreeNode>, out: &mut Vec<RenderableTreeNode>| {
        if run.is_empty() {
            return;
        }
        let p = Tag {
            name: "p".to_string(),
            attributes: std::collections::HashMap::new(),
            children: std::mem::take(run),
        };
        out.push(RenderableTreeNode::Tag(Box::new(p)));
    };
    for child in children {
        match child {
            RenderableTreeNode::Tag(t) if is_block_tag(&t.name) => {
                flush(&mut run, &mut out);
                out.push(child.clone());
            }
            _ => run.push(child.clone()),
        }
    }
    flush(&mut run, &mut out);
    out
}

// ── Callout ─────────────────────────────────────────────────────────────

fn layout_callout(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    let kind = match tag.attributes.get("type") {
        Some(Scalar::String(s)) => s.as_str(),
        _ => "note",
    };
    let cs = ctx.style.callout_styles.for_kind(kind).clone();
    let padding = ctx.style.callout_padding;
    // The left accent stripe only exists in the box framing; the bulletin
    // (rules) framing has none, so it reserves no left gutter.
    let accent_w = match cs.decoration {
        super::style::CalloutDecoration::Box => ctx.style.callout_accent_width,
        super::style::CalloutDecoration::Rules => 0.0,
    };
    // Inner column accounts for both the accent bar (on the left) and the
    // padding on both sides.
    let inner_x = x + accent_w + padding;
    let inner_w = width - accent_w - 2.0 * padding;

    // Decode the optional icon once (cached by the resolver). When
    // present, the label and body are indented past an icon gutter so
    // they sit beside it rather than under it.
    let icon = if cs.icon.trim().is_empty() {
        None
    } else {
        super::decoration::decode_media(cs.icon.trim(), ctx.assets).map(|decoded| BoxedGroupIcon {
            decoded,
            x: inner_x,
            size: ctx.style.callout_icon_size,
        })
    };
    let icon_gutter = icon
        .as_ref()
        .map(|i| i.size + ctx.style.callout_icon_gap)
        .unwrap_or(0.0);
    let content_x = inner_x + icon_gutter;
    let content_w = inner_w - icon_gutter;

    // Optional bold label as the first child line.
    let mut children: Vec<Block> = Vec::new();
    if !cs.label.trim().is_empty() {
        children.push(build_callout_label_block(
            cs.label.trim(),
            content_x,
            content_w,
            cs.label_color.unwrap_or(ctx.style.text_color).into(),
            cs.label_centered,
            ctx,
        ));
    }

    // markdoc's parser doesn't wrap a callout's text content in a `<p>` —
    // body children come through as raw `Scalar`s and inline tags. Group
    // consecutive inline children into synthetic paragraphs so they
    // actually render; block-level tags (lists, tables, nested callouts)
    // are still laid out individually.
    let wrapped = wrap_inline_runs_in_paragraphs(&tag.children);
    children.extend(layout_children(&wrapped, content_x, content_w, ctx));
    let content_height: f32 = children
        .iter()
        .map(|b| b.height + b.space_after)
        .sum::<f32>()
        - children.last().map(|b| b.space_after).unwrap_or(0.0);
    // The box must clear both the content and (if taller) the icon.
    let inner_height = content_height.max(icon.as_ref().map(|i| i.size).unwrap_or(0.0));

    // Two framings: a filled box (default) or a pair of horizontal rules
    // (bulletin style). The rule colour is the accent.
    let (background, border, accent_left, top_rule, bottom_rule) = match cs.decoration {
        super::style::CalloutDecoration::Box => (
            Some(cs.background.into()),
            Some(cs.border.into()),
            Some(cs.accent.into()),
            None,
            None,
        ),
        super::style::CalloutDecoration::Rules => {
            let rule = (cs.accent.into(), ctx.style.callout_rule_thickness);
            (None, None, None, Some(rule), Some(rule))
        }
    };

    vec![Block {
        height: inner_height + 2.0 * padding,
        space_after: ctx.style.callout_space_after,
        draw: BlockDraw::BoxedGroup {
            x,
            width,
            background,
            border,
            accent_left,
            accent_width: accent_w,
            padding,
            children,
            icon,
            top_rule,
            bottom_rule,
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    }]
}

/// Build the bold single-line label block (e.g. `WARNING`) that heads a
/// callout. Positioned at `x` with the given colour; `centered` spreads
/// it across `width` instead of left-aligning.
fn build_callout_label_block(
    label: &str,
    x: f32,
    width: f32,
    color: rgb::Color,
    centered: bool,
    ctx: &mut LayoutCtx<'_>,
) -> Block {
    let style = TextStyle {
        font_size: ctx.style.callout_label_size,
        font_weight: 700.0,
        line_height: ctx.style.body_line_height,
        color,
        font_families: ctx.body_families,
        italic: false,
    };
    let layout = if centered {
        super::text::build_layout_aligned(
            label,
            &[],
            &style,
            width,
            parley::layout::Alignment::Center,
            ctx.font_cx,
            ctx.layout_cx,
        )
    } else {
        build_layout(label, &[], &style, width, ctx.font_cx, ctx.layout_cx)
    };
    let slice = TextSlice::whole(layout, label.to_string(), Vec::new(), x);
    let height = slice.height();
    Block {
        height,
        space_after: ctx.style.callout_label_size * 0.4,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,
        tag_role: None,
    }
}

// ── Horizontal rule ─────────────────────────────────────────────────────

fn layout_rule(x: f32, width: f32, style: &Style) -> Block {
    Block {
        height: style.rule_thickness,
        space_after: style.rule_space_around,
        draw: BlockDraw::Rule {
            x,
            width,
            thickness: style.rule_thickness,
            color: style.rule_color.into(),
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    }
}

/// Sentinel anchor id stamped on the `{% toc /%}` marker block. The
/// leading control character keeps it from ever colliding with an
/// author-declared `{% tag id="…" %}` anchor.
pub const TOC_MARKER_ANCHOR: &str = "\u{0}toc-marker";

/// Zero-height marker block for `{% toc /%}`. Renders nothing; the
/// renderer finds it by [`TOC_MARKER_ANCHOR`] to place the ToC.
fn toc_marker_block(x: f32) -> Block {
    Block {
        height: 0.0,
        space_after: 0.0,
        draw: BlockDraw::Rule {
            x,
            width: 0.0,
            thickness: 0.0,
            color: rgb::Color::new(0, 0, 0),
        },
        outline: None,
        anchor_id: Some(TOC_MARKER_ANCHOR.to_string()),
        tag_role: None,
    }
}

/// A page-break marker block (flushes the current page during pagination).
fn page_break_block() -> Block {
    Block {
        height: 0.0,
        space_after: 0.0,
        draw: BlockDraw::PageBreak,
        outline: None,
        anchor_id: None,
        tag_role: None,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn spacer_block(height: f32) -> Block {
    // Use a degenerate Rule with zero thickness to represent vertical
    // whitespace; the emit pass treats zero-thickness rules as no-ops.
    Block {
        height,
        space_after: 0.0,
        draw: BlockDraw::Rule {
            x: 0.0,
            width: 0.0,
            thickness: 0.0,
            color: rgb::Color::new(0, 0, 0),
        },
        outline: None,
        anchor_id: None,

        tag_role: None,
    }
}

// ── Tables ──────────────────────────────────────────────────────────────

/// Lay out a `<table>` into a single `Table` block. Equal column widths;
/// header rows get bold text + tinted background; cells can hold any
/// block content (paragraphs, lists, …).
fn layout_table(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    // Collect rows from <thead> and <tbody>. pulldown-cmark's pipe-table
    // form puts cells directly under <thead> (no wrapping <tr>) for the
    // header, and emits <tr>s as direct children of <table> for body
    // rows. We tolerate both shapes:
    //   <table><thead><tr><td/></tr></thead><tbody><tr>…</tr></tbody></table>
    //   <table><thead><td/><td/></thead><tr/><tr/></table>
    let mut header_rows: Vec<RowSource<'_>> = Vec::new();
    let mut body_rows: Vec<RowSource<'_>> = Vec::new();
    for child in &tag.children {
        let RenderableTreeNode::Tag(t) = child else {
            continue;
        };
        match t.name.as_str() {
            "thead" => {
                // If <thead> has any child cell directly, treat thead as
                // one row; otherwise look for <tr> children.
                if t.children.iter().any(is_cell_node) {
                    header_rows.push(RowSource::CellsDirect(t));
                } else {
                    for tr in t.children.iter().filter_map(as_tr) {
                        header_rows.push(RowSource::Row(tr));
                    }
                }
            }
            "tbody" => {
                for tr in t.children.iter().filter_map(as_tr) {
                    body_rows.push(RowSource::Row(tr));
                }
            }
            "tr" => body_rows.push(RowSource::Row(t)),
            _ => {}
        }
    }
    if header_rows.is_empty() && body_rows.is_empty() {
        return Vec::new();
    }
    // A markdown table must declare a header row; when that row is wholly
    // empty (the header-less metadata table authored as `| | |`), drop it
    // so the table renders without a stray empty band and rule on top.
    if !header_rows.is_empty() && rows_are_blank(&header_rows) {
        header_rows.clear();
    }

    // Column count = max cells across all rows.
    let num_cols = header_rows
        .iter()
        .chain(body_rows.iter())
        .map(|r| r.cell_count())
        .max()
        .unwrap_or(0);
    if num_cols == 0 {
        return Vec::new();
    }

    let padding = ctx.style.table_cell_padding;
    let border_thickness = ctx.style.table_border_thickness;
    let total_borders = border_thickness * (num_cols as f32 + 1.0);
    let inner_width = width - total_borders;

    // Decide column widths.
    let column_widths = match ctx.style.table_column_sizing {
        TableColumnSizing::Equal => vec![inner_width / num_cols as f32; num_cols],
        TableColumnSizing::Auto => compute_auto_column_widths(
            &header_rows,
            &body_rows,
            num_cols,
            inner_width,
            padding,
            ctx,
        ),
    };

    // Compute per-column x origins (left edge of each column's content area).
    let mut column_xs = Vec::with_capacity(num_cols);
    let mut cursor = x + border_thickness;
    for w in &column_widths {
        column_xs.push(cursor);
        cursor += w + border_thickness;
    }

    let mut rows_out: Vec<TableRow> = Vec::new();
    for r in header_rows.iter() {
        rows_out.push(layout_row_source(
            r,
            true,
            &column_xs,
            &column_widths,
            padding,
            ctx,
        ));
    }
    for r in body_rows.iter() {
        rows_out.push(layout_row_source(
            r,
            false,
            &column_xs,
            &column_widths,
            padding,
            ctx,
        ));
    }

    let table_height: f32 = rows_out.iter().map(|r| r.height).sum::<f32>()
        + border_thickness * (rows_out.len() as f32 + 1.0);

    let table_id = ctx.next_table_id();

    vec![Block {
        height: table_height,
        space_after: ctx.style.table_space_after,
        draw: BlockDraw::Table {
            x,
            column_widths,
            rows: rows_out,
            cell_padding: padding,
            header_bg: ctx.style.table_header_background.into(),
            border_color: ctx.style.table_border_color.into(),
            border_thickness,
            border_style: ctx.style.table_borders,
            edge: ctx.style.table_edge_color.map(|c| {
                (
                    c.into(),
                    ctx.style.table_edge_thickness.unwrap_or(border_thickness),
                )
            }),
            caption: ctx.pending_table_caption.take(),
        },
        outline: None,
        anchor_id: Some(format!("__table_{table_id}")),
        tag_role: None,
    }]
}

/// A row source — either a real <tr> wrapper, or a section (e.g. <thead>)
/// whose direct children are the cells (pulldown-cmark's pipe-table form).
enum RowSource<'a> {
    Row(&'a Tag),
    CellsDirect(&'a Tag),
}

impl<'a> RowSource<'a> {
    fn cells(&self) -> impl Iterator<Item = &'a Tag> + 'a {
        let parent: &'a Tag = match self {
            RowSource::Row(t) => t,
            RowSource::CellsDirect(t) => t,
        };
        parent.children.iter().filter_map(|c| {
            if let RenderableTreeNode::Tag(t) = c
                && (t.name == "th" || t.name == "td")
            {
                return Some(t.as_ref());
            }
            None
        })
    }

    fn cell_count(&self) -> usize {
        self.cells().count()
    }
}

fn is_cell_node(c: &RenderableTreeNode) -> bool {
    matches!(c, RenderableTreeNode::Tag(t) if t.name == "th" || t.name == "td")
}

fn as_tr(c: &RenderableTreeNode) -> Option<&Tag> {
    if let RenderableTreeNode::Tag(t) = c
        && t.name == "tr"
    {
        return Some(t.as_ref());
    }
    None
}

/// True when every cell across `rows` is visually empty — the case
/// markdown's mandatory header row produces for a header-less metadata
/// table (`| | |`). Used to drop such a header so it leaves no stray
/// band or rule.
fn rows_are_blank(rows: &[RowSource<'_>]) -> bool {
    rows.iter()
        .all(|r| r.cells().all(|c| node_text_is_blank(&c.children)))
}

/// Recursively true when a node subtree carries no visible content — no
/// non-whitespace text and no image/media tag.
fn node_text_is_blank(nodes: &[RenderableTreeNode]) -> bool {
    nodes.iter().all(|n| match n {
        RenderableTreeNode::Scalar(Scalar::String(s)) => s.trim().is_empty(),
        // Numbers / bools render as visible text.
        RenderableTreeNode::Scalar(_) => false,
        RenderableTreeNode::Tag(t) => {
            !matches!(t.name.as_str(), "img" | "media") && node_text_is_blank(&t.children)
        }
    })
}

fn layout_row_source(
    src: &RowSource<'_>,
    is_header: bool,
    column_xs: &[f32],
    column_widths: &[f32],
    padding: f32,
    ctx: &mut LayoutCtx<'_>,
) -> TableRow {
    let mut cells: Vec<Vec<Block>> = Vec::with_capacity(column_xs.len());
    for (col, cell_tag) in src.cells().enumerate() {
        if col >= column_xs.len() {
            break;
        }
        let cell_x = column_xs[col] + padding;
        let cell_text_width = (column_widths[col] - 2.0 * padding).max(0.0);
        let blocks = layout_cell_content(cell_tag, cell_x, cell_text_width, is_header, ctx);
        cells.push(blocks);
    }
    while cells.len() < column_xs.len() {
        cells.push(Vec::new());
    }
    let cell_heights: Vec<f32> = cells
        .iter()
        .map(|blocks| sum_block_height(blocks))
        .collect();
    let max_height = cell_heights.iter().cloned().fold(0.0_f32, f32::max);
    TableRow {
        is_header,
        height: max_height + 2.0 * padding,
        cells,
    }
}

/// Two-pass auto-sizing: measure each cell's natural & min widths,
/// then distribute `inner_width` across columns proportionally.
fn compute_auto_column_widths(
    header_rows: &[RowSource<'_>],
    body_rows: &[RowSource<'_>],
    num_cols: usize,
    inner_width: f32,
    padding: f32,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<f32> {
    let mut col_min: Vec<f32> = vec![0.0; num_cols];
    let mut col_max: Vec<f32> = vec![0.0; num_cols];

    for src in header_rows.iter().chain(body_rows.iter()) {
        for (col, cell_tag) in src.cells().enumerate() {
            if col >= num_cols {
                break;
            }
            let weight = if matches!(src, RowSource::CellsDirect(_)) {
                700.0
            } else {
                400.0
            };
            // body rows from a Row source are non-header; safe approximation
            // (header detection is parent-based, but cells in a Row in
            // header_rows still belong to header — measurement weight isn't
            // critical here, just affects measured widths slightly).
            let style = TextStyle {
                font_size: ctx.style.body_font_size,
                font_weight: weight,
                line_height: ctx.style.body_line_height,
                color: ctx.style.text_color.into(),
                font_families: ctx.body_families,
                italic: false,
            };
            let (cell_min, cell_max) = measure_cell_widths(cell_tag, &style, ctx);
            col_min[col] = col_min[col].max(cell_min + 2.0 * padding);
            col_max[col] = col_max[col].max(cell_max + 2.0 * padding);
        }
    }

    distribute(col_min, col_max, inner_width)
}

/// Measure a cell's content widths via the inline-text projection.
/// Returns `(longest-word-width, full-natural-width)`.
fn measure_cell_widths(cell: &Tag, style: &TextStyle<'_>, ctx: &mut LayoutCtx<'_>) -> (f32, f32) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    // Measurement only — drop any footnote calls into a throwaway
    // registry so column-width measurement doesn't allocate footnote
    // numbers that the actual layout pass would re-allocate.
    collect_inlines(&mut text, &mut ranges, &cell.children, &mut Vec::new());
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (0.0, 0.0);
    }
    let natural = measure_first_line_width(trimmed, style, ctx.font_cx, ctx.layout_cx);
    let min_word = trimmed
        .split_whitespace()
        .map(|w| measure_first_line_width(w, style, ctx.font_cx, ctx.layout_cx))
        .fold(0.0_f32, f32::max);
    (min_word, natural)
}

/// Distribute `inner_width` between columns so that no column is
/// narrower than its `min_word` (so words don't get broken) but all
/// available space is consumed when content is narrower than the page.
fn distribute(col_min: Vec<f32>, col_max: Vec<f32>, inner_width: f32) -> Vec<f32> {
    let n = col_min.len();
    let sum_max: f32 = col_max.iter().sum();
    if sum_max <= inner_width {
        // Plenty of room: hand out a proportional share of the leftover
        // so the table stretches to fill (avoids ragged-right look).
        let leftover = inner_width - sum_max;
        return col_max
            .iter()
            .map(|m| m + leftover * (m / sum_max.max(1e-6)))
            .collect();
    }
    let sum_min: f32 = col_min.iter().sum();
    if sum_min >= inner_width {
        // Even minimums overflow: hand out proportional shares of mins.
        let scale = if sum_min > 0.0 {
            inner_width / sum_min
        } else {
            1.0
        };
        return col_min.iter().map(|m| m * scale).collect();
    }
    let extra = inner_width - sum_min;
    let total_diff: f32 = col_max
        .iter()
        .zip(&col_min)
        .map(|(a, b)| (a - b).max(0.0))
        .sum();
    if total_diff <= 0.0 {
        return vec![inner_width / n as f32; n];
    }
    col_min
        .iter()
        .zip(col_max.iter())
        .map(|(mn, mx)| mn + extra * ((mx - mn).max(0.0) / total_diff))
        .collect()
}

fn sum_block_height(blocks: &[Block]) -> f32 {
    let total: f32 = blocks.iter().map(|b| b.height + b.space_after).sum();
    total - blocks.last().map(|b| b.space_after).unwrap_or(0.0)
}

/// Lay out the contents of one cell. If the cell has only inline content,
/// treat it as a single paragraph (matches markdown table convention).
/// Header cells render with bold text and the configured header colour.
fn layout_cell_content(
    cell: &Tag,
    x: f32,
    width: f32,
    is_header: bool,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<Block> {
    // If there are no block-level children, render directly as a paragraph
    // so we can apply header bold + colour without nesting.
    let any_block_child = cell.children.iter().any(|c| {
        if let RenderableTreeNode::Tag(t) = c {
            matches!(
                t.name.as_str(),
                "p" | "ul"
                    | "ol"
                    | "blockquote"
                    | "pre"
                    | "callout"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "table"
            )
        } else {
            false
        }
    });
    if !any_block_child {
        return layout_table_cell_paragraph(cell, x, width, is_header, ctx);
    }
    layout_children(&cell.children, x, width, ctx)
}

// ── Media (img / media) ─────────────────────────────────────────────────

fn layout_media(tag: &Tag, x: f32, width: f32, ctx: &mut LayoutCtx<'_>) -> Vec<Block> {
    // `<img src=…>` from markdown image syntax, or `{% media src=… %}`
    // Markdoc tag. Both store the URI in `src`.
    let Some(src) = tag.attributes.get("src").and_then(|v| match v {
        Scalar::String(s) => Some(s.clone()),
        _ => None,
    }) else {
        return placeholder(x, width, ctx, "[media: missing src]");
    };

    // Alt text:
    //   - `{% media alt="..." /%}` exposes it as an attribute.
    //   - Markdown `![alt](url)` puts the alt as the Image node's
    //     children (Text scalars), since cmark only models alt-via-children.
    //   Fall through both sources.
    let alt = match tag.attributes.get("alt") {
        Some(Scalar::String(s)) if !s.is_empty() => s.clone(),
        _ => {
            let mut buf = String::new();
            for child in &tag.children {
                if let RenderableTreeNode::Scalar(Scalar::String(s)) = child {
                    buf.push_str(s);
                }
            }
            buf
        }
    };

    let bytes = match ctx.assets.fetch(&src) {
        Ok(b) => b,
        Err(e) => {
            // Loud-on-stderr so a typo'd path doesn't slip past a
            // tech writer iterating locally — the placeholder in the
            // PDF body is also there but easy to miss in a long doc.
            eprintln!("warning: media unavailable: {src} — {e}");
            let msg = if alt.is_empty() {
                format!("[media unavailable: {src} — {e}]")
            } else {
                format!("[{alt}]")
            };
            return placeholder(x, width, ctx, &msg);
        }
    };

    let format = sniff_format(&bytes);
    match format {
        MediaFormat::Png | MediaFormat::Jpeg | MediaFormat::Gif | MediaFormat::Webp => {
            let image = match format {
                MediaFormat::Png => KrillaImage::from_png(bytes.into(), false),
                MediaFormat::Jpeg => KrillaImage::from_jpeg(bytes.into(), false),
                MediaFormat::Gif => KrillaImage::from_gif(bytes.into(), false),
                MediaFormat::Webp => KrillaImage::from_webp(bytes.into(), false),
                _ => unreachable!(),
            };
            let image = match image {
                Ok(img) => img,
                Err(e) => {
                    eprintln!("warning: media decode failed: {src} — {e}");
                    return placeholder(
                        x,
                        width,
                        ctx,
                        &format!("[media decode failed: {src} — {e}]"),
                    );
                }
            };
            let (px_w, px_h) = image.size();
            let (display_w, display_h) = fit_size(px_w as f32, px_h as f32, width);
            let figure_id = ctx.next_figure_id();
            // Explicit `{% caption %}` wins over alt-derived caption.
            let caption = ctx.pending_figure_caption.take().or_else(|| {
                if alt.is_empty() {
                    None
                } else {
                    Some(alt.clone())
                }
            });
            vec![Block {
                height: display_h,
                space_after: ctx.style.paragraph_space_after,
                draw: BlockDraw::Image {
                    image,
                    x,
                    width: display_w,
                    height: display_h,
                    caption,
                },
                outline: None,
                anchor_id: Some(format!("__figure_{figure_id}")),
                tag_role: None,
            }]
        }
        MediaFormat::Svg => {
            // Build a usvg::Tree. The tree carries its own font database;
            // for now we use the default options (no system font lookup
            // for SVG-embedded text). For text-heavy SVGs, callers can
            // pre-process upstream.
            let opts = usvg::Options::default();
            match usvg::Tree::from_data(&bytes, &opts) {
                Ok(tree) => {
                    let size = tree.size();
                    let (display_w, display_h) = fit_size(size.width(), size.height(), width);
                    let figure_id = ctx.next_figure_id();
                    let caption = ctx.pending_figure_caption.take().or_else(|| {
                        if alt.is_empty() {
                            None
                        } else {
                            Some(alt.clone())
                        }
                    });
                    vec![Block {
                        height: display_h,
                        space_after: ctx.style.paragraph_space_after,
                        draw: BlockDraw::Svg {
                            tree: Arc::new(tree),
                            x,
                            width: display_w,
                            height: display_h,
                            caption,
                        },
                        outline: None,
                        anchor_id: Some(format!("__figure_{figure_id}")),
                        tag_role: None,
                    }]
                }
                Err(e) => {
                    eprintln!("warning: svg parse failed: {src} — {e}");
                    placeholder(x, width, ctx, &format!("[svg parse failed: {src} — {e}]"))
                }
            }
        }
        MediaFormat::Unknown => {
            eprintln!(
                "warning: unknown media format: {src} (sniffed bytes don't match png/jpeg/gif/webp/svg)"
            );
            placeholder(x, width, ctx, &format!("[unknown media format: {src}]"))
        }
    }
}

/// Fit `(natural_w, natural_h)` into `available_w` preserving aspect.
/// Never upscales — small images render at their natural size.
fn fit_size(natural_w: f32, natural_h: f32, available_w: f32) -> (f32, f32) {
    if natural_w <= 0.0 || natural_h <= 0.0 {
        return (available_w, available_w * 0.5);
    }
    if natural_w <= available_w {
        return (natural_w, natural_h);
    }
    let scale = available_w / natural_w;
    (available_w, natural_h * scale)
}

/// Render a small text placeholder when we can't display the actual
/// media. Used for missing files, decode failures, unsupported schemes.
fn placeholder(x: f32, width: f32, ctx: &mut LayoutCtx<'_>, message: &str) -> Vec<Block> {
    let style = TextStyle {
        font_size: ctx.style.body_font_size,
        font_weight: 400.0,
        line_height: ctx.style.body_line_height,
        color: ctx.style.blockquote_text_color.into(),
        font_families: ctx.body_families,
        italic: false,
    };
    let text = message.to_string();
    let layout = build_layout(&text, &[], &style, width, ctx.font_cx, ctx.layout_cx);
    let slice = TextSlice::whole(layout, text, Vec::new(), x);
    let height = slice.height();
    vec![Block {
        height,
        space_after: ctx.style.paragraph_space_after,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,

        tag_role: None,
    }]
}

fn layout_table_cell_paragraph(
    cell: &Tag,
    x: f32,
    width: f32,
    is_header: bool,
    ctx: &mut LayoutCtx<'_>,
) -> Vec<Block> {
    // Table cells don't carry document footnotes for v1 — the
    // pagination pool only attaches to top-level body blocks, so
    // routing cell footnotes there could end up on the wrong page.
    let inlines = Inlines::from(&cell.children, &mut Vec::new());
    if inlines.text.trim().is_empty() {
        return Vec::new();
    }
    let (weight, color) = if is_header {
        (700.0, ctx.style.table_header_text_color.into())
    } else {
        (400.0, ctx.style.text_color.into())
    };
    let style = TextStyle {
        font_size: ctx.style.body_font_size,
        font_weight: weight,
        line_height: ctx.style.body_line_height,
        color,
        font_families: ctx.body_families,
        italic: false,
    };
    let layout = build_layout(
        &inlines.text,
        &inlines.style_ranges,
        &style,
        width,
        ctx.font_cx,
        ctx.layout_cx,
    );
    let slice = TextSlice::whole(layout, inlines.text, inlines.links, x);
    let height = slice.height();
    vec![Block {
        height,
        space_after: 0.0,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,

        tag_role: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use markdoc::types::Tag as MdTag;
    use std::collections::HashMap;

    #[test]
    fn heading_counters_nest_and_reset() {
        let mut c = [0u32; 6];
        // h1, h2, h3 walk down.
        assert_eq!(bump_heading_counters(&mut c, 1, 3).as_deref(), Some("1"));
        assert_eq!(bump_heading_counters(&mut c, 2, 3).as_deref(), Some("1.1"));
        assert_eq!(
            bump_heading_counters(&mut c, 3, 3).as_deref(),
            Some("1.1.1")
        );
        // Another h3 increments only the deepest.
        assert_eq!(
            bump_heading_counters(&mut c, 3, 3).as_deref(),
            Some("1.1.2")
        );
        // A new h2 resets the h3 counter.
        assert_eq!(bump_heading_counters(&mut c, 2, 3).as_deref(), Some("1.2"));
        assert_eq!(
            bump_heading_counters(&mut c, 3, 3).as_deref(),
            Some("1.2.1")
        );
        // A new h1 resets h2 and h3.
        assert_eq!(bump_heading_counters(&mut c, 1, 3).as_deref(), Some("2"));
        assert_eq!(bump_heading_counters(&mut c, 2, 3).as_deref(), Some("2.1"));
    }

    #[test]
    fn ordered_marker_sequences_format() {
        use super::super::style::MarkerSequence::*;
        assert_eq!(format_ordered_marker(1, Decimal), "1");
        assert_eq!(format_ordered_marker(42, Decimal), "42");
        // Bijective base-26.
        assert_eq!(format_ordered_marker(1, LowerAlpha), "a");
        assert_eq!(format_ordered_marker(26, LowerAlpha), "z");
        assert_eq!(format_ordered_marker(27, LowerAlpha), "aa");
        assert_eq!(format_ordered_marker(28, LowerAlpha), "ab");
        assert_eq!(format_ordered_marker(2, UpperAlpha), "B");
        // Roman numerals.
        assert_eq!(format_ordered_marker(4, LowerRoman), "iv");
        assert_eq!(format_ordered_marker(9, LowerRoman), "ix");
        assert_eq!(format_ordered_marker(2026, LowerRoman), "mmxxvi");
        assert_eq!(format_ordered_marker(14, UpperRoman), "XIV");
        // Out-of-range roman falls back to decimal.
        assert_eq!(format_ordered_marker(4000, LowerRoman), "4000");
    }

    #[test]
    fn blank_cell_detection_drops_empty_headers() {
        let txt = |s: &str| RenderableTreeNode::Scalar(Scalar::String(s.to_string()));
        let tag = |name: &str, children: Vec<RenderableTreeNode>| {
            RenderableTreeNode::Tag(Box::new(MdTag {
                name: name.to_string(),
                attributes: HashMap::new(),
                children,
            }))
        };
        // Empty / whitespace-only content (directly or nested) is blank —
        // the markdown `| | |` header case.
        assert!(node_text_is_blank(&[]));
        assert!(node_text_is_blank(&[txt("   ")]));
        assert!(node_text_is_blank(&[tag("p", vec![txt("")])]));
        // Visible text — directly or nested — is not blank.
        assert!(!node_text_is_blank(&[txt("Date")]));
        assert!(!node_text_is_blank(&[tag("strong", vec![txt("To:")])]));
        // An image/media tag counts as visible even with no text.
        assert!(!node_text_is_blank(&[tag("img", vec![])]));
    }

    #[test]
    fn heading_numbering_respects_max_depth() {
        let mut c = [0u32; 6];
        assert_eq!(bump_heading_counters(&mut c, 1, 2).as_deref(), Some("1"));
        assert_eq!(bump_heading_counters(&mut c, 2, 2).as_deref(), Some("1.1"));
        // h3 is past max_depth = 2 → no number, and counters untouched.
        assert_eq!(bump_heading_counters(&mut c, 3, 2), None);
        // The next h2 still follows on from 1.1, unaffected by the h3.
        assert_eq!(bump_heading_counters(&mut c, 2, 2).as_deref(), Some("1.2"));
    }

    #[test]
    fn heading_numbering_clamps_and_guards() {
        let mut c = [0u32; 6];
        // level 0 never numbers.
        assert_eq!(bump_heading_counters(&mut c, 0, 3), None);
        // max_depth 0 clamps up to 1 so h1 still numbers.
        assert_eq!(bump_heading_counters(&mut c, 1, 0).as_deref(), Some("1"));
    }

    fn tag_with_attrs(attrs: &[(&str, Scalar)]) -> MdTag {
        let mut m = HashMap::new();
        for (k, v) in attrs {
            m.insert((*k).to_string(), v.clone());
        }
        MdTag {
            name: "tag".to_string(),
            attributes: m,
            children: Vec::new(),
        }
    }

    #[test]
    fn attr_is_false_recognises_falsey_forms() {
        assert!(attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::Boolean(false))]),
            "numbered"
        ));
        assert!(attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::String("false".into()))]),
            "numbered"
        ));
        assert!(attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::String("False".into()))]),
            "numbered"
        ));
        assert!(attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::String("0".into()))]),
            "numbered"
        ));
    }

    #[test]
    fn attr_is_false_rejects_truthy_or_absent() {
        assert!(!attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::Boolean(true))]),
            "numbered"
        ));
        assert!(!attr_is_false(
            &tag_with_attrs(&[("numbered", Scalar::String("true".into()))]),
            "numbered"
        ));
        // Absent attribute → not explicitly false.
        assert!(!attr_is_false(&tag_with_attrs(&[]), "numbered"));
    }
}
