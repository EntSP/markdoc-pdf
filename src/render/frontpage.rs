//! Synthesise a cover/title page from the document's frontmatter.
//!
//! Markdoc source documents stay output-agnostic — they do NOT carry
//! tags like `{% titlepage %}`. Instead the renderer materialises a
//! cover page from style configuration plus the data already on
//! `RenderContext` (title, description, authors, creation date). The
//! resulting block list is prepended to the body and ends with a
//! `PageBreak` so the first body block lands on page 2.
//!
//! Centred horizontally via parley alignment; vertical position is
//! controlled by the configured `top_margin`. Stays fully accessible
//! under PDF/UA — every text block is a normal `P`/`Hn` and the logo
//! sits inside a `Figure` group.

use std::sync::Arc;

use krilla::color::rgb;
use krilla::image::Image as KrillaImage;
use parley::layout::Alignment;
use parley::{FontContext, LayoutContext};
use usvg::Tree as SvgTree;

use crate::assets::{AssetResolver, MediaFormat, sniff_format};

use super::RenderContext;
use super::block::{Block, BlockDraw, TextSlice};
use super::style::Style;
use super::text::{TextStyle, build_layout_aligned};

/// Build the synthesised cover-page block list. Returns an empty
/// `Vec` when the front page is disabled so the caller can simply
/// concatenate without a feature check.
#[allow(clippy::too_many_arguments)]
pub fn build_frontpage_blocks(
    style: &Style,
    render_ctx: &RenderContext,
    body_families: &'static [&'static str],
    assets: &dyn AssetResolver,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<rgb::Color>,
    date_str: &str,
) -> Vec<Block> {
    let frontpage = &style.frontpage;
    if !frontpage.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    let body_left = style.margin_x;
    let column_w = style.page_width - 2.0 * style.margin_x;

    // Top spacer.
    if frontpage.top_margin > 0.0 {
        out.push(spacer_block(body_left, frontpage.top_margin));
    }

    // Logo (best-effort — silently skipped on decode failure).
    if let Some(logo) = &frontpage.logo
        && let Some(block) = build_logo_block(logo, body_left, column_w, assets)
    {
        out.push(block);
        if frontpage.logo_to_title_gap > 0.0 {
            out.push(spacer_block(body_left, frontpage.logo_to_title_gap));
        }
    }

    // Title.
    if !render_ctx.title.is_empty() {
        out.push(centered_text_block(
            &render_ctx.title,
            body_left,
            column_w,
            frontpage.title_font_size,
            700.0,
            frontpage.text_color.into(),
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Subtitle.
    let subtitle = substitute(&frontpage.subtitle, render_ctx, date_str);
    if !subtitle.trim().is_empty() {
        if frontpage.title_to_subtitle_gap > 0.0 {
            out.push(spacer_block(body_left, frontpage.title_to_subtitle_gap));
        }
        out.push(centered_text_block(
            &subtitle,
            body_left,
            column_w,
            frontpage.subtitle_font_size,
            400.0,
            frontpage.text_color.into(),
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Authors.
    if frontpage.show_authors && !render_ctx.authors.is_empty() {
        if frontpage.subtitle_to_authors_gap > 0.0 {
            out.push(spacer_block(body_left, frontpage.subtitle_to_authors_gap));
        }
        let authors = render_ctx.authors.join(", ");
        out.push(centered_text_block(
            &authors,
            body_left,
            column_w,
            frontpage.authors_font_size,
            400.0,
            frontpage.text_color.into(),
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Date.
    if frontpage.show_date && !date_str.is_empty() {
        if frontpage.authors_to_date_gap > 0.0 {
            out.push(spacer_block(body_left, frontpage.authors_to_date_gap));
        }
        out.push(centered_text_block(
            date_str,
            body_left,
            column_w,
            frontpage.date_font_size,
            400.0,
            frontpage.text_color.into(),
            body_families,
            style.body_line_height,
            font_cx,
            layout_cx,
        ));
    }

    // Page break — flushes the cover page and starts body on page 2.
    out.push(Block {
        height: 0.0,
        space_after: 0.0,
        draw: BlockDraw::PageBreak,
        outline: None,
        anchor_id: None,
        tag_role: None,
    });

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
            _ => {
                out.push('{');
                out.push_str(&name);
                out.push('}');
            }
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

#[allow(clippy::too_many_arguments)]
fn centered_text_block(
    text: &str,
    body_left: f32,
    column_w: f32,
    font_size: f32,
    font_weight: f32,
    color: rgb::Color,
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
    let layout = build_layout_aligned(
        text,
        &[],
        &style,
        column_w,
        Alignment::Center,
        font_cx,
        layout_cx,
    );
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
    assets: &dyn AssetResolver,
) -> Option<Block> {
    if logo.src.is_empty() || logo.width <= 0.0 || logo.height <= 0.0 {
        return None;
    }
    let bytes = assets.fetch(&logo.src).ok()?;
    let format = sniff_format(&bytes);
    let x = body_left + (column_w - logo.width).max(0.0) * 0.5;
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
