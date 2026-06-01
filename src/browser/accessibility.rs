use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{
    BrowserRenderOptions, CssCascade, Display, Dom, ElementData, NodeKind, TinyJsRuntime,
    collapse_ascii_whitespace, computed_style, dom_title, drain_timer_tasks,
    execute_scripts_with_runtime, load_target, parse_css, parse_html, text_content,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAccessibilityTreeReport {
    pub source: String,
    pub title: String,
    pub node_count: usize,
    pub nodes: Vec<BrowserAccessibilityNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserAccessibilityNode {
    pub id: usize,
    pub dom_node_id: usize,
    pub parent: Option<usize>,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    pub children: Vec<usize>,
}

#[derive(Debug, Clone)]
struct AccessibilityNodeDraft {
    dom_node_id: usize,
    role: String,
    name: String,
    tag: Option<String>,
    value: Option<String>,
    level: Option<u8>,
    disabled: Option<bool>,
    checked: Option<bool>,
}

#[derive(Debug, Default)]
struct AccessibilityTreeBuilder {
    nodes: Vec<BrowserAccessibilityNode>,
}

impl AccessibilityTreeBuilder {
    fn add_node(&mut self, parent: Option<usize>, draft: AccessibilityNodeDraft) -> usize {
        let id = self.nodes.len();
        self.nodes.push(BrowserAccessibilityNode {
            id,
            dom_node_id: draft.dom_node_id,
            parent,
            role: draft.role,
            name: draft.name,
            tag: draft.tag,
            value: draft.value,
            level: draft.level,
            disabled: draft.disabled,
            checked: draft.checked,
            children: Vec::new(),
        });
        if let Some(parent_id) = parent
            && let Some(parent_node) = self.nodes.get_mut(parent_id)
        {
            parent_node.children.push(id);
        }
        id
    }
}

pub async fn load_accessibility_tree(
    target: &str,
    options: BrowserRenderOptions,
) -> Result<BrowserAccessibilityTreeReport> {
    let (source, bytes) = load_target(target, options.max_bytes).await?;
    Ok(accessibility_tree_from_html(&source, &bytes))
}

pub fn accessibility_tree_from_html(source: &str, html: &[u8]) -> BrowserAccessibilityTreeReport {
    let parsed = parse_html(html);
    let mut dom = parsed.dom;
    let mut runtime = TinyJsRuntime::default();
    execute_scripts_with_runtime(&mut dom, &mut runtime, &parsed.inline_scripts);
    drain_timer_tasks(&mut dom, &mut runtime);

    let css_cascade = parse_css(&parsed.style_text);
    let title = dom_title(&dom);
    let mut builder = AccessibilityTreeBuilder::default();
    let root_id = builder.add_node(
        None,
        AccessibilityNodeDraft {
            dom_node_id: 0,
            role: "document".to_owned(),
            name: title.clone(),
            tag: None,
            value: None,
            level: None,
            disabled: None,
            checked: None,
        },
    );

    if let Some(root) = dom.nodes.first() {
        for &child in &root.children {
            collect_accessibility_node(
                &dom,
                child,
                &css_cascade,
                &mut builder,
                Some(root_id),
                false,
            );
        }
    }

    BrowserAccessibilityTreeReport {
        source: source.to_owned(),
        title,
        node_count: builder.nodes.len(),
        nodes: builder.nodes,
    }
}

fn collect_accessibility_node(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    builder: &mut AccessibilityTreeBuilder,
    parent: Option<usize>,
    suppress_text: bool,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    match &node.kind {
        NodeKind::Document | NodeKind::DocumentFragment => {
            for &child in &node.children {
                collect_accessibility_node(dom, child, css_cascade, builder, parent, suppress_text);
            }
        }
        NodeKind::Text(text) => {
            if suppress_text {
                return;
            }
            let name = collapse_ascii_whitespace(text);
            if name.is_empty() {
                return;
            }
            builder.add_node(
                parent,
                AccessibilityNodeDraft {
                    dom_node_id: node_id,
                    role: "text".to_owned(),
                    name,
                    tag: None,
                    value: None,
                    level: None,
                    disabled: None,
                    checked: None,
                },
            );
        }
        NodeKind::Element(element) => {
            if !element_is_accessibility_visible(dom, node_id, element, css_cascade) {
                return;
            }

            let own_node = accessibility_node_for_element(dom, node_id, element, css_cascade)
                .map(|draft| builder.add_node(parent, draft));
            let child_parent = own_node.or(parent);
            let child_suppress_text = own_node
                .and_then(|id| builder.nodes.get(id))
                .is_some_and(|node| role_uses_descendant_text_name(node.role.as_str()));

            for &child in &node.children {
                collect_accessibility_node(
                    dom,
                    child,
                    css_cascade,
                    builder,
                    child_parent,
                    suppress_text || child_suppress_text,
                );
            }
        }
    }
}

fn accessibility_node_for_element(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    css_cascade: &CssCascade,
) -> Option<AccessibilityNodeDraft> {
    let role = explicit_role(element).or_else(|| implicit_role(element))?;
    let name = accessible_name(dom, node_id, element, role.as_str(), css_cascade);
    if role == "image" && name.is_empty() {
        return None;
    }
    let level = heading_level(element);
    let checked = matches!(
        role.as_str(),
        "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio"
    )
    .then_some(element.checked);
    let disabled = element.disabled.then_some(true);
    let value = accessible_value(dom, node_id, element, role.as_str());

    Some(AccessibilityNodeDraft {
        dom_node_id: node_id,
        role,
        name,
        tag: Some(element.tag.clone()),
        value,
        level,
        disabled,
        checked,
    })
}

fn explicit_role(element: &ElementData) -> Option<String> {
    let role = element.attrs.get("role")?.split_whitespace().next()?.trim();
    if role.is_empty() || matches!(role, "none" | "presentation") {
        return None;
    }
    Some(role.to_ascii_lowercase())
}

fn implicit_role(element: &ElementData) -> Option<String> {
    match element.tag.as_str() {
        "a" if element
            .href
            .as_ref()
            .is_some_and(|href| !href.trim().is_empty()) =>
        {
            Some("link".to_owned())
        }
        "button" => Some("button".to_owned()),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some("heading".to_owned()),
        "img" => Some("image".to_owned()),
        "input" => input_accessibility_role(element),
        "textarea" => Some("textbox".to_owned()),
        "select" => Some("combobox".to_owned()),
        "option" => Some("option".to_owned()),
        "form" => Some("form".to_owned()),
        "main" => Some("main".to_owned()),
        "nav" => Some("navigation".to_owned()),
        "header" => Some("banner".to_owned()),
        "footer" => Some("contentinfo".to_owned()),
        "article" => Some("article".to_owned()),
        "aside" => Some("complementary".to_owned()),
        "ul" | "ol" => Some("list".to_owned()),
        "li" => Some("listitem".to_owned()),
        "table" => Some("table".to_owned()),
        "tr" => Some("row".to_owned()),
        "th" => Some("columnheader".to_owned()),
        "td" => Some("cell".to_owned()),
        _ => None,
    }
}

fn input_accessibility_role(element: &ElementData) -> Option<String> {
    match element.input_type.as_deref().unwrap_or("text") {
        "hidden" => None,
        "button" | "submit" | "reset" => Some("button".to_owned()),
        "checkbox" => Some("checkbox".to_owned()),
        "radio" => Some("radio".to_owned()),
        "range" => Some("slider".to_owned()),
        "number" => Some("spinbutton".to_owned()),
        "image" => Some("button".to_owned()),
        _ => Some("textbox".to_owned()),
    }
}

fn accessible_name(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    role: &str,
    css_cascade: &CssCascade,
) -> String {
    if let Some(label) = attr_non_empty(element, "aria-label") {
        return label;
    }
    if let Some(labelled_by) = attr_non_empty(element, "aria-labelledby") {
        let name = labelled_by
            .split_whitespace()
            .filter_map(|id| visible_text_for_id(dom, id, css_cascade))
            .collect::<Vec<_>>()
            .join(" ");
        if !name.is_empty() {
            return name;
        }
    }
    match element.tag.as_str() {
        "img" => element
            .alt
            .as_deref()
            .map(collapse_ascii_whitespace)
            .unwrap_or_default(),
        "input" | "textarea" | "select" => {
            control_accessible_name(dom, node_id, element, css_cascade)
        }
        "button" | "a" => visible_text_content(dom, node_id, css_cascade),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => visible_text_content(dom, node_id, css_cascade),
        _ if role_uses_descendant_text_name(role) => {
            visible_text_content(dom, node_id, css_cascade)
        }
        _ => attr_non_empty(element, "title").unwrap_or_default(),
    }
}

fn control_accessible_name(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    css_cascade: &CssCascade,
) -> String {
    if let Some(label) = associated_label_text(dom, node_id, element, css_cascade) {
        return label;
    }
    if matches!(
        element.input_type.as_deref(),
        Some("button" | "submit" | "reset" | "image")
    ) {
        return element
            .value
            .as_deref()
            .map(collapse_ascii_whitespace)
            .unwrap_or_else(|| default_input_button_name(element));
    }
    attr_non_empty(element, "placeholder")
        .or_else(|| attr_non_empty(element, "title"))
        .or_else(|| element.name.as_deref().map(collapse_ascii_whitespace))
        .unwrap_or_default()
}

fn accessible_value(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    role: &str,
) -> Option<String> {
    match element.tag.as_str() {
        "input" if !matches!(role, "button" | "checkbox" | "radio") => element.value.clone(),
        "textarea" => Some(
            element
                .value
                .clone()
                .unwrap_or_else(|| collapse_ascii_whitespace(&text_content(dom, node_id))),
        ),
        "select" => element.value.clone(),
        _ => None,
    }
}

fn default_input_button_name(element: &ElementData) -> String {
    match element.input_type.as_deref() {
        Some("submit") => "Submit".to_owned(),
        Some("reset") => "Reset".to_owned(),
        _ => String::new(),
    }
}

fn associated_label_text(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    css_cascade: &CssCascade,
) -> Option<String> {
    if let Some(id) = element.id.as_deref() {
        let label = dom
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(candidate_id, node)| match &node.kind {
                NodeKind::Element(label)
                    if label.tag == "label"
                        && label.attrs.get("for").is_some_and(|for_id| for_id == id) =>
                {
                    Some(visible_text_content(dom, candidate_id, css_cascade))
                }
                _ => None,
            })
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !label.is_empty() {
            return Some(label);
        }
    }

    let mut current = dom.nodes.get(node_id).and_then(|node| node.parent);
    while let Some(parent_id) = current {
        let Some(parent_node) = dom.nodes.get(parent_id) else {
            break;
        };
        if let NodeKind::Element(parent_element) = &parent_node.kind
            && parent_element.tag == "label"
        {
            let label = visible_text_content(dom, parent_id, css_cascade);
            if !label.is_empty() {
                return Some(label);
            }
        }
        current = parent_node.parent;
    }
    None
}

fn visible_text_for_id(dom: &Dom, id: &str, css_cascade: &CssCascade) -> Option<String> {
    dom.nodes
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element) if element.id.as_deref() == Some(id) => {
                let text = visible_text_content(dom, node_id, css_cascade);
                (!text.is_empty()).then_some(text)
            }
            _ => None,
        })
}

fn visible_text_content(dom: &Dom, node_id: usize, css_cascade: &CssCascade) -> String {
    let mut out = String::new();
    collect_visible_text(dom, node_id, css_cascade, &mut out);
    collapse_ascii_whitespace(&out)
}

fn collect_visible_text(dom: &Dom, node_id: usize, css_cascade: &CssCascade, out: &mut String) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.kind {
        NodeKind::Text(text) => {
            out.push(' ');
            out.push_str(text);
        }
        NodeKind::Document | NodeKind::DocumentFragment => {
            for &child in &node.children {
                collect_visible_text(dom, child, css_cascade, out);
            }
        }
        NodeKind::Element(element) => {
            if !element_is_accessibility_visible(dom, node_id, element, css_cascade) {
                return;
            }
            for &child in &node.children {
                collect_visible_text(dom, child, css_cascade, out);
            }
        }
    }
}

fn element_is_accessibility_visible(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    css_cascade: &CssCascade,
) -> bool {
    if element
        .attrs
        .get("aria-hidden")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return false;
    }
    computed_style(dom, node_id, element, css_cascade).display != Display::None
}

fn attr_non_empty(element: &ElementData, name: &str) -> Option<String> {
    element
        .attrs
        .get(name)
        .map(|value| collapse_ascii_whitespace(value))
        .filter(|value| !value.is_empty())
}

fn role_uses_descendant_text_name(role: &str) -> bool {
    matches!(
        role,
        "button" | "link" | "heading" | "option" | "listitem" | "columnheader" | "cell"
    )
}

fn heading_level(element: &ElementData) -> Option<u8> {
    match element.tag.as_str() {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node<'a>(
        report: &'a BrowserAccessibilityTreeReport,
        role: &str,
        name: &str,
    ) -> &'a BrowserAccessibilityNode {
        report
            .nodes
            .iter()
            .find(|node| node.role == role && node.name == name)
            .unwrap_or_else(|| panic!("missing accessibility node role={role:?} name={name:?}"))
    }

    #[test]
    fn accessibility_tree_reports_static_roles_names_and_states() {
        let report = accessibility_tree_from_html(
            "fixture.html",
            br#"
            <html>
              <head>
                <title>AX Fixture</title>
                <style>.hidden { display: none }</style>
              </head>
              <body>
                <main>
                  <h1>Catalog</h1>
                  <a href="/next">Next page</a>
                  <button aria-labelledby="go-label">ignored</button>
                  <span id="go-label">Launch search</span>
                  <img src="hero.png" alt="Hero art">
                  <img src="decorative.png" alt="">
                  <label for="q">Search terms</label>
                  <input id="q" name="q" value="rust browser">
                  <input type="checkbox" name="fast" checked disabled>
                  <p class="hidden">Hidden text</p>
                  <p aria-hidden="true">Also hidden</p>
                </main>
              </body>
            </html>
            "#,
        );

        assert_eq!(report.title, "AX Fixture");
        assert_eq!(report.nodes[0].role, "document");
        assert_eq!(report.nodes[0].name, "AX Fixture");

        assert_eq!(node(&report, "heading", "Catalog").level, Some(1));
        assert_eq!(node(&report, "link", "Next page").tag.as_deref(), Some("a"));
        assert_eq!(
            node(&report, "button", "Launch search").tag.as_deref(),
            Some("button")
        );
        assert_eq!(
            node(&report, "image", "Hero art").tag.as_deref(),
            Some("img")
        );

        let textbox = node(&report, "textbox", "Search terms");
        assert_eq!(textbox.value.as_deref(), Some("rust browser"));

        let checkbox = report
            .nodes
            .iter()
            .find(|node| node.role == "checkbox")
            .expect("checkbox node");
        assert_eq!(checkbox.checked, Some(true));
        assert_eq!(checkbox.disabled, Some(true));

        assert!(!report.nodes.iter().any(|node| node.name == "Hidden text"));
        assert!(!report.nodes.iter().any(|node| node.name == "Also hidden"));
        assert!(!report.nodes.iter().any(|node| {
            node.role == "image" && node.tag.as_deref() == Some("img") && node.name.is_empty()
        }));
    }

    #[test]
    fn accessibility_tree_reflects_tiny_script_dom_mutations() {
        let report = accessibility_tree_from_html(
            "scripted.html",
            br#"
            <html><body>
              <button id="go">Before</button>
              <script>
                document.getElementById("go").textContent = "After";
              </script>
            </body></html>
            "#,
        );

        assert!(
            report
                .nodes
                .iter()
                .any(|node| node.role == "button" && node.name == "After")
        );
        assert!(!report.nodes.iter().any(|node| node.name == "Before"));
    }
}
