//! End-to-end coverage for per-element font selection.
//!
//! `src/render/fonts.rs` unit-tests the *resolution* rules (what
//! inherits from what). This file tests the other half: that each
//! layout site actually consults the resolved list, so the font a
//! style asks for reaches the PDF.
//!
//! That distinction matters — the bug this suite exists to prevent was
//! precisely a correct resolution that four layout sites ignored, and
//! no amount of unit-testing `Fonts` would have caught it.
//!
//! ## Why the fonts are discovered rather than hard-coded
//!
//! Nothing is vendored; every family resolves from the system, and
//! `fontique` silently skips families it can't find. A test naming
//! "Liberation Serif" would therefore pass on Fedora and quietly
//! degrade to a meaningless tautology on a runner without it. So the
//! suite first probes for two visually distinct families that ARE
//! installed and uses those. If it can't find two, it fails with an
//! actionable message rather than a confusing assertion diff.

use markdoc::types::Config;
use markdoc::{parse, transform};
use markdoc_pdf::assets::NullAssetResolver;
use markdoc_pdf::render::{RenderContext, Style, render_pdf_with};

/// Render `source` with `style_toml` and return the PDF bytes.
fn render(source: &str, style_toml: &str) -> Vec<u8> {
    let doc = parse(source, None).expect("test source should parse");
    let rendered = transform(&doc, &Config::default()).expect("test source should transform");
    let style = Style::from_toml_str(style_toml).expect("test style should parse");
    render_pdf_with(
        &rendered,
        &style,
        &NullAssetResolver,
        &RenderContext {
            title: "Font Test".into(),
            ..Default::default()
        },
    )
    .expect("render should succeed")
}

/// PDF font names drop spaces: "Liberation Serif" is written into the
/// file as `…+LiberationSerif`. A family that never resolved leaves no
/// trace at all, which is what makes this a usable probe.
fn embeds_font(pdf: &[u8], family: &str) -> bool {
    let needle = family.replace(' ', "");
    pdf.windows(needle.len()).any(|w| w == needle.as_bytes())
}

const PROBE_DOC: &str = "Probe text.\n";

/// Is `family` installed and reachable by the renderer?
fn is_installed(family: &str) -> bool {
    let style = format!("body_font_families = [\"{family}\"]\n");
    embeds_font(&render(PROBE_DOC, &style), family)
}

/// Two distinct installed families, or a clear failure. Candidates are
/// ordered by how widely they ship on Linux CI images; each pair is
/// visually distinguishable so a human can eyeball the output too.
fn two_installed_families() -> (String, String) {
    const CANDIDATES: &[&str] = &[
        "Liberation Serif",
        "Liberation Sans",
        "Liberation Mono",
        "DejaVu Serif",
        "DejaVu Sans",
        "DejaVu Sans Mono",
        "Noto Serif",
        "Noto Sans",
        "FreeSerif",
        "FreeSans",
    ];
    let found: Vec<String> = CANDIDATES
        .iter()
        .filter(|f| is_installed(f))
        .map(|f| f.to_string())
        .take(2)
        .collect();
    assert!(
        found.len() == 2,
        "per-element font tests need two distinct installed font families to \
         tell elements apart, but found {} of: {CANDIDATES:?}. \
         Install e.g. liberation-fonts or fonts-dejavu and re-run.",
        found.len()
    );
    (found[0].clone(), found[1].clone())
}

#[test]
fn body_font_reaches_the_pdf() {
    let (body, _) = two_installed_families();
    let pdf = render(PROBE_DOC, &format!("body_font_families = [\"{body}\"]\n"));
    assert!(
        embeds_font(&pdf, &body),
        "body_font_families = [{body:?}] should be embedded"
    );
}

#[test]
fn heading_font_overrides_body() {
    let (body, heading) = two_installed_families();
    let pdf = render(
        "# A Heading\n\nBody text.\n",
        &format!(
            "body_font_families = [\"{body}\"]\n\n[heading.h1]\nfont_families = [\"{heading}\"]\n"
        ),
    );
    assert!(embeds_font(&pdf, &body), "body should keep {body:?}");
    assert!(
        embeds_font(&pdf, &heading),
        "h1 should use its own {heading:?}, not inherit the body font"
    );
}

/// The original defect: `decoration.rs` hardcoded the bundled default
/// list, so the header and footer ignored the document font entirely.
#[test]
fn header_and_footer_follow_the_body_font() {
    let (body, _) = two_installed_families();
    let pdf = render(
        PROBE_DOC,
        &format!(
            "body_font_families = [\"{body}\"]\n\n\
             [page_decoration.header]\nleft = \"HEADER\"\n\n\
             [page_decoration.footer]\nleft = \"FOOTER\"\n"
        ),
    );
    assert!(
        embeds_font(&pdf, &body),
        "header/footer should use {body:?}"
    );
    // The bundled default leaked into the output when the bug was live:
    // the header drew in Noto Sans while the body drew in the configured
    // face. Guard the specific symptom, not just the happy path.
    assert!(
        !embeds_font(&pdf, "NotoSans") || body.replace(' ', "") == "NotoSans",
        "header/footer must not fall back to the bundled Noto Sans when \
         body_font_families is set (this was the reported bug)"
    );
}

#[test]
fn header_can_override_the_body_font() {
    let (body, header) = two_installed_families();
    let pdf = render(
        PROBE_DOC,
        &format!(
            "body_font_families = [\"{body}\"]\n\n\
             [page_decoration.header]\nleft = \"HEADER\"\nfont_families = [\"{header}\"]\n"
        ),
    );
    assert!(embeds_font(&pdf, &body), "body should keep {body:?}");
    assert!(
        embeds_font(&pdf, &header),
        "header should use its own {header:?}"
    );
}

/// The second defect: `code_font_family` was accepted, documented, and
/// then discarded (`let _ = primary;`).
#[test]
fn code_font_family_reaches_code_blocks() {
    let (body, code) = two_installed_families();
    let pdf = render(
        "Body text.\n\n```\nfenced code\n```\n",
        &format!("body_font_families = [\"{body}\"]\ncode_font_family = \"{code}\"\n"),
    );
    assert!(embeds_font(&pdf, &body), "prose should use {body:?}");
    assert!(
        embeds_font(&pdf, &code),
        "code_font_family = {code:?} must reach fenced code blocks"
    );
}

/// Inline code had the same bug independently — it hardcoded
/// "Noto Sans Mono" rather than reading the configured family.
#[test]
fn code_font_family_reaches_inline_code() {
    let (body, code) = two_installed_families();
    let pdf = render(
        "Prose with `inline code` inside.\n",
        &format!("body_font_families = [\"{body}\"]\ncode_font_family = \"{code}\"\n"),
    );
    assert!(
        embeds_font(&pdf, &code),
        "code_font_family = {code:?} must reach inline code too"
    );
}

/// A style that overrides nothing must still render — the inheritance
/// chain bottoms out in the bundled defaults.
#[test]
fn empty_style_still_renders() {
    let pdf = render("# Heading\n\nBody.\n", "");
    assert!(pdf.starts_with(b"%PDF-"), "should produce a valid PDF");
}

/// Unknown style keys are rejected, so a typo is loud instead of
/// silently doing nothing.
#[test]
fn misspelled_font_key_is_rejected() {
    let err = Style::from_toml_str("[heading.h1]\nfont_familes = [\"X\"]\n")
        .expect_err("a misspelled key must not be silently ignored");
    let msg = err.to_string();
    assert!(
        msg.contains("font_familes"),
        "error should name the offending key, got: {msg}"
    );
    assert!(
        msg.contains("font_families"),
        "error should suggest the valid key, got: {msg}"
    );
}
