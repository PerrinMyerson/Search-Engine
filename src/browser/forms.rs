use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded};

use super::{
    BrowserRender, DisplayCommand, DisplayHitTarget, Dom, ElementData, NodeKind, TextHitTargetRun,
    collapse_ascii_whitespace, resolve_browser_href, text_content,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserForm {
    pub index: usize,
    pub method: String,
    pub action: String,
    pub resolved_action: String,
    #[serde(default)]
    pub no_validate: bool,
    pub controls: Vec<BrowserFormControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFormControl {
    pub name: String,
    pub kind: String,
    pub value: String,
    pub disabled: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub form_no_validate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_resolved_action: Option<String>,
    pub checked: bool,
    pub options: Vec<BrowserFormOption>,
    #[serde(skip)]
    pub(super) node_id: usize,
    #[serde(skip)]
    pub(super) renders_inline_widget: bool,
    #[serde(skip)]
    pub(super) placeholder: Option<String>,
    #[serde(skip)]
    pub(super) widget_fallback_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFormOption {
    pub value: String,
    pub label: String,
    pub disabled: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct BrowserFormFieldKey {
    pub(super) form_index: usize,
    pub(super) name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BrowserFormControlKey {
    pub(super) form_index: usize,
    pub(super) control_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BrowserFormSubmission {
    Get { target: String },
    PostUrlEncoded { target: String, body: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BrowserFormSubmitter {
    pub(super) fields: Vec<(String, String)>,
    pub(super) no_validate: bool,
    pub(super) method: Option<String>,
    pub(super) resolved_action: Option<String>,
}

pub(super) fn build_form_submission_with_submitter(
    form: &BrowserForm,
    overrides: &[(String, String)],
    submitter: &BrowserFormSubmitter,
) -> Result<BrowserFormSubmission> {
    let method = submitter.method.as_deref().unwrap_or(&form.method);
    let resolved_action = submitter
        .resolved_action
        .as_deref()
        .unwrap_or(&form.resolved_action);
    if method.eq_ignore_ascii_case("GET") {
        return Ok(BrowserFormSubmission::Get {
            target: build_get_form_url_with_submitter(form, overrides, submitter)?,
        });
    }
    if method.eq_ignore_ascii_case("POST") {
        return Ok(BrowserFormSubmission::PostUrlEncoded {
            target: resolved_action.to_owned(),
            body: build_post_form_body_with_submitter(form, overrides, submitter)?,
        });
    }
    bail!("unsupported form method {:?}", method)
}

pub fn build_get_form_url(form: &BrowserForm, overrides: &[(String, String)]) -> Result<String> {
    ensure!(
        form.method.eq_ignore_ascii_case("GET"),
        "cannot build GET form URL for {} form",
        form.method
    );

    let fields = form_fields_with_overrides_and_submitter_fields(form, overrides, &[]);
    build_get_form_url_from_fields(&form.resolved_action, &fields)
}

fn build_get_form_url_with_submitter(
    form: &BrowserForm,
    overrides: &[(String, String)],
    submitter: &BrowserFormSubmitter,
) -> Result<String> {
    let method = submitter.method.as_deref().unwrap_or(&form.method);
    ensure!(
        method.eq_ignore_ascii_case("GET"),
        "cannot build GET form URL for {} form",
        method
    );
    let fields =
        form_fields_with_overrides_and_submitter_fields(form, overrides, &submitter.fields);
    let resolved_action = submitter
        .resolved_action
        .as_deref()
        .unwrap_or(&form.resolved_action);
    build_get_form_url_from_fields(resolved_action, &fields)
}

fn build_get_form_url_from_fields(
    resolved_action: &str,
    fields: &[(String, String)],
) -> Result<String> {
    let query = urlencoded_form_fields(fields);

    if let Ok(mut url) = Url::parse(resolved_action) {
        url.set_query(None);
        if !query.is_empty() {
            url.set_query(Some(&query));
        }
        return Ok(url.to_string());
    }

    let (without_fragment, fragment) = resolved_action
        .split_once('#')
        .map_or((resolved_action, ""), |(base, fragment)| (base, fragment));
    let base = without_fragment
        .split_once('?')
        .map_or(without_fragment, |(base, _)| base);
    let mut target = base.to_owned();
    if !query.is_empty() {
        target.push('?');
        target.push_str(&query);
    }
    if !fragment.is_empty() {
        target.push('#');
        target.push_str(fragment);
    }
    Ok(target)
}

pub fn build_post_form_body(form: &BrowserForm, overrides: &[(String, String)]) -> Result<String> {
    ensure!(
        form.method.eq_ignore_ascii_case("POST"),
        "cannot build POST form body for {} form",
        form.method
    );
    let fields = form_fields_with_overrides_and_submitter_fields(form, overrides, &[]);
    Ok(urlencoded_form_fields(&fields))
}

fn build_post_form_body_with_submitter(
    form: &BrowserForm,
    overrides: &[(String, String)],
    submitter: &BrowserFormSubmitter,
) -> Result<String> {
    let method = submitter.method.as_deref().unwrap_or(&form.method);
    ensure!(
        method.eq_ignore_ascii_case("POST"),
        "cannot build POST form body for {} form",
        method
    );
    let fields =
        form_fields_with_overrides_and_submitter_fields(form, overrides, &submitter.fields);
    Ok(urlencoded_form_fields(&fields))
}

fn form_fields_with_overrides_and_submitter_fields(
    form: &BrowserForm,
    overrides: &[(String, String)],
    submitter_fields: &[(String, String)],
) -> Vec<(String, String)> {
    let mut fields = successful_form_fields(form);
    for (name, value) in overrides {
        if let Some(existing) = fields.iter_mut().find(|(field_name, _)| field_name == name) {
            existing.1 = value.clone();
        } else {
            fields.push((name.clone(), value.clone()));
        }
    }
    fields.extend(
        submitter_fields
            .iter()
            .filter(|(name, _)| !name.is_empty())
            .cloned(),
    );
    fields
}

fn urlencoded_form_fields(fields: &[(String, String)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            fields
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        )
        .finish()
}

pub(super) fn successful_form_fields(form: &BrowserForm) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    for control in &form.controls {
        if control.disabled || control.name.is_empty() {
            continue;
        }
        match control.kind.as_str() {
            "checkbox" | "radio" if !control.checked => continue,
            "submit" | "button" | "reset" | "image" => continue,
            _ => {}
        }
        fields.push((control.name.clone(), control.value.clone()));
    }
    fields
}

pub(super) fn validate_supported_form_controls(
    form: &BrowserForm,
    overrides: &[(String, String)],
) -> Result<()> {
    if form.no_validate {
        return Ok(());
    }

    let checked_radio_names = form
        .controls
        .iter()
        .filter(|control| {
            control.kind.eq_ignore_ascii_case("radio") && !control.disabled && control.checked
        })
        .map(|control| control.name.clone())
        .collect::<HashSet<_>>();

    for (control_index, control) in form.controls.iter().enumerate() {
        if control.disabled || !control.required {
            continue;
        }
        if control.kind.eq_ignore_ascii_case("radio") {
            if !checked_radio_names.contains(&control.name) {
                bail!(
                    "form {} required radio group {:?} has no checked option",
                    form.index,
                    control.name
                );
            }
            continue;
        }
        if control.kind.eq_ignore_ascii_case("checkbox") {
            if !control.checked {
                bail!(
                    "form {} required {} is not checked",
                    form.index,
                    form_control_label(control_index, control)
                );
            }
            continue;
        }
        if !form_control_accepts_fill_state(control) {
            continue;
        }
        let value = effective_control_value(control, overrides);
        if value.is_empty() {
            bail!(
                "form {} required {} is empty",
                form.index,
                form_control_label(control_index, control)
            );
        }
        validate_control_value_syntax(form.index, control_index, control, &value)?;
    }

    for (control_index, control) in form.controls.iter().enumerate() {
        if control.disabled || control.required || !form_control_accepts_fill_state(control) {
            continue;
        }
        let value = effective_control_value(control, overrides);
        if !value.is_empty() {
            validate_control_value_syntax(form.index, control_index, control, &value)?;
        }
    }

    Ok(())
}

fn validate_control_value_syntax(
    form_index: usize,
    control_index: usize,
    control: &BrowserFormControl,
    value: &str,
) -> Result<()> {
    if control.kind.eq_ignore_ascii_case("email") && !is_valid_email_value(value) {
        bail!(
            "form {} {} has invalid email value {:?}",
            form_index,
            form_control_label(control_index, control),
            value
        );
    }
    if control.kind.eq_ignore_ascii_case("url") && Url::parse(value).is_err() {
        bail!(
            "form {} {} has invalid url value {:?}",
            form_index,
            form_control_label(control_index, control),
            value
        );
    }
    Ok(())
}

fn is_valid_email_value(value: &str) -> bool {
    if value.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && !domain.contains('@')
}

fn effective_control_value(control: &BrowserFormControl, overrides: &[(String, String)]) -> String {
    if !control.name.is_empty()
        && let Some((_, value)) = overrides
            .iter()
            .rev()
            .find(|(name, _)| name == &control.name)
    {
        return value.clone();
    }
    control.value.clone()
}

fn form_control_label(control_index: usize, control: &BrowserFormControl) -> String {
    if control.name.is_empty() {
        format!("control {control_index}")
    } else {
        format!("field {:?}", control.name)
    }
}

pub(super) fn effective_form_overrides(
    form: &BrowserForm,
    form_state: &HashMap<BrowserFormFieldKey, String>,
    form_index: usize,
    overrides: &[(String, String)],
) -> Vec<(String, String)> {
    let mut effective = form_state
        .iter()
        .filter(|(key, _)| {
            key.form_index == form_index
                && form.controls.iter().any(|control| {
                    control.name == key.name && form_control_accepts_fill_state(control)
                })
        })
        .map(|(key, value)| (key.name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (name, value) in overrides {
        if let Some((_, existing_value)) = effective
            .iter_mut()
            .find(|(field_name, _)| field_name == name)
        {
            *existing_value = value.clone();
        } else {
            effective.push((name.clone(), value.clone()));
        }
    }
    effective
}

pub(super) fn apply_form_state_to_render(
    render: &mut BrowserRender,
    form_state: &HashMap<BrowserFormFieldKey, String>,
) {
    for (key, value) in form_state {
        let Some(form) = render.forms.get_mut(key.form_index) else {
            continue;
        };
        for control in form
            .controls
            .iter_mut()
            .filter(|control| control.name == key.name && form_control_accepts_fill_state(control))
        {
            if form_control_accepts_select_state(control) {
                apply_select_value(control, value);
            } else {
                control.value = value.clone();
            }
        }
    }
    apply_form_visual_state_to_render(render);
}

pub(super) fn apply_form_checked_state_to_render(
    render: &mut BrowserRender,
    checked_state: &HashMap<BrowserFormControlKey, bool>,
) {
    for (key, checked) in checked_state {
        let Some(control) = render
            .forms
            .get_mut(key.form_index)
            .and_then(|form| form.controls.get_mut(key.control_index))
        else {
            continue;
        };
        if form_control_accepts_checked_state(control) {
            control.checked = *checked;
        }
    }
    apply_form_visual_state_to_render(render);
}

fn apply_form_visual_state_to_render(render: &mut BrowserRender) {
    let widget_text_by_node = render
        .forms
        .iter()
        .flat_map(|form| form.controls.iter())
        .filter(|control| control.renders_inline_widget)
        .filter_map(|control| {
            inline_widget_text_for_control(control).map(|text| (control.node_id, text))
        })
        .collect::<HashMap<_, _>>();
    if widget_text_by_node.is_empty() {
        return;
    }

    let (display_list, hit_targets) = (&mut render.display_list, &mut render.hit_targets);
    let mut x_delta_by_y = HashMap::<usize, isize>::new();
    for index in 0..display_list.len() {
        let Some(hit_target) = hit_targets.get_mut(index) else {
            continue;
        };
        if hit_target.text_runs.is_empty() {
            continue;
        }
        let (x, y, text) = match &mut display_list[index] {
            DisplayCommand::Text { x, y, text } | DisplayCommand::StyledText { x, y, text, .. } => {
                (x, *y, text)
            }
            _ => continue,
        };
        let old_width = text.chars().count();
        let x_delta = x_delta_by_y.get(&y).copied().unwrap_or(0);
        *x = add_signed_to_usize(*x, x_delta);
        if let Some((replacement, replacement_runs)) =
            replace_widget_text_runs(text, hit_target, &widget_text_by_node)
        {
            *text = replacement;
            hit_target.text_runs = replacement_runs;
        }
        let new_width = text.chars().count();
        let width_delta = new_width as isize - old_width as isize;
        if width_delta != 0 {
            *x_delta_by_y.entry(y).or_insert(0) += width_delta;
        }
    }
    render.text = render_text_from_display_list(&render.display_list);
}

fn inline_widget_text_for_control(control: &BrowserFormControl) -> Option<String> {
    let kind = control.kind.to_ascii_lowercase();
    match kind.as_str() {
        "hidden" => Some(String::new()),
        "checkbox" => Some(if control.checked { "[x]" } else { "[ ]" }.to_owned()),
        "radio" => Some(if control.checked { "(x)" } else { "( )" }.to_owned()),
        "submit" => Some(format!(
            "[{}]",
            non_empty_control_value(control).unwrap_or("Submit")
        )),
        "reset" => Some(format!(
            "[{}]",
            non_empty_control_value(control).unwrap_or("Reset")
        )),
        "button" => Some(format!(
            "[{}]",
            non_empty_control_value(control).unwrap_or("Button")
        )),
        "image" => Some(format!(
            "[{}]",
            non_empty_control_value(control).unwrap_or("image")
        )),
        "password" => Some(format!("[{}]", "*".repeat(control.value.chars().count()))),
        "select" => Some(format!("[{}]", selected_option_label(control))),
        "textarea" => Some(format!("[{}]", control.value)),
        _ => Some(format!(
            "[{}]",
            if control.value.is_empty() {
                control.placeholder.as_deref().unwrap_or_default()
            } else {
                control.value.as_str()
            }
        )),
    }
}

fn non_empty_control_value(control: &BrowserFormControl) -> Option<&str> {
    if !control.value.is_empty() {
        return Some(control.value.as_str());
    }
    control
        .widget_fallback_value
        .as_deref()
        .filter(|value| !value.is_empty())
}

fn selected_option_label(control: &BrowserFormControl) -> &str {
    control
        .options
        .iter()
        .find(|option| option.selected)
        .or_else(|| control.options.first())
        .map(|option| option.label.as_str())
        .unwrap_or_default()
}

fn replace_widget_text_runs(
    text: &str,
    hit_target: &DisplayHitTarget,
    widget_text_by_node: &HashMap<usize, String>,
) -> Option<(String, Vec<TextHitTargetRun>)> {
    let mut replacement = String::new();
    let mut replacement_runs = Vec::new();
    let mut cursor = 0usize;
    let mut changed = false;
    let text_width = text.chars().count();

    for run in &hit_target.text_runs {
        if run.start > cursor {
            push_replacement_piece(
                &mut replacement,
                &mut replacement_runs,
                &char_slice(text, cursor, run.start.saturating_sub(cursor)),
                None,
            );
        }

        let piece = char_slice(text, run.start, run.width);
        if let Some(target_node) = run.target_node
            && let Some(widget_text) = widget_text_by_node.get(&target_node)
        {
            changed |= piece != *widget_text;
            push_replacement_piece(
                &mut replacement,
                &mut replacement_runs,
                widget_text,
                run.target_node,
            );
        } else {
            push_replacement_piece(
                &mut replacement,
                &mut replacement_runs,
                &piece,
                run.target_node,
            );
        }
        cursor = run.start.saturating_add(run.width).min(text_width);
    }

    if cursor < text_width {
        push_replacement_piece(
            &mut replacement,
            &mut replacement_runs,
            &char_slice(text, cursor, text_width.saturating_sub(cursor)),
            None,
        );
    }

    changed.then_some((replacement, replacement_runs))
}

fn push_replacement_piece(
    text: &mut String,
    runs: &mut Vec<TextHitTargetRun>,
    piece: &str,
    target_node: Option<usize>,
) {
    let width = piece.chars().count();
    if width == 0 {
        return;
    }
    let start = text.chars().count();
    text.push_str(piece);
    if let Some(last) = runs.last_mut()
        && last.target_node == target_node
        && last.start.saturating_add(last.width) == start
    {
        last.width = last.width.saturating_add(width);
        return;
    }
    runs.push(TextHitTargetRun {
        start,
        width,
        target_node,
    });
}

fn char_slice(text: &str, start: usize, width: usize) -> String {
    text.chars().skip(start).take(width).collect()
}

fn add_signed_to_usize(value: usize, delta: isize) -> usize {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta as usize)
    }
}

fn render_text_from_display_list(display_list: &[DisplayCommand]) -> String {
    let mut lines = BTreeMap::<usize, String>::new();
    for command in display_list {
        match command {
            DisplayCommand::Text { y, text, .. } | DisplayCommand::StyledText { y, text, .. } => {
                lines.entry(*y).or_default().push_str(text);
            }
            _ => {}
        }
    }
    lines.into_values().collect::<Vec<_>>().join("\n")
}

pub(super) fn clear_form_state_for_form(
    form_state: &mut HashMap<BrowserFormFieldKey, String>,
    form_index: usize,
) {
    form_state.retain(|key, _| key.form_index != form_index);
}

pub(super) fn clear_form_checked_state_for_form(
    checked_state: &mut HashMap<BrowserFormControlKey, bool>,
    form_index: usize,
) {
    checked_state.retain(|key, _| key.form_index != form_index);
}

pub(super) fn form_control_accepts_fill_state(control: &BrowserFormControl) -> bool {
    form_control_kind_accepts_fill_state(control.kind.as_str()) && !control.disabled
}

pub(super) fn form_control_accepts_checked_state(control: &BrowserFormControl) -> bool {
    matches!(
        control.kind.to_ascii_lowercase().as_str(),
        "checkbox" | "radio"
    ) && !control.disabled
}

pub(super) fn form_control_accepts_focus_state(control: &BrowserFormControl) -> bool {
    form_control_accepts_fill_state(control)
        || form_control_accepts_checked_state(control)
        || form_control_accepts_form_action_state(control)
}

pub(super) fn form_control_accepts_form_action_state(control: &BrowserFormControl) -> bool {
    matches!(
        control.kind.to_ascii_lowercase().as_str(),
        "submit" | "reset"
    ) && !control.disabled
}

pub(super) fn default_form_submitter(form: &BrowserForm) -> BrowserFormSubmitter {
    form.controls
        .iter()
        .find(|control| form_control_is_submit_action(control))
        .map(form_control_submitter)
        .unwrap_or_default()
}

pub(super) fn form_control_is_submit_action(control: &BrowserFormControl) -> bool {
    control.kind.eq_ignore_ascii_case("submit") && !control.disabled
}

pub(super) fn form_control_is_reset_action(control: &BrowserFormControl) -> bool {
    control.kind.eq_ignore_ascii_case("reset") && !control.disabled
}

pub(super) fn form_control_submitter(control: &BrowserFormControl) -> BrowserFormSubmitter {
    if form_control_is_submit_action(control) && !control.name.is_empty() {
        BrowserFormSubmitter {
            fields: vec![(control.name.clone(), control.value.clone())],
            no_validate: control.form_no_validate,
            method: control.form_method.clone(),
            resolved_action: control.form_resolved_action.clone(),
        }
    } else if form_control_is_submit_action(control) {
        BrowserFormSubmitter {
            fields: Vec::new(),
            no_validate: control.form_no_validate,
            method: control.form_method.clone(),
            resolved_action: control.form_resolved_action.clone(),
        }
    } else {
        BrowserFormSubmitter::default()
    }
}

pub(super) fn form_control_accepts_text_edit_state(control: &BrowserFormControl) -> bool {
    form_control_kind_accepts_text_edit_state(control.kind.as_str()) && !control.disabled
}

pub(super) fn form_control_accepts_select_state(control: &BrowserFormControl) -> bool {
    control.kind.eq_ignore_ascii_case("select") && !control.disabled
}

pub(super) fn form_control_has_enabled_option(control: &BrowserFormControl, value: &str) -> bool {
    control
        .options
        .iter()
        .any(|option| !option.disabled && option.value == value)
}

pub(super) fn apply_select_value(control: &mut BrowserFormControl, value: &str) {
    control.value = value.to_owned();
    for option in &mut control.options {
        option.selected = option.value == value;
    }
}

pub(super) fn select_options(dom: &Dom, select_node_id: usize) -> Vec<BrowserFormOption> {
    let mut options = Vec::new();
    collect_select_options(dom, select_node_id, &mut options);
    options
}

pub(super) fn select_value(options: &[BrowserFormOption]) -> Option<String> {
    options
        .iter()
        .find(|option| option.selected)
        .or_else(|| options.first())
        .map(|option| option.value.clone())
}

fn form_control_kind_accepts_fill_state(kind: &str) -> bool {
    form_control_kind_accepts_text_edit_state(kind) || kind.eq_ignore_ascii_case("select")
}

fn form_control_kind_accepts_text_edit_state(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "text"
            | "search"
            | "url"
            | "email"
            | "password"
            | "tel"
            | "number"
            | "date"
            | "datetime-local"
            | "month"
            | "time"
            | "week"
            | "color"
            | "textarea"
    )
}

fn collect_select_options(dom: &Dom, node_id: usize, options: &mut Vec<BrowserFormOption>) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    if let NodeKind::Element(element) = &node.kind
        && element.tag == "option"
    {
        let value = element
            .value
            .clone()
            .unwrap_or_else(|| collapse_ascii_whitespace(&text_content(dom, node_id)));
        options.push(BrowserFormOption {
            value,
            label: collapse_ascii_whitespace(&text_content(dom, node_id)),
            disabled: element.disabled,
            selected: element.selected,
        });
    }
    for &child in &node.children {
        collect_select_options(dom, child, options);
    }
}

pub(super) fn collect_forms(dom: &Dom, source: &str) -> Vec<BrowserForm> {
    let mut form_node_ids = Vec::new();
    collect_form_node_ids(dom, 0, &mut form_node_ids);
    form_node_ids
        .into_iter()
        .enumerate()
        .filter_map(|(index, node_id)| build_form(dom, source, node_id, index))
        .collect()
}

pub(super) fn build_form(
    dom: &Dom,
    source: &str,
    node_id: usize,
    index: usize,
) -> Option<BrowserForm> {
    let element = match dom.nodes.get(node_id).map(|node| &node.kind) {
        Some(NodeKind::Element(element)) => element,
        _ => return None,
    };
    let action = element.action.clone().unwrap_or_default();
    let resolved_action = if action.trim().is_empty() {
        source.to_owned()
    } else {
        resolve_browser_href(source, action.trim())
    };
    let method = match element.method.as_deref() {
        Some(method) if method.eq_ignore_ascii_case("POST") => "POST",
        _ => "GET",
    }
    .to_owned();
    let mut controls = Vec::new();
    collect_form_controls(dom, source, node_id, &mut controls);
    Some(BrowserForm {
        index,
        method,
        action,
        resolved_action,
        no_validate: element.attrs.contains_key("novalidate"),
        controls,
    })
}

pub(super) fn nearest_form_ancestor(dom: &Dom, mut node_id: usize) -> Option<usize> {
    loop {
        let node = dom.nodes.get(node_id)?;
        node_id = node.parent?;
        let parent = dom.nodes.get(node_id)?;
        if matches!(&parent.kind, NodeKind::Element(element) if element.tag == "form") {
            return Some(node_id);
        }
    }
}

pub(super) fn form_index_for_node(dom: &Dom, form_node_id: usize) -> Option<usize> {
    let mut form_node_ids = Vec::new();
    collect_form_node_ids(dom, 0, &mut form_node_ids);
    form_node_ids
        .iter()
        .position(|candidate| *candidate == form_node_id)
}

pub(super) fn form_node_id_for_index(dom: &Dom, form_index: usize) -> Option<usize> {
    let mut form_node_ids = Vec::new();
    collect_form_node_ids(dom, 0, &mut form_node_ids);
    form_node_ids.get(form_index).copied()
}

pub(super) fn form_control_index_for_node(
    dom: &Dom,
    form_node_id: usize,
    target_node_id: usize,
) -> Option<usize> {
    let mut index = 0;
    form_control_index_for_node_at(dom, form_node_id, target_node_id, &mut index)
}

fn collect_form_node_ids(dom: &Dom, node_id: usize, form_node_ids: &mut Vec<usize>) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    if matches!(&node.kind, NodeKind::Element(element) if element.tag == "form") {
        form_node_ids.push(node_id);
    }
    for &child in &node.children {
        collect_form_node_ids(dom, child, form_node_ids);
    }
}

fn collect_form_controls(
    dom: &Dom,
    source: &str,
    node_id: usize,
    controls: &mut Vec<BrowserFormControl>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    if let NodeKind::Element(element) = &node.kind {
        match element.tag.as_str() {
            "input" => {
                let kind = element
                    .input_type
                    .clone()
                    .unwrap_or_else(|| "text".to_owned());
                let kind_lower = kind.to_ascii_lowercase();
                let value = if matches!(kind_lower.as_str(), "checkbox" | "radio") {
                    element.value.clone().unwrap_or_else(|| "on".to_owned())
                } else {
                    element.value.clone().unwrap_or_default()
                };
                let widget_fallback_value = input_widget_fallback_value(element, &kind_lower);
                controls.push(BrowserFormControl {
                    name: element.name.clone().unwrap_or_default(),
                    kind,
                    value,
                    disabled: element.disabled,
                    required: element.attrs.contains_key("required"),
                    form_no_validate: element.attrs.contains_key("formnovalidate"),
                    form_method: submitter_form_method(element),
                    form_action: submitter_form_action(element),
                    form_resolved_action: submitter_resolved_form_action(element, source),
                    checked: element.checked,
                    options: Vec::new(),
                    node_id,
                    renders_inline_widget: true,
                    placeholder: element.attrs.get("placeholder").cloned(),
                    widget_fallback_value,
                });
            }
            "textarea" => controls.push(BrowserFormControl {
                name: element.name.clone().unwrap_or_default(),
                kind: "textarea".to_owned(),
                value: element
                    .value
                    .clone()
                    .unwrap_or_else(|| text_content(dom, node_id)),
                disabled: element.disabled,
                required: element.attrs.contains_key("required"),
                form_no_validate: false,
                form_method: None,
                form_action: None,
                form_resolved_action: None,
                checked: false,
                options: Vec::new(),
                node_id,
                renders_inline_widget: true,
                placeholder: element.attrs.get("placeholder").cloned(),
                widget_fallback_value: None,
            }),
            "select" => {
                let options = select_options(dom, node_id);
                controls.push(BrowserFormControl {
                    name: element.name.clone().unwrap_or_default(),
                    kind: "select".to_owned(),
                    value: select_value(&options).unwrap_or_default(),
                    disabled: element.disabled,
                    required: element.attrs.contains_key("required"),
                    form_no_validate: false,
                    form_method: None,
                    form_action: None,
                    form_resolved_action: None,
                    checked: false,
                    options,
                    node_id,
                    renders_inline_widget: true,
                    placeholder: None,
                    widget_fallback_value: None,
                });
            }
            "button" => controls.push(BrowserFormControl {
                name: element.name.clone().unwrap_or_default(),
                kind: element
                    .input_type
                    .clone()
                    .unwrap_or_else(|| "submit".to_owned()),
                value: element.value.clone().unwrap_or_default(),
                disabled: element.disabled,
                required: false,
                form_no_validate: element.attrs.contains_key("formnovalidate"),
                form_method: submitter_form_method(element),
                form_action: submitter_form_action(element),
                form_resolved_action: submitter_resolved_form_action(element, source),
                checked: false,
                options: Vec::new(),
                node_id,
                renders_inline_widget: false,
                placeholder: None,
                widget_fallback_value: None,
            }),
            _ => {}
        }
    }

    for &child in &node.children {
        collect_form_controls(dom, source, child, controls);
    }
}

fn input_widget_fallback_value(element: &ElementData, kind: &str) -> Option<String> {
    match kind {
        "submit" if element.value.is_none() => Some("Submit".to_owned()),
        "reset" if element.value.is_none() => Some("Reset".to_owned()),
        "button" if element.value.is_none() => Some("Button".to_owned()),
        "image" => element
            .alt
            .clone()
            .or_else(|| element.value.clone())
            .or_else(|| Some("image".to_owned())),
        _ => None,
    }
}

pub(super) fn submitter_form_method(element: &ElementData) -> Option<String> {
    match element.attrs.get("formmethod").map(|method| method.trim()) {
        Some(method) if method.eq_ignore_ascii_case("POST") => Some("POST".to_owned()),
        Some(method) if method.eq_ignore_ascii_case("GET") => Some("GET".to_owned()),
        _ => None,
    }
}

pub(super) fn submitter_form_action(element: &ElementData) -> Option<String> {
    element
        .attrs
        .get("formaction")
        .map(|action| action.trim().to_owned())
        .filter(|action| !action.is_empty())
}

pub(super) fn submitter_resolved_form_action(
    element: &ElementData,
    source: &str,
) -> Option<String> {
    submitter_form_action(element).map(|action| resolve_browser_href(source, &action))
}

fn form_control_index_for_node_at(
    dom: &Dom,
    node_id: usize,
    target_node_id: usize,
    index: &mut usize,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    if let NodeKind::Element(element) = &node.kind
        && is_form_control_element(element)
    {
        let current = *index;
        *index += 1;
        if node_id == target_node_id {
            return Some(current);
        }
    }
    for &child in &node.children {
        if let Some(found) = form_control_index_for_node_at(dom, child, target_node_id, index) {
            return Some(found);
        }
    }
    None
}

fn is_form_control_element(element: &ElementData) -> bool {
    matches!(
        element.tag.as_str(),
        "input" | "textarea" | "select" | "button"
    )
}
