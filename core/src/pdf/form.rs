//! Emitting HTML form fields as PDF AcroForm fields.
//!
//! [`collect_form_widgets`] walks a laid-out page (the same walk as
//! `collect_link_areas`), [`FieldSet`] groups the widgets into fields
//! (radio groups and same-named checkboxes become one field with several
//! widgets), and the `write_*` functions turn a field into PDF objects
//! through `pdf_writer`. Both the batch writer (`pdf::document`) and the
//! streaming writer (`pdf::streaming`) use them.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pdf_writer::types::{ActionType, FieldFlags, FieldType};
use pdf_writer::writers::{Annotation, Field, Form};
use pdf_writer::{Finish, Name, Rect as PdfRect, Ref, Str, TextStr};

use crate::html::NodeId;
use crate::layout::form::{group_key, resolved_name, FormField, FormFieldKind, PushAction};
use crate::layout::{LaidOutBox, LaidOutContent, PageSettings};
use crate::style::ComputedStyle;

use super::document::to_pdf_y;

/// One widget annotation to write: a field plus its page rectangle.
/// Coordinates are PDF page coordinates in CSS px (before output scale),
/// as with `LinkArea`.
#[derive(Debug, Clone)]
pub struct FormWidget {
    pub field: Rc<FormField>,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    /// The control's computed font size, for the field's `/DA`.
    pub font_size: f32,
}

/// Collect the form widgets of one laid-out box tree.
pub fn collect_form_widgets(
    b: &LaidOutBox,
    styles: &HashMap<NodeId, Rc<ComputedStyle>>,
    settings: &PageSettings,
    seen: &mut HashSet<NodeId>,
    out: &mut Vec<FormWidget>,
) {
    if let (Some(field), Some(node)) = (&b.form_field, b.node) {
        let border_box = b.layout.border_box();
        if border_box.width > 0.0 && border_box.height > 0.0 && seen.insert(node) {
            let x0 = settings.margin.left + border_box.x;
            out.push(FormWidget {
                field: field.clone(),
                x0,
                y0: to_pdf_y(settings, border_box.y + border_box.height),
                x1: x0 + border_box.width,
                y1: to_pdf_y(settings, border_box.y),
                font_size: styles.get(&node).map(|s| s.font_size.0).unwrap_or(0.0),
            });
        }
    }
    match &b.content {
        LaidOutContent::Blocks(children) | LaidOutContent::Flex(children) => {
            for child in children {
                collect_form_widgets(child, styles, settings, seen, out);
            }
        }
        LaidOutContent::Grid(grid) => {
            for child in grid.rows.iter().flat_map(|row| &row.items) {
                collect_form_widgets(child, styles, settings, seen, out);
            }
        }
        LaidOutContent::Table(table) => {
            if let Some(caption) = &table.caption {
                collect_form_widgets(caption, styles, settings, seen, out);
            }
            for row in &table.rows {
                for cell in &row.cells {
                    collect_form_widgets(cell, styles, settings, seen, out);
                }
            }
        }
        LaidOutContent::Inline(lines) => {
            for line in lines {
                for atomic in &line.atomics {
                    collect_form_widgets(&atomic.content, styles, settings, seen, out);
                }
            }
        }
        LaidOutContent::Image(_) => {}
    }
}

/// One PDF field: one merged field+widget, or a parent field with
/// several widget kids (a radio group / same-named checkboxes).
#[derive(Debug)]
pub struct FieldGroup {
    pub name: String,
    /// The first widget's field data (name, flags and kind source).
    pub source: Rc<FormField>,
    /// Indices into [`FieldSet::widgets`].
    pub widgets: Vec<usize>,
    /// Whether the field dictionary is a separate parent object.
    pub grouped: bool,
    /// The parent field's Ref (grouped fields only, once allocated).
    pub field_ref: Option<Ref>,
}

/// All fields known so far, in first-appearance order.
#[derive(Debug, Default)]
pub struct FieldSet {
    pub widgets: Vec<FormWidget>,
    /// The widget annotations' Refs, parallel to `widgets`.
    pub widget_refs: Vec<Option<Ref>>,
    pub groups: Vec<FieldGroup>,
    group_index: HashMap<String, usize>,
    unnamed: usize,
}

impl FieldSet {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    /// Add one page's widgets, grouping them into fields. Returns the
    /// global widget indices that were added.
    pub fn add_widgets(&mut self, widgets: Vec<FormWidget>) -> Vec<usize> {
        let mut added = Vec::with_capacity(widgets.len());
        for widget in widgets {
            let index = self.widgets.len();
            let group_key = match &widget.field.kind {
                FormFieldKind::Button {
                    radio,
                    pushbutton: false,
                    ..
                } => group_key(&widget.field).map(|name| format!("btn:{radio}:{name}")),
                _ => None,
            };
            let group_index = match &group_key {
                Some(key) => match self.group_index.get(key) {
                    Some(&existing) => existing,
                    None => {
                        let created = self.groups.len();
                        self.group_index.insert(key.clone(), created);
                        self.groups.push(FieldGroup {
                            name: resolved_name(&widget.field, 0),
                            source: widget.field.clone(),
                            widgets: Vec::new(),
                            grouped: true,
                            field_ref: None,
                        });
                        created
                    }
                },
                None => {
                    let is_unnamed = widget
                        .field
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .is_none()
                        && widget
                            .field
                            .id
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .is_none();
                    if is_unnamed {
                        self.unnamed += 1;
                    }
                    let created = self.groups.len();
                    self.groups.push(FieldGroup {
                        name: resolved_name(&widget.field, self.unnamed),
                        source: widget.field.clone(),
                        widgets: Vec::new(),
                        grouped: false,
                        field_ref: None,
                    });
                    created
                }
            };
            self.groups[group_index].widgets.push(index);
            self.widgets.push(widget);
            self.widget_refs.push(None);
            added.push(index);
        }
        added
    }

    /// The Refs of the root fields (merged widgets, or group parents).
    pub fn root_refs(&self) -> Vec<Ref> {
        self.groups
            .iter()
            .filter_map(|group| {
                if group.grouped {
                    group.field_ref
                } else {
                    group.widgets.first().and_then(|&w| self.widget_refs[w])
                }
            })
            .collect()
    }
}

/// The on-state name of a checkbox/radio widget.
fn state_name(field: &FormField) -> Vec<u8> {
    match &field.kind {
        FormFieldKind::Button {
            export_value,
            checked,
            ..
        } => {
            if *checked && !export_value.is_empty() {
                export_value.as_bytes().to_vec()
            } else if *checked {
                b"Yes".to_vec()
            } else {
                b"Off".to_vec()
            }
        }
        _ => b"Off".to_vec(),
    }
}

fn field_flags(field: &FormField) -> FieldFlags {
    let mut flags = FieldFlags::empty();
    if field.read_only {
        flags |= FieldFlags::READ_ONLY;
    }
    if field.required {
        flags |= FieldFlags::REQUIRED;
    }
    match &field.kind {
        FormFieldKind::Text {
            multiline,
            password,
            ..
        } => {
            if *multiline {
                flags |= FieldFlags::MULTILINE;
            }
            if *password {
                flags |= FieldFlags::PASSWORD;
            }
        }
        FormFieldKind::Button {
            radio, pushbutton, ..
        } => {
            if *pushbutton {
                flags |= FieldFlags::PUSHBUTTON;
            } else if *radio {
                flags |= FieldFlags::RADIO | FieldFlags::NO_TOGGLE_TO_OFF;
            }
        }
        FormFieldKind::Choice { combo, multi, .. } => {
            if *combo {
                flags |= FieldFlags::COMBO;
            }
            if *multi {
                flags |= FieldFlags::MULTI_SELECT;
            }
        }
    }
    flags
}

/// The selected option indices of a choice field, applying the HTML
/// default (a combo box with nothing selected shows its first option).
fn effective_selected(field: &FormField) -> Vec<i32> {
    let FormFieldKind::Choice {
        options,
        combo,
        selected,
        ..
    } = &field.kind
    else {
        return Vec::new();
    };
    if selected.is_empty() && *combo && !options.is_empty() {
        vec![0]
    } else {
        selected.clone()
    }
}

/// Write the field-dictionary entries shared by merged and parent fields.
fn fill_field(field: &mut Field<'_>, group: &FieldGroup) {
    let source = &group.source;
    let field_type = match &source.kind {
        FormFieldKind::Text { .. } => FieldType::Text,
        FormFieldKind::Button { .. } => FieldType::Button,
        FormFieldKind::Choice { .. } => FieldType::Choice,
    };
    field.field_type(field_type);
    field.partial_name(TextStr(&group.name));
    if let Some(alternate) = &source.alternate_name {
        field.alternate_name(TextStr(alternate));
    }
    field.field_flags(field_flags(source));

    match &source.kind {
        FormFieldKind::Text {
            value,
            max_len,
            format,
            ..
        } => {
            field.text_value(TextStr(value));
            field.text_default_value(TextStr(value));
            if let Some(max_len) = max_len {
                field.text_max_len(*max_len);
            }
            if let Some(format) = format {
                let (format_fn, keystroke_fn) = if format.is_time() {
                    ("AFTime_FormatEx", "AFTime_KeystrokeEx")
                } else {
                    ("AFDate_FormatEx", "AFDate_KeystrokeEx")
                };
                let pattern = format.pattern();
                let mut actions = field.additional_actions();
                let mut format_action = actions.form_format();
                format_action.action_type(ActionType::JavaScript);
                format_action.js_string(TextStr(&format!("{format_fn}(\"{pattern}\")")));
                format_action.finish();
                let mut keystroke_action = actions.form_calculate_partial();
                keystroke_action.action_type(ActionType::JavaScript);
                keystroke_action.js_string(TextStr(&format!("{keystroke_fn}(\"{pattern}\")")));
                keystroke_action.finish();
            }
        }
        FormFieldKind::Button {
            radio, pushbutton, ..
        } => {
            if *pushbutton {
                // No retained value.
            } else if group.grouped {
                // The value is the checked widget's export, resolved by
                // the caller through the widgets (see write_parent_field).
            } else if *radio {
                field.radio_value(Name(&state_name(source)));
            } else {
                field.pair(Name(b"V"), Name(&state_name(source)));
                field.pair(Name(b"DV"), Name(&state_name(source)));
            }
        }
        FormFieldKind::Choice { options, multi, .. } => {
            {
                let mut choice_options = field.choice_options();
                for option in options {
                    if option.export_value == option.display {
                        choice_options.option(TextStr(&option.display));
                    } else {
                        choice_options
                            .export(TextStr(&option.display), TextStr(&option.export_value));
                    }
                }
                choice_options.finish();
            }
            let selected = effective_selected(source);
            let exports: Vec<&str> = selected
                .iter()
                .filter_map(|&i| options.get(i as usize))
                .map(|o| o.export_value.as_str())
                .collect();
            if *multi {
                field.choice_values(exports.iter().map(|s| TextStr(s)));
                field.choice_default_values(exports.iter().map(|s| TextStr(s)));
                field.choice_indices(selected.iter().copied());
            } else if let Some(first) = exports.first() {
                field.choice_value(Some(TextStr(first)));
                field.choice_default_value(Some(TextStr(first)));
            } else {
                field.choice_value(None);
            }
        }
    }
}

/// The `/DA` for a variable-text field: the document default font at the
/// control's own size (0 would be viewer-chosen; the size is known here).
fn fill_default_appearance(field: &mut Field<'_>, widget: &FormWidget) {
    let size = widget.font_size;
    if size > 0.0 {
        let appearance = format!("/Helv {} Tf 0 g", trim_number(size));
        field.vartext_default_appearance(Str(appearance.as_bytes()));
    }
}

fn trim_number(value: f32) -> String {
    if (value - value.round()).abs() < 0.01 {
        format!("{}", value.round() as i32)
    } else {
        format!("{value:.2}")
    }
}

/// Write one merged field+widget annotation (`pdf.form_field(id)` or
/// `chunk.form_field(id)`), for a non-grouped group.
pub fn write_merged_field(
    mut field: Field<'_>,
    group: &FieldGroup,
    widget: &FormWidget,
    scale: f32,
) {
    fill_field(&mut field, group);
    if matches!(
        group.source.kind,
        FormFieldKind::Text { .. } | FormFieldKind::Choice { .. }
    ) {
        fill_default_appearance(&mut field, widget);
    }
    let mut annotation = field.into_annotation();
    write_widget_annotation(&mut annotation, group, widget, scale, None);
}

/// Write one widget kid of a grouped field (`pdf.annotation(id)`).
pub fn write_widget_kid(
    mut annotation: Annotation<'_>,
    group: &FieldGroup,
    widget: &FormWidget,
    scale: f32,
    parent: Ref,
) {
    annotation.subtype(pdf_writer::types::AnnotationType::Widget);
    write_widget_annotation(&mut annotation, group, widget, scale, Some(parent));
}

/// Write the parent field of a radio group / same-named checkboxes.
pub fn write_parent_field(mut field: Field<'_>, group: &FieldGroup, set: &FieldSet) {
    fill_field(&mut field, group);
    let kids: Vec<Ref> = group
        .widgets
        .iter()
        .filter_map(|&w| set.widget_refs[w])
        .collect();
    field.children(kids.iter().copied());
    if let FormFieldKind::Button { radio, .. } = &group.source.kind {
        if *radio {
            let checked = group
                .widgets
                .iter()
                .map(|&w| &set.widgets[w].field)
                .find(|f| matches!(&f.kind, FormFieldKind::Button { checked, .. } if *checked));
            let state = checked
                .map(|f| state_name(f))
                .unwrap_or_else(|| b"Off".to_vec());
            field.radio_value(Name(&state));
            field.radio_default_value(Name(&state));
        } else {
            let exports: Vec<&str> = group
                .widgets
                .iter()
                .map(|&w| match &set.widgets[w].field.kind {
                    FormFieldKind::Button { export_value, .. } => export_value.as_str(),
                    _ => "",
                })
                .collect();
            field.button_options(exports.iter().map(|s| TextStr(s)));
            let checked = group
                .widgets
                .iter()
                .map(|&w| &set.widgets[w].field)
                .find(|f| matches!(&f.kind, FormFieldKind::Button { checked, .. } if *checked));
            let state = checked
                .map(|f| state_name(f))
                .unwrap_or_else(|| b"Off".to_vec());
            field.pair(Name(b"V"), Name(&state));
            field.pair(Name(b"DV"), Name(&state));
        }
    }
}

/// The widget-annotation entries (rectangle, border, state, caption,
/// action) shared by merged fields and grouped kids.
fn write_widget_annotation(
    annotation: &mut Annotation<'_>,
    group: &FieldGroup,
    widget: &FormWidget,
    scale: f32,
    parent: Option<Ref>,
) {
    annotation.rect(PdfRect::new(
        widget.x0 * scale,
        widget.y0 * scale,
        widget.x1 * scale,
        widget.y1 * scale,
    ));
    // The static page drawing already shows the border; the widget itself
    // takes no border, as with link annotations here.
    annotation.border(0.0, 0.0, 0.0, None);
    if let Some(parent) = parent {
        annotation.parent(parent);
    }
    if let FormFieldKind::Button {
        pushbutton,
        caption,
        action,
        ..
    } = &group.source.kind
    {
        if *pushbutton {
            if let Some(caption) = caption.as_ref().filter(|c| !c.is_empty()) {
                // For a grouped kid the caption is per-widget; kids are
                // never pushbuttons, so this is the merged case and the
                // group's source is this widget's own field.
                let mut characteristics = annotation.appearance_characteristics();
                characteristics.normal_caption(TextStr(caption));
                characteristics.finish();
            }
            if let Some(action) = action {
                let mut pdf_action = annotation.action();
                match action {
                    PushAction::SubmitForm(url) => {
                        pdf_action.action_type(ActionType::SubmitForm);
                        pdf_action.pair(Name(b"F"), Str(url.as_bytes()));
                    }
                    PushAction::ResetForm => {
                        pdf_action.action_type(ActionType::ResetForm);
                    }
                }
                pdf_action.finish();
            }
        } else {
            annotation.appearance_state(Name(&state_name(&widget.field)));
        }
    }
}

/// Write the catalogue `/AcroForm` dictionary entries into `form`.
pub fn write_acroform(form: &mut Form<'_>, root_refs: &[Ref]) {
    form.fields(root_refs.iter().copied());
    form.pair(Name(b"NeedAppearances"), true);
    form.default_appearance(Str(b"/Helv 0 Tf 0 g"));
    let mut resources = form.default_resources();
    {
        let mut fonts = resources.fonts();
        let mut helvetica = fonts
            .insert(Name(b"Helv"))
            .start::<pdf_writer::writers::Type1Font>();
        helvetica.base_font(Name(b"Helvetica"));
        helvetica.encoding_predefined(Name(b"WinAnsiEncoding"));
        helvetica.finish();
        fonts.finish();
    }
    resources.finish();
}
