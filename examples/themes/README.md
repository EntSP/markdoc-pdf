# Theme catalogue

Ready-made `.style.toml` files for common document types. Pass any of
them to `markdoc-pdf` with `--style themes/<name>.style.toml`. Mix and
match: every field has a sensible default, so a theme can override
just the parts that matter and inherit the rest.

| File | Use for | Notable features |
|------|---------|------------------|
| `technical-manual.style.toml` | Product manuals, reference docs | A4, ToC + LoF + LoT, chapter/section header, page-of-total footer, hyphenation |
| `academic-paper.style.toml` | Papers, theses | US Letter, looser leading, justified body, footnote pool, page-number footer |
| `draft-report.style.toml` | Internal review reports | PDF/UA-1 export, "DRAFT" diagonal watermark, build-date footer |
| `letter.style.toml` | Letters, memos | Generous margins, no decoration, single-page friendly |
| `book.style.toml` | Long-form publications | 6×9", verso/recto headers (chapter on recto, title on verso), small body font, ToC |
| `release-notes.style.toml` | Changelogs, release notes | Compact dense layout, version in header, no front matter |
| `whitepaper.style.toml` | Sales / position papers | Large title page from frontmatter, ToC at start, generous margins, justified body, hyphenation |
| `cheatsheet.style.toml` | Quick references, CLI cards | Landscape A4, narrow margins, dense body; ideal for one-page reference cards |
| `poster-a3.style.toml` | A3 wall posters (297 × 420 mm) | Single page, body text sized for ~1.5 m reading distance |
| `poster-a2.style.toml` | A2 conference-board posters (420 × 594 mm) | Single page, body text sized for 2-3 m reading distance |
| `poster-a1.style.toml` | A1 conference / poster-session (594 × 841 mm) | Single page, body text sized for 3-4 m reading distance |
| `poster-a0.style.toml` | A0 lecture-hall / large display (841 × 1189 mm) | Single page, body text sized for 4-6 m reading distance |

## Authoring your own

Themes are just TOML — start with one of the above as a baseline and
override what you want. Every section is documented in
`markdoc-pdf/src/render/style.rs`. Common knobs:

- `pdf_export = "u_a1" | "a1_b" | "a2_b" | "a3_b" | "a4" | "none"`
- `[toc]`, `[lof]`, `[lot]` — sections at start/end of the doc
- `[page_decoration.header]` / `[page_decoration.footer]` with
  `left`, `center`, `right` template strings
- `{page}`, `{total}`, `{title}`, `{chapter}`, `{section}`, `{date}`
- `[watermark]` for diagonal-text or full-bleed-image overlays
- `text_align = "left" | "justify" | "center" | "right"` for body prose
- `[hyphenation]` to insert soft hyphens (English-US bundled) — pairs
  well with `text_align = "justify"` for even inter-word spacing
- `font_paths = [...]` to load extra `.ttf`/`.otf` families
- `body_font_families = ["My Custom Family", "fallback"]`
- `[coverpage]` to synthesise a cover page from frontmatter

## Cover pages

Source `.mdoc` documents stay output-agnostic — they don't include a
title-page tag. Instead, enable a synthesised cover page in the style:

```toml
[coverpage]
enabled = true
top_margin = 180
title_font_size = 36
subtitle = "{description}"   # template — `{title}`, `{description}`, `{date}`

[coverpage.logo]
src = "logo.svg"             # any AssetResolver URI
width = 200
height = 80

[page_decoration]
skip_first_page = true       # cover page renders without header/footer
```

The renderer pulls the title, description, authors, and date from the
document's frontmatter (via `RenderContext`), draws the logo if
present, then forces a page break so body content starts on page 2.
