//! Architecture spike for `markdoc-pdf`.
//!
//! Validates two integrations end-to-end:
//!   1. parley (text shaping & line-breaking) -> krilla (glyph emission)
//!   2. usvg + krilla-svg (SVG -> vector PDF content)
//!
//! Modeled on the upstream `crates/krilla/examples/parley.rs` and
//! `crates/krilla-svg/examples/svg.rs` from the krilla repository. If this
//! binary writes a readable `spike.pdf` containing both the paragraph and
//! the SVG callout, the rendering layer of markdoc-pdf is unblocked.
//!
//! Run from the markdoc-pdf directory:
//!     cargo run --example parley_krilla_spike
//!
//! Requires the Noto font family to be discoverable by parley's FontContext.
//! On Fedora/toolbox install: `google-noto-sans-fonts`,
//! `google-noto-sans-arabic-fonts`, `google-noto-sans-devanagari-fonts`,
//! `google-noto-color-emoji-fonts`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path;
use std::sync::Arc;

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{Point, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla_svg::{SurfaceExt, SvgSettings};
use parley::layout::Alignment;
use parley::style::{FontFamily, FontFamilyName, FontWeight, LineHeight, StyleProperty};
use parley::{FontContext, LayoutContext};

// A4 in PDF points. 1 pt = 1/72 inch.
const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 72.0;

const SAMPLE_SVG: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 90">
  <rect x="2" y="2" width="356" height="86" rx="8" ry="8"
        fill="#fff7e6" stroke="#d97706" stroke-width="2"/>
  <circle cx="32" cy="45" r="16" fill="#d97706"/>
  <text x="32" y="52" font-family="sans-serif" font-size="22"
        text-anchor="middle" fill="#fff" font-weight="bold">!</text>
  <text x="64" y="40" font-family="sans-serif" font-size="14"
        font-weight="bold" fill="#92400e">Warning</text>
  <text x="64" y="62" font-family="sans-serif" font-size="11" fill="#92400e">
    Sample SVG callout, drawn as native PDF vectors via krilla-svg.
  </text>
</svg>"##;

fn main() {
    let pdf = build_pdf();
    let out = path::absolute("spike.pdf").unwrap();
    std::fs::write(&out, &pdf).unwrap();
    eprintln!("Wrote {} ({} bytes)", out.display(), pdf.len());
}

fn build_pdf() -> Vec<u8> {
    // 1. Lay out a paragraph with parley.
    let text = String::from(
        "Markdoc -> PDF spike. The text wraps to fit within the body column. \
         Mixed scripts must shape correctly: हैलो वर्ल्ड and مرحبا بالعالم. \
         Emoji should also work: 🦀 🚀 ✨.",
    );
    let max_advance = Some(PAGE_WIDTH - 2.0 * MARGIN);

    let mut font_cx = FontContext::default();
    let mut layout_cx = LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, &text, 1.0, false);
    builder.push_default(StyleProperty::Brush(rgb::Color::new(20, 20, 20)));
    builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Borrowed(
        &[
            FontFamilyName::Named(Cow::Borrowed("Noto Sans")),
            FontFamilyName::Named(Cow::Borrowed("Noto Sans Arabic")),
            FontFamilyName::Named(Cow::Borrowed("Noto Sans Devanagari")),
            FontFamilyName::Named(Cow::Borrowed("Noto Color Emoji")),
        ],
    ))));
    builder.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.4)));
    builder.push_default(StyleProperty::FontSize(12.0));
    // Bold the lead-in so per-range styling is exercised in the same run.
    builder.push(StyleProperty::FontWeight(FontWeight::new(700.0)), 0..21);

    let mut layout = builder.build(&text);
    layout.break_all_lines(max_advance);
    layout.align(max_advance, Alignment::Start, Default::default());

    // 2. Open a krilla document with one A4 page.
    let mut document = Document::new();
    let mut page =
        document.start_page_with(PageSettings::from_wh(PAGE_WIDTH, PAGE_HEIGHT).unwrap());
    let mut surface = page.surface();

    // 3. Walk parley's lines/runs/clusters/glyphs and emit krilla glyph runs.
    //    Each surface.draw_glyphs call carries one fill, so glyphs are flushed
    //    whenever the per-glyph style index changes within a run.
    let mut font_cache = HashMap::new();
    let text_top = MARGIN;

    for line in layout.lines() {
        let y = text_top + line.metrics().baseline;
        let mut x = MARGIN;
        for run in line.runs() {
            let mut cur_x = x;
            let font = run.font().clone();
            let (font_data, id) = font.data.into_raw_parts();
            let krilla_font = font_cache
                .entry(id)
                .or_insert_with(|| Font::new(font_data.into(), font.index).unwrap());
            let font_size = run.font_size();

            let mut cur_style: Option<u16> = None;
            let mut glyphs = Vec::<KrillaGlyph>::new();

            for cluster in run.visual_clusters() {
                if cluster.is_ligature_continuation() {
                    if let Some(g) = glyphs.last_mut() {
                        g.text_range.end = cluster.text_range().end;
                    }
                    continue;
                }
                for glyph in cluster.glyphs() {
                    let glyph_style = glyph.style_index;
                    if let Some(prev) = cur_style {
                        if prev != glyph_style {
                            // Flush the run-so-far before switching styles.
                            cur_style = Some(glyph_style);
                            surface.set_fill(Some(Fill {
                                paint: layout.styles()[prev as usize].brush.into(),
                                opacity: NormalizedF32::ONE,
                                rule: Default::default(),
                            }));
                            surface.draw_glyphs(
                                Point::from_xy(cur_x, y),
                                &glyphs,
                                krilla_font.clone(),
                                &text,
                                font_size,
                                false,
                            );
                            glyphs.clear();
                            cur_x = x;
                        }
                    } else {
                        cur_style = Some(glyph_style);
                    }
                    glyphs.push(KrillaGlyph::new(
                        GlyphId::new(glyph.id),
                        glyph.advance / font_size,
                        glyph.x / font_size,
                        glyph.y / font_size,
                        0.0,
                        cluster.text_range(),
                        None,
                    ));
                    x += glyph.advance;
                }
            }

            if !glyphs.is_empty() {
                surface.set_fill(Some(Fill {
                    paint: layout.styles()[cur_style.unwrap() as usize].brush.into(),
                    opacity: NormalizedF32::ONE,
                    rule: Default::default(),
                }));
                surface.draw_glyphs(
                    Point::from_xy(cur_x, y),
                    &glyphs,
                    krilla_font.clone(),
                    &text,
                    font_size,
                    false,
                );
            }
        }
    }

    // 4. Embed an SVG callout below the paragraph as native PDF vectors.
    let svg_tree = {
        let mut fontdb = fontdb::Database::new();
        fontdb.load_system_fonts();
        let opts = usvg::Options {
            fontdb: Arc::new(fontdb),
            ..Default::default()
        };
        usvg::Tree::from_str(SAMPLE_SVG, &opts).unwrap()
    };
    let svg_size = Size::from_wh(svg_tree.size().width(), svg_tree.size().height()).unwrap();

    surface.push_transform(&Transform::from_translate(MARGIN, text_top + 200.0));
    surface.draw_svg(&svg_tree, svg_size, SvgSettings::default());
    surface.pop();

    // 5. Finalise.
    surface.finish();
    page.finish();
    document.finish().unwrap()
}
