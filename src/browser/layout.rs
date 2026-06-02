use super::{CssCascade, CssListStyleType, Dom, NodeKind};

const NESTED_LIST_INDENT_CELLS: usize = 2;

pub(super) fn list_item_marker(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> Option<String> {
    let Some(parent_id) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
        return Some("- ".to_owned());
    };
    let Some(parent_node) = dom.nodes.get(parent_id) else {
        return Some("- ".to_owned());
    };
    let NodeKind::Element(parent) = &parent_node.kind else {
        return Some("- ".to_owned());
    };
    if let Some(list_style_type) = inherited_list_style_type(dom, node_id, css_cascade) {
        if list_style_type == CssListStyleType::NoMarker {
            return None;
        }
        if let Some(marker_style) =
            OrderedListMarkerStyle::from_css_list_style_type(list_style_type)
        {
            return Some(marker_style.marker(list_item_counter_value(
                dom,
                parent_node,
                parent,
                node_id,
                css_cascade,
            )));
        }
        if let Some(marker_style) =
            UnorderedListMarkerStyle::from_css_list_style_type(list_style_type)
        {
            return Some(marker_style.marker().to_owned());
        }
    }
    if let Some(item) = item_element(dom, node_id)
        && item.tag == "summary"
        && parent.tag == "details"
    {
        return Some(if parent.attrs.contains_key("open") {
            "v ".to_owned()
        } else {
            "> ".to_owned()
        });
    }
    if matches!(parent.tag.as_str(), "menu" | "ul") {
        let marker_style = item_element(dom, node_id)
            .and_then(|element| UnorderedListMarkerStyle::from_type_attr(element.attrs.get("type")))
            .or_else(|| UnorderedListMarkerStyle::from_type_attr(parent.attrs.get("type")))
            .unwrap_or_else(|| {
                UnorderedListMarkerStyle::default_for_depth(ancestor_list_count(dom, parent_id))
            });
        return Some(marker_style.marker().to_owned());
    }
    if parent.tag != "ol" {
        return Some(UnorderedListMarkerStyle::Disc.marker().to_owned());
    }

    let marker_style = item_element(dom, node_id)
        .and_then(|element| OrderedListMarkerStyle::from_item_type_attr(element.attrs.get("type")))
        .unwrap_or_else(|| OrderedListMarkerStyle::from_type_attr(parent.attrs.get("type")));
    Some(marker_style.marker(list_item_counter_value(
        dom,
        parent_node,
        parent,
        node_id,
        css_cascade,
    )))
}

fn list_item_counter_value(
    dom: &Dom,
    parent_node: &super::Node,
    parent: &super::ElementData,
    node_id: usize,
    css_cascade: &CssCascade,
) -> i64 {
    let ordered_parent = parent.tag == "ol";
    let reversed = ordered_parent && parent.attrs.contains_key("reversed");
    let start = parent
        .attrs
        .get("start")
        .and_then(|value| parse_list_counter_value(value))
        .unwrap_or_else(|| {
            if reversed && ordered_parent {
                parent_node
                    .children
                    .iter()
                    .filter(|&&child_id| is_rendered_list_item(dom, child_id, css_cascade))
                    .count()
                    .max(1) as i64
            } else {
                1
            }
        });
    let mut counter = start;

    for &child_id in &parent_node.children {
        if !is_rendered_list_item(dom, child_id, css_cascade) {
            continue;
        }
        let list_item = item_element(dom, child_id);
        let value = list_item
            .and_then(|element| element.attrs.get("value"))
            .and_then(|value| parse_list_counter_value(value))
            .unwrap_or(counter);
        if child_id == node_id {
            return value;
        }
        counter = if reversed {
            value.saturating_sub(1)
        } else {
            value.saturating_add(1)
        };
    }

    start
}

#[derive(Debug, Clone, Copy)]
enum UnorderedListMarkerStyle {
    Disc,
    Circle,
    Square,
}

impl UnorderedListMarkerStyle {
    fn default_for_depth(depth: usize) -> Self {
        match depth {
            0 => Self::Disc,
            1 => Self::Circle,
            _ => Self::Square,
        }
    }

    fn from_type_attr(value: Option<&String>) -> Option<Self> {
        let value = value?.trim();
        if value.eq_ignore_ascii_case("disc") {
            Some(Self::Disc)
        } else if value.eq_ignore_ascii_case("circle") {
            Some(Self::Circle)
        } else if value.eq_ignore_ascii_case("square") {
            Some(Self::Square)
        } else {
            None
        }
    }

    fn from_css_list_style_type(value: CssListStyleType) -> Option<Self> {
        match value {
            CssListStyleType::Disc => Some(Self::Disc),
            CssListStyleType::Circle => Some(Self::Circle),
            CssListStyleType::Square => Some(Self::Square),
            CssListStyleType::NoMarker
            | CssListStyleType::Decimal
            | CssListStyleType::LowerAlpha
            | CssListStyleType::UpperAlpha
            | CssListStyleType::LowerRoman
            | CssListStyleType::UpperRoman => None,
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Disc => "- ",
            Self::Circle => "o ",
            Self::Square => "* ",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum OrderedListMarkerStyle {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

impl OrderedListMarkerStyle {
    fn from_type_attr(value: Option<&String>) -> Self {
        match value.map(|value| value.trim()) {
            Some("a") => Self::LowerAlpha,
            Some("A") => Self::UpperAlpha,
            Some("i") => Self::LowerRoman,
            Some("I") => Self::UpperRoman,
            _ => Self::Decimal,
        }
    }

    fn from_item_type_attr(value: Option<&String>) -> Option<Self> {
        match value.map(|value| value.trim()) {
            Some("1") => Some(Self::Decimal),
            Some("a") => Some(Self::LowerAlpha),
            Some("A") => Some(Self::UpperAlpha),
            Some("i") => Some(Self::LowerRoman),
            Some("I") => Some(Self::UpperRoman),
            _ => None,
        }
    }

    fn from_css_list_style_type(value: CssListStyleType) -> Option<Self> {
        match value {
            CssListStyleType::Decimal => Some(Self::Decimal),
            CssListStyleType::LowerAlpha => Some(Self::LowerAlpha),
            CssListStyleType::UpperAlpha => Some(Self::UpperAlpha),
            CssListStyleType::LowerRoman => Some(Self::LowerRoman),
            CssListStyleType::UpperRoman => Some(Self::UpperRoman),
            CssListStyleType::NoMarker
            | CssListStyleType::Disc
            | CssListStyleType::Circle
            | CssListStyleType::Square => None,
        }
    }

    fn marker(self, value: i64) -> String {
        let marker = match self {
            Self::Decimal => value.to_string(),
            Self::LowerAlpha => alpha_marker(value, false).unwrap_or_else(|| value.to_string()),
            Self::UpperAlpha => alpha_marker(value, true).unwrap_or_else(|| value.to_string()),
            Self::LowerRoman => roman_marker(value, false).unwrap_or_else(|| value.to_string()),
            Self::UpperRoman => roman_marker(value, true).unwrap_or_else(|| value.to_string()),
        };
        format!("{marker}. ")
    }
}

fn parse_list_counter_value(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

fn alpha_marker(value: i64, uppercase: bool) -> Option<String> {
    let mut value = u64::try_from(value).ok()?;
    if value == 0 {
        return None;
    }
    let base = if uppercase { b'A' } else { b'a' };
    let mut chars = Vec::new();
    while value > 0 {
        value -= 1;
        chars.push((base + (value % 26) as u8) as char);
        value /= 26;
    }
    chars.reverse();
    Some(chars.into_iter().collect())
}

fn roman_marker(value: i64, uppercase: bool) -> Option<String> {
    let mut value = u16::try_from(value).ok()?;
    if value == 0 || value > 3999 {
        return None;
    }
    let numerals = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut marker = String::new();
    for (amount, numeral) in numerals {
        while value >= amount {
            marker.push_str(numeral);
            value -= amount;
        }
    }
    if uppercase {
        Some(marker)
    } else {
        Some(marker.to_ascii_lowercase())
    }
}

pub(super) fn nested_list_indent(dom: &Dom, node_id: usize) -> usize {
    if !is_list_container(dom, node_id) {
        return 0;
    }

    if ancestor_list_count(dom, node_id) > 0 {
        NESTED_LIST_INDENT_CELLS
    } else {
        0
    }
}

fn ancestor_list_count(dom: &Dom, node_id: usize) -> usize {
    let mut count = 0usize;
    let mut current = dom.nodes.get(node_id).and_then(|node| node.parent);
    while let Some(node_id) = current {
        if is_list_container(dom, node_id) {
            count = count.saturating_add(1);
        }
        current = dom.nodes.get(node_id).and_then(|node| node.parent);
    }
    count
}

fn is_list_container(dom: &Dom, node_id: usize) -> bool {
    matches!(
        dom.nodes.get(node_id).map(|node| &node.kind),
        Some(NodeKind::Element(element)) if matches!(element.tag.as_str(), "menu" | "ol" | "ul")
    )
}

fn is_rendered_list_item(dom: &Dom, node_id: usize, css_cascade: &CssCascade) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    super::computed_style(dom, node_id, element, css_cascade).display == super::Display::ListItem
}

fn inherited_list_style_type(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> Option<CssListStyleType> {
    let mut current = Some(node_id);
    while let Some(current_id) = current {
        let Some(node) = dom.nodes.get(current_id) else {
            return None;
        };
        if let NodeKind::Element(element) = &node.kind {
            let style = super::computed_style(dom, current_id, element, css_cascade);
            if let Some(list_style_type) = style.list_style_type {
                return Some(list_style_type);
            }
        }
        current = node.parent;
    }
    None
}

fn item_element(dom: &Dom, node_id: usize) -> Option<&super::ElementData> {
    dom.nodes.get(node_id).and_then(|node| match &node.kind {
        NodeKind::Element(element) => Some(element.as_ref()),
        _ => None,
    })
}
