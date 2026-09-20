use super::{
    ComputedStyle, CssCascade, Display, Dom, ElementData, NodeKind, computed_style,
    scale_image_dimension,
};

pub(super) enum LazyMediaLeadingContent<'a> {
    Image(&'a ElementData),
    Text,
}

pub(super) fn lazy_media_container_reserved_height(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    width: usize,
) -> Option<usize> {
    lazy_media_only_container_reserved_height(dom, node_id, css_cascade, width)
        .or_else(|| lazy_media_leading_child_reserved_height(dom, node_id, css_cascade, width))
}

pub(super) fn lazy_media_positioning_context_height(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    width: usize,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    let mut preceding_flow_height = 0usize;
    for &child in &node.children {
        let Some(content) = first_lazy_media_or_visible_text(dom, child, css_cascade) else {
            preceding_flow_height = preceding_flow_height
                .saturating_add(simple_flow_child_reserved_height(dom, child, css_cascade));
            continue;
        };
        match content {
            LazyMediaLeadingContent::Text => return None,
            LazyMediaLeadingContent::Image(image) => {
                if lazy_media_child_reserves_own_flow(dom, child, css_cascade) {
                    return None;
                }
                let (intrinsic_width, intrinsic_height) = declared_image_intrinsic_size(image)?;
                let media_height =
                    scale_image_dimension(width.max(1), intrinsic_height, intrinsic_width)
                        .or(Some(intrinsic_height))?
                        .clamp(1, 24);
                return Some(if preceding_flow_height > 0 {
                    preceding_flow_height
                } else {
                    media_height
                });
            }
        }
    }
    None
}

pub(super) fn first_lazy_media_or_visible_text<'a>(
    dom: &'a Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> Option<LazyMediaLeadingContent<'a>> {
    let node = dom.nodes.get(node_id)?;
    match &node.kind {
        NodeKind::Text(text) => (!text.trim().is_empty()).then_some(LazyMediaLeadingContent::Text),
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            if style.display == Display::None {
                return None;
            }
            if element.tag == "img"
                && image_has_lazy_load_metadata(element)
                && declared_image_intrinsic_size(element).is_some()
            {
                return Some(LazyMediaLeadingContent::Image(element));
            }
            node.children
                .iter()
                .find_map(|&child| first_lazy_media_or_visible_text(dom, child, css_cascade))
        }
        NodeKind::Document | NodeKind::DocumentFragment => node
            .children
            .iter()
            .find_map(|&child| first_lazy_media_or_visible_text(dom, child, css_cascade)),
    }
}

pub(super) fn lazy_media_child_reserves_own_flow(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    style.display.is_block_flow()
        && !style.position.is_out_of_flow()
        && lazy_media_only_container_reserved_height(dom, node_id, css_cascade, 1).is_some()
}

pub(super) fn image_has_explicit_layout_size(element: &ElementData, style: &ComputedStyle) -> bool {
    style.height.is_some()
        || style.aspect_ratio.is_some()
        || element.attrs.contains_key("width")
        || element.attrs.contains_key("height")
}

pub(super) fn declared_image_intrinsic_size(element: &ElementData) -> Option<(usize, usize)> {
    first_non_empty_element_attr(
        element,
        &["data-image-dimensions", "data-dimensions", "data-size"],
    )
    .and_then(parse_image_dimension_pair_cells)
    .or_else(|| {
        for (width_attr, height_attr) in [
            ("data-image-width", "data-image-height"),
            ("data-width", "data-height"),
            ("data-original-width", "data-original-height"),
            ("data-natural-width", "data-natural-height"),
            ("data-img-width", "data-img-height"),
        ] {
            let width = element
                .attrs
                .get(width_attr)
                .and_then(|value| parse_image_dimension_pixels(value));
            let height = element
                .attrs
                .get(height_attr)
                .and_then(|value| parse_image_dimension_pixels(value));
            if let Some(size) = width
                .zip(height)
                .and_then(|(width, height)| image_pixel_size_to_cells(width, height))
            {
                return Some(size);
            }
        }
        None
    })
}

pub(super) fn image_has_lazy_load_metadata(element: &ElementData) -> bool {
    [
        "data-src",
        "data-srcset",
        "data-image",
        "data-image-src",
        "data-original",
        "data-lazy-src",
        "data-lazy-srcset",
        "data-load",
        "data-loaded",
        "loading",
    ]
    .iter()
    .any(|attr| element.attrs.contains_key(*attr))
}

fn lazy_media_only_container_reserved_height(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    width: usize,
) -> Option<usize> {
    if lazy_media_subtree_has_visible_text(dom, node_id, css_cascade) {
        return None;
    }
    let image = first_lazy_media_descendant(dom, node_id, css_cascade)?;
    let (intrinsic_width, intrinsic_height) = declared_image_intrinsic_size(image)?;
    scale_image_dimension(width.max(1), intrinsic_height, intrinsic_width)
        .or(Some(intrinsic_height))
        .map(|height| height.clamp(1, 24))
}

fn lazy_media_leading_child_reserved_height(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    width: usize,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    for &child in &node.children {
        let Some(content) = first_lazy_media_or_visible_text(dom, child, css_cascade) else {
            continue;
        };
        match content {
            LazyMediaLeadingContent::Text => return None,
            LazyMediaLeadingContent::Image(image) => {
                if lazy_media_child_reserves_own_flow(dom, child, css_cascade) {
                    return None;
                }
                let (intrinsic_width, intrinsic_height) = declared_image_intrinsic_size(image)?;
                return scale_image_dimension(width.max(1), intrinsic_height, intrinsic_width)
                    .or(Some(intrinsic_height))
                    .map(|height| height.clamp(1, 24));
            }
        }
    }
    None
}

fn simple_flow_child_reserved_height(dom: &Dom, node_id: usize, css_cascade: &CssCascade) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return 0;
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    if style.display == Display::None
        || style.position.is_out_of_flow()
        || !style.display.is_block_flow()
    {
        return 0;
    }
    let border_height = style
        .border
        .map(|border| border.width.saturating_mul(2))
        .unwrap_or(0);
    style
        .resolved_outer_height_for_containing_block(None)
        .unwrap_or_else(|| {
            style
                .resolved_min_outer_height_for_containing_block(None)
                .max(style.padding.top.saturating_add(style.padding.bottom))
                .max(border_height)
        })
        .saturating_add(style.margin.top)
        .saturating_add(style.margin.bottom)
}

fn lazy_media_subtree_has_visible_text(
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
            style.display != Display::None
                && node
                    .children
                    .iter()
                    .any(|&child| lazy_media_subtree_has_visible_text(dom, child, css_cascade))
        }
        NodeKind::Document | NodeKind::DocumentFragment => node
            .children
            .iter()
            .any(|&child| lazy_media_subtree_has_visible_text(dom, child, css_cascade)),
    }
}

fn first_lazy_media_descendant<'a>(
    dom: &'a Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> Option<&'a ElementData> {
    let node = dom.nodes.get(node_id)?;
    if let NodeKind::Element(element) = &node.kind {
        let style = computed_style(dom, node_id, element, css_cascade);
        if style.display == Display::None {
            return None;
        }
        if element.tag == "img"
            && image_has_lazy_load_metadata(element)
            && declared_image_intrinsic_size(element).is_some()
        {
            return Some(element);
        }
    }
    node.children
        .iter()
        .find_map(|&child| first_lazy_media_descendant(dom, child, css_cascade))
}

fn first_non_empty_element_attr<'a>(element: &'a ElementData, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| {
        element
            .attrs
            .get(*name)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    })
}

fn parse_image_dimension_pair_cells(value: &str) -> Option<(usize, usize)> {
    for separator in ['x', 'X', ','] {
        if let Some((width, height)) = value.split_once(separator) {
            let width = parse_image_dimension_pixels(width)?;
            let height = parse_image_dimension_pixels(height)?;
            return image_pixel_size_to_cells(width, height);
        }
    }
    None
}

fn parse_image_dimension_pixels(value: &str) -> Option<usize> {
    let value = value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches("px")
        .trim();
    let pixels = value.parse::<f32>().ok()?;
    if !pixels.is_finite() || pixels <= 0.0 {
        return None;
    }
    Some((pixels.ceil() as usize).max(1))
}

fn image_pixel_size_to_cells(width: usize, height: usize) -> Option<(usize, usize)> {
    (width > 0 && height > 0).then(|| (width.div_ceil(8).max(1), height.div_ceil(12).max(1)))
}
