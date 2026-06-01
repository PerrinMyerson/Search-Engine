use super::forms::{
    build_form, form_control_index_for_node, form_index_for_node, nearest_form_ancestor,
};
use super::{
    BrowserRender, Dom, NodeKind,
    forms::{form_control_accepts_focus_state, form_control_accepts_form_action_state},
    labels::associated_label_control_node,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserFocusedFormControl {
    pub(super) node_id: usize,
    pub(super) form_index: usize,
    pub(super) control_index: usize,
    pub(super) name: String,
    pub(super) kind: String,
}

pub(super) fn focusable_form_control_for_node(
    dom: &Dom,
    node_id: usize,
) -> Option<BrowserFocusedFormControl> {
    focusable_form_control_for_node_or_ancestor(dom, node_id).or_else(|| {
        associated_label_control_node(dom, node_id).and_then(|control_node_id| {
            focusable_form_control_for_node_or_ancestor(dom, control_node_id)
        })
    })
}

fn focusable_form_control_for_node_or_ancestor(
    dom: &Dom,
    node_id: usize,
) -> Option<BrowserFocusedFormControl> {
    let control_node_id = nearest_focusable_form_control_node(dom, node_id)?;
    let form_node_id = nearest_form_ancestor(dom, control_node_id)?;
    let form_index = form_index_for_node(dom, form_node_id)?;
    let control_index = form_control_index_for_node(dom, form_node_id, control_node_id)?;
    let form = build_form(dom, "mem://focus", form_node_id, form_index)?;
    let control = form.controls.get(control_index)?;
    if !form_control_accepts_focus_state(control)
        || (control.name.is_empty() && !form_control_accepts_form_action_state(control))
    {
        return None;
    }
    Some(BrowserFocusedFormControl {
        node_id: control_node_id,
        form_index,
        control_index,
        name: control.name.clone(),
        kind: control.kind.clone(),
    })
}

pub(super) fn focusable_controls_for_render(
    render: &BrowserRender,
) -> Vec<BrowserFocusedFormControl> {
    render
        .forms
        .iter()
        .flat_map(|form| {
            form.controls
                .iter()
                .enumerate()
                .filter(|(_, control)| {
                    form_control_accepts_focus_state(control)
                        && (!control.name.is_empty()
                            || form_control_accepts_form_action_state(control))
                })
                .map(|(control_index, control)| BrowserFocusedFormControl {
                    node_id: control.node_id,
                    form_index: form.index,
                    control_index,
                    name: control.name.clone(),
                    kind: control.kind.clone(),
                })
        })
        .collect()
}

fn nearest_focusable_form_control_node(dom: &Dom, mut node_id: usize) -> Option<usize> {
    loop {
        let node = dom.nodes.get(node_id)?;
        if let NodeKind::Element(element) = &node.kind {
            if matches!(
                element.tag.as_str(),
                "input" | "textarea" | "select" | "button"
            ) {
                return Some(node_id);
            }
            if element.tag == "form" {
                return None;
            }
        }
        node_id = node.parent?;
    }
}
