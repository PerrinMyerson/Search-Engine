use super::{Dom, ElementData, NodeKind};

pub(super) fn associated_label_control_node(dom: &Dom, node_id: usize) -> Option<usize> {
    let label_node_id = nearest_label_node(dom, node_id)?;
    let label = element_data(dom, label_node_id)?;
    if let Some(control_id) = label.attrs.get("for").map(|value| value.trim())
        && !control_id.is_empty()
    {
        return find_labelable_element_by_id(dom, control_id);
    }
    first_labelable_descendant(dom, label_node_id)
}

fn nearest_label_node(dom: &Dom, mut node_id: usize) -> Option<usize> {
    loop {
        let node = dom.nodes.get(node_id)?;
        if matches!(&node.kind, NodeKind::Element(element) if element.tag == "label") {
            return Some(node_id);
        }
        node_id = node.parent?;
    }
}

fn element_data(dom: &Dom, node_id: usize) -> Option<&ElementData> {
    match dom.nodes.get(node_id).map(|node| &node.kind) {
        Some(NodeKind::Element(element)) => Some(element),
        _ => None,
    }
}

fn find_labelable_element_by_id(dom: &Dom, id: &str) -> Option<usize> {
    dom.nodes
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element)
                if element.id.as_deref() == Some(id) && is_labelable_element(element) =>
            {
                Some(node_id)
            }
            _ => None,
        })
}

fn first_labelable_descendant(dom: &Dom, node_id: usize) -> Option<usize> {
    for &child in &dom.nodes.get(node_id)?.children {
        if let Some(element) = element_data(dom, child)
            && is_labelable_element(element)
        {
            return Some(child);
        }
        if let Some(found) = first_labelable_descendant(dom, child) {
            return Some(found);
        }
    }
    None
}

fn is_labelable_element(element: &ElementData) -> bool {
    match element.tag.as_str() {
        "button" | "meter" | "output" | "progress" | "select" | "textarea" => true,
        "input" => !element
            .input_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse_html;
    use super::*;

    #[test]
    fn explicit_label_for_resolves_labelable_control() {
        let parsed = parse_html(
            br#"<label for="fast">Fast</label><form><input id="fast" type="checkbox"></form>"#,
        );
        let label = first_element_by_tag(&parsed.dom, "label").unwrap();
        let input = first_element_by_tag(&parsed.dom, "input").unwrap();

        assert_eq!(
            associated_label_control_node(&parsed.dom, label),
            Some(input)
        );
    }

    #[test]
    fn wrapped_label_resolves_first_labelable_descendant() {
        let parsed = parse_html(
            br#"<form><label>Fast <span>now</span><input type="checkbox"></label></form>"#,
        );
        let span = first_element_by_tag(&parsed.dom, "span").unwrap();
        let input = first_element_by_tag(&parsed.dom, "input").unwrap();

        assert_eq!(
            associated_label_control_node(&parsed.dom, span),
            Some(input)
        );
    }

    fn first_element_by_tag(dom: &Dom, tag: &str) -> Option<usize> {
        dom.nodes
            .iter()
            .enumerate()
            .find_map(|(node_id, node)| match &node.kind {
                NodeKind::Element(element) if element.tag == tag => Some(node_id),
                _ => None,
            })
    }
}
