//! Form-field data extracted from HTML form controls.
//!
//! This is the layout-side half of AcroForm support: it reads a control's
//! attributes (and, for `<select>`, its options) from the DOM once, at
//! box-tree time, and carries the result on the box so the PDF layer —
//! which never sees the DOM — can emit a real field. The mapping to PDF
//! field types lives in `pdf::form`; the plan is
//! `docs/plans/2026-11-08-html-form-fields-to-acroform.md`.

use crate::html::{Dom, NodeData, NodeId};

/// One form control's PDF-field data.
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    /// The `name` attribute (the field name), if present and non-empty.
    pub name: Option<String>,
    /// The `id` attribute, the field-name fallback.
    pub id: Option<String>,
    /// `title`, else `placeholder`: the user-facing name (`/TU`).
    pub alternate_name: Option<String>,
    /// `required` was present.
    pub required: bool,
    /// `readonly` or `disabled` was present.
    pub read_only: bool,
    pub kind: FormFieldKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormFieldKind {
    Text {
        value: String,
        multiline: bool,
        password: bool,
        max_len: Option<i32>,
        /// A date/time flavour needing a format action, if any.
        format: Option<DateTimeFormat>,
    },
    /// A checkbox or radio button. `export_value` is the on-state name.
    Button {
        radio: bool,
        pushbutton: bool,
        export_value: String,
        checked: bool,
        caption: Option<String>,
        action: Option<PushAction>,
    },
    Choice {
        options: Vec<ChoiceOption>,
        combo: bool,
        multi: bool,
        /// Indices into `options` that are selected.
        selected: Vec<i32>,
    },
}

/// Which `AFDate_*`/`AFTime_*` pattern a date/time text field uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTimeFormat {
    Date,
    DateTime,
    Month,
    Time,
}

impl DateTimeFormat {
    /// The Acrobat date/time pattern for this flavour.
    pub fn pattern(self) -> &'static str {
        match self {
            Self::Date => "yyyy-mm-dd",
            Self::DateTime => "yyyy-mm-dd HH:MM",
            Self::Month => "yyyy-mm",
            Self::Time => "HH:MM",
        }
    }

    /// Whether the pattern is a time-of-day (uses `AFTime_*`, not `AFDate_*`).
    pub fn is_time(self) -> bool {
        matches!(self, Self::Time)
    }
}

/// What activating a pushbutton does.
#[derive(Debug, Clone, PartialEq)]
pub enum PushAction {
    SubmitForm(String),
    ResetForm,
}

/// One selectable option of a choice field.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceOption {
    /// The submitted value (the `value` attribute, else the option text).
    pub export_value: String,
    /// The shown text, prefixed with the `<optgroup>` label when grouped.
    pub display: String,
}

/// The field name for `field`: `name`, else `id`, else a generated name.
/// Periods become `_` (a partial field name may not contain one).
pub fn resolved_name(field: &FormField, generated: usize) -> String {
    let raw = field
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| field.id.as_deref().map(str::trim).filter(|s| !s.is_empty()));
    match raw {
        Some(name) => name.replace('.', "_"),
        None => format!("field_{generated}"),
    }
}

/// Whether two fields share a name and so are one PDF field (radio groups
/// and same-named checkboxes). Unnamed fields never group.
pub fn group_key(field: &FormField) -> Option<String> {
    let name = field
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(name.replace('.', "_"))
}

/// Extract the field data for `node`, or `None` when the element is not a
/// control with a PDF-standard equivalent (see the plan's non-goals:
/// `hidden`, `file`, `color` and `range` inputs get no field).
pub fn form_field_for(dom: &Dom, node: NodeId) -> Option<FormField> {
    let NodeData::Element { name, attrs, .. } = &dom.node(node).data else {
        return None;
    };
    let attr = |key: &str| {
        attrs
            .iter()
            .find(|a| &*a.name.local == key)
            .map(|a| a.value.to_string())
    };
    let has = |key: &str| attrs.iter().any(|a| &*a.name.local == key);
    let common = |kind: FormFieldKind| FormField {
        name: attr("name"),
        id: attr("id"),
        alternate_name: attr("title")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| attr("placeholder").filter(|s| !s.trim().is_empty())),
        required: has("required"),
        read_only: has("readonly") || has("disabled"),
        kind,
    };
    let max_len = attr("maxlength")
        .and_then(|v| v.trim().parse::<i32>().ok())
        .filter(|&v| v > 0);

    let kind = match &*name.local {
        "input" => {
            let input_type = attr("type")
                .unwrap_or_else(|| "text".to_string())
                .trim()
                .to_ascii_lowercase();
            match input_type.as_str() {
                // No PDF-standard widget equivalent: static rendering only.
                "hidden" | "file" | "color" | "range" => return None,
                "checkbox" | "radio" => FormFieldKind::Button {
                    radio: input_type == "radio",
                    pushbutton: false,
                    export_value: attr("value")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Yes".to_string()),
                    checked: has("checked"),
                    caption: None,
                    action: None,
                },
                "submit" | "reset" | "button" | "image" => FormFieldKind::Button {
                    radio: false,
                    pushbutton: true,
                    export_value: String::new(),
                    checked: false,
                    caption: Some(match input_type.as_str() {
                        "submit" => attr("value").unwrap_or_else(|| "Submit".to_string()),
                        "reset" => attr("value").unwrap_or_else(|| "Reset".to_string()),
                        "image" => attr("alt")
                            .filter(|s| !s.is_empty())
                            .or_else(|| attr("value"))
                            .unwrap_or_default(),
                        _ => attr("value").unwrap_or_default(),
                    }),
                    action: push_action(dom, node, &input_type, &attr),
                },
                _ => FormFieldKind::Text {
                    value: attr("value").unwrap_or_default(),
                    multiline: false,
                    password: input_type == "password",
                    max_len,
                    format: match input_type.as_str() {
                        "date" => Some(DateTimeFormat::Date),
                        "datetime-local" => Some(DateTimeFormat::DateTime),
                        "month" => Some(DateTimeFormat::Month),
                        "time" => Some(DateTimeFormat::Time),
                        _ => None,
                    },
                },
            }
        }
        "textarea" => FormFieldKind::Text {
            value: raw_text(dom, node)
                .strip_prefix('\n')
                .map(str::to_string)
                .unwrap_or_else(|| raw_text(dom, node)),
            multiline: true,
            password: false,
            max_len,
            format: None,
        },
        "select" => {
            let (options, mut selected) = choice_options(dom, node);
            let multi = has("multiple");
            let size: i64 = attr("size")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let combo = !multi && size <= 1;
            // A combo box with nothing selected shows its first option.
            if selected.is_empty() && combo && !options.is_empty() {
                selected.push(0);
            }
            FormFieldKind::Choice {
                options,
                combo,
                multi,
                selected,
            }
        }
        "button" => {
            let button_type = attr("type")
                .unwrap_or_else(|| "submit".to_string())
                .trim()
                .to_ascii_lowercase();
            FormFieldKind::Button {
                radio: false,
                pushbutton: true,
                export_value: String::new(),
                checked: false,
                caption: Some(collapsed_text(dom, node)),
                action: push_action(dom, node, &button_type, &attr),
            }
        }
        _ => return None,
    };
    Some(common(kind))
}

/// The action of a submit/reset control: the enclosing `<form>`'s
/// `action` for a submit, `ResetForm` for a reset, nothing otherwise.
fn push_action(
    dom: &Dom,
    node: NodeId,
    input_type: &str,
    attr: &impl Fn(&str) -> Option<String>,
) -> Option<PushAction> {
    match input_type {
        "reset" => Some(PushAction::ResetForm),
        "submit" => {
            if let Some(action) = attr("formaction").filter(|s| !s.trim().is_empty()) {
                return Some(PushAction::SubmitForm(action));
            }
            let mut current = dom.parent(node);
            while let Some(id) = current {
                if let NodeData::Element { name, attrs, .. } = &dom.node(id).data {
                    if &*name.local == "form" {
                        return attrs
                            .iter()
                            .find(|a| &*a.name.local == "action")
                            .map(|a| a.value.to_string())
                            .filter(|s| !s.trim().is_empty())
                            .map(PushAction::SubmitForm);
                    }
                }
                current = dom.parent(id);
            }
            None
        }
        _ => None,
    }
}

/// The options of a `<select>`, flattened in document order, and the
/// indices of the selected ones. An `<optgroup>` is never an option; its
/// label prefixes the display of the options inside it, and a disabled
/// group (or a disabled option) cannot supply the selected value.
fn choice_options(dom: &Dom, select: NodeId) -> (Vec<ChoiceOption>, Vec<i32>) {
    let mut options: Vec<ChoiceOption> = Vec::new();
    let mut selected: Vec<i32> = Vec::new();

    fn walk(
        dom: &Dom,
        node: NodeId,
        group: Option<&str>,
        group_disabled: bool,
        options: &mut Vec<ChoiceOption>,
        selected: &mut Vec<i32>,
    ) {
        for child in dom.children(node) {
            let NodeData::Element { name, attrs, .. } = &dom.node(child).data else {
                continue;
            };
            let has = |key: &str| attrs.iter().any(|a| &*a.name.local == key);
            let attr = |key: &str| {
                attrs
                    .iter()
                    .find(|a| &*a.name.local == key)
                    .map(|a| a.value.to_string())
            };
            match &*name.local {
                "optgroup" => {
                    let label = attr("label").unwrap_or_default();
                    walk(
                        dom,
                        child,
                        Some(label.trim()),
                        group_disabled || has("disabled"),
                        options,
                        selected,
                    );
                }
                "option" => {
                    let text = collapsed_text(dom, child);
                    let export_value = attr("value")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| text.clone());
                    if has("selected") && !has("disabled") && !group_disabled {
                        selected.push(options.len() as i32);
                    }
                    let display = match group {
                        Some(label) if !label.is_empty() => {
                            format!("{label} - {text}")
                        }
                        _ => text,
                    };
                    options.push(ChoiceOption {
                        export_value,
                        display,
                    });
                }
                _ => {}
            }
        }
    }

    walk(dom, select, None, false, &mut options, &mut selected);
    (options, selected)
}

/// All descendant text, whitespace-collapsed (an option's or button's label).
fn collapsed_text(dom: &Dom, node: NodeId) -> String {
    let mut out = String::new();
    fn walk(dom: &Dom, node: NodeId, out: &mut String) {
        if let NodeData::Text { contents } = &dom.node(node).data {
            out.push_str(contents);
        }
        for child in dom.children(node) {
            walk(dom, child, out);
        }
    }
    walk(dom, node, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// All descendant text as written (a `<textarea>`'s value).
fn raw_text(dom: &Dom, node: NodeId) -> String {
    let mut out = String::new();
    fn walk(dom: &Dom, node: NodeId, out: &mut String) {
        if let NodeData::Text { contents } = &dom.node(node).data {
            out.push_str(contents);
        }
        for child in dom.children(node) {
            walk(dom, child, out);
        }
    }
    walk(dom, node, &mut out);
    out
}
