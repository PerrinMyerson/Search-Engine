use std::path::PathBuf;

use anyhow::{Result, anyhow};
use brutal_search::browser::{
    BrowserDocumentViewportReport, BrowserViewportFrame, BrowserViewportState,
};

pub(crate) fn parse_previous_viewport_state(
    x: Option<usize>,
    y: Option<usize>,
    width: Option<usize>,
    height: Option<usize>,
) -> Result<Option<BrowserViewportState>> {
    match (x, y, width, height) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(width), Some(height)) => Ok(Some(BrowserViewportState {
            x,
            y,
            width,
            height,
        })),
        _ => Err(anyhow!(
            "previous viewport requires --previous-x, --previous-y, --previous-width, and --previous-height"
        )),
    }
}

pub(crate) fn print_document_viewport(
    report: &BrowserDocumentViewportReport,
    json: bool,
) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "viewport: current={}x{}+{}+{} requested={}x{}+{}+{} max={}+{} document={}x{} scroll_delta={}+{} commands visible={} culled={} boxes visible={}/{} invalidated_area={} reused_area={} full_repaint={}",
        report.viewport.width,
        report.viewport.height,
        report.viewport.x,
        report.viewport.y,
        report.requested.width,
        report.requested.height,
        report.requested.x,
        report.requested.y,
        report.max_scroll_x,
        report.max_scroll_y,
        report.document_width,
        report.document_height,
        report.scroll_delta_x,
        report.scroll_delta_y,
        report.visible_command_count,
        report.culled_command_count,
        report.visible_layout_box_count,
        report.layout_box_count,
        report.invalidated_area,
        report.reused_area,
        report.full_repaint
    );
    for region in &report.invalidated_regions {
        println!(
            "dirty: {}x{}+{}+{}",
            region.width, region.height, region.x, region.y
        );
    }
    Ok(())
}

pub(crate) fn print_viewport_frame(
    frame: &BrowserViewportFrame,
    output: Option<&PathBuf>,
    json: bool,
) -> Result<()> {
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, frame.raster.encode_png()?)?;
    }

    let report = &frame.report;
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }

    println!(
        "viewport_frame: frame={}x{} viewport={}x{}+{}+{} dirty_pixels={} dirty_pixel_area={} commands visible={} culled={} ink={} format={} hash={}",
        report.frame_width,
        report.frame_height,
        report.viewport.viewport.width,
        report.viewport.viewport.height,
        report.viewport.viewport.x,
        report.viewport.viewport.y,
        report.dirty_pixel_regions.len(),
        report.dirty_pixel_area,
        report.frame.visible_command_count,
        report.frame.culled_command_count,
        report.non_background_pixels,
        report.artifact_format,
        report.pixel_hash
    );
    for region in &report.dirty_pixel_regions {
        println!(
            "dirty_pixel: {}x{}+{}+{} viewport={}x{}+{}+{}",
            region.width,
            region.height,
            region.x,
            region.y,
            region.viewport_width,
            region.viewport_height,
            region.viewport_x,
            region.viewport_y
        );
    }
    if let Some(path) = output {
        println!("wrote: {}", path.display());
    }
    Ok(())
}
