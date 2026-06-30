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
| `callout-styles.style.toml` | Every callout-related knob spelled out: `callout_padding`, `callout_accent_width`, `callout_space_after`, plus per-severity `[callout_styles.<kind>]` blocks for all seven kinds. Values match the built-in defaults — copy and edit. |
| `coverpage.style.toml` (+ `coverpage.mdoc`) | Synthesised cover page: large title at the top, hero image below (`logo_position = "below_title"`), then subtitle / authors / publication date. Renders against `coverpage.mdoc`, which carries the rich frontmatter the cover page needs. |
| `badges.style.toml` (+ `badges.mdoc`) | Ordered-list circle badges via `[list_marker]`: numbers in filled circles, with `ordered_sequences` cycling `decimal` → `lower-alpha` as the list nests (1 → a). |
| `table-borders-{grid,horizontal,none}.style.toml` (+ `table-borders.mdoc`) | The three `table_borders` modes — a full grid, horizontal row rules only, or none. Render `table-borders.mdoc` with each to compare. |
| `table-styling.style.toml` (+ `table-styling.mdoc`) | Styling tables *differently within one document*: a document-wide table style (incl. zebra striping via `table_stripe_color`) plus per-table overrides by wrapping a pipe table in `{% table %}` with `borders`, `border_color`, `edge_color`, `header_background`, `stripe` (colour or `none`), `column_weights`, `cell_padding`, and `header_column` (make column 0 row headers) attributes. |
| `table-forms.style.toml` (+ `table-forms.mdoc`) | The table *forms* this renderer supports: a plain pipe table, a header-less table (blank header row dropped), a header column (`{% table header_column=true %}`), and per-column text alignment via CommonMark `:---` / `:-:` / `---:` delimiter markers. |
| `table-list-syntax.mdoc` (+ `table-forms.style.toml`) | The Markdoc **list-syntax** `{% table %}` (cells as `*` items, rows separated by `---`): basic, header-less (leading `---`), rich content (code, lists, and `{% list type="checkmark" %}` in cells), and column / per-cell alignment via `{% align %}`. Builds the same table structure as a pipe table. (`colspan`/`rowspan` annotations parse but don't yet render as merged cells.) |
| `pdfa.style.toml` | One-liner: `pdf_export = "a2_b"`. Switches to PDF/A-2B output. |
| `ua1.style.toml` | One-liner: `pdf_export = "u_a1"`. Switches to PDF/UA-1 (accessibility) output. |

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
