use super::{
    ChildLayout, ComputedStyle, CssCascade, Display, Dom, FlexDirection, JustifyContent, NodeKind,
    Position, collapse_ascii_whitespace, computed_style, is_replaced_media_element,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct RowJustification {
    pub(super) leading_space: usize,
    pub(super) column_gap: usize,
}

#[derive(Debug, Clone)]
pub(super) struct RowFlexGrowth {
    pub(super) extra_widths: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct RowFlexShrink {
    pub(super) widths: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RowLayoutChildMargin {
    pub(super) left: usize,
    pub(super) right: usize,
    pub(super) left_auto: bool,
    pub(super) right_auto: bool,
}

impl RowLayoutChildMargin {
    pub(super) fn fixed_width(self) -> usize {
        self.left.saturating_add(self.right)
    }

    pub(super) fn auto_count(self) -> usize {
        usize::from(self.left_auto) + usize::from(self.right_auto)
    }
}

pub(super) fn child_layout_is_non_wrapping_flex_row(child_layout: ChildLayout) -> bool {
    child_layout.flex_items
        && child_layout.row_items
        && !child_layout.wrap_items
        && child_layout.wrap_after.is_none()
}

pub(super) fn row_layout_flex_growth(
    dom: &Dom,
    child_sequence: &[usize],
    css_cascade: &CssCascade,
    child_layout: ChildLayout,
    available_width: usize,
) -> Option<RowFlexGrowth> {
    if !child_layout_is_non_wrapping_flex_row(child_layout) {
        return None;
    }

    let mut content_width = row_layout_effective_column_gap(child_layout)
        .saturating_mul(child_sequence.len().saturating_sub(1));
    let mut grow_total = 0u64;
    let mut grow_factors = Vec::with_capacity(child_sequence.len());
    for &child in child_sequence {
        let base_width = row_layout_child_width_estimate(dom, child, css_cascade, available_width)
            .unwrap_or_else(|| {
                let margin = row_layout_child_horizontal_margin(dom, child, css_cascade);
                margin.fixed_width()
            });
        content_width = content_width.saturating_add(base_width);
        let grow = row_layout_child_flex_grow(dom, child, css_cascade);
        grow_total = grow_total.saturating_add(u64::from(grow));
        grow_factors.push(grow);
    }
    if grow_total == 0 {
        return None;
    }
    let remaining = available_width.checked_sub(content_width)?;
    if remaining == 0 {
        return None;
    }

    let mut assigned = 0usize;
    let mut extra_widths = Vec::with_capacity(grow_factors.len());
    for (index, grow) in grow_factors.iter().copied().enumerate() {
        let extra = if index == grow_factors.len().saturating_sub(1) {
            remaining.saturating_sub(assigned)
        } else if grow == 0 {
            0
        } else {
            ((remaining as u64).saturating_mul(u64::from(grow)) / grow_total) as usize
        };
        assigned = assigned.saturating_add(extra);
        extra_widths.push(extra);
    }
    Some(RowFlexGrowth { extra_widths })
}

pub(super) fn row_layout_flex_shrink(
    dom: &Dom,
    child_sequence: &[usize],
    css_cascade: &CssCascade,
    child_layout: ChildLayout,
    available_width: usize,
) -> Option<RowFlexShrink> {
    if !child_layout_is_non_wrapping_flex_row(child_layout) {
        return None;
    }

    let column_gap = row_layout_effective_column_gap(child_layout);
    let mut content_width = column_gap.saturating_mul(child_sequence.len().saturating_sub(1));
    let mut widths = Vec::with_capacity(child_sequence.len());
    let mut min_widths = Vec::with_capacity(child_sequence.len());
    let mut shrink_weights = Vec::with_capacity(child_sequence.len());
    for &child in child_sequence {
        let width = row_layout_child_width_estimate(dom, child, css_cascade, available_width)
            .unwrap_or_else(|| {
                let margin = row_layout_child_horizontal_margin(dom, child, css_cascade);
                margin.fixed_width()
            })
            .min(available_width.max(1));
        let min_width = row_layout_child_min_width(dom, child, css_cascade, available_width)
            .min(width)
            .min(available_width.max(1));
        let shrink = row_layout_child_flex_shrink(dom, child, css_cascade);
        content_width = content_width.saturating_add(width);
        widths.push(width);
        min_widths.push(min_width);
        shrink_weights.push(u64::from(shrink).saturating_mul(width as u64));
    }

    let mut remaining = content_width.checked_sub(available_width)?;
    if remaining == 0 {
        return None;
    }

    while remaining > 0 {
        let total_weight = widths
            .iter()
            .zip(&min_widths)
            .zip(&shrink_weights)
            .filter_map(|((&width, &min_width), &weight)| {
                (width > min_width && weight > 0).then_some(weight)
            })
            .sum::<u64>();
        if total_weight == 0 {
            break;
        }

        let mut assigned = 0usize;
        for index in 0..widths.len() {
            if widths[index] <= min_widths[index] || shrink_weights[index] == 0 {
                continue;
            }
            let capacity = widths[index].saturating_sub(min_widths[index]);
            let proportional =
                (remaining as u64).saturating_mul(shrink_weights[index]) / total_weight;
            let shrink = proportional.max(1).min(capacity as u64) as usize;
            widths[index] = widths[index].saturating_sub(shrink);
            assigned = assigned.saturating_add(shrink);
            if assigned >= remaining {
                break;
            }
        }
        if assigned == 0 {
            break;
        }
        remaining = remaining.saturating_sub(assigned);
    }

    Some(RowFlexShrink { widths })
}

pub(super) fn row_layout_auto_margin_width(
    dom: &Dom,
    child_sequence: &[usize],
    css_cascade: &CssCascade,
    child_layout: ChildLayout,
    available_width: usize,
) -> Option<usize> {
    if !child_layout_is_non_wrapping_flex_row(child_layout) {
        return None;
    }

    let mut content_width = row_layout_effective_column_gap(child_layout)
        .saturating_mul(child_sequence.len().saturating_sub(1));
    let mut auto_count = 0usize;
    for &child in child_sequence {
        let margin = row_layout_child_horizontal_margin(dom, child, css_cascade);
        auto_count = auto_count.saturating_add(margin.auto_count());
        let width = row_layout_child_width_estimate(dom, child, css_cascade, available_width)
            .unwrap_or_else(|| margin.fixed_width());
        content_width = content_width.saturating_add(width);
    }
    (auto_count > 0).then(|| available_width.saturating_sub(content_width) / auto_count)
}

pub(super) fn row_layout_effective_column_gap(child_layout: ChildLayout) -> usize {
    child_layout.column_gap.unwrap_or(1)
}

pub(super) fn row_layout_justification(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    child_layout: ChildLayout,
    available_width: usize,
) -> Option<RowJustification> {
    if !child_layout.row_items
        || child_layout.wrap_items
        || child_layout.wrap_after.is_some()
        || child_layout.justify_content == JustifyContent::Start
    {
        return None;
    }
    let node = dom.nodes.get(node_id)?;
    let mut item_widths = Vec::new();
    for &child in &node.children {
        if !row_layout_child_participates(dom, child, css_cascade) {
            continue;
        }
        item_widths.push(row_layout_child_width_estimate(
            dom,
            child,
            css_cascade,
            available_width,
        )?);
    }
    if item_widths.is_empty() {
        return None;
    }
    let base_gap = row_layout_effective_column_gap(child_layout);
    let gap_count = item_widths.len().saturating_sub(1);
    let content_width = item_widths
        .iter()
        .copied()
        .sum::<usize>()
        .saturating_add(base_gap.saturating_mul(gap_count));
    let remaining = available_width.checked_sub(content_width)?;
    match child_layout.justify_content {
        JustifyContent::Start => None,
        JustifyContent::Center => Some(RowJustification {
            leading_space: remaining / 2,
            column_gap: base_gap,
        }),
        JustifyContent::End => Some(RowJustification {
            leading_space: remaining,
            column_gap: base_gap,
        }),
        JustifyContent::SpaceBetween => Some(RowJustification {
            leading_space: 0,
            column_gap: if gap_count > 0 {
                base_gap.saturating_add(remaining / gap_count)
            } else {
                base_gap
            },
        }),
        JustifyContent::SpaceAround => {
            let item_count = item_widths.len();
            Some(RowJustification {
                leading_space: remaining / item_count.saturating_mul(2).max(1),
                column_gap: base_gap.saturating_add(remaining / item_count.max(1)),
            })
        }
        JustifyContent::SpaceEvenly => {
            let spacing_slots = item_widths.len().saturating_add(1);
            let distributed = remaining / spacing_slots.max(1);
            Some(RowJustification {
                leading_space: distributed,
                column_gap: base_gap.saturating_add(distributed),
            })
        }
    }
}

pub(super) fn row_layout_child_width_estimate(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    basis: usize,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    match &node.kind {
        NodeKind::Text(text) => {
            let width = collapse_ascii_whitespace(text).chars().count();
            (width > 0).then_some(width)
        }
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            if style.display == Display::None {
                return None;
            }
            let horizontal_margin =
                row_layout_child_horizontal_margin(dom, node_id, css_cascade).fixed_width();
            if let Some(width) = row_layout_style_outer_width(&style, basis) {
                return Some(
                    width
                        .saturating_add(horizontal_margin)
                        .clamp(1, basis.max(1)),
                );
            }
            if element.tag == "img"
                || element.tag == "svg"
                || is_replaced_media_element(&element.tag)
            {
                return Some(10usize.saturating_add(horizontal_margin).min(basis.max(1)));
            }
            let child_widths = node.children.iter().filter_map(|&child| {
                row_layout_child_width_estimate(dom, child, css_cascade, basis)
            });
            let child_layout = style.child_layout_for_width(basis);
            let child_width = if child_layout.flex_items && !child_layout.row_items {
                child_widths.max().unwrap_or(0)
            } else {
                child_widths.sum::<usize>()
            };
            if child_width > 0 {
                return Some(
                    child_width
                        .saturating_add(horizontal_margin)
                        .clamp(1, basis.max(1)),
                );
            }
            None
        }
        NodeKind::Document | NodeKind::DocumentFragment => None,
    }
}

pub(super) fn row_layout_child_width_hint(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    basis: usize,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    let NodeKind::Element(element) = &node.kind else {
        return None;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    let horizontal_margin =
        row_layout_child_horizontal_margin(dom, node_id, css_cascade).fixed_width();
    row_layout_style_outer_width(&style, basis).map(|width| {
        width
            .saturating_add(horizontal_margin)
            .clamp(1, basis.max(1))
    })
}

fn row_layout_style_outer_width(style: &ComputedStyle, basis: usize) -> Option<usize> {
    if style.flex_basis.is_none()
        && style.width.is_none()
        && style.max_width.is_none()
        && style.min_width.resolve(basis) == 0
    {
        return None;
    }
    let mut style = style.clone();
    if let Some(flex_basis) = style.flex_basis {
        style.width = Some(flex_basis);
    }
    Some(style.positioned_outer_width(basis))
}

pub(super) fn row_layout_child_horizontal_margin(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> RowLayoutChildMargin {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return RowLayoutChildMargin::default();
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    RowLayoutChildMargin {
        left: style.margin.left,
        right: style.margin.right,
        left_auto: style.margin_left_auto,
        right_auto: style.margin_right_auto,
    }
}

pub(super) fn row_layout_child_flex_grow(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> u32 {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return 0;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    if style.display == Display::None {
        0
    } else {
        style.flex_grow
    }
}

pub(super) fn row_layout_child_flex_shrink(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> u32 {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return 0;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    if style.display == Display::None {
        0
    } else {
        style.flex_shrink
    }
}

pub(super) fn row_layout_child_min_width(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    basis: usize,
) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return 0;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    let horizontal_margin =
        row_layout_child_horizontal_margin(dom, node_id, css_cascade).fixed_width();
    style
        .min_width
        .resolve(basis)
        .saturating_add(horizontal_margin)
        .min(basis.max(1))
}

pub(super) fn row_layout_child_participates(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    let Some(node) = dom.nodes.get(node_id) else {
        return false;
    };
    match &node.kind {
        NodeKind::Text(text) => !text.trim().is_empty(),
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            style.display != Display::None && !style.position.is_out_of_flow()
        }
        NodeKind::Document | NodeKind::DocumentFragment => true,
    }
}

pub(super) fn row_layout_child_is_out_of_flow(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    style.display != Display::None && style.position.is_out_of_flow()
}

pub(super) fn row_layout_child_has_constrained_block_slot(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    style.max_width.is_some() && !style.clips_overflow()
}

pub(super) fn row_item_should_inline_children(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    let Some(node) = dom.nodes.get(node_id) else {
        return false;
    };
    let mut any = false;
    for &child in &node.children {
        if !row_layout_child_participates(dom, child, css_cascade) {
            continue;
        }
        any = true;
        if !row_item_subtree_is_inline(dom, child, css_cascade, 0) {
            return false;
        }
    }
    any
}

pub(super) fn row_layout_child_is_inline_subtree(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    row_item_subtree_is_inline(dom, node_id, css_cascade, 0)
}

fn row_item_subtree_is_inline(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    depth: usize,
) -> bool {
    if depth > 6 {
        return false;
    }
    let Some(node) = dom.nodes.get(node_id) else {
        return false;
    };
    match &node.kind {
        NodeKind::Text(text) => !text.trim().is_empty(),
        NodeKind::Document | NodeKind::DocumentFragment => false,
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            if style.display == Display::None {
                return true;
            }
            if matches!(
                element.tag.as_str(),
                "img" | "svg" | "br" | "wbr" | "input" | "select"
            ) || is_replaced_media_element(&element.tag)
            {
                return true;
            }
            let sized_flex_row = row_item_subtree_sized_flex_row_can_inline(&style);
            if style.position.is_out_of_flow()
                || style.position == Position::Sticky
                || style.aspect_ratio.is_some()
                || style.clips_overflow()
                || (style.width.is_some() && style.display.is_block_flow())
                || ((style.resolved_height().is_some() || style.resolved_min_height() > 0)
                    && !sized_flex_row)
            {
                return false;
            }
            if matches!(
                element.tag.as_str(),
                "article"
                    | "section"
                    | "main"
                    | "aside"
                    | "header"
                    | "footer"
                    | "figure"
                    | "figcaption"
                    | "p"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "ul"
                    | "ol"
                    | "table"
                    | "tr"
                    | "td"
            ) {
                return false;
            }
            node.children
                .iter()
                .copied()
                .filter(|&child| row_layout_child_participates(dom, child, css_cascade))
                .all(|child| row_item_subtree_is_inline(dom, child, css_cascade, depth + 1))
        }
    }
}

fn row_item_subtree_sized_flex_row_can_inline(style: &ComputedStyle) -> bool {
    matches!(style.display, Display::Flex | Display::InlineFlex)
        && !matches!(
            style.flex_direction,
            FlexDirection::Column | FlexDirection::ColumnReverse
        )
        && !style.flex_wrap
}
