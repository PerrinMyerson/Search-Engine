use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    DisplayCommand, DisplayHitTarget, Dom, ElementData, NodeKind, display_command_bounds,
    is_descendant_of,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFragmentTarget {
    pub name: String,
    pub y: usize,
}

pub(super) fn source_fragment(source: &str) -> Option<String> {
    if let Ok(url) = Url::parse(source) {
        return url
            .fragment()
            .filter(|fragment| !fragment.is_empty())
            .map(str::to_owned);
    }
    source
        .split_once('#')
        .map(|(_, fragment)| fragment)
        .filter(|fragment| !fragment.is_empty())
        .map(str::to_owned)
}

pub(super) fn collect_fragment_targets(
    dom: &Dom,
    display_list: &[DisplayCommand],
    hit_targets: &[DisplayHitTarget],
) -> Vec<BrowserFragmentTarget> {
    let mut seen = std::collections::HashSet::new();
    let mut fragments = Vec::new();
    for (node_id, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = &node.kind else {
            continue;
        };
        for name in fragment_names_for_element(element) {
            if seen.insert(name.clone())
                && let Some(y) = first_display_y_for_node(dom, display_list, hit_targets, node_id)
            {
                fragments.push(BrowserFragmentTarget { name, y });
            }
        }
    }
    fragments
}

fn fragment_names_for_element(element: &ElementData) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(id) = element.id.as_deref().map(str::trim)
        && !id.is_empty()
    {
        names.push(id.to_owned());
    }
    if element.tag == "a"
        && let Some(name) = element.name.as_deref().map(str::trim)
        && !name.is_empty()
        && !names.iter().any(|existing| existing == name)
    {
        names.push(name.to_owned());
    }
    names
}

fn first_display_y_for_node(
    dom: &Dom,
    display_list: &[DisplayCommand],
    hit_targets: &[DisplayHitTarget],
    node_id: usize,
) -> Option<usize> {
    display_list
        .iter()
        .zip(hit_targets.iter())
        .filter_map(|(command, target)| {
            display_target_references_node(dom, target, node_id)
                .then(|| display_command_bounds(command).y)
        })
        .min()
}

fn display_target_references_node(dom: &Dom, target: &DisplayHitTarget, node_id: usize) -> bool {
    target.target_node.is_some_and(|target_node| {
        target_node == node_id || is_descendant_of(dom, target_node, node_id)
    }) || target.text_runs.iter().any(|run| {
        run.target_node.is_some_and(|target_node| {
            target_node == node_id || is_descendant_of(dom, target_node, node_id)
        })
    })
}
