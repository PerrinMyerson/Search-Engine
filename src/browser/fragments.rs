use serde::{Deserialize, Serialize};
use url::Url;

use super::{DisplayCommand, DisplayHitTarget, Dom, ElementData, NodeKind, display_command_bounds};

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
    let min_display_y = min_display_y_by_node(dom, display_list, hit_targets);
    let mut seen = std::collections::HashSet::new();
    let mut fragments = Vec::new();
    for (node_id, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = &node.kind else {
            continue;
        };
        for name in fragment_names_for_element(element) {
            if seen.insert(name.clone())
                && let Some(&y) = min_display_y.get(&node_id)
            {
                fragments.push(BrowserFragmentTarget { name, y });
            }
        }
    }
    fragments
}

/// Minimum display-command y for every node, counting commands that target the
/// node itself or any of its descendants. Built in one pass by propagating each
/// command's y up the ancestor chain, stopping early once an ancestor already
/// records an equal-or-smaller y (its own ancestors then do too).
fn min_display_y_by_node(
    dom: &Dom,
    display_list: &[DisplayCommand],
    hit_targets: &[DisplayHitTarget],
) -> std::collections::HashMap<usize, usize> {
    let mut min_display_y = std::collections::HashMap::new();
    let mut record = |target_node: Option<usize>, y: usize| {
        let Some(mut node_id) = target_node else {
            return;
        };
        loop {
            match min_display_y.entry(node_id) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if *entry.get() <= y {
                        break;
                    }
                    entry.insert(y);
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(y);
                }
            }
            let Some(parent) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
                break;
            };
            node_id = parent;
        }
    };
    for (command, target) in display_list.iter().zip(hit_targets.iter()) {
        let y = display_command_bounds(command).y;
        record(target.target_node, y);
        for run in &target.text_runs {
            record(run.target_node, y);
        }
    }
    min_display_y
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
