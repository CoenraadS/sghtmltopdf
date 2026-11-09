# HTML form inputs → PDF AcroForm fields

## Goal

Every HTML form control that has an equivalent in the PDF standard
(ISO 32000 interactive forms, “AcroForm”) is emitted as a real, fillable
PDF field — not only as a static drawing. Controls with no PDF-standard
equivalent keep today's static rendering, and that split is explicit.

## Success criteria

- A document containing the supported controls produces a PDF whose
  catalogue carries `/AcroForm /Fields […]`, whose pages carry the widget
  annotations in `/Annots`, and which viewers (Acrobat, pdf.js, browsers)
  open as a fillable form with the HTML values prefilled.
- Field identity follows HTML: `name` is the field name (`id` fallback,
  generated `field_N` last), a radio `name` group is one field, and
  same-named checkboxes are one field with several widgets.
- Static appearance is preserved: the page content stream still draws
  each control as today, so printed / non-interactive rendering is
  unchanged in layout. `/NeedAppearances true` lets interactive viewers
  regenerate field appearance from the field values.
- Batch (`encode_pdf`) and streaming (`StreamingPdfWriter`) output both
  carry the fields. No `/AcroForm` is written when there are no fields.
- Existing behaviour for non-form documents is byte-unchanged in
  structure (no new objects), and the existing test suite still passes.

## Mapping (HTML → PDF standard part)

| HTML | PDF part | Notes |
|---|---|---|
| `input type=text, search, tel, url, email` and missing/unknown `type` | `/FT /Tx`, single-line | `value` → `/V`+`/DV`; `maxlength` → `/MaxLen` |
| `input type=password` | `/Tx` + `PASSWORD` flag (Ff bit 14) | value carried, not echoed by viewers |
| `input type=number` | `/Tx` | PDF has no numeric field type; `min`/`max`/`step` have no standard field keys and are not emitted |
| `input type=date, datetime-local, month, time` | `/Tx` + `/AA` format/keystroke JavaScript (`AFDate_*` / `AFTime_*`) | This is the standard way a text field becomes a date/time picker in Acrobat-class viewers; the submitted value stays the HTML value (`yyyy-mm-dd`, …) |
| `input type=week` | `/Tx` | ISO week strings (`2026-W45`) have no `AFDate` pattern, so no format action is attached |
| `textarea` | `/Tx` + `MULTILINE` flag (Ff bit 13) | text content → `/V`+`/DV`; `maxlength` → `/MaxLen` |
| `input type=checkbox` | `/FT /Btn` checkbox | on-state name is the sanitised `value` (default `Yes`, PDF's conventional on-state; HTML's default `on` is kept when explicitly given — see Decisions); `checked` → on-state, else `/Off`. Same `name` → one field, one widget per box, `/Opt` lists the export values |
| `input type=radio` | `/FT /Btn` + `RADIO` + `NO_TOGGLE_TO_OFF` flags | One field per `name` group, one widget kid per button, `/V` is the checked button's export value or `/Off` |
| `select` (single, `size<=1`) | `/FT /Ch` + `COMBO` flag | A combo box (drop-down) |
| `select multiple` or `size>1` | `/FT /Ch` (+ `MULTI_SELECT` when `multiple`) | A list box |
| `option` | one `/Opt` entry | Entry is `[export display]` when `value` differs from the option text, else the plain string. `selected` → `/V` (and `/I` for multi-select) |
| `optgroup` | grouping only — never an `/Opt` entry itself | See “Grouping” below. Its options are flattened in document order; a `disabled` optgroup disables its options |
| `button`, `input type=submit, reset, button, image` | `/FT /Btn` + `PUSHBUTTON` flag | Caption (`value` or element text, `Submit`/`Reset` defaults) → `/MK /CA`. A `submit` inside `<form action="…">` also gets a `SubmitForm` action, `reset` a `ResetForm` action |
| `datalist` (with `input list=`) | the input stays `/Tx`; the referenced options become an editable combo's options only when the input itself is a `select`-equivalent — otherwise no field change | `datalist` alone has no field type; documented as non-goal below |

Common attributes, all field kinds:

| HTML | PDF |
|---|---|
| `name` | `/T` (`.` → `_`: a partial name may not contain a period) |
| `id` | `/T` fallback when `name` is absent/empty |
| (neither) | generated `/T field_N` in document order |
| `title`, else `placeholder` | `/TU` (user-facing name) |
| `required` | `REQUIRED` flag (Ff bit 2) |
| `readonly`, `disabled` | `READ_ONLY` flag (Ff bit 1); the widget stays visible with its static grey rendering |
| computed `font-size`, page position/size | `/DA` (`/Helv <size> Tf 0 g`, size from layout, `0` = auto) and the widget `/Rect` from the laid-out border box — the same rectangle the static drawing uses |

### No PDF-standard equivalent — static rendering kept (non-goals)

- `input type=hidden`: no widget. It is `display:none` today and stays
  invisible; PDF has no “hidden input” widget concept.
- `input type=file, color, range`: ISO 32000 defines no file-picker,
  colour-picker or slider field. (`FILE_SELECT` exists only as a text/
  choice-field flag meaning “the value is a file path to submit”, which
  is not a picker UI, so it is not used to fake one.)
- `<progress>`, `<meter>`, `<output>`: no field type.
- `/FT /Sig` (signature field): no HTML control maps to it, so no
  signature field is invented.
- HTML form submission semantics beyond the pushbutton actions above
  (validation, `pattern`, `inputmode`, `autocomplete`): no standard
  field keys exist for them.

### Grouping (`optgroup`) — what “grouped correctly” means here

ISO 32000 choice fields are flat: `/Opt` is an array of strings or
`[export display]` pairs (see `pdf_writer::ChoiceOptions`), with no
hierarchy entry. So grouping is preserved as follows, and this is the
contract the tests pin:

1. An `<optgroup>` never becomes a selectable `/Opt` entry — its
   `label` is not offered as a value.
2. Options inside groups appear in `/Opt` in document order, groups in
   document order (no reordering, no deduplication).
3. An option's display string is prefixed with its group label
   (`Group – Option`) when it is inside an `<optgroup>`, so the grouping
   stays visible in the drop-down/list while the export value (`/V`,
   the submitted data) remains the plain option `value`/text.
4. A `disabled` `<optgroup>` makes its options unselectable in HTML;
   in the flat PDF list they are kept (PDF has no per-option disabled
   key) but a `selected` option inside a disabled group does not become
   the field value.
5. Radio/checkbox grouping is by `name` as described above; controls
   without a `name` never merge with each other.

## Approach

Follow the existing `<a href>` → `/Link` annotation path, which already
solves “DOM data that layout must carry to the PDF layer”:

1. **Extract at box-tree time** (`layout::box_tree`, new
   `layout::form` module): `form_field_for(dom, node) -> Option<Rc<FormField>>`
   reads the element's attributes (and, for `select`, its
   `option`/`optgroup` subtree; for `textarea`/`button`, its text).
   Stored on `LayoutBox.form_field`, next to `LayoutBox.link`.
2. **Carry through layout**: `LaidOutBox.form_field` is set from the
   `LayoutBox` in the one real box constructor (`layout::block`);
   pagination fragments and synthesised boxes leave it `None`, and the
   PDF collector dedupes by `NodeId` so a split box cannot double-emit.
   Inline controls reach the PDF layer as the `AtomicInline`'s
   `LaidOutBox`, exactly like atomic link areas do.
3. **Collect per page** (`pdf::form`): walk a page's boxes like
   `collect_link_areas`, producing `FormWidget { field, rect }` with the
   border box converted to PDF coordinates (margin offset, Y-flip,
   before the output scale — the scale is applied when writing, as for
   links).
4. **Group and write** (`pdf::form`): group radio/same-name-checkbox
   widgets into single parent fields; every other widget becomes a
   merged field+widget annotation. Refs are allocated through the
   existing `RefAllocator` (batch) / `StreamingPdfWriter::alloc`.
   Per page, widget refs join `/Annots` next to the link refs. After all
   pages: root field refs go to `catalog.form().fields(…)`, with
   `/NeedAppearances true` and `/DA` + `/DR` (a standard Type1
   `/Helvetica` under `/Helv`, WinAnsi) so viewers can regenerate
   appearances in the document's default field font. Field text values
   use `TextStr` (PDFDocEncoding/UTF-16BE as needed); appearance-state
   names (`/AS`, button `/V`) are sanitised to PDF name tokens
   (`#XX`-free: characters outside the regular name set are replaced by
   `_`, empty falls back to `Yes`/`ValueN`).
5. **Streaming**: `write_page` collects that page's widgets, writes
   merged field+widgets immediately (radios/same-name checkboxes write
   their widget kids immediately and record the group), and `finish`
   writes the deferred parent fields plus the catalogue `/AcroForm`.
   Nothing about layout or page content is retained — only the small
   field records, as with destinations/bookmarks today.
6. **Docs/CLI surface**: `--enable-forms` stops being “unsupported”
   (forms are now generated); `--disable-forms` becomes a real option
   that suppresses field emission (static rendering only), threaded as
   `PdfOutputOptions.forms` (default `true`) through the CLI/Ruby/server
   option table the same way as the link switches. The limitations page
   and this mapping are updated to match.

## Steps

1. `core/src/layout/form.rs`: field data types + DOM extraction,
   including `optgroup`-aware option collection and name sanitising.
   Unit tests for extraction (each input type, grouping, naming).
2. `box_tree.rs` / `block.rs` (+ `paginate.rs` constructors): carry
   `form_field` from `LayoutBox` to `LaidOutBox`.
3. `core/src/pdf/form.rs`: per-page collection, grouping, and the
   writers shared by batch and streaming (one function writes a merged
   field/widget or a parent+widgets group into a `Chunk`/`Pdf`).
4. `pdf/document.rs`: allocate/write fields per page, add refs to
   `/Annots`, write `/AcroForm` on the catalogue; honour
   `PdfOutputOptions.forms`.
5. `pdf/streaming.rs`: same per page; parents + `/AcroForm` in `finish`.
6. CLI: move `--enable-forms`/`--disable-forms` out of
   `cli/unsupported.rs` into real options; document in the options docs.
7. Tests + docs: new `core/tests/acroform.rs` (below), update
   `docs/src/appendix/limitations.md`, add a changelog entry.

## Validation plan

- Focused, maintained tests in `core/tests/acroform.rs` (the project's
  normal test layout; each asserts on the emitted PDF bytes):
  one per row of the mapping table — text/password/number/date(+`AFDate`
  action)/textarea-multiline, checkbox checked+unchecked, a 3-button
  radio group (one `/Btn` field, three widgets, `/V` = checked export),
  same-name checkboxes grouped, single `select` → `/Ch`+`COMBO` with
  `[export display]` pairs, `optgroup` (label absent from `/Opt`,
  grouped display prefix, order, disabled-group selection rule),
  multi-select → list box + `MULTI_SELECT` + `/I`, pushbuttons
  (+SubmitForm/ResetForm actions), `required`/`readonly`/`disabled`/
  `maxlength`/naming (name, id fallback, generated), hidden/file/color/
  range producing no widget, a no-form document producing no
  `/AcroForm`, and a multi-page document (widgets on the right pages'
  `/Annots`). Streaming parity: one test through
  `StreamingPdfWriter` asserting the same field markers.
  First run these against the unmodified code to observe them failing
  (no `/AcroForm` today), then implement until green.
- `cargo test` (workspace default features) once after the final edit;
  `cargo test --test acroform` during development.
- `cargo clippy` / `cargo fmt --check` if the repo's CI names them —
  check `.github/workflows` before finishing and run what it runs.
- Manual black-box check: render a sample form HTML with the CLI,
  confirm the PDF opens as fillable (field markers present via the
  byte-level assertions above are the automatable proxy for this).

## Risks / open questions

- **Appearance streams**: the plan relies on `/NeedAppearances true`
  plus the unchanged static page drawing instead of hand-writing `/AP`
  streams per widget. Risk: a viewer that honours neither would show
  the static drawing only — still correct-looking, just not editable
  there. Rejected alternative: generating `/AP` for every kind
  (checkbox state dictionaries, text appearance with the embedded
  subset fonts) — large, and subset Type0 fonts make text `/AP`
  fragile. `/AP` is left as follow-up work, noted in the docs.
- **Duplicate text-field names**: two same-named text inputs become
  two merged field+widgets with identical `/T` and `/V`, which PDF
  treats as one field shown twice (value mirroring) — the HTML
  “submit both values” nuance cannot be represented in AcroForm.
- **Checkbox on-state naming**: PDF convention is `/Yes`; when the
  author gave an explicit `value`, it (sanitised) is the export/on
  state so submitted data matches the HTML form. With no `value`, the
  on-state is `Yes` rather than HTML's `on`, following PDF convention.
- None of this changes layout: field emission is read-only over the
  laid-out tree, so pagination/geometry regressions are out of scope —
  the existing pagination/link tests are the guard.
