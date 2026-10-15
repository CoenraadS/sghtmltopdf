//! End-to-end tests for `@property` registrations.

use std::path::PathBuf;

use sghtmltopdf::engine::{Engine, EngineOptions, FontSpec, Mode};
use sghtmltopdf::html::{self, Dom, NodeData, NodeId};
use sghtmltopdf::img::{DocumentImageCache, ImageFetcher};
use sghtmltopdf::sink::MemorySink;
use sghtmltopdf::style::{
    compute_styles, extract_author_stylesheet, user_agent_stylesheet, BorderStyle, ComputedStyle,
    LengthPercentage, LengthPercentageOrAuto,
};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");

fn dom_with_style(body: &str, css: &str) -> Dom {
    html::parse(
        format!("<html><head><style>{css}</style></head><body>{body}</body></html>").as_bytes(),
    )
}

fn find_all_divs(dom: &Dom, node: NodeId, out: &mut Vec<NodeId>) {
    if let NodeData::Element { name, .. } = &dom.node(node).data {
        if &*name.local == "div" {
            out.push(node);
        }
    }
    for child in dom.children(node) {
        find_all_divs(dom, child, out);
    }
}

fn style_of(body: &str, css: &str, class: &str) -> ComputedStyle {
    let dom = dom_with_style(body, css);
    let fetcher = ImageFetcher::new(PathBuf::from("."), false);
    let author = extract_author_stylesheet(&dom, &fetcher, &DocumentImageCache::new());
    let styles = compute_styles(&dom, &user_agent_stylesheet(), &author);
    let mut divs = Vec::new();
    find_all_divs(&dom, dom.document(), &mut divs);
    let id = divs
        .into_iter()
        .find(|id| match &dom.node(*id).data {
            NodeData::Element { attrs, .. } => attrs.iter().any(|attr| {
                &*attr.name.local == "class" && attr.value.split(' ').any(|c| c == class)
            }),
            _ => false,
        })
        .expect("element with the class");
    (*styles[&id]).clone()
}

fn margin_left(style: &ComputedStyle) -> f32 {
    match style.margin_left {
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Length(px)) => px,
        LengthPercentageOrAuto::LengthPercentage(LengthPercentage::Calc { px, percent: 0.0 }) => px,
        other => panic!("unexpected margin-left {other:?}"),
    }
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

fn render(css: &str, body_parts: &[&str], mode: Mode) -> Vec<u8> {
    let mut options = EngineOptions::default();
    options.mode = mode;
    options.fonts = vec![FontSpec {
        path: PathBuf::from(FONT_PATH),
        index: 0,
    }];
    options.output.compress = false;
    let mut engine = Engine::new(options, MemorySink::new());
    let head = format!("<html><head><style>{css}</style></head><body>");
    engine.feed(head.as_bytes()).unwrap();
    for part in body_parts {
        engine.feed(part.as_bytes()).unwrap();
    }
    engine.feed(b"</body></html>").unwrap();
    engine.finish().unwrap()
}

#[test]
fn registered_initial_value_applies_without_a_declaration() {
    let css = r#"
        @property --shift { syntax: "*"; inherits: false; initial-value: 24px }
        .probe { margin-left: var(--shift) }
    "#;
    assert_eq!(
        margin_left(&style_of("<div class=\"probe\">x</div>", css, "probe")),
        24.0
    );
}

#[test]
fn a_non_inheriting_registration_does_not_leak_the_parents_value() {
    let css = r#"
        @property --tw-border-style { syntax: "*"; inherits: false; initial-value: solid }
        .border { border-style: var(--tw-border-style); border-width: 4px }
        .border-none { --tw-border-style: none }
    "#;
    let body = r#"<div class="border-none"><div class="border">x</div></div>"#;
    assert_eq!(
        style_of(body, css, "border").border_top_style,
        BorderStyle::Solid
    );
}

#[test]
fn an_inheriting_registration_uses_the_parents_value() {
    let css = r#"
        @property --shift { syntax: "*"; inherits: true; initial-value: 8px }
        .parent { --shift: 31px }
        .probe { margin-left: var(--shift) }
    "#;
    let body = r#"<div class="parent"><div class="probe">x</div></div>"#;
    assert_eq!(margin_left(&style_of(body, css, "probe")), 31.0);
}

#[test]
fn css_wide_keywords_follow_the_registration() {
    let css = r#"
        @property --non { syntax: "*"; inherits: false; initial-value: 10px }
        @property --yes { syntax: "*"; inherits: true; initial-value: 5px }
        .parent { --non: 40px; --yes: 30px }
        .use { margin-left: var(--value) }
        .non-initial { --non: initial; --value: var(--non) }
        .non-inherit { --non: inherit; --value: var(--non) }
        .non-unset { --non: unset; --value: var(--non) }
        .yes-initial { --yes: initial; --value: var(--yes) }
        .yes-inherit { --yes: inherit; --value: var(--yes) }
        .yes-unset { --yes: unset; --value: var(--yes) }
    "#;
    let body = r#"<div class="parent">
        <div class="use non-initial"></div><div class="use non-inherit"></div>
        <div class="use non-unset"></div><div class="use yes-initial"></div>
        <div class="use yes-inherit"></div><div class="use yes-unset"></div>
    </div>"#;
    for (class, expected) in [
        ("non-initial", 10.0),
        ("non-inherit", 40.0),
        ("non-unset", 10.0),
        ("yes-initial", 5.0),
        ("yes-inherit", 30.0),
        ("yes-unset", 30.0),
    ] {
        assert_eq!(
            margin_left(&style_of(body, css, class)),
            expected,
            "{class}"
        );
    }
}

#[test]
fn registrations_missing_required_descriptors_are_ignored() {
    let css = r#"
        @property --missing-syntax { inherits: false; initial-value: 1px }
        @property --missing-inherits { syntax: "*"; initial-value: 2px }
        @property --missing-initial { syntax: "<length>"; inherits: false }
        .parent { --missing-syntax: 11px; --missing-inherits: 12px; --missing-initial: 13px }
        .a { margin-left: var(--missing-syntax) }
        .b { margin-left: var(--missing-inherits) }
        .c { margin-left: var(--missing-initial) }
    "#;
    let body = r#"<div class="parent"><div class="a"></div><div class="b"></div><div class="c"></div></div>"#;
    for (class, expected) in [("a", 11.0), ("b", 12.0), ("c", 13.0)] {
        assert_eq!(
            margin_left(&style_of(body, css, class)),
            expected,
            "{class}"
        );
    }
}

#[test]
fn the_last_valid_registration_wins() {
    let css = r#"
        @property --shift { syntax: "*"; inherits: false; initial-value: 1px }
        @property --shift { syntax: "*"; inherits: true; initial-value: 2px }
        @property --shift { inherits: false; initial-value: 3px }
        .parent { --shift: 30px }
        .probe { margin-left: var(--shift) }
    "#;
    let body = r#"<div class="parent"><div class="probe"></div></div>"#;
    assert_eq!(margin_left(&style_of(body, css, "probe")), 30.0);
}

#[test]
fn a_registration_inside_a_layer_applies() {
    let css = r#"
        @layer theme {
            @property --shift { syntax: "*"; inherits: false; initial-value: 19px }
        }
        .probe { margin-left: var(--shift) }
    "#;
    assert_eq!(
        margin_left(&style_of("<div class=\"probe\"></div>", css, "probe")),
        19.0
    );
}

#[test]
fn registrations_follow_matching_media_rules() {
    let css = r#"
        @media screen {
            @property --shift { syntax: "*"; inherits: false; initial-value: 99px }
        }
        @media print {
            @property --shift { syntax: "*"; inherits: false; initial-value: 18px }
        }
        .probe { margin-left: var(--shift) }
    "#;
    assert_eq!(
        margin_left(&style_of("<div class=\"probe\"></div>", css, "probe")),
        18.0
    );
}

#[test]
fn tailwinds_border_utility_uses_the_registered_initial_value() {
    let css = r#"
        @property --tw-border-style { syntax: "*"; inherits: false; initial-value: solid }
        .border { border-style: var(--tw-border-style); border-width: 1px }
    "#;
    assert_eq!(
        style_of("<div class=\"border\">x</div>", css, "border").border_top_style,
        BorderStyle::Solid
    );
}

#[test]
fn a_registered_initial_value_wins_over_var_fallback() {
    let css = r#"
        @property --shift { syntax: "*"; inherits: false; initial-value: 17px }
        .probe { margin-left: var(--shift, 99px) }
    "#;
    assert_eq!(
        margin_left(&style_of("<div class=\"probe\"></div>", css, "probe")),
        17.0
    );
}

#[test]
fn invalid_registered_values_behave_as_unset() {
    let css = r#"
        @property --non { syntax: "*"; inherits: false; initial-value: 12px }
        @property --yes { syntax: "*"; inherits: true; initial-value: 7px }
        .parent { --yes: 34px }
        .cycle { --non: var(--non); margin-left: var(--non) }
        .missing { --yes: var(--missing); margin-left: var(--yes) }
    "#;
    let body = r#"<div class="parent"><div class="cycle"></div><div class="missing"></div></div>"#;
    assert_eq!(margin_left(&style_of(body, css, "cycle")), 12.0);
    assert_eq!(margin_left(&style_of(body, css, "missing")), 34.0);
}

#[test]
fn page_rules_see_a_registered_initial_value() {
    let css = r#"
        @property --page-width { syntax: "*"; inherits: false; initial-value: 300px }
        @page { size: var(--page-width) 400px; margin: 0 }
    "#;
    let expected = format!(
        "/MediaBox [0 0 {} {}]",
        300.0 * sghtmltopdf::pdf::DEFAULT_SCALE,
        400.0 * sghtmltopdf::pdf::DEFAULT_SCALE
    );
    let pdf = render(css, &["<p>x</p>"], Mode::Batch);
    assert_eq!(count_occurrences(&pdf, expected.as_bytes()), 1);
}

#[test]
fn registrations_behave_the_same_in_batch_and_streaming_modes() {
    let css = r#"
        @property --ink { syntax: "*"; inherits: false; initial-value: #cc0000 }
        .use { color: var(--ink) }
        .blue { --ink: #0000cc }
    "#;
    let parts = [
        r#"<div class="use">a</div>"#,
        r#"<div class="use blue">b</div>"#,
        r#"<div class="use">c</div>"#,
    ];
    for mode in [Mode::Batch, Mode::Streaming] {
        let pdf = render(css, &parts, mode);
        assert_eq!(count_occurrences(&pdf, b"0.8 0 0 rg"), 2, "{mode:?}");
        assert_eq!(count_occurrences(&pdf, b"0 0 0.8 rg"), 1, "{mode:?}");
    }
}
