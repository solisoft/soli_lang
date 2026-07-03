#!/usr/bin/env python3
"""Generate the "Feature tour" PDF sample — the self-documenting soli-pdf
reference served by the PDF playground.

Each language feature is presented as a "card": a heading, a one-line
description, the JSON that produces it (a real code sample in a tinted box),
and the rendered result — with generous, consistent vertical spacing.

Code-box heights are computed from the snippet's line count so the boxes always
fit snugly, and each heading's `minSpaceBelow` keeps a whole card together on
one page; every spacing constant lives here so the rhythm stays uniform. This
script is the source of truth for the two generated sample files — edit it, not
the JSON.

Usage (from the repo root):

    python3 scripts/gen_pdf_feature_tour.py \\
        www/public/pdf-samples/features.template.json \\
        www/public/pdf-samples/features.data.json

Preview the result with the standalone renderer:

    pdf/target/release/render_pdf \\
        --template www/public/pdf-samples/features.template.json \\
        --data     www/public/pdf-samples/features.data.json \\
        --font-dir www/font -o /tmp/features.pdf
"""
import json

# ── palette ────────────────────────────────────────────────────────────────
TEAL      = "0F766E"
TEAL_DK   = "115E56"
TEAL_TINT = "f0fdfa"
TEAL_PALE = "ccfbf1"
AMBER     = "f59e0b"
INK       = "0f172a"
SLATE_800 = "1e293b"
SLATE_700 = "334155"
SLATE_600 = "475569"
SLATE_500 = "64748b"
SLATE_400 = "94a3b8"
HAIR      = "e2e8f0"
HAIR_2    = "eef2f6"
CODE_BG   = "f8fafc"
LINK      = "2563eb"

CONTENT_W = 495          # A4 (595) − 2×50 margins

# ── code-box metrics ─────────────────────────────────────────────────────────
CODE_FS   = 8.5
CODE_LH   = 1.55
PAD_TOP   = 10
PAD_BOT   = 9
ACCENT_W  = 3
TEXT_X    = 15           # indent past the accent bar

# ── vertical rhythm ──────────────────────────────────────────────────────────
GAP_CARD      = 24       # between feature cards
GAP_HEAD_DESC = 4        # heading → description
GAP_DESC_CODE = 7        # description → code box
GAP_CODE_OUT  = 13       # code box → rendered result
GAP_AFTER_CH  = 16       # chapter title → first card


def para(value=None, spans=None, **opts):
    el = {"type": "paragraph"}
    if spans is not None:
        el["spans"] = spans
    else:
        el["value"] = value
    if opts:
        el["options"] = opts
    return el


def move(x=0, y=0):
    return {"type": "move", "x": x, "y": y}


def rect(w, h, fill=None, border=None, border_w=None, radius=None, dash=None):
    el = {"type": "rect", "width": w, "height": h}
    if fill is not None:
        el["fill"] = fill
    if border is not None:
        el["border"] = border
    if border_w is not None:
        el["borderWidth"] = border_w
    if radius is not None:
        el["radius"] = radius
    if dash is not None:
        el["dash"] = dash
    return el


def hr(color=HAIR, thickness=0.6, width=None):
    el = {"type": "hr", "color": color, "thickness": thickness}
    if width is not None:
        el["width"] = width
    return el


def heading(text, anchor=None, bookmark=None, level=2, keep=90):
    """A feature heading — mono teal. `keep` is the space (pt) that must remain
    below it on the page; the engine breaks first if the whole card won't fit,
    so a fixed-height code box or chart never overflows into the footer."""
    o = {
        "fontSize": 12,
        "fontWeight": "bold",
        "mono": True,
        "color": TEAL,
        "minSpaceBelow": keep,
    }
    if bookmark:
        o["bookmark"] = bookmark
        o["bookmarkLevel"] = level
    if anchor:
        o["anchor"] = anchor
    return para(text, **o)


def _lit(text):
    """Escape `${` so prose can name template syntax without interpolating it."""
    return text.replace("${", "$${")


def desc(text):
    return para(_lit(text), fontSize=9.5, color=SLATE_500, lineHeight=1.35)


def code_box(lines, width=CONTENT_W):
    """A tinted, accent-barred code sample. Height derives from the line count
    so the box always fits. Emits the box + a trailing move that lands the
    cursor GAP_CODE_OUT below the box."""
    n = len(lines)
    text_h = n * CODE_FS * CODE_LH
    box_h = PAD_TOP + text_h + PAD_BOT
    # Escape `${` so the engine prints template syntax verbatim in the sample
    # instead of interpolating it (see interpolate.rs — `$${` → literal `${`).
    body = "\n".join(lines).replace("${", "$${")
    out = [
        rect(width, box_h, fill=CODE_BG, border=HAIR, border_w=0.8, radius=6),
        rect(ACCENT_W, box_h, fill=TEAL, radius=1.5),
        move(TEXT_X, PAD_TOP),
        para(body, fontSize=CODE_FS, mono=True, color=SLATE_700, lineHeight=CODE_LH),
        move(-TEXT_X, PAD_BOT + GAP_CODE_OUT),
    ]
    return out


def renders_label():
    return para("renders", fontSize=7.5, mono=True, color=SLATE_400,
                spacing=3)


def _est_height(description, code, result_keep):
    """Estimate a card's height so the heading can keep it together on one page.
    Fixed parts (heading, description, code box) are measured exactly; the
    result is reserved via `result_keep`."""
    import math
    h = 16 + GAP_HEAD_DESC
    if description:
        lines = max(1, math.ceil(len(_lit(description)) / 100))
        h += lines * (9.5 * 1.35) + GAP_DESC_CODE
    if code:
        h += PAD_TOP + len(code) * CODE_FS * CODE_LH + PAD_BOT + GAP_CODE_OUT
    if result_keep:
        h += 12 + result_keep          # "renders" label + reserved result
    return min(h, 560)                  # never exceed the usable content height


def card(head, anchor, description, code, result, gap=GAP_CARD, result_keep=44):
    """One documentation card: heading · description · code sample · result.
    The heading also becomes a level-2 bookmark so the PDF outline mirrors the
    table of contents. `result_keep` is the result's reserved height for the
    keep-together calculation (raise it for tall charts / shape rows)."""
    keep = _est_height(description, code, result_keep if result else 0)
    out = [heading(head, anchor=anchor, bookmark=head, keep=keep),
           move(0, GAP_HEAD_DESC)]
    if description:
        out += [desc(description), move(0, GAP_DESC_CODE)]
    if code:
        out += code_box(code)
    if result:
        out += [renders_label()]
        out += result
    out += [move(0, gap)]
    return out


def chapter_opener(num, title, subtitle, anchor, bookmark, first=False):
    """A page-topping chapter header: number chip + title + subtitle + rule."""
    out = []
    if not first:
        out.append({"type": "page_break"})
    out += [
        rect(30, 30, fill=TEAL, radius=8),
        move(11, 6),
        para(str(num), fontSize=13, fontWeight="bold", color="ffffff"),
        move(30, -19),
        para(title, fontSize=20, fontWeight="bold", color=INK,
             bookmark=bookmark, anchor=anchor),
        move(-41, 12),
        para(subtitle, fontSize=10, color=SLATE_500, italic=True),
        move(0, 10),
        hr(color=HAIR, thickness=0.8),
        move(0, GAP_AFTER_CH),
    ]
    return out


# ═══════════════════════════════════════════════════════════════════════════
# COVER
# ═══════════════════════════════════════════════════════════════════════════
def cover():
    out = []
    # Hero band: amber top accent + teal panel + logo + title.
    out += [
        rect(CONTENT_W, 6, fill=AMBER, radius=3),
        move(0, 6),
        rect(CONTENT_W, 150, fill=TEAL, radius=14),
        move(30, 30),
        {"type": "image", "width": 52, "value":
            "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' width='24' height='24'>"
            "<circle cx='12' cy='12' r='10' fill='white'/>"
            "<path d='M7 13l3 3 7-8' stroke='%230F766E' stroke-width='2.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/></svg>"},
        move(72, 6),
        para("The soli-pdf template", fontSize=25, fontWeight="bold",
             color="ffffff", bookmark="Cover", anchor="sec-cover"),
        para("A complete, worked reference — rendered by the engine it documents.",
             fontSize=10.5, color=TEAL_PALE, italic=True),
        move(0, 20),
        para(spans=[
            {"text": "•  ", "color": AMBER},
            {"text": "every element & option", "color": "ffffff", "fontSize": 9.5},
            {"text": "      •  ", "color": AMBER},
            {"text": "each shown with its JSON", "color": "ffffff", "fontSize": 9.5},
            {"text": "      •  ", "color": AMBER},
            {"text": "one template, one call", "color": "ffffff", "fontSize": 9.5},
        ], fontSize=9.5),
        move(-102, 56),
    ]
    # Intro.
    out += [
        para("This document is itself a soli-pdf template. Everything you see — the "
             "cover band, the tables, the charts, the QR code — was produced by a "
             "single JSON template passed to one pdf_render() call, with no browser "
             "and no HTML. Each feature below is shown twice: the JSON that declares "
             "it, and the result it renders to.",
             fontSize=10.5, color=SLATE_700, lineHeight=1.5, alignment="justify"),
        move(0, 26),
    ]
    # Contents.
    out += [
        para("CONTENTS", fontSize=10, fontWeight="bold", mono=True, color=TEAL),
        move(0, 6),
        hr(color=HAIR, thickness=0.8),
        move(0, 12),
    ]
    toc = [
        ("1", "Text & flow", "paragraphs, spans, lists", "sec-text"),
        ("2", "Graphics & codes", "rects, strokes, images, QR, barcodes", "sec-graphics"),
        ("3", "Tables", "data binding, merges, rich cells", "sec-tables"),
        ("4", "Charts", "bar, line, pie, donut", "sec-charts"),
        ("5", "Layout & render options", "columns, branches, the render call", "sec-layout"),
    ]
    for num, title, sub, anchor in toc:
        out += [
            para(spans=[
                {"text": num + "   ", "fontWeight": "bold", "color": TEAL, "mono": True},
                {"text": title, "fontWeight": "bold", "color": INK},
                {"text": "   —   " + sub, "color": SLATE_500, "fontSize": 9},
            ], fontSize=11.5, linkTo=anchor),
            move(0, -13.8),                       # pull the page number onto the row
            para("p. #PAGE_OF:" + anchor + "#", fontSize=9.5, color=LINK,
                 alignment="right", linkTo=anchor),
            move(0, 13),
        ]
    out += [
        hr(color=HAIR, thickness=0.8),
        move(0, 10),
        para("solilang.com/docs/builtins/pdf  ·  edit this template live in the playground",
             fontSize=8.5, color=SLATE_400, italic=True,
             link="https://solilang.com/docs/builtins/pdf"),
    ]
    return out


# ── result helpers (place non-advancing shapes and reserve their height) ─────
def boxed(w, h, label, label_color=TEAL, **rectargs):
    """A rect with a vertically-centred label inside; leaves the cursor just
    below the box (rect does not advance, so we choreograph it)."""
    yoff = h / 2 - 7
    return [
        rect(w, h, **rectargs),
        move(14, yoff),
        para(label, fontSize=9.5, fontWeight="bold", color=label_color),
        move(-14, h - yoff - 11.4),
    ]


def caption(text):
    return para(_lit(text), fontSize=8.5, color=SLATE_400, italic=True, spacing=2)


def spans_code_box(span_lines, width=CONTENT_W):
    """Like code_box, but each line is a spans paragraph — so page tokens
    (#PAGE#, #PAGE_OF:…#) render literally instead of being substituted. Used
    where the sample must show the token syntax itself."""
    n = len(span_lines)
    text_h = n * CODE_FS * CODE_LH
    box_h = PAD_TOP + text_h + PAD_BOT
    out = [
        rect(width, box_h, fill=CODE_BG, border=HAIR, border_w=0.8, radius=6),
        rect(ACCENT_W, box_h, fill=TEAL, radius=1.5),
        move(TEXT_X, PAD_TOP),
    ]
    for spans in span_lines:
        out.append(para(spans=spans, fontSize=CODE_FS, mono=True,
                        color=SLATE_700, lineHeight=CODE_LH))
    out.append(move(-TEXT_X, PAD_BOT + GAP_CODE_OUT))
    return out


# ═══════════════════════════════════════════════════════════════════════════
# CHAPTER 1 — Text & flow
# ═══════════════════════════════════════════════════════════════════════════
def chapter_text():
    out = chapter_opener(
        1, "Text & flow",
        "Paragraphs, inline rich text, and lists — the words on the page.",
        "sec-text", "Text & flow")

    out += card(
        "paragraph", "sec-para",
        "The workhorse: a wrapped block of text. Options set size, weight, colour and alignment.",
        ['{ "type": "paragraph",',
         '  "value": "Invoice ${invoice.number}",',
         '  "options": { "fontSize": 16, "fontWeight": "bold", "color": "0F766E" } }'],
        [para("Invoice INV-2026-0042", fontSize=16, fontWeight="bold", color=TEAL)],
    )

    out += card(
        "alignment & justify", None,
        "alignment is left · center · right · justify. justify spreads each full line "
        "to both margins by widening word gaps; the last line stays left.",
        ['{ "type": "paragraph", "options": { "alignment": "justify" },',
         '  "value": "soli-pdf typesets text itself..." }'],
        [para("soli-pdf typesets text itself — measuring, wrapping and justifying every "
              "line in-process, with no headless browser in sight. This paragraph is set "
              "justify, so both edges line up while the final line stays left.",
              fontSize=10, color=SLATE_700, alignment="justify", lineHeight=1.4)],
    )

    out += card(
        "lineHeight & spacing", None,
        "lineHeight multiplies the leading (engine default 1.2); spacing adds points below "
        "the block, replacing a trailing move.",
        ['{ "type": "paragraph",',
         '  "options": { "lineHeight": 1.7 },',
         '  "value": "Loose leading lets dense reference text breathe." }'],
        [para("Loose leading lets dense reference text breathe — this block is set at "
              "lineHeight 1.7, so the wrapped lines sit further apart than the 1.2 default.",
              fontSize=10, color=SLATE_700, lineHeight=1.7)],
    )

    out += card(
        "spans — inline rich text", None,
        "spans mix weight, size, colour, links, underline and strike within one wrapped "
        "flow; each span inherits the paragraph options it omits.",
        ['{ "type": "paragraph", "spans": [',
         '  { "text": "Mix " }, { "text": "bold", "fontWeight": "bold" },',
         '  { "text": ", a ", }, { "text": "link", "link": "https://solilang.com",',
         '    "color": "2563eb", "underline": true },',
         '  { "text": ", and a redline was " },',
         '  { "text": "99.00", "strike": true, "color": "b91c1c" },',
         '  { "text": " now 79.00", "fontWeight": "bold", "color": "0F766E" } ] }'],
        [para(spans=[
            {"text": "Mix "},
            {"text": "bold", "fontWeight": "bold"},
            {"text": ", "},
            {"text": "italic", "italic": True},
            {"text": ", "},
            {"text": "mono", "mono": True},
            {"text": ", a "},
            {"text": "link", "link": "https://solilang.com", "color": LINK, "underline": True},
            {"text": ", and a redline was "},
            {"text": "99.00", "strike": True, "color": "b91c1c"},
            {"text": " now "},
            {"text": "79.00 EUR", "fontWeight": "bold", "color": TEAL},
        ], fontSize=10.5)],
    )

    out += card(
        "list — ordered · nested · custom marker", None,
        "ordered numbers from start; items may nest a list or carry spans; marker and "
        "indent tune the look.",
        ['{ "type": "list", "ordered": true, "items": [',
         '  "Grind the beans",',
         '  { "text": "Brew", "list": { "items": ["Bloom 30 s", "Pour to 250 g"] } },',
         '  { "spans": [ { "text": "Enjoy " },',
         '               { "text": "responsibly", "italic": true } ] } ] }'],
        [{"type": "list", "ordered": True, "options": {"fontSize": 10, "color": SLATE_700},
          "spacing": 3, "items": [
              "Grind the beans",
              {"text": "Brew", "list": {"items": ["Bloom 30 s", "Pour to 250 g"]}},
              {"spans": [{"text": "Enjoy "}, {"text": "responsibly", "italic": True}]},
          ]}],
        result_keep=70,
    )
    return out


# ═══════════════════════════════════════════════════════════════════════════
# CHAPTER 2 — Graphics & codes
# ═══════════════════════════════════════════════════════════════════════════
def chapter_graphics():
    out = chapter_opener(
        2, "Graphics & codes",
        "Boxes, strokes, images, and machine-readable codes.",
        "sec-graphics", "Graphics & codes")

    out += card(
        "move — the cursor primitive", None,
        "The layout primitive. +y is down, −y up, +x right. rect, line, ellipse, image, "
        "qr and barcode draw at the cursor without advancing it — move positions them "
        "(that is how these result rows are placed).",
        ['{ "type": "move", "x": 20, "y": -8 }'],
        None,
    )

    out += card(
        "rect — panels & boxes", None,
        "A filled and/or stroked box. radius rounds the corners; dash strokes the border. "
        "Drawn at the cursor, never advancing it.",
        ['{ "type": "rect", "width": 495, "height": 44,',
         '  "fill": "f0fdfa", "border": "99f6e4", "radius": 10 }'],
        boxed(240, 44, "rounded · filled · bordered", fill=TEAL_TINT,
              border="99f6e4", border_w=0.9, radius=10)
        + [move(255, -44)]
        + boxed(240, 44, '"dash": [5, 3]', label_color=SLATE_500,
                border="94a3b8", border_w=1, radius=6, dash=[5, 3])
        + [move(-255, 0)],
        result_keep=54,
    )

    out += card(
        "ellipse · line · hr — strokes & dots", None,
        "ellipse is a circle when rx == ry; line runs from the cursor to +(dx, dy); hr "
        "rules across the width (or an explicit width). line & hr take a dash too.",
        ['{ "type": "ellipse", "rx": 6, "ry": 6, "fill": "0F766E" }',
         '{ "type": "line", "dx": 170, "dy": 0, "dash": [5, 3] }',
         '{ "type": "hr", "color": "f59e0b", "thickness": 1.4, "width": 220 }'],
        [
            {"type": "ellipse", "rx": 6, "ry": 6, "fill": "16a34a"}, move(20, 0),
            {"type": "ellipse", "rx": 6, "ry": 6, "fill": "eab308"}, move(20, 0),
            {"type": "ellipse", "rx": 6, "ry": 6, "fill": "dc2626"}, move(28, -1),
            {"type": "ellipse", "rx": 24, "ry": 8, "fill": "e0f2fe",
             "border": "0284c7", "borderWidth": 0.8}, move(70, 6),
            {"type": "line", "dx": 170, "dy": 0, "color": "94a3b8", "width": 1,
             "dash": [5, 3]},
            move(-138, 16),
            hr(color=AMBER, thickness=1.4, width=220),
            move(0, 6),
        ],
    )

    out += card(
        "image — raster & SVG", None,
        "PNG · JPEG · WebP · GIF and SVG (auto-rasterised, so vector logos stay crisp). "
        "width alone derives the height from the aspect ratio.",
        ['{ "type": "image", "width": 150,',
         '  "value": "data:image/svg+xml,<svg ...>soli-pdf</svg>" }'],
        [{"type": "image", "width": 150, "value":
            "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 40' width='120' height='40'>"
            "<rect width='120' height='40' rx='8' fill='%230F766E'/>"
            "<circle cx='22' cy='20' r='11' fill='%2314b8a6'/>"
            "<path d='M16 21l4 4 8-9' stroke='white' stroke-width='2.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/>"
            "<text x='44' y='26' font-family='Titillium Web' font-size='16' fill='white' font-weight='bold'>soli-pdf</text></svg>"},
         move(0, 55)],
        result_keep=60,
    )

    out += card(
        "qr — scan-to-pay & text", None,
        'kind "epc" builds a SEPA GiroCode from the payment data — scan it in a banking '
        'app to pre-fill the transfer; kind "text" encodes any string. Both rasterised '
        'locally, no network.',
        ['{ "type": "qr", "kind": "epc", "width": 88,',
         '  "name": "${payment.name}", "iban": "${payment.iban}",',
         '  "amount": "${payment.amount}", "currency": "EUR",',
         '  "remittance": "${doc.ref}" }'],
        [
            {"type": "qr", "kind": "epc", "name": "${payment.name}",
             "iban": "${payment.iban}", "amount": "${payment.amount}",
             "currency": "EUR", "remittance": "${doc.ref}", "width": 88},
            move(116, 0),
            {"type": "qr", "kind": "text",
             "value": "https://solilang.com/docs/builtins/pdf", "width": 88},
            move(-116, 94),
            caption("left: EPC scan-to-pay (79.00 EUR to Soli Demo SARL)   ·   right: a URL"),
        ],
        result_keep=112,
    )

    out += card(
        "barcode — code128 · ean13", None,
        "1-D barcodes. code128 takes any printable ASCII; ean13 takes 12 digits and "
        "computes the check digit. humanReadable prints the caption under the bars.",
        ['{ "type": "barcode", "symbology": "code128", "value": "${doc.ref}",',
         '  "width": 210, "height": 46, "humanReadable": true }'],
        [
            {"type": "barcode", "symbology": "code128", "value": "${doc.ref}",
             "width": 210, "height": 46, "humanReadable": True},
            move(250, 0),
            {"type": "barcode", "symbology": "ean13", "value": "400638133393",
             "width": 185, "height": 46, "humanReadable": True},
            move(-250, 62),
        ],
        result_keep=68,
    )
    return out


# ═══════════════════════════════════════════════════════════════════════════
# CHAPTER 3 — Tables
# ═══════════════════════════════════════════════════════════════════════════
def chapter_tables():
    out = chapter_opener(
        3, "Tables",
        "Grids of cells — data-bound rows, merges, rich content, and per-table stamps.",
        "sec-tables", "Tables")

    out += card(
        "table — data binding · header · stripe", None,
        'data binds the template row to an array, repeating it per item. The '
        'header_columns band repeats on every page; options.stripe zebra-fills every '
        'second body row.',
        ['{ "type": "table", "data": "items",',
         '  "header_columns": [ { "text": "SERVICE", "width": 250 }, ... ],',
         '  "rows": [ [ { "text": "${name}", "width": 250 }, ... ] ],',
         '  "options": { "header": { "fillColor": "0F766E", "textColor": "FFFFFF" },',
         '               "stripe": "f1f5f9" } }'],
        [items_table()],
        result_keep=130,
    )

    out += card(
        "colspan & per-cell fill — a totals row", None,
        "colspan merges a cell across column slots; a cell's own fill paints its "
        "background over the stripe — the recipe for summary and total rows.",
        ['{ "rows": [ [',
         '  { "text": "TOTAL DUE", "colspan": 2, "alignment": "right",',
         '    "fontWeight": "bold" },',
         '  { "text": "${doc.total}", "alignment": "right", "fill": "fef3c7" } ] ] }'],
        [totals_table()],
    )

    out += card(
        "rich cells · borderSides · per-table watermark", None,
        "A rich cell stacks text lines and an image in one cell; borderSides sets each "
        "edge with a borderColor; the table carries its own centred watermark.",
        ['{ "type": "table",',
         '  "watermark": { "text": "SAMPLE", "fontSize": 44, "color": "fecaca" },',
         '  "rows": [ [ { "content": [ { "type": "text", ... },',
         '                             { "type": "image", ... } ] }, ... ] ] }'],
        # The rotated SAMPLE stamp overhangs the table box top and bottom, so pad
        # generously above and below it so it never crowds the label or the next card.
        [move(0, 14), rich_table(), move(0, 20)],
        result_keep=140,
        gap=GAP_CARD + 6,
    )

    out += card(
        "repeat — block-level data binding", None,
        "The block-level analogue of a data-bound row: lay out any elements once per "
        "array item, with ${field} scoped to each item.",
        ['{ "type": "repeat", "data": "milestones", "content": [',
         '  { "type": "paragraph", "spans": [',
         '    { "text": "${date}  ", "mono": true, "color": "64748b" },',
         '    { "text": "${title}", "fontWeight": "bold" },',
         '    { "text": " — ${note}" } ] } ] }'],
        [{"type": "repeat", "data": "milestones", "content": [
            {"type": "paragraph", "spans": [
                {"text": "${date}   ", "mono": True, "fontSize": 9, "color": SLATE_500},
                {"text": "${title}", "fontWeight": "bold", "color": INK},
                {"text": "  —  ${note}", "color": SLATE_600},
            ], "options": {"fontSize": 10}},
            move(0, 5),
        ]}],
        result_keep=66,
    )
    return out


def items_table():
    return {
        "type": "table", "data": "items",
        "header_columns": [
            {"text": "SERVICE", "width": 250, "fontWeight": "bold", "fontSize": 9.5},
            {"text": "QTY", "width": 60, "fontWeight": "bold", "fontSize": 9.5, "alignment": "right"},
            {"text": "AMOUNT", "width": 185, "fontWeight": "bold", "fontSize": 9.5, "alignment": "right"},
        ],
        "rows": [[
            {"text": "${name}", "width": 250, "fontSize": 10},
            {"text": "${qty}", "width": 60, "fontSize": 10, "alignment": "right"},
            {"text": "${amount}", "width": 185, "fontSize": 10, "alignment": "right"},
        ]],
        "options": {
            "header": {"fillColor": TEAL, "textColor": "FFFFFF", "borderColor": TEAL},
            "stripe": "f1f5f9", "padding_x": 8, "padding_y": 6,
        },
    }


def totals_table():
    return {
        "type": "table",
        "header_columns": [
            {"text": "", "width": 250}, {"text": "", "width": 60}, {"text": "", "width": 185},
        ],
        "rows": [[
            {"text": "TOTAL DUE", "colspan": 2, "alignment": "right", "fontWeight": "bold",
             "fontSize": 10.5, "borderSides": {"top": "true", "bottom": "true", "left": "true", "right": "true"},
             "borderColor": TEAL},
            {"text": "${doc.total}", "alignment": "right", "fontWeight": "bold", "fontSize": 10.5,
             "fill": "fef3c7", "borderSides": {"top": "true", "bottom": "true", "left": "true", "right": "true"},
             "borderColor": TEAL},
        ]],
        "options": {"padding_x": 8, "padding_y": 6},
    }


def rich_table():
    return {
        "type": "table",
        "watermark": {"text": "SAMPLE", "fontSize": 44, "color": "fecaca"},
        "rows": [[
            {"width": 300,
             "borderSides": {"top": "true", "bottom": "true", "left": "true", "right": "true"},
             "borderColor": TEAL,
             "content": [
                 {"type": "text", "value": "${payment.name}", "fontSize": 11, "fontWeight": "bold"},
                 {"type": "text", "value": "IBAN ${payment.iban}", "fontSize": 9},
                 {"type": "image", "width": 74, "value":
                     "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 120 40' width='120' height='40'>"
                     "<rect width='120' height='40' rx='8' fill='%230F766E'/>"
                     "<text x='12' y='26' font-family='Titillium Web' font-size='16' fill='white' font-weight='bold'>logo</text></svg>"},
             ]},
            {"text": 'valign "bottom"', "fontSize": 10, "alignment": "right", "valign": "bottom"},
        ]],
        "options": {"padding_x": 10, "padding_y": 9},
    }


# ═══════════════════════════════════════════════════════════════════════════
# CHAPTER 4 — Charts
# ═══════════════════════════════════════════════════════════════════════════
def chapter_charts():
    out = chapter_opener(
        4, "Charts",
        "Bar, line, pie and donut — drawn from the data, no plotting library.",
        "sec-charts", "Charts")

    out += card(
        "chart — grouped bars · multi-series · gridlines", None,
        "values is an array of { field, name, color }: each series reads its own field, "
        "bars group per category, and the legend lists the names. gridlines adds the "
        "value axis.",
        ['{ "type": "chart", "kind": "bar", "data": "quarters", "label": "q",',
         '  "gridlines": true, "width": 495, "height": 150,',
         '  "values": [ { "field": "fy25", "name": "FY 2025", "color": "94a3b8" },',
         '              { "field": "fy26", "name": "FY 2026", "color": "0F766E" } ] }'],
        [bar_chart(grouped=True)],
        result_keep=178,
    )

    out += card(
        "chart — stacked bars", None,
        'mode "stacked" piles the series per category instead of grouping them; a colors '
        "palette is cycled when series set no colour of their own.",
        ['{ "type": "chart", "kind": "bar", "mode": "stacked",',
         '  "data": "quarters", "label": "q", "gridlines": true,',
         '  "colors": [ "0F766E", "f59e0b" ],',
         '  "values": [ { "field": "fy25" }, { "field": "fy26" } ] }'],
        [bar_chart(grouped=False)],
        result_keep=178,
    )

    out += card(
        "chart — line", None,
        "One line per series over the category axis; share the same data binding as the "
        "bar charts.",
        ['{ "type": "chart", "kind": "line", "data": "quarters", "label": "q",',
         '  "width": 495, "height": 120,',
         '  "values": [ { "field": "fy26", "name": "FY 2026", "color": "0F766E" } ] }'],
        [{"type": "chart", "kind": "line", "title": "FY 2026 by quarter",
          "data": "quarters", "label": "q",
          "values": [{"field": "fy26", "name": "FY 2026", "color": TEAL}],
          "width": 495, "height": 120}],
        result_keep=148,
    )

    out += card(
        "chart — pie & donut", None,
        "points may be inline instead of data-bound. pie draws a swatch + percentage "
        'legend; donut is a pie with a ring cutout ("kind": "donut").',
        ['{ "type": "chart", "kind": "pie", "width": 240, "height": 130,',
         '  "points": [ { "label": "Templates", "value": 45 },',
         '              { "label": "Charts", "value": 25 }, ... ] }'],
        [pie_chart("pie", "Pie"), move(0, 6), pie_chart("donut", "Donut")],
        result_keep=312,
    )
    return out


def bar_chart(grouped):
    c = {
        "type": "chart", "kind": "bar", "data": "quarters", "label": "q",
        "gridlines": True, "width": 495, "height": 150,
    }
    if grouped:
        c["title"] = "Grouped — one bar per series"
        c["values"] = [
            {"field": "fy25", "name": "FY 2025", "color": "94a3b8"},
            {"field": "fy26", "name": "FY 2026", "color": TEAL},
        ]
    else:
        c["title"] = "Stacked — series pile per category"
        c["mode"] = "stacked"
        c["colors"] = [TEAL, AMBER]
        c["values"] = [{"field": "fy25", "name": "FY 2025"},
                       {"field": "fy26", "name": "FY 2026"}]
    return c


def pie_chart(kind, title):
    return {
        "type": "chart", "kind": kind, "title": title, "legend": True,
        "width": 495, "height": 128,
        "points": [
            {"label": "Templates", "value": 45},
            {"label": "Charts", "value": 25},
            {"label": "Codes", "value": 18},
            {"label": "Everything else", "value": 12},
        ],
    }


# ═══════════════════════════════════════════════════════════════════════════
# CHAPTER 5 — Layout & render options
# ═══════════════════════════════════════════════════════════════════════════
def chapter_layout():
    out = chapter_opener(
        5, "Layout & render options",
        "Multi-column flow, data-driven branches, and the options that live in the "
        "render call rather than the template.",
        "sec-layout", "Layout & render options")

    out += card(
        "columns — multi-column flow", None,
        "Children fill column 1 to the bottom, then column 2 (sequential fill); a "
        "page_break inside is a column break. Paragraphs, lists and images flow; tables "
        "and charts do not.",
        ['{ "type": "columns", "count": 2, "gap": 22, "content": [',
         '  { "type": "paragraph", "value": "flows down column 1..." },',
         '  { "type": "paragraph", "value": "...then into column 2." } ] }'],
        [{"type": "columns", "count": 2, "gap": 22, "content": [
            {"type": "paragraph", "value":
                "Column one begins here and fills top to bottom. When it reaches the "
                "bottom of the region the flow hops into column two automatically — "
                "sequential fill.", "options": {"fontSize": 9, "alignment": "justify",
                                                "color": SLATE_600, "spacing": 6}},
            {"type": "paragraph", "value":
                "This sentence has spilled into the second column, which is how you can "
                "tell the block is genuinely two columns filled left to right.",
                "options": {"fontSize": 9, "alignment": "justify", "color": TEAL,
                            "fontWeight": "bold"}},
        ]}, move(0, 6)],
        result_keep=90,
    )

    out += card(
        "if / unless — data-driven branches", None,
        "Render content only when a condition holds (if) or fails (unless); an optional "
        "else is the other branch. With equals it is a string test, otherwise a "
        "truthiness test.",
        ['{ "type": "if", "when": "doc.paid", "equals": "true",',
         '  "content": [ { "type": "paragraph", "value": "PAID IN FULL" } ],',
         '  "else":    [ { "type": "paragraph", "value": "Balance due" } ] }'],
        [{"type": "if", "when": "doc.paid", "equals": "true",
          "content": [para(spans=[
              {"text": "PAID IN FULL", "fontWeight": "bold", "color": "16a34a"},
              {"text": "   —   doc.paid is \"true\", so the content branch renders "
                       "(the else branch is skipped).",
               "color": SLATE_500, "fontSize": 9},
          ], fontSize=10.5)],
          "else": [para("Balance due", color="b91c1c")]}],
    )

    # The one card whose sample must show token syntax literally, so it uses the
    # spans code box (page tokens are not substituted inside spans).
    interp = [heading("interpolation & page tokens", bookmark="interpolation & page tokens",
                      keep=170),
              move(0, GAP_HEAD_DESC),
              desc("A dotted path pulls from the data; page tokens resolve after "
                   "pagination and turn a contents list into real page numbers. Print a "
                   "literal brace token by doubling the dollar."),
              move(0, GAP_DESC_CODE)]
    interp += spans_code_box([
        [{"text": '"Invoice '}, {"text": "$${invoice.number}", "color": TEAL},
         {"text": ' — page '}, {"text": "#PAGE#", "color": TEAL},
         {"text": ' of '}, {"text": "#TOTAL_PAGE#", "color": TEAL}, {"text": '"'}],
        [{"text": '"See charts on p. '}, {"text": "#PAGE_OF:sec-charts#", "color": TEAL},
         {"text": '"   with '}, {"text": '"linkTo": "sec-charts"', "color": SLATE_500}],
        [{"text": '"'}, {"text": "$${", "color": "b45309"},
         {"text": ' prints a literal '}, {"text": "${", "color": "b45309"},
         {"text": '"   — this document\'s code boxes rely on it', "color": SLATE_500}],
    ])
    interp += [renders_label(), para(spans=[
        {"text": "The running header and footer of this page both resolve "},
        {"text": "#PAGE#", "mono": True, "color": TEAL},
        {"text": " / "},
        {"text": "#TOTAL_PAGE#", "mono": True, "color": TEAL},
        {"text": ", and the cover's contents used "},
        {"text": "#PAGE_OF:…#", "mono": True, "color": TEAL},
        {"text": " for its page numbers."},
    ], fontSize=10, color=SLATE_700, lineHeight=1.4), move(0, GAP_CARD)]
    out += interp

    out += card(
        "document options", None,
        "The options block sets page size, margins, the reserved header_height, a page "
        "background or backgroundImage (this cover's wash — opacity fades it to a soft "
        "tint), a document watermark, and tagged for accessible output.",
        ['"options": {',
         '  "page": "a4", "margins": { "top": 42, "left": 50 },',
         '  "header_height": 34,',
         '  "watermark": { "text": "DRAFT", "front": true, "pages": "first" },',
         '  "backgroundImage": { "src": "...", "pages": "first",',
         '                       "opacity": 0.15 } }'],
        None,
    )

    out += card(
        "the render call — pdf_render(tpl, data, options)", None,
        "Some features live in the call, not the template JSON: a letterhead to draw on, "
        "files to embed, Info-dictionary metadata, a password, or PDF/A archival output.",
        ['pdf_render(tpl, data, {',
         '  "stationery": "pdf/letterhead.pdf",   # draw on a letterhead PDF',
         '  "attachments": [{ "path": "data.csv" }],',
         '  "title": "Invoice 42", "author": "Soli",',
         '  "font_dirs": ["font"], "fetch_images": false })'],
        None,
    )

    # Closing band.
    out += [
        move(0, 4),
        rect(CONTENT_W, 58, fill=TEAL, radius=12),
        rect(CONTENT_W, 4, fill=AMBER, radius=2),
        move(22, 13),
        para("Built with Soli — one JSON template, one pdf_render() call.",
             fontSize=13, fontWeight="bold", color="ffffff"),
        para("solilang.com/docs/builtins/pdf  ·  try this very template live in the playground",
             fontSize=9, color=TEAL_PALE, link="https://solilang.com/docs/builtins/pdf"),
        move(-22, 20),
    ]
    return out


# ═══════════════════════════════════════════════════════════════════════════
def build():
    content = []
    content += cover()
    content += chapter_text()
    content += chapter_graphics()
    content += chapter_tables()
    content += chapter_charts()
    content += chapter_layout()
    return content


# ═══════════════════════════════════════════════════════════════════════════
# Data document (kept in step with the template above)
# ═══════════════════════════════════════════════════════════════════════════
def data_doc():
    return {
        "data": {
            "invoice": {"number": "INV-2026-0042"},
            "doc": {"ref": "TOUR-2026-001", "paid": "true", "total": "1,940.00 EUR"},
            "payment": {"name": "Soli Demo SARL",
                        "iban": "FR7630006000011234567890189", "amount": "79.00"},
            "items": [
                {"name": "Template engine", "qty": "1", "amount": "890.00 EUR"},
                {"name": "Chart pack (bar · line · pie · donut)", "qty": "4", "amount": "450.00 EUR"},
                {"name": "QR & barcode primitives", "qty": "2", "amount": "300.00 EUR"},
                {"name": "Factur-X / PDF-A-3b embedding", "qty": "1", "amount": "300.00 EUR"},
            ],
            "milestones": [
                {"date": "2026-03", "title": "Charts & lists", "note": "bar, line, pie, nested lists"},
                {"date": "2026-06", "title": "SVG & barcodes", "note": "crisp vector logos, Code 128 / EAN"},
                {"date": "2026-07", "title": "Flow & polish", "note": "columns, tokens, the ${ escape"},
            ],
            "quarters": [
                {"q": "Q1", "fy25": 120, "fy26": 180},
                {"q": "Q2", "fy25": 150, "fy26": 210},
                {"q": "Q3", "fy25": 170, "fy26": 260},
                {"q": "Q4", "fy25": 160, "fy26": 300},
            ],
        }
    }


def template():
    return {
        "fonts": ["TitilliumWeb"],
        "options": {
            "page": "a4",
            "margins": {"top": 42, "right": 50, "bottom": 46, "left": 50},
            "header_height": 34,
            "backgroundImage": {
                "src": "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 595 842'>"
                       "<defs><linearGradient id='g' x1='0' y1='0' x2='0' y2='1'>"
                       "<stop offset='0' stop-color='%23ecfeff'/><stop offset='0.4' stop-color='%23ffffff'/>"
                       "<stop offset='1' stop-color='%23f8fafc'/></linearGradient></defs>"
                       "<rect width='595' height='842' fill='url(%23g)'/>"
                       "<g opacity='0.05' fill='none' stroke='%230F766E' stroke-width='10'>"
                       "<circle cx='560' cy='788' r='150'/><circle cx='560' cy='788' r='96'/>"
                       "<circle cx='560' cy='788' r='44'/></g></svg>",
                "pages": "first",
            },
        },
        "header": [
            {"type": "image", "width": 15, "value":
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' width='24' height='24'>"
                "<circle cx='12' cy='12' r='10' fill='%230F766E'/>"
                "<path d='M7 13l3 3 7-8' stroke='white' stroke-width='2.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/></svg>"},
            move(0, 1),
            para("the soli-pdf template reference", alignment="right",
                 fontSize=8.5, color=SLATE_400),
            move(0, 4),
            hr(color=HAIR, thickness=0.6),
        ],
        "footer": [
            para("soli-pdf  ·  page #PAGE# of #TOTAL_PAGE#", alignment="center",
                 fontSize=8, color=SLATE_400),
        ],
        "content": build(),
    }


if __name__ == "__main__":
    import sys
    tpl = sys.argv[1] if len(sys.argv) > 1 else "features.template.json"
    with open(tpl, "w") as f:
        json.dump(template(), f, indent=2, ensure_ascii=False)
    print("wrote", tpl)
    if len(sys.argv) > 2:
        dat = sys.argv[2]
        with open(dat, "w") as f:
            json.dump(data_doc(), f, indent=2, ensure_ascii=False)
        print("wrote", dat)
