//! Synthesise a cover page from the document's frontmatter.
//!
//! Markdoc source documents stay output-agnostic — they do NOT carry
//! tags like `{% titlepage %}`. Instead the renderer materialises a
//! cover page from style configuration plus the data already on
//! `RenderContext` (title, description, authors, creation date). The
//! resulting block list is prepended to the body and ends with a
//! `PageBreak` so the first body block lands on page 2.
//!
//! Horizontal alignment is configurable (centred or left); vertical
//! position is controlled by the configured `top_margin`. Stays fully
//! accessible under PDF/UA — every text block is a normal `P`/`Hn` and
//! the logo sits inside a `Figure` group.

use std::sync::Arc;

use krilla::color::rgb;
use krilla::image::Image as KrillaImage;
use parley::layout::Alignment;
use parley::{FontContext, LayoutContext};
use usvg::Tree as SvgTree;

use crate::assets::{AssetResolver, MediaFormat, sniff_format};

use super::RenderContext;
use super::block::{Block, BlockDraw, TextSlice};
use super::style::{CoverAlign, LogoPosition, Style};
use super::text::{TextStyle, build_layout_aligned};

/// Build the synthesised cover-page block list. Returns an empty
/// `Vec` when the cover page is disabled so the caller can simply
/// concatenate without a feature check.
#[allow(clippy::too_many_arguments)]
pub fn build_coverpage_blocks(
    style: &Style,
    render_ctx: &RenderContext,
    body_families: &'static [&'static str],
    assets: &dyn AssetResolver,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
    date_str: &str,
) -> Vec<Block> {
    let coverpage = &style.coverpage;
    if !coverpage.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    let body_left = style.margin_x;
    let column_w = style.page_width - 2.0 * style.margin_x;
    let align = cover_alignment(coverpage.align);

    // Top spacer.
    if coverpage.top_margin > 0.0 {
        out.push(spacer_block(body_left, coverpage.top_margin));
    }

    // Optional logo / hero image (best-effort — silently skipped on
    // decode failure). Decoded once here so we can place it either
    // above the title or between title and subtitle without
    // duplicating the asset-resolver code.
    let logo_block = coverpage
        .logo
        .as_ref()
        .and_then(|logo| build_logo_block(logo, body_left, column_w, coverpage.align, assets));

    // Logo above the title (default).
    if coverpage.logo_position == LogoPosition::Above
        && let Some(block) = logo_block.clone()
    {
        out.push(block);
        if coverpage.logo_to_title_gap > 0.0 {
            out.push(spacer_block(body_left, coverpage.logo_to_title_gap));
        }
    }

    // Title.
    if !render_ctx.title.is_empty() {
        out.push(cover_text_block(
            &render_ctx.title,
            body_left,
            column_w,
            coverpage.title_font_size,
            700.0,
            coverpage.text_color.into(),
            align,
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Detail lines (e.g. "Date: {date}", "Version: {version}"). Each is
    // a template; lines that substitute to nothing are skipped.
    let detail_color = coverpage
        .detail_color
        .unwrap_or(coverpage.text_color)
        .into();
    let mut first_detail = true;
    for line_tpl in &coverpage.detail_lines {
        let line = substitute(line_tpl, render_ctx, date_str);
        if line.trim().is_empty() {
            continue;
        }
        let gap = if first_detail {
            coverpage.title_to_detail_gap
        } else {
            coverpage.detail_line_gap
        };
        if gap > 0.0 {
            out.push(spacer_block(body_left, gap));
        }
        first_detail = false;
        out.push(cover_text_block(
            &line,
            body_left,
            column_w,
            coverpage.detail_font_size,
            400.0,
            detail_color,
            align,
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Logo below the title (hero-image variant).
    if coverpage.logo_position == LogoPosition::BelowTitle
        && let Some(block) = logo_block
    {
        if coverpage.logo_to_title_gap > 0.0 {
            out.push(spacer_block(body_left, coverpage.logo_to_title_gap));
        }
        out.push(block);
    }

    // Subtitle.
    let subtitle = substitute(&coverpage.subtitle, render_ctx, date_str);
    if !subtitle.trim().is_empty() {
        if coverpage.title_to_subtitle_gap > 0.0 {
            out.push(spacer_block(body_left, coverpage.title_to_subtitle_gap));
        }
        out.push(cover_text_block(
            &subtitle,
            body_left,
            column_w,
            coverpage.subtitle_font_size,
            400.0,
            coverpage.text_color.into(),
            align,
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Authors.
    if coverpage.show_authors && !render_ctx.authors.is_empty() {
        if coverpage.subtitle_to_authors_gap > 0.0 {
            out.push(spacer_block(body_left, coverpage.subtitle_to_authors_gap));
        }
        let authors = render_ctx.authors.join(", ");
        out.push(cover_text_block(
            &authors,
            body_left,
            column_w,
            coverpage.authors_font_size,
            400.0,
            coverpage.text_color.into(),
            align,
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Date.
    if coverpage.show_date && !date_str.is_empty() {
        if coverpage.authors_to_date_gap > 0.0 {
            out.push(spacer_block(body_left, coverpage.authors_to_date_gap));
        }
        out.push(cover_text_block(
            date_str,
            body_left,
            column_w,
            coverpage.date_font_size,
            400.0,
            coverpage.text_color.into(),
            align,
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Page break — flushes the cover page.
    out.push(Block {
        height: 0.0,
        space_after: 0.0,
        draw: BlockDraw::PageBreak,
        outline: None,
        anchor_id: None,
        tag_role: None,
    });

    // Optional blank verso — for double-sided printing the body
    // typically wants to start on a recto (right-hand) page. The
    // paginator dedupes consecutive PageBreaks, so we need a
    // near-zero-height spacer between the two breaks to satisfy its
    // "current page must be non-empty" check. The spacer renders
    // nothing visible, so the resulting page is genuinely blank.
    if coverpage.blank_page_after {
        out.push(spacer_block(body_left, 0.001));
        out.push(Block {
            height: 0.0,
            space_after: 0.0,
            draw: BlockDraw::PageBreak,
            outline: None,
            anchor_id: None,
            tag_role: None,
        });
    }

    out
}

/// Subset of the header/footer template variables that make sense at
/// layout time — `{page}` / `{total}` / `{chapter}` / `{section}`
/// aren't known until pagination, so they pass through unchanged.
fn substitute(template: &str, ctx: &RenderContext, date_str: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            name.push(next);
        }
        if !closed {
            out.push('{');
            out.push_str(&name);
            continue;
        }
        match name.as_str() {
            "title" => out.push_str(&ctx.title),
            "description" => out.push_str(ctx.description.as_deref().unwrap_or("")),
            "date" => out.push_str(date_str),
            other => match ctx.vars.get(other) {
                Some(v) => out.push_str(v),
                None => {
                    out.push('{');
                    out.push_str(&name);
                    out.push('}');
                }
            },
        }
    }
    out
}

fn spacer_block(x: f32, height: f32) -> Block {
    Block {
        height,
        space_after: 0.0,
        draw: BlockDraw::Rule {
            x,
            width: 0.0,
            thickness: 0.0,
            color: krilla::color::rgb::Color::new(0, 0, 0),
        },
        outline: None,
        anchor_id: None,
        tag_role: None,
    }
}

/// Map the style's cover alignment onto a parley alignment.
fn cover_alignment(align: CoverAlign) -> Alignment {
    match align {
        CoverAlign::Center => Alignment::Center,
        CoverAlign::Left => Alignment::Start,
    }
}

#[allow(clippy::too_many_arguments)]
fn cover_text_block(
    text: &str,
    body_left: f32,
    column_w: f32,
    font_size: f32,
    font_weight: f32,
    color: rgb::Color,
    align: Alignment,
    body_families: &'static [&'static str],
    line_height: f32,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
) -> Block {
    let style = TextStyle {
        font_size,
        font_weight,
        line_height,
        color,
        font_families: body_families,
        italic: false,
    };
    let layout = build_layout_aligned(text, &[], &style, column_w, align, font_cx, layout_cx);
    let slice = TextSlice::whole(layout, text.to_string(), Vec::new(), body_left);
    let height = slice.height();
    Block {
        height,
        space_after: 0.0,
        draw: BlockDraw::Text(slice),
        outline: None,
        anchor_id: None,
        tag_role: None,
    }
}

/// Decode the configured logo via the asset resolver and return a
/// centred Image (raster) or Svg block. Width/height come from the
/// `LogoSpec`; horizontal position is centred in the body column.
fn build_logo_block(
    logo: &super::style::LogoSpec,
    body_left: f32,
    column_w: f32,
    align: CoverAlign,
    assets: &dyn AssetResolver,
) -> Option<Block> {
    if logo.src.is_empty() || logo.width <= 0.0 || logo.height <= 0.0 {
        return None;
    }
    let bytes = assets.fetch(&logo.src).ok()?;
    let format = sniff_format(&bytes);
    // Match the cover's text alignment: flush-left for a left title page,
    // centred otherwise.
    let x = match align {
        CoverAlign::Left => body_left,
        CoverAlign::Center => body_left + (column_w - logo.width).max(0.0) * 0.5,
    };
    let block = match format {
        MediaFormat::Png | MediaFormat::Jpeg | MediaFormat::Gif | MediaFormat::Webp => {
            let image = match format {
                MediaFormat::Png => KrillaImage::from_png(bytes.into(), false).ok()?,
                MediaFormat::Jpeg => KrillaImage::from_jpeg(bytes.into(), false).ok()?,
                MediaFormat::Gif => KrillaImage::from_gif(bytes.into(), false).ok()?,
                MediaFormat::Webp => KrillaImage::from_webp(bytes.into(), false).ok()?,
                _ => unreachable!(),
            };
            Block {
                height: logo.height,
                space_after: 0.0,
                draw: BlockDraw::Image {
                    image,
                    x,
                    width: logo.width,
                    height: logo.height,
                    caption: None,
                },
                outline: None,
                anchor_id: None,
                tag_role: None,
            }
        }
        MediaFormat::Svg => {
            let opts = usvg::Options::default();
            let tree = SvgTree::from_data(&bytes, &opts).ok()?;
            Block {
                height: logo.height,
                space_after: 0.0,
                draw: BlockDraw::Svg {
                    tree: Arc::new(tree),
                    x,
                    width: logo.width,
                    height: logo.height,
                    caption: None,
                },
                outline: None,
                anchor_id: None,
                tag_role: None,
            }
        }
        _ => return None,
    };
    Some(block)
}
