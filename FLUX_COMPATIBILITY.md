# Flux PR compatibility

This is a snapshot of where the MVP workflow (`markdoc` + `flux-types`
+ `markdoc-pdf`) stands against the in-flight Flux specification PR
(`EntSP/flux#1`). Tracks which PR features render today, which need
small fixes, and which are genuinely out-of-scope until later.

Last verified: pr-1 head `2638396`.

## Frontmatter schema — fully aligned

`flux-types` already deserialises every field the pr-1 README defines:

| pr-1 common field | flux-types field | Notes |
|-------------------|------------------|-------|
| `id` | `id` | |
| `type` | `doc_type` (renamed; serde rename) | |
| `title` | `title` | |
| `documentNumber` | `document_number` | u64; tolerates float-encoded ints |
| `status` | `status` | |
| `version` | `version` | |
| `language` | `language` | |
| `firstReleaseDate` | `first_release_date` | drives `/CreationDate` and `{date}` |
| `updateDate` | `update_date` | |
| `accessLevel` | `access_level` | accepts both string and string[] |
| `tags` | `tags` | |
| `files` | `files` | both `path:` and `url:` variants |
| `documentHistory` | `document_history` | array of `{version, date, description}` |

Per-document-type fields (Manual: `hwVersionRobot/TM`, `swVersion`,
`products`, `configFile`, `sections`; Notice: `category`,
`affectedProducts`, `affectedHwRanges`, `expiryDate`; Article:
`orderNumber`; FAQ: `question`, `popularity`; Release Note:
`swVersion`, `swAccess`; Product Note: `noteType`,
`replacementProducts`, `effectiveDate`) all deserialise — they're
collapsed into the same struct as `Option<…>` so adding a document
type later means adding more fields, not a new sum type.

`flux-types` also keeps `description`, `authors`, and `creator` —
the pr-1 README dropped them from the common-fields table but its
own example YAML still includes `description`. The renderer needs
all three for PDF metadata, so they stay regardless.

## Markdoc tags — what works today

| pr-1 syntax | Status | Notes |
|-------------|--------|-------|
| `{% tag id="X" /%}` | ✅ Works | The Markdoc-canonical form |
| `{% tag "X" /%}` | ✅ Works | Markdoc primary-attribute shorthand |
| `{% tag X /%}` | ✅ Works | Bare (unquoted) primary shorthand |
| `{% tag="X" /%}` | ❌ Removed | Use `{% tag "X" /%}` instead — same meaning, less ambiguous |
| `{% tagref id="X" /%}` | ✅ Works | Resolves to PDF `GoTo` annotation |
| `{% tagref "X" /%}` | ✅ Works | Primary shorthand on tagref too |
| `{% tagref tag="X" %}` … `{% /tagref %}` | ⚠️ Avoid | Block form not used by Flux; renderer expects self-closing |
| `{% callout type="warning" %}` … `{% /callout %}` | ✅ Works | All four pr-1 types: caution / warning / note / info |
| `{% media src="…" /%}` | ✅ Works | Plus `alt` / `title` / `caption` / `kind` / `side` / `size` / `width`; `{% img %}` is an alias. `size` scales the image to `small` / `medium` / `large` (50 / 75 / 100 % of the available width). `title` acts as a caption / alt fallback only when the style sets `image_title_fallback` |
| `{% media id="…" /%}` | ✅ Works | Arca asset id. Scriptor rewrites it to a concrete `src` upstream; locally markdoc-pdf finds `<id>.<ext>` under `--assets-root` |
| `{% caption %}` … `{% /caption %}` | ✅ Works | Auto-numbered Figure N / Table N |
| `{% footnote %}` … `{% /footnote %}` | ✅ Works | Per-page pool with separator rule |
| `{% if $var %}` … `{% /if %}` | ✅ Works | Undefined `$var` evaluates to falsy → branch dropped |
| `{% if A %}…{% else $B /%}…{% else /%}…{% /if %}` | ✅ Works | Else-if chain; bare `{% else /%}` is the unconditional fallback |
| `{% partial file="…" /%}` | ✅ Works | Recursive, cycle-detected; `file=` resolves against the input file's directory — relative `../` and absolute paths reach other dirs |
| `![alt](path/to/file)` | ✅ Works | Asset resolved against `--assets-root` |
| `[link text](https://…)` | ✅ Works | External link annotation |
| Tables, lists, blockquotes, fenced code | ✅ Work | Hyphenation, syntax highlighting per style |
| Frontmatter variable interpolation `{% $markdoc.frontmatter.version %}` | ✅ Works | |

## Renderer extensions — beyond the Flux spec

markdoc-pdf also renders several layout / form tags that are **not** part of
the pr-1 spec (the web / app renderers realise them with CSS). They're safe
in a PDF-targeted source, and each has a worked example under `examples/`:
`{% columns %}` (side-by-side, with optional `align` centring and a
`background` panel), `{% grid %}` (cells reflowing into as many equal columns
as fit — the print analogue of CSS `repeat(auto-fill, minmax(min, 1fr))`),
`{% float %}` (text wrapping an image, incl. inline-anchored magazine floats),
`{% input %}` (print form field, PDF/A-safe), `{% color %}` / `{% c %}`
(inline colour), `{% swatch %}` / `{% chip %}` (a block colour bar and its
inline colour dot — solid, or a linear gradient for the bar), and
`{% list %}` (custom marker).

## ⚠️ Known gaps — fixable with author edits

These need writers to use a slightly different syntax than the pr-1
examples show; nothing renderer-side blocks them.

### `{% tag="X" %}` without the closing `/`

`tag` is declared `self_closing: true` in the Markdoc schema, so the
canonical form is `{% tag="X" /%}` (note the slash). Without `/` the
parser opens an unbalanced tag and consumes everything that follows
into its children — visually you'll see headings and paragraphs
fusing together.

**Fix in the source**: append `/` before `%}` everywhere `{% tag … %}`
appears as an anchor declaration. Same for `{% media … %}` when used
without a body.

### Heading anchors via `{% #id %}` shorthand

Markdoc supports `# Heading {% #my-anchor %}` as a heading-level
anchor. This works alongside `{% tag id="…" /%}`. Both produce a
`GoTo`-able destination.

## ❌ Not yet supported — out of MVP scope

These need work in the parser or renderer; document-side workarounds
are noted where they exist.

### Ticket components / function calls

```markdoc
{{ renderReleaseTickets("fixVersion"="x.x.x", "ReleaseNote"="Published") }}
{{ files.find(f => f.path && f.path.includes('front-panel')).cdnUrl }}
```

Inline JavaScript-style expressions and function-call components
aren't supported. These are intended for the eventual Adeptus
runtime; markdoc-pdf renders them as literal text.

### Variable expansion inside attribute strings

```markdoc
{% img src="img/path/file_name_{$config.model}.png" %}
```

The `{$var}` interpolation inside an attribute value isn't expanded
by the renderer. **Workaround**: parameterise via the asset URI
itself, e.g. an `arca://…` URI carrying the model in its path, so
expansion happens at the resolver layer.

### Conditional rendering binding to `$frontendType`

The pr-1 README mentions `{% if $frontendType === "pdf" %}` blocks.
This relies on the Adeptus runtime injecting `$frontendType`.
markdoc-pdf currently runs without any predefined Context bindings,
so all `$frontendType`-keyed conditionals evaluate to falsy — the
"pdf" branches will be DROPPED, not included. **Workaround for
MVP**: avoid this pattern until Scriptor wires up the per-render
Context.

## Summary

For the MVP loop (write `.mdoc` → `markdoc-pdf` → review), the pr-1
spec is largely usable today provided writers:

1. Always self-close `{% tag … /%}`, `{% media … /%}`, `{% tagref … /%}`.
2. Avoid JS-style function components and `{$var}` interpolation in
   attribute strings — push those concerns into the asset URI
   instead.
3. Don't depend on `$frontendType` blocks until Scriptor wires up
   per-render Context bindings.

`{% partial %}` (recursive, with cycle detection) and `{% else %}` /
`{% else $cond /%}` are now first-class — composed manuals and
multi-branch conditionals render correctly without preprocessing.
