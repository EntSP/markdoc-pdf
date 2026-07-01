#!/bin/sh
#
# impose.sh — render an A5 .mdoc and impose it two-up onto A4 as a
# saddle-stitch pamphlet (fold one A4 sheet → four A5 pages).
#
#   examples/impose.sh [input.mdoc] [style.toml] [out-booklet.pdf]
#
# With no arguments it builds the bundled A5 pamphlet example. The two
# steps are deliberately separate, because markdoc-pdf only does the first:
#
#   1. render  — markdoc-pdf emits A5 pages in reading order 1, 2, 3, …
#   2. impose  — pdfjam reorders them (booklet), places two per A4 sheet, and
#                pads the count up to a multiple of four with blank pages.
#
# Print the result double-sided, flipping on the SHORT edge, then fold.
#
# POSIX sh — no bashisms.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"   # examples/
root="$(dirname "$here")"               # markdoc-pdf/

input="${1:-$here/a5-pamphlet.mdoc}"
style="${2:-$here/a5-pamphlet.style.toml}"
out="${3:-$root/a5-pamphlet-booklet.pdf}"
a5="${out%.pdf}.a5.pdf"                 # single-up A5 intermediate

# ── Locate the binary (expects markdoc-pdf on PATH) ────────────────────
bin="markdoc-pdf"
if ! command -v "$bin" >/dev/null 2>&1; then
    echo "error: '$bin' not found on PATH — build it (cargo build --release) and put target/release on PATH, or install it" >&2
    exit 1
fi

# ── 1. Render A5 pages ─────────────────────────────────────────────────
echo "render : $input"
echo "       → $a5  (A5 pages)"
"$bin" --input "$input" --output "$a5" --style "$style"

# ── 2. Impose two-up onto A4 (external — markdoc-pdf does not impose) ───
# The A5 pages are already sized for two-up on A4, so pdfjam's booklet mode
# imposes them directly — no cropping or rescaling. (That crop pass is the
# only reason pdfbook2 wrapped pdfjam in Python; we don't need it here, so we
# call pdfjam straight and avoid the Python dependency.)
if ! command -v pdfjam >/dev/null 2>&1; then
    cat >&2 <<EOF

note   : pdfjam not found, so the fold step was skipped.
         The A5 pages are ready at:
             $a5
         Install pdfjam to finish the pamphlet:
             Fedora : sudo dnf install texlive-pdfjam
             Debian : sudo apt install texlive-extra-utils
         …or impose straight from your print dialog
         (Booklet layout, A4 paper, duplex flip on SHORT edge).
EOF
    exit 1
fi

echo "impose : $a5"
echo "       → $out  (2-up A4 booklet)"
# Booklet imposition: reorder the pages and place two A5 per A4 landscape
# sheet, padding to a multiple of four with blanks. Writes $out directly.
pdfjam --landscape --booklet true --paper a4paper --quiet --outfile "$out" "$a5"

echo "done   : $out"
if command -v pdfinfo >/dev/null 2>&1; then
    pdfinfo "$out" | awk '/^Pages:|^Page size:/ { print "         " $0 }'
fi
