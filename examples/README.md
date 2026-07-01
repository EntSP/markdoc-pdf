# Examples

Two kinds of things live here:

- **Single-feature style demos** — tiny `.style.toml` files that
  illustrate one knob at a time (`small`, `pdfa`, `toc`, …). Useful
  as copy-paste starting points when authoring a real theme.
- **Source fixtures** — `rich.mdoc` and the assets it references.
  Used by the CI smoke-test and by everything below as the input
  document.

Ready-to-deploy themes (manuals, papers, posters, …) live one
directory deeper in [`themes/`](themes/README.md) — start there if
you want a finished look rather than a feature reference.

## Quick render

From the `markdoc-pdf/` directory after building the binary
(`cargo build --release`):

```sh
./target/release/markdoc-pdf \
    --input  examples/rich.mdoc \
    --output rich.pdf \
    --style  examples/themes/technical-manual.style.toml \
    --assets-root examples
```

Swap `--style` for any of the files below to see a single feature in
isolation.

## Style demos

Every file in `examples/*.style.toml` is a *partial* style — it only
sets the fields it cares about; everything else inherits from the
built-in default (A4, Noto Sans, no decoration). That makes them
small and readable.

| File | Demonstrates |
|------|--------------|
| `small.style.toml` | Tiny page height (300 pt). Forces pagination on `rich.mdoc`. |
| `letter.style.toml` | US Letter geometry, header / footer with template variables, custom callout colours. The most complete "real-world" example. |
| `toc.style.toml` | Just a table of contents at the start. |
| `full-toc.style.toml` | ToC + List of Figures + List of Tables — three front-matter sections. |
| `abbrev.style.toml` | Abbreviated caption prefixes (`Fig.` / `Tab.`) plus LoF and LoT. |
| `below.style.toml` | Caption positioned **below** the figure / table instead of above. |
| `copyright-footer.style.toml` | Three-slot footer (copyright left, confidentiality + title centre, page-of-total right). Reference for footer authoring. |
| `link-style.style.toml` | `[link]` block exercising all four knobs (`color`, `italic`, `bold`, `underline`). Renders links in bold italic purple with an underline. |
| `decorations.style.toml` (+ `decorations.mdoc`) | Every inline text **decoration** in one document: bold, italic, strikethrough, `inline code` (monospace), links (coloured + underlined via `[link]`), and `{% color %}` spans — shown alone, mid-paragraph, and combined. The style only enables link underlining so all decorations are visible together. |
| `callout-styles.style.toml` | Every callout-related knob spelled out: `callout_padding`, `callout_accent_width`, `callout_space_after`, plus per-severity `[callout_styles.<kind>]` blocks for all seven kinds. Values match the built-in defaults — copy and edit. |
| `coverpage.style.toml` (+ `coverpage.mdoc`) | Synthesised cover page: large title at the top, hero image below (`logo_position = "below_title"`), then subtitle / authors / publication date. Renders against `coverpage.mdoc`, which carries the rich frontmatter the cover page needs. |
| `badges.style.toml` (+ `badges.mdoc`) | Ordered-list circle badges via `[list_marker]`: numbers in filled circles, with `ordered_sequences` cycling `decimal` → `lower-alpha` as the list nests (1 → a). |
| `table-borders-{grid,horizontal,none}.style.toml` (+ `table-borders.mdoc`) | The three `table_borders` modes — a full grid, horizontal row rules only, or none. Render `table-borders.mdoc` with each to compare. |
| `table-styling.style.toml` (+ `table-styling.mdoc`) | Styling tables *differently within one document*: a document-wide table style (incl. zebra striping via `table_stripe_color`) plus per-table overrides by wrapping a pipe table in `{% table %}` with `borders`, `border_color`, `edge_color`, `header_background`, `stripe` (colour or `none`), `column_weights`, `cell_padding`, and `header_column` (make column 0 row headers) attributes. |
| `table-forms.style.toml` (+ `table-forms.mdoc`) | The table *forms* this renderer supports: a plain pipe table, a header-less table (blank header row dropped), a header column (`{% table header_column=true %}`), and per-column text alignment via CommonMark `:---` / `:-:` / `---:` delimiter markers. |
| `table-list-syntax.mdoc` (+ `table-forms.style.toml`) | The Markdoc **list-syntax** `{% table %}` (cells as `*` items, rows separated by `---`): basic, header-less (leading `---`), rich content (code, lists, and `{% list type="checkmark" %}` in cells), column / per-cell alignment via `{% align %}`, and `{% colspan=N %}` / `{% rowspan=N %}` (merged columns and rows). Builds the same table structure as a pipe table. |
| `columns.mdoc` | Side-by-side layout with `{% columns %}` — equal or uneven (`widths="2 1"` / `widths=[2, 1]`) columns of images (or any blocks); each column is a list item (`*`) or a blank-line-separated block, `gap` sets the spacing, and a per-column `{% caption %}` (loose-list form) can sit above or below (`position="below"`) and be tinted with `color="#…"` (or the document-wide `caption_color` style field). A `widths="2 1 2"` row of text / image / text (+ `figure.svg`) centres a figure with independent commentary flanking it on both sides. Images now also render inside ordinary table cells. Render with `--assets-root examples`. |
| `float.mdoc` (+ `figure.svg`) | Content wrapping around an image with `{% float %}` — a `side="left"` (default) or `side="right"` image that the following content wraps beside, then flows full-width below once it clears the image. `width` sizes the image (a fraction ≤ 1 of the column, an explicit point value, or a `"NN%"` string; default 40 %) and `gap` sets the space to the content. The wrap keeps its real structure: a paragraph reflows around the image, while lists, `inline code`, code blocks and callouts render as themselves, and footnotes / anchors inside the wrap are preserved. Give each `{% media %}` an explicit `side` and drop it inline, and the region switches to the **magazine** mode: several images floated to either side, anchored where they appear, with one continuous prose stream wrapping around all of them (narrowing left, right, or into the channel between overlapping floats). The whole float is one indivisible unit — it never straddles a page break (moves to the next page if it won't fit). Use `{% columns %}` instead when you want fully independent side-by-side lanes. Render with `--assets-root examples`. |
| `form.mdoc` | Print form fields via `{% input %}` — a label above a ruled box (sized by `maxlength`; a small square for `type="checkbox"`), a red required marker, and a grey hint spelling out the type / length / value constraints (`text`, `number`, `email`, `date`, `min`/`max`, `minlength`/`maxlength`). Pure graphics, so PDF/A-safe — no interactive widget, which PDF/A forbids. The same tag is a native validated `<input>` on the web. |
| `a5-pamphlet.style.toml` (+ `a5-pamphlet.mdoc`) | A5 portrait pages (148 × 210 mm) sized to be imposed two-up onto an A4 sheet and folded into a saddle-stitch pamphlet: tighter margins, smaller body font, and a centred page-number footer (bare front cover via `skip_first_page`). The renderer makes the A5 *pages* only — `impose.sh` (below) wraps render → `pdfbook2` into fold-ready A4 sheets, since imposition is not a renderer feature. |
| `pdfa.style.toml` | One-liner: `pdf_export = "a2_b"`. Switches to PDF/A-2B output. |
| `ua1.style.toml` | One-liner: `pdf_export = "u_a1"`. Switches to PDF/UA-1 (accessibility) output. |

## Pamphlet imposition

`impose.sh` turns the A5 pamphlet — or any A5 `.mdoc` — into a
fold-ready A4 booklet. It renders the A5 pages, then runs `pdfbook2`
to reorder them and place two per sheet (padding to a multiple of four
with blanks). The renderer never imposes; that is entirely this
script's `pdfbook2` step.

```sh
examples/impose.sh                              # bundled example → a5-pamphlet-booklet.pdf
examples/impose.sh in.mdoc style.toml out.pdf   # any A5 document
```

`pdfbook2` ships with TeX Live (`texlive-pdfbook2` on Fedora,
`texlive-extra-utils` on Debian). Without it the A5 pages are still
written, and you can impose from your print dialog instead (Booklet
layout, A4 paper, duplex flipped on the **short** edge). Print
double-sided, fold, staple the spine.

## Source fixture

`rich.mdoc` exercises most rendering paths in a single document:
headings, lists (ordered and unordered), block quotes, code with
syntax highlighting, callouts (info / warning / danger), inline and
block media (PNG + SVG + a deliberately-missing reference to test the
placeholder), a wide table, cross-references with `{% tagref %}`, and
a long-paragraph tail that forces pagination.

Its frontmatter is intentionally minimal (just `title:`) so style
demos drive the visual output rather than metadata.

The referenced assets:

| File | Used for |
|------|----------|
| `swatch.png` | Raster image referenced via `![…](swatch.png)`. |
| `swatch.svg` | SVG referenced via `{% media src="swatch.svg" /%}`. |

Pass `--assets-root examples` so the renderer resolves these relative
to this directory.
