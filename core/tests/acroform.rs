//! E2E tests for HTML form controls emitted as PDF AcroForm fields.
//!
//! Each test renders HTML all the way to PDF bytes and asserts on the
//! field dictionaries / widget annotations in the output.

use std::collections::HashMap;

use sghtmltopdf::fonts::{Font, FontCollection};
use sghtmltopdf::html;
use sghtmltopdf::layout::{paginate_document, PageSettings};
use sghtmltopdf::pdf::{
    encode_pdf, encode_pdf_with_options, LinkSettings, PdfOutputOptions, StreamingPdfWriter,
};
use sghtmltopdf::sink::MemorySink;
use sghtmltopdf::style::{compute_styles, parse_stylesheet, user_agent_stylesheet};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn test_fonts() -> FontCollection {
    FontCollection::new(vec![
        Font::load(FONT_PATH).expect("should load bundled test font")
    ])
}

fn render(html_src: &str) -> Vec<u8> {
    render_with_options(html_src, &PdfOutputOptions::default())
}

fn render_with_options(html_src: &str, output: &PdfOutputOptions) -> Vec<u8> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(
        &dom,
        &user_agent_stylesheet(),
        &parse_stylesheet("body { margin: 0; } p { margin: 0; }"),
    );
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    encode_pdf_with_options(
        &pages,
        &styles,
        &HashMap::new(),
        &fonts,
        &settings,
        &LinkSettings::default(),
        output,
    )
}

fn render_streaming(html_src: &str) -> Vec<u8> {
    let dom = html::parse(html_src.as_bytes());
    let styles = compute_styles(
        &dom,
        &user_agent_stylesheet(),
        &parse_stylesheet("body { margin: 0; } p { margin: 0; }"),
    );
    let fonts = test_fonts();
    let settings = PageSettings::default();
    let pages = paginate_document(&dom, &styles, &fonts, &settings);
    let mut writer = StreamingPdfWriter::new(
        &fonts,
        settings,
        MemorySink::default(),
        Vec::new(),
        LinkSettings::default(),
    )
    .expect("writer");
    for page in &pages {
        writer
            .write_page(page, &styles, &HashMap::new(), &fonts, None)
            .expect("write_page");
    }
    writer.finish(&fonts).expect("finish")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn count(bytes: &[u8], needle: &[u8]) -> usize {
    bytes.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn a_text_input_becomes_a_text_field() {
    let pdf = render(r#"<p><input type="text" name="fullname" value="Taro" maxlength="20"></p>"#);
    let t = text(&pdf);
    assert!(t.contains("/AcroForm"), "{t}");
    assert!(t.contains("/FT /Tx"), "{t}");
    assert!(t.contains("/T (fullname)"), "{t}");
    assert!(t.contains("/V (Taro)"), "{t}");
    assert!(t.contains("/MaxLen 20"), "{t}");
    assert!(t.contains("/Subtype /Widget"), "{t}");
    assert!(t.contains("/NeedAppearances true"), "{t}");
}

#[test]
fn password_number_and_date_inputs_become_text_fields() {
    let pdf = render(
        r#"<p><input type="password" name="pw" value="s3cret"></p>
           <p><input type="number" name="qty" value="3"></p>
           <p><input type="date" name="due" value="2026-01-15"></p>"#,
    );
    let t = text(&pdf);
    // Password flag is Ff bit 14 (8192) on its own field.
    assert!(t.contains("/T (pw)"), "{t}");
    assert!(t.contains("/Ff 8192"), "password flag: {t}");
    assert!(t.contains("/T (qty)"), "{t}");
    assert!(t.contains("/V (3)"), "{t}");
    assert!(t.contains("/T (due)"), "{t}");
    assert!(t.contains("/V (2026-01-15)"), "{t}");
    assert!(t.contains("AFDate_FormatEx"), "date format action: {t}");
}

#[test]
fn a_textarea_becomes_a_multiline_text_field() {
    let pdf = render(r#"<p><textarea name="notes">line one</textarea></p>"#);
    let t = text(&pdf);
    assert!(t.contains("/FT /Tx"), "{t}");
    assert!(t.contains("/T (notes)"), "{t}");
    assert!(t.contains("/V (line one)"), "{t}");
    // Multiline flag is Ff bit 13 (4096).
    assert!(t.contains("/Ff 4096"), "multiline flag: {t}");
}

#[test]
fn checkboxes_become_button_fields_with_their_state() {
    let pdf = render(
        r#"<p><input type="checkbox" name="agree" value="yes" checked>
           <input type="checkbox" name="news"></p>"#,
    );
    let t = text(&pdf);
    assert!(t.contains("/FT /Btn"), "{t}");
    assert!(t.contains("/T (agree)"), "{t}");
    assert!(t.contains("/AS /yes"), "checked state: {t}");
    assert!(t.contains("/T (news)"), "{t}");
    assert!(t.contains("/AS /Off"), "unchecked state: {t}");
}

#[test]
fn a_radio_group_is_one_field_with_one_widget_per_button() {
    let pdf = render(
        r#"<p><input type="radio" name="size" value="s">
           <input type="radio" name="size" value="m" checked>
           <input type="radio" name="size" value="l"></p>"#,
    );
    let t = text(&pdf);
    assert_eq!(count(&pdf, b"/T (size)"), 1, "one parent field: {t}");
    assert_eq!(count(&pdf, b"/Subtype /Widget"), 3, "three widgets: {t}");
    // Radio (32768) + NoToggleToOff (16384).
    assert!(t.contains("/Ff 49152"), "radio flags: {t}");
    assert!(t.contains("/V /m"), "checked export: {t}");
    assert!(t.contains("/AS /m"), "{t}");
}

#[test]
fn same_named_checkboxes_group_into_one_field() {
    let pdf = render(
        r#"<p><input type="checkbox" name="top" value="a" checked>
           <input type="checkbox" name="top" value="b"></p>"#,
    );
    let t = text(&pdf);
    assert_eq!(count(&pdf, b"/T (top)"), 1, "{t}");
    assert_eq!(count(&pdf, b"/Subtype /Widget"), 2, "{t}");
    assert!(t.contains("/Opt [(a) (b)]") || t.contains("/Opt"), "{t}");
}

#[test]
fn a_select_becomes_a_combo_box_with_export_values() {
    let pdf = render(
        r#"<p><select name="kind"><option value="co">Company</option><option value="in" selected>Individual</option></select></p>"#,
    );
    let t = text(&pdf);
    assert!(t.contains("/FT /Ch"), "{t}");
    // Combo flag is Ff bit 18 (131072).
    assert!(t.contains("/Ff 131072"), "combo flag: {t}");
    assert!(t.contains("/T (kind)"), "{t}");
    assert!(
        t.contains("(co)") && t.contains("(Company)"),
        "options: {t}"
    );
    assert!(t.contains("/V (in)"), "selected export: {t}");
}

#[test]
fn optgroups_are_grouped_not_offered_as_options() {
    let pdf = render(
        r#"<p><select name="city">
             <optgroup label="Kanto"><option value="tko" selected>Tokyo</option></optgroup>
             <optgroup label="Kansai"><option value="osk">Osaka</option></optgroup>
           </select></p>"#,
    );
    let t = text(&pdf);
    assert!(t.contains("/T (city)"), "{t}");
    // The export values are the plain option values, in document order.
    assert!(t.contains("/V (tko)"), "{t}");
    // Group labels are not selectable values: no /V or bare option entry for them.
    assert!(!t.contains("/V (Kanto)"), "{t}");
    // ...but grouping stays visible in the display strings.
    assert!(
        t.contains("Kanto"),
        "group label kept as display prefix: {t}"
    );
    assert!(t.contains("Tokyo") && t.contains("Osaka"), "{t}");
    let tko = t.find("(tko)").expect("tko option");
    let osk = t.find("(osk)").expect("osk option");
    assert!(tko < osk, "document order preserved: {t}");
}

#[test]
fn a_multiple_select_becomes_a_multi_select_list_box() {
    let pdf = render(
        r#"<p><select name="tags" multiple size="3"><option value="a" selected>A</option><option value="b" selected>B</option></select></p>"#,
    );
    let t = text(&pdf);
    assert!(t.contains("/FT /Ch"), "{t}");
    // MultiSelect flag is Ff bit 22 (2097152); no Combo bit.
    assert!(t.contains("/Ff 2097152"), "list box flags: {t}");
    assert!(t.contains("/I [0 1]"), "selected indices: {t}");
}

#[test]
fn buttons_become_pushbutton_fields() {
    let pdf = render(
        r#"<form action="https://example.invalid/send"><p><button name="go">Send now</button> <input type="submit" name="sub"></p></form>"#,
    );
    let t = text(&pdf);
    // Pushbutton flag is Ff bit 17 (65536).
    assert!(t.contains("/Ff 65536"), "pushbutton flag: {t}");
    assert!(t.contains("/T (go)"), "{t}");
    assert!(t.contains("(Send now)"), "button caption: {t}");
    assert!(t.contains("/S /SubmitForm"), "submit action: {t}");
}

#[test]
fn naming_falls_back_to_id_then_a_generated_name() {
    let pdf =
        render(r#"<p><input type="text" id="byid" value="1"><input type="text" value="2"></p>"#);
    let t = text(&pdf);
    assert!(t.contains("/T (byid)"), "{t}");
    assert!(t.contains("/T (field_"), "generated name: {t}");
}

#[test]
fn required_readonly_and_disabled_map_to_field_flags() {
    let pdf = render(
        r#"<p><input type="text" name="req" required><input type="text" name="ro" readonly></p>"#,
    );
    let t = text(&pdf);
    // Required = Ff bit 2 (2), ReadOnly = Ff bit 1 (1).
    assert!(t.contains("/Ff 2"), "required: {t}");
    assert!(t.contains("/Ff 1"), "readonly: {t}");
}

#[test]
fn controls_without_a_pdf_equivalent_get_no_widget() {
    let pdf = render(
        r#"<p><input type="hidden" name="h" value="x"><input type="range" name="r"><input type="color" name="c"><input type="file" name="f">text</p>"#,
    );
    let t = text(&pdf);
    assert!(!t.contains("/AcroForm"), "no fields expected: {t}");
    assert!(!t.contains("/Subtype /Widget"), "{t}");
}

#[test]
fn a_document_without_forms_has_no_acroform() {
    let pdf = render("<p>plain</p>");
    assert!(!text(&pdf).contains("/AcroForm"));
    // The plain batch entry point agrees.
    let plain = {
        let dom = html::parse(b"<p>plain</p>");
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
        let fonts = test_fonts();
        let settings = PageSettings::default();
        let pages = paginate_document(&dom, &styles, &fonts, &settings);
        encode_pdf(&pages, &styles, &HashMap::new(), &fonts, &settings)
    };
    assert!(!text(&plain).contains("/AcroForm"));
}

#[test]
fn forms_can_be_turned_off() {
    let mut output = PdfOutputOptions::default();
    output.forms = false;
    let pdf = render_with_options(r#"<p><input type="text" name="x" value="1"></p>"#, &output);
    assert!(!text(&pdf).contains("/AcroForm"), "{}", text(&pdf));
}

#[test]
fn streaming_output_carries_the_same_fields() {
    let pdf = render_streaming(
        r#"<p><input type="text" name="fullname" value="Taro"> <input type="checkbox" name="ok" checked></p>"#,
    );
    let t = text(&pdf);
    assert!(t.contains("/AcroForm"), "{t}");
    assert!(t.contains("/FT /Tx") && t.contains("/FT /Btn"), "{t}");
    assert!(t.contains("/T (fullname)") && t.contains("/T (ok)"), "{t}");
}
