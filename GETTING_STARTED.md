# Getting started — markdoc-pdf for tech writers

A 5-minute tour. By the end of it you'll have a working `.mdoc`
source, a chosen style, and a generated PDF.

If you don't yet have the `markdoc-pdf` binary installed, see
[INSTALL.md](INSTALL.md) first.

## 1. Make a project folder

A markdoc-pdf project is just a folder with the source `.mdoc`
alongside any media it references:

```
my-doc/
├── intro.mdoc
└── images/
    ├── logo.svg
    └── diagram.png
```

That layout — source at the top, media in a sibling folder — is the
default markdoc-pdf assumes. The `--assets-root` flag overrides it
when you need something else.

## 2. Write your first `.mdoc`

Create `my-doc/intro.mdoc`:

```markdown
---
title: Coffee Brewing Guide
description: Pour-over basics for the home barista
authors:
  - Aki Tanaka
  - Maria Linde
language: en-us
firstReleaseDate: "2026-05-03"
---

# Pour-over essentials

Pour-over coffee is **deceptively simple**: hot water, ground beans,
a filter, and a few minutes of attention.

## What you need

- A *gooseneck* kettle for controlled pouring
- Medium-coarse grind, around 20 g for a single cup
- 92–96 °C water (just off the boil)

## The basic recipe

1. Bloom: add 40 g of water, wait 30 s.
2. First pour: spiral out to 150 g over 30 s.
3. Second pour: bring it to 320 g over another 60 s.
4. Drawdown finishes around 3:00 to 3:30 total.

![A clean v60 setup](images/diagram.png)

> Tip: if it tastes sour, grind finer; if it tastes bitter, grind
> coarser. Adjust one variable at a time.
```

Things to notice:

- **Frontmatter** (the YAML between `---` lines) drives the PDF
  metadata and template variables like `{title}` and `{date}`. None
  of the fields is required, but `title` matters for ToC pages and
  the cover page.
- **Markdown** works as you'd expect — `**bold**`, `*italic*`,
  `[link text](url)`, lists, blockquotes, fenced code blocks (with
  optional language for syntax highlighting), tables.
- **Markdoc tags** look like `{% tag %}…{% /tag %}`. The most useful
  for writers right now: `{% callout %}`, `{% caption %}`, `{% media src="…" /%}`,
  `{% footnote %}`. See *Markdoc tags* below.

## 3. Pick a style

Styles live in `markdoc-pdf/examples/themes/`. Pick one that matches
the document type:

| You're writing… | Try… |
|-----------------|------|
| Product manual | `technical-manual.style.toml` |
| Academic paper | `academic-paper.style.toml` |
| Internal review draft | `draft-report.style.toml` |
| Letter / memo | `letter.style.toml` |
| Book | `book.style.toml` |
| Changelog | `release-notes.style.toml` |
| Sales whitepaper | `whitepaper.style.toml` |
| One-page reference card | `cheatsheet.style.toml` |

You can also omit `--style` entirely — the built-in default (A4,
Noto Sans, no decoration) is a fine starting point.

## 4. Render

From inside `my-doc/`:

```sh
markdoc-pdf \
    --input intro.mdoc \
    --output intro.pdf \
    --style /path/to/themes/technical-manual.style.toml
```

That writes `intro.pdf` and prints `wrote intro.pdf (… bytes) from intro.mdoc`.

If something is wrong with your input, you'll get a single-line
`error: …` plus a `hint: …` follow-up. Common ones:

- `error: input file not found: …` — typo in the path.
- `error: couldn't load style …: missing field …` — the style file
  is missing a field; copy a working theme as a starting point.
- `warning: media unavailable: …` — an `<img>` or `{% media %}`
  references a file the resolver couldn't find. The PDF still
  renders, with a placeholder where the image should be.

## 5. Iterate

The CLI is one-shot, so a typical loop is:

```sh
markdoc-pdf -i intro.mdoc -o intro.pdf -s themes/technical-manual.style.toml \
  && xdg-open intro.pdf      # or: open intro.pdf  on macOS
```

Wire it to your editor's "save and run" hook for a tight feedback
loop.

## Markdoc tags worth knowing

Tags extend Markdown with semantic markers that the renderer
interprets. The ones writers will use most:

### `{% callout %}` — note / warning / info boxes

```markdown
{% callout type="warning" %}
Don't pour boiling water — let it sit 30 s after the kettle whistles.
{% /callout %}
```

`type` accepts `note`, `info`, `warning`, `caution`, `danger`,
`success`, `notice`. Each renders as a coloured panel.

### `{% caption %}` — captions for figures and tables

Place immediately before an image or table:

```markdown
{% caption %}V60 brewing geometry — top view{% /caption %}
![…](images/diagram.png)
```

The caption gets a `Figure N:` (or `Table N:`) prefix and feeds the
List of Figures / List of Tables sections when the style enables them.

### `{% media src="…" /%}` — images with explicit alt text

Equivalent to `![alt](src)` but lets you separate `alt` and `src`
attributes — useful when alt text gets long:

```markdown
{% media src="images/diagram.png" alt="Side view of the V60 dripper sitting on a glass server, with a paper filter and ground coffee inside." /%}
```

### `{% footnote %}` — inline footnotes

```markdown
The Treaty of Westphalia{% footnote %}Signed in 1648, ending the Thirty Years' War.{% /footnote %} is widely cited.
```

Numbers are auto-assigned. Bodies appear in the per-page footnote
pool at the bottom of the page. Configure separator/font/spacing in
the style's `[footnote]` section.

### `{% tag id="name" %}` and `{% tagref id="name" %}` — cross-references

```markdown
## Configuration {% tag id="config" %}

…

For details see the {% tagref id="config" %} chapter.
```

`{% tagref %}` becomes a clickable link in the PDF. Every heading
also gets an auto-assigned anchor (`__heading_<n>`) so the ToC and
List of Figures entries are linkable.

## Where to next

- The full set of style knobs (margins, fonts, colours, headers,
  footers, watermarks, coverpage, hyphenation, custom font loading)
  lives in [`examples/themes/README.md`](examples/themes/README.md).
- For the full Markdoc tag/schema reference, see the parent
  `markdoc/` crate's documentation.
- Frontmatter fields supported out of the box are defined in
  `flux-types/` — start with `title`, `description`, `authors`,
  `language`, `firstReleaseDate`, `creator`.
