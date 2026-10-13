// Default invoice design. Reads everything from /data.json (built by
// src/pdf/payload.rs): every value is pre-formatted and every label resolved,
// so this file holds layout only. A space overrides any file via its design (`invoiceapp design push`, Settings).

#let d = json("/data.json")

#let ink = rgb("#16181d")
#let muted = rgb("#6b7280")
#let rule = rgb("#e3e5e8")
#let accent = rgb("#0f766e")

#let has(v) = v != none and v != ""

#set document(title: if d.number != none { d.title + " " + d.number } else { d.title })
#set text(font: "Inter", size: 8.8pt, fill: ink, lang: d.locale, number-width: "tabular")
#set par(leading: 0.55em, spacing: 0.9em)

#set page(
  paper: "a4",
  margin: (x: 17mm, top: 16mm, bottom: 22mm),
  background: if d.watermark != none {
    rotate(-35deg, text(size: 96pt, weight: "bold", fill: rgb(15, 118, 110, 22), tracking: 6pt, d.watermark))
  },
  footer: context {
    set text(size: 7.4pt, fill: muted)
    line(length: 100%, stroke: 0.5pt + rule)
    v(-1.5mm)
    grid(
      columns: (1fr, auto),
      column-gutter: 8mm,
      align: (left + top, right + top),
      if has(d.footer.registration) { d.footer.registration },
      [#d.footer.pageLabel #counter(page).display() / #counter(page).final().first()],
    )
  },
)

#let caption(body) = text(size: 7pt, weight: "semibold", fill: muted, tracking: 0.6pt, upper(body))

#let kv(rows) = if rows.len() > 0 {
  grid(
    columns: (auto, 1fr),
    column-gutter: 5mm,
    row-gutter: 1.9mm,
    ..rows.map(r => (text(fill: muted, r.label), text(weight: "medium", r.value))).flatten(),
  )
}

#let party(p) = {
  caption(p.title)
  v(2.2mm)
  text(size: 10.5pt, weight: "semibold", p.name)
  v(1mm)
  for l in p.lines { [#l \ ] }
  if p.ids.len() > 0 {
    v(0.6mm)
    for l in p.ids { [#l \ ] }
  }
  if p.contact.len() > 0 {
    v(0.6mm)
    for l in p.contact { text(fill: muted, l); linebreak() }
  }
}

// ---- Header -----------------------------------------------------------------
#grid(
  columns: (1fr, auto),
  align: (left + horizon, right + horizon),
  if d.assets.logo != none {
    image(d.assets.logo, height: 13mm)
  } else {
    text(size: 13pt, weight: "bold", d.supplier.name)
  },
  {
    text(size: 8pt, weight: "semibold", fill: muted, tracking: 0.6pt, upper(d.title))
    if d.number != none {
      linebreak()
      text(size: 19pt, weight: "bold", fill: accent, d.number)
    }
  },
)
#v(4mm)
#line(length: 100%, stroke: 1.2pt + accent)
#v(5mm)

// ---- Parties ----------------------------------------------------------------
#grid(
  columns: (1fr, 1fr),
  column-gutter: 10mm,
  party(d.supplier), if d.customer != none { party(d.customer) },
)
#v(6mm)

// ---- Dates and payment ------------------------------------------------------
#block(
  width: 100%,
  inset: (x: 4mm, y: 3.5mm),
  fill: rgb("#f6f7f8"),
  radius: 1.5mm,
  grid(
    columns: (2fr, 3fr),
    column-gutter: 8mm,
    kv(d.dates), kv(d.payment),
  ),
)

#if has(d.reference) {
  v(4mm)
  text(weight: "medium", d.reference)
}
#if has(d.headerNote) {
  v(4mm)
  d.headerNote
}
#v(6mm)

// ---- Lines ------------------------------------------------------------------
#let cols = (
  (key: "description", width: 1fr, align: left),
  (key: "quantity", width: auto, align: right),
  (key: "unitPrice", width: auto, align: right),
  ..if d.hasDiscount { ((key: "discount", width: auto, align: right),) },
  ..if d.showVat { ((key: "vatRate", width: auto, align: right),) },
  (key: "base", width: auto, align: right),
)

#let cell-value(line, key) = {
  let v = line.at(key, default: none)
  if v == none { [] } else if line.strong { text(weight: "semibold", v) } else { v }
}

#table(
  columns: cols.map(c => c.width),
  align: cols.map(c => c.align),
  stroke: none,
  inset: (x: 1.6mm, y: 2.1mm),
  table.header(
    ..cols.map(c => caption(d.columns.at(c.key))),
    table.hline(stroke: 0.8pt + ink),
  ),
  ..d.lines.map(line => {
    if line.kind == "text" {
      (table.cell(colspan: cols.len(), text(fill: muted, style: "italic", line.description)),)
    } else {
      let row = cols.map(c => cell-value(line, c.key))
      if line.strong {
        (table.hline(stroke: 0.5pt + muted), ..row)
      } else {
        row
      }
    }
  }).map(r => (..r, table.hline(stroke: 0.4pt + rule))).flatten(),
)
#v(5mm)

// ---- VAT recap and totals ---------------------------------------------------
#let recap(r) = {
  caption(r.title)
  if has(r.at("rateNote", default: none)) {
    h(2mm)
    text(size: 7.4pt, fill: muted, r.rateNote)
  }
  v(1mm)
  table(
    columns: (auto, 1fr, 1fr, 1fr),
    align: (left, right, right, right),
    stroke: none,
    inset: (x: 1.4mm, y: 1.6mm),
    table.header(
      ..(r.columns.rate, r.columns.base, r.columns.vat, r.columns.total).map(t => text(size: 7.4pt, fill: muted, t)),
      table.hline(stroke: 0.5pt + rule),
    ),
    ..r.rows.map(x => (x.rate, x.base, x.vat, x.total)).flatten(),
    table.hline(stroke: 0.5pt + rule),
    ..(r.total.rate, r.total.base, r.total.vat, r.total.total).map(t => text(weight: "semibold", t)),
  )
}

#let totals = {
  grid(
    columns: (1fr, auto),
    column-gutter: 6mm,
    row-gutter: 2.2mm,
    align: (left + horizon, right + horizon),
    ..d.totals.map(t => if t.strong {
      (
        grid.hline(stroke: 0.8pt + ink),
        grid.cell(inset: (top: 2.4mm), text(size: 10pt, weight: "semibold", t.label)),
        grid.cell(inset: (top: 2.4mm), text(size: 13pt, weight: "bold", fill: accent, t.value)),
      )
    } else {
      (text(fill: muted, t.label), t.value)
    }).flatten(),
  )
}

#block(breakable: false, grid(
  columns: (1fr, 68mm),
  column-gutter: 10mm,
  {
    if d.vatRecap != none { recap(d.vatRecap) }
    if d.vatRecapCzk != none {
      v(3mm)
      recap(d.vatRecapCzk)
    }
  },
  totals,
))

// ---- QR, notes, signature ---------------------------------------------------
#v(7mm)
#let notes = (d.legalNote, d.paidNote, d.footerNote).filter(has)

#let qr-box = if d.qr != none {
  box(stroke: 0.5pt + rule, inset: 2.5mm, radius: 1.5mm, {
    image(d.qr.image, width: 27mm)
    v(-1mm)
    align(center, text(size: 7pt, fill: muted, d.qr.label))
  })
}

#block(breakable: false, grid(
  columns: if qr-box != none { (auto, 1fr, auto) } else { (1fr, auto) },
  column-gutter: 7mm,
  align: (x, _) => if x == 0 or (x == 1 and qr-box != none) { left + top } else { right + bottom },
  ..if qr-box != none { (qr-box,) },
  {
    for n in notes {
      if n == d.paidNote { text(weight: "semibold", fill: accent, n) } else { n }
      parbreak()
    }
  },
  {
    if d.assets.signature != none {
      image(d.assets.signature, height: 18mm)
      v(-1mm)
    } else {
      v(14mm)
    }
    line(length: 52mm, stroke: 0.5pt + muted)
    v(-1mm)
    text(size: 7.4pt, fill: muted, d.footer.issuedBy)
  },
))
