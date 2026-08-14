//! Per-element font-family resolution.
//!
//! Every text-bearing style struct carries its own `font_families`
//! list. An empty list means "inherit the document body families",
//! which in turn fall back to the bundled Noto set. Resolving that
//! chain once per render — rather than at every layout site — keeps
//! the hot path a slice copy and gives one place to reason about
//! inheritance.
//!
//! ## On leaking
//!
//! `TextStyle::font_families` is `&[&'static str]`, so a resolved
//! list has to outlive the layout pass. We leak, exactly as the
//! previous single-list code did — but only for elements the style
//! actually customises. An element with an empty list shares the body
//! slice and allocates nothing, so a document that overrides no fonts
//! leaks precisely what it did before this module existed.

use super::style::{CalloutStyle, Style, WatermarkKind};
use super::text::{default_families, monospace_families};

/// Resolved family lists, one per element that can carry its own.
pub struct Fonts {
    pub body: &'static [&'static str],
    /// Code blocks and inline code, led by `code_font_family`.
    pub code: &'static [&'static str],
    /// h1..h6, indexed by `level - 1`.
    headings: [&'static [&'static str]; 6],
    /// Per callout kind, in `CalloutStyles` declaration order:
    /// note, info, warning, caution, danger, success, notice.
    callouts: [(&'static str, &'static [&'static str]); 7],
    pub list_marker: &'static [&'static str],
    pub footnote: &'static [&'static str],
    pub toc: &'static [&'static str],
    pub lof: &'static [&'static str],
    pub lot: &'static [&'static str],
    pub header: &'static [&'static str],
    pub footer: &'static [&'static str],
    pub watermark: &'static [&'static str],
    pub qr: &'static [&'static str],
    pub notice_banner: &'static [&'static str],
    pub coverpage: &'static [&'static str],
}

/// Resolve one element's list against an inherited default. An empty
/// list inherits — and shares the parent's slice rather than cloning
/// it, so the common "no override" case costs nothing.
fn resolve(own: &[String], inherit: &'static [&'static str]) -> &'static [&'static str] {
    if own.is_empty() {
        return inherit;
    }
    let leaked: Vec<&'static str> = own
        .iter()
        .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
        .collect();
    Box::leak(leaked.into_boxed_slice())
}

/// Resolve through an `Option<T>` container — an absent header /
/// watermark / banner simply inherits.
fn resolve_opt<T>(
    opt: Option<&T>,
    pick: impl Fn(&T) -> &[String],
    inherit: &'static [&'static str],
) -> &'static [&'static str] {
    match opt {
        Some(t) => resolve(pick(t), inherit),
        None => inherit,
    }
}

impl Fonts {
    /// Resolve every element's families from `style`. Call once per
    /// render, before layout.
    pub fn resolve(style: &Style) -> Self {
        // The document-wide default sits at the root of every chain.
        let body = resolve(&style.body_font_families, default_families());
        let h = &style.heading;
        let c = &style.callout_styles;
        let pd = &style.page_decoration;

        let callout = |cs: &CalloutStyle| resolve(&cs.font_families, body);

        Self {
            body,
            code: Box::leak(monospace_families(&style.code_font_family).into_boxed_slice()),
            headings: [
                resolve(&h.h1.font_families, body),
                resolve(&h.h2.font_families, body),
                resolve(&h.h3.font_families, body),
                resolve(&h.h4.font_families, body),
                resolve(&h.h5.font_families, body),
                resolve(&h.h6.font_families, body),
            ],
            callouts: [
                ("note", callout(&c.note)),
                ("info", callout(&c.info)),
                ("warning", callout(&c.warning)),
                ("caution", callout(&c.caution)),
                ("danger", callout(&c.danger)),
                ("success", callout(&c.success)),
                ("notice", callout(&c.notice)),
            ],
            list_marker: resolve(&style.list_marker.font_families, body),
            footnote: resolve(&style.footnote.font_families, body),
            toc: resolve(&style.toc.font_families, body),
            lof: resolve(&style.lof.font_families, body),
            lot: resolve(&style.lot.font_families, body),
            header: resolve_opt(pd.header.as_ref(), |s| &s.font_families, body),
            footer: resolve_opt(pd.footer.as_ref(), |s| &s.font_families, body),
            qr: resolve_opt(pd.last_page_qr.as_ref(), |s| &s.font_families, body),
            notice_banner: resolve_opt(pd.banner.as_ref(), |s| &s.font_families, body),
            watermark: match style.watermark.as_ref().map(|w| &w.kind) {
                Some(WatermarkKind::Text(t)) => resolve(&t.font_families, body),
                // An image watermark draws no text; the slice is never
                // read, but keep it valid rather than empty.
                _ => body,
            },
            coverpage: resolve(&style.coverpage.font_families, body),
        }
    }

    /// Families for a heading level (1-based). Levels outside 1..=6
    /// fall back to h6, matching how the renderer clamps deep nesting.
    pub fn heading(&self, level: u8) -> &'static [&'static str] {
        let idx = (level.clamp(1, 6) - 1) as usize;
        self.headings[idx]
    }

    /// Families for a callout kind. Unknown kinds fall back to
    /// `note`, exactly as `CalloutStyles::for_kind` does — so a
    /// callout's font and its colours can never come from different
    /// kinds.
    pub fn callout(&self, kind: &str) -> &'static [&'static str] {
        self.callouts
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, f)| *f)
            .unwrap_or(self.callouts[0].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled(f: impl FnOnce(&mut Style)) -> Fonts {
        let mut s = Style::default();
        f(&mut s);
        Fonts::resolve(&s)
    }

    #[test]
    fn empty_style_inherits_bundled_defaults_everywhere() {
        let fonts = styled(|_| {});
        let d = default_families();
        assert_eq!(fonts.body, d);
        assert_eq!(fonts.heading(1), d);
        assert_eq!(fonts.toc, d);
        assert_eq!(fonts.header, d);
        assert_eq!(fonts.footer, d);
        assert_eq!(fonts.callout("warning"), d);
    }

    #[test]
    fn elements_inherit_body_when_they_specify_nothing() {
        let fonts = styled(|s| s.body_font_families = vec!["Fake Body".into()]);
        assert_eq!(fonts.body, ["Fake Body"]);
        // Every element that didn't opt out follows the body — this is
        // the bug the tech writer hit: header/footer used to ignore it.
        for (label, got) in [
            ("h1", fonts.heading(1)),
            ("h6", fonts.heading(6)),
            ("toc", fonts.toc),
            ("lof", fonts.lof),
            ("lot", fonts.lot),
            ("footnote", fonts.footnote),
            ("list_marker", fonts.list_marker),
            ("coverpage", fonts.coverpage),
            ("callout", fonts.callout("note")),
        ] {
            assert_eq!(
                got,
                ["Fake Body"],
                "{label} should inherit the body families"
            );
        }
    }

    #[test]
    fn per_element_override_beats_body() {
        let fonts = styled(|s| {
            s.body_font_families = vec!["Fake Body".into()];
            s.heading.h2.font_families = vec!["Fake Heading".into()];
        });
        assert_eq!(fonts.heading(2), ["Fake Heading"]);
        // Siblings are unaffected.
        assert_eq!(fonts.heading(1), ["Fake Body"]);
        assert_eq!(fonts.body, ["Fake Body"]);
    }

    #[test]
    fn heading_level_clamps_outside_one_to_six() {
        let fonts = styled(|s| {
            s.heading.h1.font_families = vec!["First".into()];
            s.heading.h6.font_families = vec!["Last".into()];
        });
        assert_eq!(fonts.heading(0), ["First"], "level 0 clamps up to h1");
        assert_eq!(fonts.heading(9), ["Last"], "deep nesting clamps down to h6");
    }

    #[test]
    fn unknown_callout_kind_follows_note_not_body() {
        let fonts = styled(|s| {
            s.body_font_families = vec!["Fake Body".into()];
            s.callout_styles.note.font_families = vec!["Fake Note".into()];
        });
        // `CalloutStyles::for_kind` sends unknown kinds to `note`, so the
        // font must go the same way or a callout renders with one kind's
        // colours and another's typeface.
        assert_eq!(fonts.callout("no-such-kind"), ["Fake Note"]);
        assert_eq!(fonts.callout("note"), ["Fake Note"]);
        assert_eq!(fonts.callout("danger"), ["Fake Body"]);
    }

    #[test]
    fn code_leads_with_configured_family_then_bundled_fallbacks() {
        let fonts = styled(|s| s.code_font_family = "Fake Mono".into());
        assert_eq!(fonts.code, ["Fake Mono", "Noto Sans Mono", "Noto Sans"]);
    }

    #[test]
    fn code_does_not_duplicate_the_bundled_default() {
        let fonts = styled(|s| s.code_font_family = "Noto Sans Mono".into());
        assert_eq!(fonts.code, ["Noto Sans Mono", "Noto Sans"]);
    }

    /// An element that overrides nothing must SHARE the body slice, not
    /// copy it — that is what keeps resolution allocation-free for the
    /// overwhelmingly common case.
    #[test]
    fn inheriting_elements_share_the_body_allocation() {
        let fonts = styled(|s| s.body_font_families = vec!["Fake Body".into()]);
        assert!(std::ptr::eq(fonts.toc, fonts.body));
        assert!(std::ptr::eq(fonts.heading(3), fonts.body));
    }
}
