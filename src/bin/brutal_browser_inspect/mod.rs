use anyhow::Result;
use brutal_search::browser::{
    BrowserAccessibilityTreeReport, BrowserHitTestReport, BrowserLayerTreeReport,
    BrowserLayoutTreeReport,
};

pub(crate) fn print_hit_test(report: &BrowserHitTestReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    match &report.hit {
        Some(hit) => {
            println!(
                "hit x={} y={} kind={} command_index={} bounds={}x{}+{}+{} text={} alt={} url={} shade={}",
                report.x,
                report.y,
                hit.kind,
                hit.command_index,
                hit.width,
                hit.height,
                hit.x,
                hit.y,
                hit.text.as_deref().unwrap_or(""),
                hit.alt.as_deref().unwrap_or(""),
                hit.url.as_deref().unwrap_or(""),
                hit.shade.map(|shade| shade.to_string()).unwrap_or_default()
            );
        }
        None => println!("miss x={} y={}", report.x, report.y),
    }
    Ok(())
}

pub(crate) fn print_layer_tree(report: &BrowserLayerTreeReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "layers={} paint_commands={} viewport_width={}",
        report.layer_count, report.paint_command_count, report.viewport_width
    );
    for layer in &report.layers {
        println!(
            "layer id={} parent={} kind={} reason={} bounds={}x{}+{}+{} paint_order={} commands={:?}",
            layer.id,
            layer
                .parent
                .map(|parent| parent.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            layer.kind,
            layer.reason,
            layer.width,
            layer.height,
            layer.x,
            layer.y,
            layer.paint_order,
            layer.command_indices
        );
    }
    Ok(())
}

pub(crate) fn print_layout_tree(report: &BrowserLayoutTreeReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "layout_boxes={} retained_boxes={} viewport_width={}",
        report.layout_box_count, report.retained_box_count, report.viewport_width
    );
    for layout_box in &report.boxes {
        println!(
            "box id={} parent={} node={} tag={} kind={} bounds={}x{}+{}+{} children={:?} commands={:?}",
            layout_box.id,
            layout_box
                .parent
                .map(|parent| parent.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            layout_box.node_id,
            layout_box.tag,
            layout_box.kind,
            layout_box.width,
            layout_box.height,
            layout_box.x,
            layout_box.y,
            layout_box.children,
            layout_box.command_indices
        );
    }
    Ok(())
}

pub(crate) fn print_accessibility_tree(
    report: &BrowserAccessibilityTreeReport,
    json: bool,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "accessibility_tree source={} title={:?} nodes={}",
        report.source, report.title, report.node_count
    );
    for node in report.nodes.iter().filter(|node| node.parent.is_none()) {
        print_accessibility_node(report, node.id, 0);
    }
    Ok(())
}

fn print_accessibility_node(report: &BrowserAccessibilityTreeReport, node_id: usize, depth: usize) {
    let Some(node) = report.nodes.get(node_id) else {
        return;
    };
    let indent = "  ".repeat(depth);
    let mut parts = vec![
        format!("role={}", node.role),
        format!("name={:?}", node.name),
        format!("dom_node={}", node.dom_node_id),
    ];
    if let Some(tag) = &node.tag {
        parts.push(format!("tag={tag}"));
    }
    if let Some(level) = node.level {
        parts.push(format!("level={level}"));
    }
    if let Some(value) = &node.value {
        parts.push(format!("value={value:?}"));
    }
    if let Some(checked) = node.checked {
        parts.push(format!("checked={checked}"));
    }
    if let Some(disabled) = node.disabled {
        parts.push(format!("disabled={disabled}"));
    }
    println!("{indent}{}", parts.join(" "));
    for &child in &node.children {
        print_accessibility_node(report, child, depth + 1);
    }
}
