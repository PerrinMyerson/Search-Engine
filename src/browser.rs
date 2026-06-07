use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use html_escape::decode_html_entities;
use memchr::memchr;
use serde::{Deserialize, Serialize};
use url::Url;

mod accessibility;
mod app;
mod cookies;
mod coverage;
#[cfg(test)]
mod coverage_tests;
mod events;
mod focus;
mod forms;
mod fragments;
mod images;
mod labels;
mod layout;
#[cfg(test)]
mod layout_tests;
mod raster_pgm;
mod resources;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod session_event_tests;
mod session_forms;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod style_tests;
#[cfg(test)]
mod viewport_tests;

pub use accessibility::{
    BrowserAccessibilityNode, BrowserAccessibilityTreeReport, accessibility_tree_from_html,
    load_accessibility_tree,
};
pub use app::{
    BrowserApp, BrowserAppAction, BrowserAppFindState, BrowserAppOptions, BrowserAppReport,
    BrowserAppTabSummary, BrowserAppWindowClickReport, BrowserAppWindowFrame,
    BrowserAppWindowFrameOptions, BrowserAppWindowFrameReport, BrowserAppWindowHit,
};
pub use cookies::{BrowserCookie, BrowserCookieJar};
pub use coverage::{
    BrowserCoverageGate, BrowserCoverageReport, BrowserFeatureCoverage, BrowserFeatureState,
    browser_coverage_report, unsupported_feature_summary,
};
pub use forms::{
    BrowserForm, BrowserFormControl, BrowserFormOption, build_get_form_url, build_post_form_body,
};
pub use fragments::BrowserFragmentTarget;
pub use resources::{
    BrowserImageRenderReport, BrowserResource, BrowserResourceFetch, BrowserResourceFetchReport,
    BrowserScriptRenderReport, BrowserStylesheetRenderReport,
};

use events::{
    BrowserClickDispatch, BrowserEventDispatch, BrowserEventPayload, BrowserEventPhase,
    BrowserEventTarget, JsEventListener, TinyJsEvent, begin_click_dispatch, begin_event_dispatch,
    dispatch_event_listener_group as dispatch_event_listener_group_core, event_path_to_window,
    restore_event_dispatch, restore_runtime_this_target, set_current_event_phase,
    set_runtime_this_target,
};
use focus::{
    BrowserFocusedFormControl, focusable_controls_for_render, focusable_form_control_for_node,
};
use forms::{
    BrowserFormControlKey, BrowserFormFieldKey, BrowserFormSubmission, BrowserFormSubmitter,
    apply_form_checked_state_to_render, apply_form_state_to_render,
    build_form_submission_with_submitter, clear_form_checked_state_for_form,
    clear_form_state_for_form, collect_forms, effective_form_overrides,
    form_control_accepts_checked_state, form_control_accepts_fill_state,
    form_control_accepts_select_state, form_control_accepts_text_edit_state,
    form_control_has_enabled_option, form_control_index_for_node, form_index_for_node,
    form_node_id_for_index, nearest_form_ancestor, select_options, select_value,
    submitter_form_method, submitter_resolved_form_action, validate_supported_form_controls,
};
use fragments::{collect_fragment_targets, source_fragment};
use images::{
    DecodedImage, DecodedImageEntry, DecodedImageInfo, background_image_render_source,
    decode_image_reference, decoded_cached_images, decoded_image_entry, image_render_source,
};
#[cfg(test)]
use images::{decode_simple_png, tiny_test_jpeg_bytes, tiny_test_jpeg_data_url};
use labels::associated_label_control_node;
use layout::{list_item_marker, nested_list_indent};
use raster_pgm::{compare_raster_with_pgm, diff_within_threshold, encode_diff_pgm};
use resources::{
    BrowserResourceCache, collect_resources, collect_selected_image_resources,
    fetch_resource_with_cache, load_post_form_target_with_cookie_jar, load_target,
    load_target_with_cookie_jar, local_path_without_url_parts,
};

const TABLE_COLUMN_GAP_CELLS: usize = 2;
const MAX_UNRESOLVED_IMAGE_PLACEHOLDER_HEIGHT: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct BrowserRenderOptions {
    pub width: usize,
    pub max_bytes: usize,
}

impl Default for BrowserRenderOptions {
    fn default() -> Self {
        Self {
            width: 100,
            max_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserRender {
    pub source: String,
    pub title: String,
    pub viewport_width: usize,
    pub dom_node_count: usize,
    pub css_rule_count: usize,
    pub layout_box_count: usize,
    #[serde(default)]
    pub layout_boxes: Vec<BrowserLayoutBox>,
    pub paint_command_count: usize,
    #[serde(default)]
    pub links: Vec<BrowserLink>,
    #[serde(default)]
    pub forms: Vec<BrowserForm>,
    #[serde(default)]
    pub resources: Vec<BrowserResource>,
    #[serde(default)]
    pub fragment_targets: Vec<BrowserFragmentTarget>,
    #[serde(skip)]
    decoded_images: Vec<DecodedImageEntry>,
    #[serde(skip)]
    hit_targets: Vec<DisplayHitTarget>,
    pub display_list: Vec<DisplayCommand>,
    pub text: String,
}

impl BrowserRender {
    fn decoded_image(&self, url: &str) -> Option<&DecodedImage> {
        self.decoded_images
            .iter()
            .find(|image| image.url == url)
            .map(|image| &image.image)
    }

    pub fn fragment_scroll_y(&self, fragment: &str) -> Option<usize> {
        let fragment = fragment.trim_start_matches('#');
        self.fragment_targets
            .iter()
            .find(|target| target.name == fragment)
            .map(|target| target.y)
    }

    pub fn source_fragment_scroll_y(&self) -> Option<usize> {
        source_fragment(&self.source).and_then(|fragment| self.fragment_scroll_y(&fragment))
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRenderTimings {
    pub parse_us: u128,
    pub script_us: u128,
    pub style_us: u128,
    pub collect_us: u128,
    pub layout_us: u128,
    pub total_us: u128,
}

impl BrowserRenderTimings {
    pub(crate) fn add(self, other: Self) -> Self {
        Self {
            parse_us: self.parse_us + other.parse_us,
            script_us: self.script_us + other.script_us,
            style_us: self.style_us + other.style_us,
            collect_us: self.collect_us + other.collect_us,
            layout_us: self.layout_us + other.layout_us,
            total_us: self.total_us + other.total_us,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BrowserProfiledRender {
    pub render: BrowserRender,
    pub timings: BrowserRenderTimings,
    #[serde(skip)]
    click_default_action: Option<BrowserClickDefaultAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BrowserClickDefaultAction {
    Anchor {
        resolved: String,
        default_prevented: bool,
    },
    SubmitForm {
        form_index: usize,
        submitter: BrowserFormSubmitter,
        default_prevented: bool,
    },
    ResetForm {
        form_index: usize,
        default_prevented: bool,
    },
    ToggleFormControl {
        form_index: usize,
        control_index: usize,
        default_prevented: bool,
    },
}

impl BrowserClickDefaultAction {
    fn default_prevented(&self) -> bool {
        match self {
            BrowserClickDefaultAction::Anchor {
                default_prevented, ..
            }
            | BrowserClickDefaultAction::SubmitForm {
                default_prevented, ..
            }
            | BrowserClickDefaultAction::ResetForm {
                default_prevented, ..
            }
            | BrowserClickDefaultAction::ToggleFormControl {
                default_prevented, ..
            } => *default_prevented,
        }
    }

    fn drains_post_click_timers(&self) -> bool {
        self.default_prevented() || matches!(self, BrowserClickDefaultAction::ResetForm { .. })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DisplayHitTarget {
    target_node: Option<usize>,
    text_runs: Vec<TextHitTargetRun>,
    viewport_fixed: bool,
    viewport_sticky_top: Option<usize>,
    source_bounds: Option<DisplaySourceBounds>,
}

impl DisplayHitTarget {
    fn node(target_node: Option<usize>) -> Self {
        Self {
            target_node,
            text_runs: Vec::new(),
            viewport_fixed: false,
            viewport_sticky_top: None,
            source_bounds: None,
        }
    }

    fn text(text_runs: Vec<TextHitTargetRun>) -> Self {
        Self {
            target_node: None,
            text_runs,
            viewport_fixed: false,
            viewport_sticky_top: None,
            source_bounds: None,
        }
    }

    fn target_at_column(&self, column: usize) -> Option<usize> {
        if !self.text_runs.is_empty() {
            return self.text_runs.iter().find_map(|run| {
                (column >= run.start && column < run.start.saturating_add(run.width))
                    .then_some(run.target_node)
                    .flatten()
            });
        }
        self.target_node
    }

    fn target_near_column(&self, column: usize, tolerance: usize) -> Option<usize> {
        self.target_at_column(column).or_else(|| {
            self.text_runs
                .iter()
                .filter_map(|run| {
                    let end = run.start.saturating_add(run.width);
                    let distance = if column < run.start {
                        run.start.saturating_sub(column)
                    } else if column >= end {
                        column.saturating_sub(end.saturating_sub(1))
                    } else {
                        0
                    };
                    (distance <= tolerance)
                        .then_some((distance, run.target_node))
                        .and_then(|(distance, node)| node.map(|node| (distance, node)))
                })
                .min_by_key(|(distance, _)| *distance)
                .map(|(_, node)| node)
        })
    }

    fn with_viewport_fixed(mut self, viewport_fixed: bool) -> Self {
        self.viewport_fixed = viewport_fixed;
        self
    }

    fn with_viewport_sticky_top(mut self, viewport_sticky_top: Option<usize>) -> Self {
        self.viewport_sticky_top = viewport_sticky_top;
        self
    }

    fn with_source_bounds(mut self, source_bounds: Option<DisplaySourceBounds>) -> Self {
        self.source_bounds = source_bounds;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisplaySourceBounds {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TextHitTargetRun {
    start: usize,
    width: usize,
    target_node: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLink {
    pub text: String,
    pub href: String,
    pub resolved: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserHitTestReport {
    pub source: String,
    pub x: usize,
    pub y: usize,
    pub hit: Option<BrowserHitTest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserHitTest {
    pub command_index: usize,
    pub kind: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLayerTreeReport {
    pub source: String,
    pub viewport_width: usize,
    pub paint_command_count: usize,
    pub layer_count: usize,
    pub layers: Vec<BrowserLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLayer {
    pub id: usize,
    pub parent: Option<usize>,
    pub kind: String,
    pub reason: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub paint_order: usize,
    pub command_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLayoutTreeReport {
    pub source: String,
    pub viewport_width: usize,
    pub layout_box_count: usize,
    pub retained_box_count: usize,
    pub boxes: Vec<BrowserLayoutBox>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLayoutBox {
    pub id: usize,
    pub parent: Option<usize>,
    pub node_id: usize,
    pub tag: String,
    pub kind: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub children: Vec<usize>,
    pub command_indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserVisibleLayoutBox {
    pub id: usize,
    pub parent: Option<usize>,
    pub node_id: usize,
    pub tag: String,
    pub kind: String,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub visible_x: usize,
    pub visible_y: usize,
    pub visible_width: usize,
    pub visible_height: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserViewportState {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserViewportRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserDocumentViewportReport {
    pub source: String,
    pub title: String,
    pub document_width: usize,
    pub document_height: usize,
    pub requested: BrowserViewportState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<BrowserViewportState>,
    pub viewport: BrowserViewportState,
    pub max_scroll_x: usize,
    pub max_scroll_y: usize,
    pub scroll_delta_x: isize,
    pub scroll_delta_y: isize,
    pub display_command_count: usize,
    pub visible_command_count: usize,
    pub culled_command_count: usize,
    pub layout_box_count: usize,
    pub visible_layout_box_count: usize,
    pub culled_layout_box_count: usize,
    pub visible_layout_boxes: Vec<BrowserVisibleLayoutBox>,
    pub invalidated_regions: Vec<BrowserViewportRect>,
    pub invalidated_area: usize,
    pub reused_area: usize,
    pub full_repaint: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserViewportFrameDirtyRect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub viewport_x: usize,
    pub viewport_y: usize,
    pub viewport_width: usize,
    pub viewport_height: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserViewportFrameReport {
    pub viewport: BrowserDocumentViewportReport,
    pub frame: BrowserRgbaRasterReport,
    pub dirty_pixel_regions: Vec<BrowserViewportFrameDirtyRect>,
    pub dirty_pixel_area: usize,
    pub frame_width: usize,
    pub frame_height: usize,
    pub cell_width: usize,
    pub cell_height: usize,
    pub padding_x: usize,
    pub padding_y: usize,
    pub bytes_per_pixel: usize,
    pub pixel_hash: String,
    pub non_background_pixels: usize,
    pub artifact_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserViewportFrame {
    pub report: BrowserViewportFrameReport,
    pub raster: BrowserRgbaRaster,
}

const GLYPH_WIDTH: usize = 5;
const GLYPH_HEIGHT: usize = 7;
const INLINE_WIDGET_BACKGROUND_SHADE: u8 = 250;
const INLINE_WIDGET_BORDER_SHADE: u8 = 176;
const LINK_UNDERLINE_SHADE: u8 = 144;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLayerMetrics {
    pub layer_count: usize,
    pub root_command_count: usize,
    pub image_layer_count: usize,
    pub root_layer_width: usize,
    pub root_layer_height: usize,
    pub max_layer_area: usize,
    pub total_layer_area: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundImageSize {
    Auto,
    Cover,
    Contain,
}

impl Default for BackgroundImageSize {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackgroundImagePosition {
    pub x_percent: i32,
    pub y_percent: i32,
}

impl Default for BackgroundImagePosition {
    fn default() -> Self {
        Self {
            x_percent: 0,
            y_percent: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundImageRepeat {
    Repeat,
    NoRepeat,
}

impl Default for BackgroundImageRepeat {
    fn default() -> Self {
        Self::Repeat
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum DisplayCommand {
    Text {
        x: usize,
        y: usize,
        text: String,
    },
    StyledText {
        x: usize,
        y: usize,
        text: String,
        shade: u8,
    },
    Rect {
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        shade: u8,
    },
    ColorRect {
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        red: u8,
        green: u8,
        blue: u8,
        shade: u8,
    },
    Image {
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        shade: u8,
        alt: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_width: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_height: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_hash: Option<String>,
    },
    BackgroundImage {
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        shade: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_width: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_height: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decoded_hash: Option<String>,
        #[serde(default)]
        size: BackgroundImageSize,
        #[serde(default)]
        position: BackgroundImagePosition,
        #[serde(default)]
        repeat: BackgroundImageRepeat,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRasterOptions {
    pub cell_width: usize,
    pub cell_height: usize,
    pub padding_x: usize,
    pub padding_y: usize,
    pub max_pixels: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_x: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_y: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_height: Option<usize>,
}

impl Default for BrowserRasterOptions {
    fn default() -> Self {
        Self {
            cell_width: 12,
            cell_height: 18,
            padding_x: 4,
            padding_y: 4,
            max_pixels: 32 * 1024 * 1024,
            viewport_x: None,
            viewport_y: None,
            viewport_width: None,
            viewport_height: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRaster {
    pub width: usize,
    pub height: usize,
    pub background: u8,
    pub foreground: u8,
    pub pixels: Vec<u8>,
}

impl BrowserRaster {
    pub fn pixel_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"brutal-browser-raster-v1");
        hasher.update(&(self.width as u64).to_le_bytes());
        hasher.update(&(self.height as u64).to_le_bytes());
        hasher.update(&[self.background, self.foreground]);
        hasher.update(&self.pixels);
        hasher.finalize().to_hex().to_string()
    }

    pub fn non_background_pixels(&self) -> usize {
        self.pixels
            .iter()
            .filter(|&&pixel| pixel != self.background)
            .count()
    }

    pub fn encode_pgm(&self) -> Vec<u8> {
        let mut encoded = format!("P5\n{} {}\n255\n", self.width, self.height).into_bytes();
        encoded.extend_from_slice(&self.pixels);
        encoded
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRgbaRaster {
    pub width: usize,
    pub height: usize,
    pub background: [u8; 4],
    pub pixels: Vec<u8>,
}

impl BrowserRgbaRaster {
    pub fn from_grayscale(raster: &BrowserRaster) -> Self {
        let mut pixels = Vec::with_capacity(raster.pixels.len().saturating_mul(4));
        for &pixel in &raster.pixels {
            pixels.extend_from_slice(&[pixel, pixel, pixel, 255]);
        }
        Self {
            width: raster.width,
            height: raster.height,
            background: [raster.background, raster.background, raster.background, 255],
            pixels,
        }
    }

    pub fn pixel_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"brutal-browser-rgba-raster-v1");
        hasher.update(&(self.width as u64).to_le_bytes());
        hasher.update(&(self.height as u64).to_le_bytes());
        hasher.update(&self.background);
        hasher.update(&self.pixels);
        hasher.finalize().to_hex().to_string()
    }

    pub fn non_background_pixels(&self) -> usize {
        self.pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel != self.background.as_slice())
            .count()
    }

    pub fn encode_png(&self) -> Result<Vec<u8>> {
        use std::io::Write as _;

        ensure!(self.width <= u32::MAX as usize, "PNG width exceeds u32");
        ensure!(self.height <= u32::MAX as usize, "PNG height exceeds u32");
        ensure!(
            self.pixels.len() == self.width.saturating_mul(self.height).saturating_mul(4),
            "RGBA buffer length does not match raster dimensions"
        );

        let row_len = self.width.saturating_mul(4);
        let mut raw = Vec::with_capacity(self.height.saturating_mul(row_len.saturating_add(1)));
        for row in self.pixels.chunks_exact(row_len) {
            raw.push(0);
            raw.extend_from_slice(row);
        }

        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw)?;
        let compressed = encoder.finish()?;

        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&(self.width as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.height as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        append_png_chunk(&mut png, b"IHDR", &ihdr);
        append_png_chunk(&mut png, b"IDAT", &compressed);
        append_png_chunk(&mut png, b"IEND", &[]);
        Ok(png)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRasterReport {
    pub source: String,
    pub viewport_width: usize,
    pub width: usize,
    pub height: usize,
    pub cell_width: usize,
    pub cell_height: usize,
    pub display_command_count: usize,
    #[serde(default)]
    pub visible_command_count: usize,
    #[serde(default)]
    pub culled_command_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_x: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_y: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_height: Option<usize>,
    pub non_background_pixels: usize,
    pub pixel_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserRgbaRasterReport {
    pub source: String,
    pub viewport_width: usize,
    pub width: usize,
    pub height: usize,
    pub cell_width: usize,
    pub cell_height: usize,
    pub bytes_per_pixel: usize,
    pub display_command_count: usize,
    #[serde(default)]
    pub visible_command_count: usize,
    #[serde(default)]
    pub culled_command_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_x: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_y: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_height: Option<usize>,
    pub non_background_pixels: usize,
    pub pixel_hash: String,
    pub artifact_format: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTextViewportOptions {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Default for BrowserTextViewportOptions {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 100,
            height: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTextViewportReport {
    pub source: String,
    pub title: String,
    pub document_width: usize,
    pub document_height: usize,
    pub x: usize,
    pub y: usize,
    #[serde(default)]
    pub max_scroll_x: usize,
    #[serde(default)]
    pub max_scroll_y: usize,
    pub width: usize,
    pub height: usize,
    pub display_command_count: usize,
    pub visible_command_count: usize,
    pub culled_command_count: usize,
    #[serde(default)]
    pub layout_box_count: usize,
    #[serde(default)]
    pub visible_layout_box_count: usize,
    #[serde(default)]
    pub culled_layout_box_count: usize,
    #[serde(default)]
    pub visible_layout_boxes: Vec<BrowserVisibleLayoutBox>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserVisualReport {
    pub fixture_count: usize,
    pub checked: usize,
    pub passed: usize,
    pub failed: usize,
    pub missing_baseline: usize,
    pub artifact_dir: Option<String>,
    pub baseline_dir: Option<String>,
    pub diff_checked: usize,
    pub diff_passed: usize,
    pub diff_failed: usize,
    pub max_diff_pixels: Option<usize>,
    pub max_diff_ratio: Option<f64>,
    pub comparisons: Vec<BrowserVisualComparison>,
    pub failures: Vec<BrowserFixtureFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserVisualComparison {
    pub name: String,
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub display_command_count: usize,
    #[serde(default)]
    pub visible_command_count: usize,
    #[serde(default)]
    pub culled_command_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_x: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_y: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raster_viewport_height: Option<usize>,
    pub non_background_pixels: usize,
    pub expected_raster_hash: Option<String>,
    pub actual_raster_hash: String,
    pub matched: Option<bool>,
    pub artifact: Option<String>,
    pub baseline_artifact: Option<String>,
    pub diff_artifact: Option<String>,
    pub diff_pixels: Option<usize>,
    pub diff_ratio: Option<f64>,
    pub diff_passed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserFixtureManifest {
    pub fixtures: Vec<BrowserFixture>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserFixture {
    pub name: Option<String>,
    pub path: PathBuf,
    #[serde(default = "default_fixture_width")]
    pub width: usize,
    #[serde(default)]
    pub external_scripts: bool,
    pub click_selector: Option<String>,
    pub expected_title: Option<String>,
    pub expected_text: Option<String>,
    pub expected_display_list: Option<Vec<DisplayCommand>>,
    #[serde(default)]
    pub expected_hit_tests: Vec<BrowserFixtureHitTest>,
    #[serde(default)]
    pub expected_layers: Option<Vec<BrowserLayer>>,
    #[serde(default)]
    pub raster_viewport_x: Option<usize>,
    #[serde(default)]
    pub raster_viewport_y: Option<usize>,
    #[serde(default)]
    pub raster_viewport_width: Option<usize>,
    #[serde(default)]
    pub raster_viewport_height: Option<usize>,
    #[serde(default)]
    pub expected_visible_command_count: Option<usize>,
    #[serde(default)]
    pub expected_culled_command_count: Option<usize>,
    pub expected_raster_hash: Option<String>,
    #[serde(default)]
    pub expected_screenshot_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserFixtureHitTest {
    pub x: usize,
    pub y: usize,
    pub expected: Option<BrowserHitTestExpectation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHitTestExpectation {
    pub kind: String,
    #[serde(default)]
    pub command_index: Option<usize>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub alt: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub shade: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserFixtureReport {
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub failures: Vec<BrowserFixtureFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserFixtureFailure {
    pub name: String,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserChromiumParityReport {
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub chrome: Option<String>,
    pub comparisons: Vec<BrowserChromiumParityComparison>,
    pub failures: Vec<BrowserFixtureFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserChromiumParityComparison {
    pub name: String,
    pub path: String,
    pub title_match: bool,
    pub text_match: bool,
    pub brutal_title: String,
    pub chromium_title: String,
    pub brutal_text: String,
    pub chromium_text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChromiumStaticRender {
    title: String,
    text: String,
}

#[derive(Debug, Clone)]
pub struct BrowserSession {
    options: BrowserRenderOptions,
    entries: Vec<BrowserSessionEntry>,
    current_index: Option<usize>,
    cookie_jar: BrowserCookieJar,
    resource_cache: BrowserResourceCache,
    local_storage: BrowserLocalStorage,
    session_storage: BrowserLocalStorage,
}

#[derive(Debug, Clone)]
struct BrowserSessionEntry {
    target: String,
    html: Vec<u8>,
    page_state: BrowserPageState,
    render: BrowserRender,
    form_state: HashMap<BrowserFormFieldKey, String>,
    checked_state: HashMap<BrowserFormControlKey, bool>,
    focused_control: Option<BrowserFocusedFormControl>,
}

#[derive(Debug, Clone)]
struct BrowserPageState {
    dom: Dom,
    css_text: String,
    runtime: TinyJsRuntime,
    cached_images: Vec<DecodedImageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHistorySnapshot {
    pub current_index: Option<usize>,
    pub entries: Vec<BrowserHistoryEntry>,
    pub retained_entry_limit: usize,
    pub retained_entry_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHistoryEntry {
    pub target: String,
    pub source: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserFocusedControl {
    pub form_index: usize,
    pub control_index: usize,
    pub name: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowserLocalStorage {
    origins: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLocalStorageEntry {
    pub origin: String,
    pub key: String,
    pub value: String,
}

fn browser_storage_entries(storage: &BrowserLocalStorage) -> Vec<BrowserLocalStorageEntry> {
    let mut entries = storage
        .origins
        .iter()
        .flat_map(|(origin, values)| {
            values.iter().map(|(key, value)| BrowserLocalStorageEntry {
                origin: origin.clone(),
                key: key.clone(),
                value: value.clone(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.origin
            .cmp(&right.origin)
            .then_with(|| left.key.cmp(&right.key))
    });
    entries
}

fn default_fixture_width() -> usize {
    BrowserRenderOptions::default().width
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Dom {
    nodes: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    kind: NodeKind,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeKind {
    Document,
    DocumentFragment,
    Element(Box<ElementData>),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElementData {
    tag: String,
    attrs: HashMap<String, String>,
    id: Option<String>,
    classes: Vec<String>,
    style: Option<String>,
    href: Option<String>,
    src: Option<String>,
    srcset: Option<String>,
    rel: Option<String>,
    media: Option<String>,
    alt: Option<String>,
    data: Option<String>,
    name: Option<String>,
    value: Option<String>,
    input_type: Option<String>,
    type_hint: Option<String>,
    poster: Option<String>,
    action: Option<String>,
    method: Option<String>,
    onclick: Option<String>,
    hidden: bool,
    disabled: bool,
    checked: bool,
    selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Display {
    None,
    Inline,
    InlineBlock,
    InlineFlex,
    InlineGrid,
    Block,
    Flex,
    Grid,
    FlowRoot,
    ListItem,
    Table,
    TableRow,
    TableCell,
    Contents,
}

impl Display {
    fn is_block_flow(self) -> bool {
        matches!(
            self,
            Self::Block
                | Self::Flex
                | Self::Grid
                | Self::FlowRoot
                | Self::ListItem
                | Self::Table
                | Self::TableRow
        )
    }

    fn lays_out_children_in_row(self) -> bool {
        matches!(
            self,
            Self::InlineBlock | Self::Flex | Self::Grid | Self::InlineFlex | Self::InlineGrid
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FloatSide {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearSide {
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JustifyContent {
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

impl Default for JustifyContent {
    fn default() -> Self {
        Self::Start
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlignItems {
    Start,
    Center,
    End,
    Baseline,
}

impl Default for AlignItems {
    fn default() -> Self {
        Self::Start
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextAlign {
    Start,
    Center,
    End,
}

impl TextAlign {
    fn offset(self, available_width: usize, line_width: usize) -> usize {
        let remaining = available_width.saturating_sub(line_width);
        match self {
            TextAlign::Start => 0,
            TextAlign::Center => remaining / 2,
            TextAlign::End => remaining,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhiteSpace {
    Normal,
    Nowrap,
    Pre,
    PreLine,
    PreWrap,
    BreakSpaces,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverflowWrap {
    Normal,
    BreakWord,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordBreak {
    Normal,
    BreakAll,
    BreakWord,
    KeepAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaintOpacity {
    Opaque,
    Transparent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overflow {
    Visible,
    Clip,
}

impl Overflow {
    fn clips(self) -> bool {
        matches!(self, Self::Clip)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Static,
    Relative,
    Absolute,
    Fixed,
    Sticky,
}

impl Position {
    fn is_out_of_flow(self) -> bool {
        matches!(self, Self::Absolute | Self::Fixed)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CssPositionOffset {
    cells: isize,
    percent_basis_points: i32,
}

impl CssPositionOffset {
    fn resolve(self, basis: usize) -> isize {
        let percent_cells =
            (basis as i64).saturating_mul(self.percent_basis_points as i64) / 10_000;
        self.cells.saturating_add(percent_cells as isize)
    }

    fn is_zero(self) -> bool {
        self.cells == 0 && self.percent_basis_points == 0
    }

    fn add(self, other: Self) -> Self {
        Self {
            cells: self.cells.saturating_add(other.cells),
            percent_basis_points: self
                .percent_basis_points
                .saturating_add(other.percent_basis_points),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CssTranslate {
    x: CssPositionOffset,
    y: CssPositionOffset,
}

impl CssTranslate {
    fn add_x(&mut self, offset: CssPositionOffset) {
        self.x = self.x.add(offset);
    }

    fn add_y(&mut self, offset: CssPositionOffset) {
        self.y = self.y.add(offset);
    }
}

fn saturating_add_signed(base: usize, offset: isize) -> usize {
    if offset >= 0 {
        base.saturating_add(offset as usize)
    } else {
        base.saturating_sub(offset.unsigned_abs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoxSizing {
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssDimension {
    Cells(usize),
    Percent(i32),
    Min([CssDimensionTerm; 4], usize),
    Max([CssDimensionTerm; 4], usize),
    Clamp(CssDimensionTerm, CssDimensionTerm, CssDimensionTerm),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssDimensionTerm {
    Cells(usize),
    Percent(i32),
}

impl CssDimension {
    fn zero() -> Self {
        Self::Cells(0)
    }

    fn resolve(self, basis: usize) -> usize {
        match self {
            Self::Cells(cells) => cells,
            Self::Percent(basis_points) => {
                let resolved = (basis as i64).saturating_mul(basis_points as i64) / 10_000;
                resolved.max(0) as usize
            }
            Self::Min(terms, len) => terms
                .iter()
                .take(len)
                .map(|term| term.resolve(basis))
                .min()
                .unwrap_or(0),
            Self::Max(terms, len) => terms
                .iter()
                .take(len)
                .map(|term| term.resolve(basis))
                .max()
                .unwrap_or(0),
            Self::Clamp(minimum, preferred, maximum) => {
                let minimum = minimum.resolve(basis);
                let preferred = preferred.resolve(basis);
                let maximum = maximum.resolve(basis);
                preferred.clamp(minimum.min(maximum), maximum.max(minimum))
            }
        }
    }
}

impl CssDimensionTerm {
    fn zero() -> Self {
        Self::Cells(0)
    }

    fn resolve(self, basis: usize) -> usize {
        match self {
            Self::Cells(cells) => cells,
            Self::Percent(basis_points) => {
                let resolved = (basis as i64).saturating_mul(basis_points as i64) / 10_000;
                resolved.max(0) as usize
            }
        }
    }

    fn into_dimension(self) -> CssDimension {
        match self {
            Self::Cells(cells) => CssDimension::Cells(cells),
            Self::Percent(percent) => CssDimension::Percent(percent),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssAspectRatio {
    width: usize,
    height: usize,
}

impl CssAspectRatio {
    fn height_for_width(self, width: usize) -> Option<usize> {
        if self.width == 0 {
            return None;
        }
        let width_px = width as f32 * css_axis_cell_px(CssAxis::Horizontal);
        let height_px = width_px * self.height as f32 / self.width as f32;
        Some((height_px / css_axis_cell_px(CssAxis::Vertical)).ceil() as usize)
            .filter(|height| *height > 0)
    }

    fn width_for_height(self, height: usize) -> Option<usize> {
        if self.height == 0 {
            return None;
        }
        let height_px = height as f32 * css_axis_cell_px(CssAxis::Vertical);
        let width_px = height_px * self.width as f32 / self.height as f32;
        Some((width_px / css_axis_cell_px(CssAxis::Horizontal)).ceil() as usize)
            .filter(|width| *width > 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CssListStyleType {
    NoMarker,
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComputedStyle {
    display: Display,
    float: Option<FloatSide>,
    clear: Option<ClearSide>,
    background_shade: Option<u8>,
    background_image_url: Option<String>,
    background_image_size: BackgroundImageSize,
    background_image_position: BackgroundImagePosition,
    background_image_repeat: BackgroundImageRepeat,
    text_shade: Option<u8>,
    text_align: Option<TextAlign>,
    visibility: Option<Visibility>,
    opacity: PaintOpacity,
    animation_reveals_opacity: bool,
    overflow_x: Overflow,
    overflow_y: Overflow,
    flex_direction: FlexDirection,
    flex_wrap: bool,
    flex_basis: Option<CssDimension>,
    justify_content: JustifyContent,
    align_items: AlignItems,
    grid_columns: Option<usize>,
    grid_auto_min_column_width: Option<usize>,
    position: Position,
    position_top: Option<CssPositionOffset>,
    position_bottom: Option<CssPositionOffset>,
    position_left: Option<CssPositionOffset>,
    position_right: Option<CssPositionOffset>,
    transform_translate: CssTranslate,
    z_index: i32,
    z_index_specified: bool,
    white_space: Option<WhiteSpace>,
    text_transform: Option<TextTransform>,
    letter_spacing: Option<usize>,
    word_spacing: Option<usize>,
    overflow_wrap: Option<OverflowWrap>,
    word_break: Option<WordBreak>,
    text_indent: Option<usize>,
    line_height: Option<usize>,
    font_scale: Option<usize>,
    row_gap: Option<usize>,
    column_gap: Option<usize>,
    box_sizing: BoxSizing,
    list_style_type: Option<CssListStyleType>,
    border: Option<BorderPaint>,
    padding: BoxSpacing,
    margin: BoxSpacing,
    width: Option<CssDimension>,
    max_width: Option<CssDimension>,
    min_width: CssDimension,
    height: Option<CssDimension>,
    max_height: Option<CssDimension>,
    aspect_ratio: Option<CssAspectRatio>,
    margin_left_auto: bool,
    margin_right_auto: bool,
    min_height: CssDimension,
}

impl ComputedStyle {
    fn suppresses_paint(&self) -> bool {
        self.opacity == PaintOpacity::Transparent && !self.animation_reveals_opacity
    }

    fn positioned_outer_width(&self, available_width: usize) -> usize {
        let horizontal_box_extra = self
            .padding
            .left
            .saturating_add(self.padding.right)
            .saturating_add(
                self.border
                    .map(|border| border.width.saturating_mul(2))
                    .unwrap_or(0),
            );
        let mut width = match (
            self.width.map(|width| width.resolve(available_width)),
            self.box_sizing,
        ) {
            (Some(width), BoxSizing::ContentBox) => width.saturating_add(horizontal_box_extra),
            (Some(width), BoxSizing::BorderBox) => width,
            (None, _) => available_width,
        };
        if let Some(max_width) = self.max_width.map(|width| width.resolve(available_width)) {
            let max_outer_width = match self.box_sizing {
                BoxSizing::ContentBox => max_width.saturating_add(horizontal_box_extra),
                BoxSizing::BorderBox => max_width,
            };
            width = width.min(max_outer_width);
        }
        let min_width = self.min_width.resolve(available_width);
        if min_width > 0 {
            let min_outer_width = match self.box_sizing {
                BoxSizing::ContentBox => min_width.saturating_add(horizontal_box_extra),
                BoxSizing::BorderBox => min_width,
            };
            width = width.max(min_outer_width);
        }
        width.clamp(1, available_width.max(1))
    }

    fn positioned_outer_height(&self) -> usize {
        self.height
            .map(|height| height.resolve(default_vertical_dimension_basis()))
            .unwrap_or(0)
            .max(self.min_height.resolve(default_vertical_dimension_basis()))
            .max(1)
    }

    fn resolved_width(&self, basis: usize) -> Option<usize> {
        self.width.map(|width| width.resolve(basis))
    }

    fn resolved_max_width(&self, basis: usize) -> Option<usize> {
        self.max_width.map(|width| width.resolve(basis))
    }

    fn resolved_min_width(&self, basis: usize) -> usize {
        self.min_width.resolve(basis)
    }

    fn resolved_height(&self) -> Option<usize> {
        self.height
            .map(|height| height.resolve(default_vertical_dimension_basis()))
    }

    fn resolved_max_height(&self) -> Option<usize> {
        self.max_height
            .map(|height| height.resolve(default_vertical_dimension_basis()))
    }

    fn resolved_min_height(&self) -> usize {
        self.min_height.resolve(default_vertical_dimension_basis())
    }

    fn horizontal_projection_offset(&self, containing_width: usize) -> isize {
        let own_width = self.positioned_outer_width(containing_width);
        let mut offset = if let Some(left) = self.position_left {
            left.resolve(containing_width)
        } else if let Some(right) = self.position_right {
            containing_width as isize - own_width as isize - right.resolve(containing_width)
        } else {
            0
        };
        offset = offset.saturating_add(self.transform_translate.x.resolve(own_width));
        offset
    }

    fn vertical_projection_offset(&self, containing_height: Option<usize>) -> Option<isize> {
        let has_top = self.position_top.is_some();
        let has_bottom = self.position_bottom.is_some();
        let own_height = self.positioned_outer_height();
        let mut offset = if let Some(top) = self.position_top {
            top.resolve(containing_height.unwrap_or(0))
        } else if let (Some(bottom), Some(containing_height)) =
            (self.position_bottom, containing_height)
        {
            containing_height as isize - own_height as isize - bottom.resolve(containing_height)
        } else {
            0
        };
        offset = offset.saturating_add(self.transform_translate.y.resolve(own_height));
        (has_top
            || (has_bottom && containing_height.is_some())
            || !self.transform_translate.y.is_zero())
        .then_some(offset)
    }

    fn child_layout(&self) -> ChildLayout {
        let grid_columns = matches!(self.display, Display::Grid | Display::InlineGrid)
            .then_some(self.grid_columns)
            .flatten();
        self.child_layout_with_grid_columns(grid_columns)
    }

    fn child_layout_for_width(&self, available_width: usize) -> ChildLayout {
        let grid_columns = if matches!(self.display, Display::Grid | Display::InlineGrid) {
            self.grid_columns.or_else(|| {
                self.grid_auto_min_column_width
                    .map(|min_width| auto_grid_column_count(available_width, min_width))
            })
        } else {
            None
        };
        self.child_layout_with_grid_columns(grid_columns)
    }

    fn child_layout_with_grid_columns(&self, grid_columns: Option<usize>) -> ChildLayout {
        let flex_container = matches!(self.display, Display::Flex | Display::InlineFlex);
        let flex_column = flex_container
            && matches!(
                self.flex_direction,
                FlexDirection::Column | FlexDirection::ColumnReverse
            );
        let flex_row = matches!(self.display, Display::Flex | Display::InlineFlex) && !flex_column;
        ChildLayout {
            row_items: self.display.lays_out_children_in_row() && !flex_column,
            flex_items: flex_container,
            reverse_items: matches!(
                self.flex_direction,
                FlexDirection::RowReverse | FlexDirection::ColumnReverse
            ),
            row_gap: if flex_column {
                Some(self.row_gap.unwrap_or(0))
            } else {
                (grid_columns.is_some() || (flex_row && self.flex_wrap))
                    .then(|| self.row_gap.unwrap_or(0))
            },
            column_gap: self.column_gap,
            justify_content: self.justify_content,
            align_items: self.align_items,
            wrap_after: grid_columns,
            wrap_items: flex_row && self.flex_wrap,
        }
    }

    fn clips_overflow(&self) -> bool {
        self.overflow_x.clips() || self.overflow_y.clips()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ChildLayout {
    row_items: bool,
    flex_items: bool,
    reverse_items: bool,
    row_gap: Option<usize>,
    column_gap: Option<usize>,
    justify_content: JustifyContent,
    align_items: AlignItems,
    wrap_after: Option<usize>,
    wrap_items: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssRule {
    selector: CssSelector,
    declarations: CssDeclarations,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CssDeclarations {
    display: Option<Display>,
    float: Option<Option<FloatSide>>,
    clear: Option<Option<ClearSide>>,
    flex_direction: Option<FlexDirection>,
    flex_wrap: Option<bool>,
    flex_basis: Option<CssDimension>,
    justify_content: Option<JustifyContent>,
    align_items: Option<AlignItems>,
    grid_columns: Option<usize>,
    grid_auto_min_column_width: Option<usize>,
    background_shade: Option<u8>,
    background_image_url: Option<String>,
    background_image_size: Option<BackgroundImageSize>,
    background_image_position: Option<BackgroundImagePosition>,
    background_image_repeat: Option<BackgroundImageRepeat>,
    text_shade: Option<u8>,
    text_align: Option<TextAlign>,
    visibility: Option<Visibility>,
    opacity: Option<PaintOpacity>,
    animation_reveals_opacity: Option<bool>,
    overflow_x: Option<Overflow>,
    overflow_y: Option<Overflow>,
    position: Option<Position>,
    position_top: Option<CssPositionOffset>,
    position_bottom: Option<CssPositionOffset>,
    position_left: Option<CssPositionOffset>,
    position_right: Option<CssPositionOffset>,
    transform_translate: Option<CssTranslate>,
    z_index: Option<i32>,
    white_space: Option<WhiteSpace>,
    text_transform: Option<TextTransform>,
    letter_spacing: Option<usize>,
    word_spacing: Option<usize>,
    overflow_wrap: Option<OverflowWrap>,
    word_break: Option<WordBreak>,
    text_indent: Option<usize>,
    line_height: Option<usize>,
    font_scale: Option<usize>,
    row_gap: Option<usize>,
    column_gap: Option<usize>,
    box_sizing: Option<BoxSizing>,
    list_style_type: Option<CssListStyleType>,
    border: Option<BorderPaint>,
    padding: Option<BoxSpacing>,
    margin: Option<BoxSpacing>,
    width: Option<CssDimension>,
    max_width: Option<CssDimension>,
    min_width: Option<CssDimension>,
    height: Option<CssDimension>,
    max_height: Option<CssDimension>,
    aspect_ratio: Option<CssAspectRatio>,
    margin_left_auto: Option<bool>,
    margin_right_auto: Option<bool>,
    min_height: Option<CssDimension>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BorderPaint {
    width: usize,
    shade: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BoxSpacing {
    top: usize,
    right: usize,
    bottom: usize,
    left: usize,
}

impl BoxSpacing {
    fn is_empty(self) -> bool {
        self.top == 0 && self.right == 0 && self.bottom == 0 && self.left == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssCascade {
    rules: Vec<CssRule>,
    id_rules: HashMap<String, Vec<usize>>,
    class_rules: HashMap<String, Vec<usize>>,
    tag_rules: HashMap<String, Vec<usize>>,
    attr_rules: HashMap<String, Vec<usize>>,
    universal_rules: Vec<usize>,
    custom_properties: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssSelector {
    steps: Vec<SelectorStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectorStep {
    compound: CompoundSelector,
    combinator: Option<SelectorCombinator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompoundSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attributes: Vec<AttributeSelector>,
    not_selectors: Vec<CompoundSelector>,
    first_child: bool,
    universal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeSelector {
    name: String,
    value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorCombinator {
    Descendant,
    Child,
}

pub async fn load_and_render(target: &str, options: BrowserRenderOptions) -> Result<BrowserRender> {
    let (source, bytes) = load_target(target, options.max_bytes).await?;
    Ok(render_html(&source, &bytes, options))
}

pub const BROWSER_ABOUT_BLANK_TARGET: &str = "about:blank";
const BROWSER_ABOUT_BLANK_HTML: &[u8] =
    b"<!doctype html><html><head><title></title></head><body></body></html>";
const BROWSER_SESSION_HISTORY_MAX_ENTRIES: usize = 64;

async fn load_session_document(
    target: &str,
    options: BrowserRenderOptions,
    cookie_jar: &mut BrowserCookieJar,
    local_storage: &mut BrowserLocalStorage,
    session_storage: &mut BrowserLocalStorage,
) -> Result<(String, Vec<u8>, BrowserPageState, BrowserRender)> {
    let (source, bytes) = if target == BROWSER_ABOUT_BLANK_TARGET {
        (
            BROWSER_ABOUT_BLANK_TARGET.to_owned(),
            BROWSER_ABOUT_BLANK_HTML.to_vec(),
        )
    } else {
        load_target_with_cookie_jar(target, options.max_bytes, Some(cookie_jar)).await?
    };
    let (page_state, profiled) = render_html_prepared_with_state(
        &source,
        &bytes,
        options,
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: Some(local_storage),
            session_storage: Some(session_storage),
            cached_images: &[],
        },
    )
    .expect("session render without interaction should not fail");
    Ok((source, bytes, page_state, profiled.render))
}

async fn load_session_post_form_document(
    target: &str,
    body: String,
    options: BrowserRenderOptions,
    cookie_jar: &mut BrowserCookieJar,
    local_storage: &mut BrowserLocalStorage,
    session_storage: &mut BrowserLocalStorage,
) -> Result<(String, Vec<u8>, BrowserPageState, BrowserRender)> {
    let (source, bytes) =
        load_post_form_target_with_cookie_jar(target, body, options.max_bytes, cookie_jar).await?;
    let (page_state, profiled) = render_html_prepared_with_state(
        &source,
        &bytes,
        options,
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: Some(local_storage),
            session_storage: Some(session_storage),
            cached_images: &[],
        },
    )
    .expect("session POST render without interaction should not fail");
    Ok((source, bytes, page_state, profiled.render))
}

impl BrowserSession {
    pub fn new(options: BrowserRenderOptions) -> Self {
        Self::new_with_cookie_jar(options, BrowserCookieJar::default())
    }

    pub fn new_with_cookie_jar(
        options: BrowserRenderOptions,
        cookie_jar: BrowserCookieJar,
    ) -> Self {
        Self::new_with_state(options, cookie_jar, BrowserLocalStorage::default())
    }

    pub fn new_with_state(
        options: BrowserRenderOptions,
        cookie_jar: BrowserCookieJar,
        local_storage: BrowserLocalStorage,
    ) -> Self {
        Self {
            options,
            entries: Vec::new(),
            current_index: None,
            cookie_jar,
            resource_cache: BrowserResourceCache::default(),
            local_storage,
            session_storage: BrowserLocalStorage::default(),
        }
    }

    pub async fn navigate(&mut self, target: &str) -> Result<&BrowserRender> {
        let (source, html, page_state, render) = load_session_document(
            target,
            self.options,
            &mut self.cookie_jar,
            &mut self.local_storage,
            &mut self.session_storage,
        )
        .await?;
        Ok(self.push_entry(source, html, page_state, render))
    }

    pub async fn reload(&mut self) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot reload: session has no current page");
        };
        let target = self.entries[current_index].target.clone();
        let (_, html, page_state, render) = load_session_document(
            &target,
            self.options,
            &mut self.cookie_jar,
            &mut self.local_storage,
            &mut self.session_storage,
        )
        .await?;
        let entry = &mut self.entries[current_index];
        entry.html = html;
        entry.page_state = page_state;
        entry.render = render;
        entry.form_state.clear();
        entry.checked_state.clear();
        entry.focused_control = None;
        Ok(&entry.render)
    }

    fn push_entry(
        &mut self,
        target: String,
        html: Vec<u8>,
        page_state: BrowserPageState,
        render: BrowserRender,
    ) -> &BrowserRender {
        if let Some(current_index) = self.current_index {
            self.entries.truncate(current_index + 1);
        }
        self.entries.push(BrowserSessionEntry {
            target,
            html,
            page_state,
            render,
            form_state: HashMap::new(),
            checked_state: HashMap::new(),
            focused_control: None,
        });
        self.compact_history_entries();
        self.current_index = Some(self.entries.len() - 1);
        &self.entries[self.entries.len() - 1].render
    }

    fn compact_history_entries(&mut self) {
        if self.entries.len() <= BROWSER_SESSION_HISTORY_MAX_ENTRIES {
            return;
        }
        let remove_count = self.entries.len() - BROWSER_SESSION_HISTORY_MAX_ENTRIES;
        self.entries.drain(..remove_count);
        self.current_index = self
            .current_index
            .map(|index| index.saturating_sub(remove_count));
    }

    pub async fn submit_form(
        &mut self,
        form_index: usize,
        overrides: &[(String, String)],
    ) -> Result<&BrowserRender> {
        self.submit_form_with_submitter(form_index, overrides, &BrowserFormSubmitter::default())
            .await
    }

    async fn submit_form_with_submitter(
        &mut self,
        form_index: usize,
        overrides: &[(String, String)],
        submitter: &BrowserFormSubmitter,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot submit form: session has no current page");
        };
        let submit_event = self.dispatch_current_form_event(current_index, form_index, "submit")?;
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        if submit_event.default_prevented {
            return Ok(&self.entries[current_index].render);
        }
        let submission = self.build_current_form_submission(form_index, overrides, submitter)?;
        match submission {
            BrowserFormSubmission::Get { target } => self.navigate(&target).await,
            BrowserFormSubmission::PostUrlEncoded { target, body } => {
                self.navigate_post_form(&target, body).await
            }
        }
    }

    pub async fn submit_get_form(
        &mut self,
        form_index: usize,
        overrides: &[(String, String)],
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot submit form: session has no current page");
        };
        let submit_event = self.dispatch_current_form_event(current_index, form_index, "submit")?;
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        if submit_event.default_prevented {
            return Ok(&self.entries[current_index].render);
        }
        let submission = self.build_current_form_submission(
            form_index,
            overrides,
            &BrowserFormSubmitter::default(),
        )?;
        let BrowserFormSubmission::Get { target } = submission else {
            bail!(
                "form {} uses POST; use submit_form to submit non-GET forms",
                form_index
            );
        };
        self.navigate(&target).await
    }

    fn build_current_form_submission(
        &self,
        form_index: usize,
        overrides: &[(String, String)],
        submitter: &BrowserFormSubmitter,
    ) -> Result<BrowserFormSubmission> {
        let Some(current_index) = self.current_index else {
            bail!("cannot submit form: session has no current page");
        };
        let entry = &self.entries[current_index];
        let current = &entry.render;
        let Some(form) = current.forms.get(form_index) else {
            bail!(
                "form index {} not found; current page has {} form(s)",
                form_index,
                current.forms.len()
            );
        };
        let effective_overrides =
            effective_form_overrides(form, &entry.form_state, form_index, overrides);
        if !submitter.no_validate {
            validate_supported_form_controls(form, &effective_overrides)?;
        }
        build_form_submission_with_submitter(form, &effective_overrides, submitter)
    }

    async fn navigate_post_form(&mut self, target: &str, body: String) -> Result<&BrowserRender> {
        let (source, html, page_state, render) = load_session_post_form_document(
            target,
            body,
            self.options,
            &mut self.cookie_jar,
            &mut self.local_storage,
            &mut self.session_storage,
        )
        .await?;
        Ok(self.push_entry(source, html, page_state, render))
    }

    pub fn set_form_field(
        &mut self,
        form_index: usize,
        name: &str,
        value: &str,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot fill form field: session has no current page");
        };
        ensure!(
            !name.is_empty(),
            "cannot fill a form field with an empty name"
        );
        {
            let current = &self.entries[current_index].render;
            let Some(form) = current.forms.get(form_index) else {
                bail!(
                    "form index {} not found; current page has {} form(s)",
                    form_index,
                    current.forms.len()
                );
            };
            let matching = form
                .controls
                .iter()
                .filter(|control| control.name == name)
                .collect::<Vec<_>>();
            ensure!(
                !matching.is_empty(),
                "form {} has no field named {:?}",
                form_index,
                name
            );
            ensure!(
                matching
                    .iter()
                    .all(|control| form_control_accepts_fill_state(control)),
                "field {:?} in form {} is not a fillable form control",
                name,
                form_index
            );
            for control in matching {
                if form_control_accepts_select_state(control) {
                    ensure!(
                        form_control_has_enabled_option(control, value),
                        "select field {:?} in form {} has no enabled option value {:?}",
                        name,
                        form_index,
                        value
                    );
                }
            }
        }
        let targets = {
            let form = &self.entries[current_index].render.forms[form_index];
            form.controls
                .iter()
                .filter(|control| control.name == name && form_control_accepts_fill_state(control))
                .map(|control| (control.node_id, control.kind.clone()))
                .collect::<Vec<_>>()
        };
        self.entries[current_index].form_state.insert(
            BrowserFormFieldKey {
                form_index,
                name: name.to_owned(),
            },
            value.to_owned(),
        );
        for (node_id, kind) in targets {
            self.apply_live_form_value(current_index, node_id, &kind, value);
            self.dispatch_live_form_events(current_index, node_id, &["input", "change"]);
        }
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    pub fn focus_selector(&mut self, selector: &str) -> Result<BrowserFocusedControl> {
        let Some(current_index) = self.current_index else {
            bail!("cannot focus: session has no current page");
        };
        let focused = self.focusable_control_for_selector(current_index, selector)?;
        self.set_focused_control(current_index, focused)?;
        self.focused_control()
            .context("focused control disappeared from current render")
    }

    pub fn focus_next_control(&mut self) -> Result<BrowserFocusedControl> {
        self.focus_relative_control(false)
    }

    pub fn focus_previous_control(&mut self) -> Result<BrowserFocusedControl> {
        self.focus_relative_control(true)
    }

    pub fn blur_focused_control(&mut self) -> Result<bool> {
        let Some(current_index) = self.current_index else {
            bail!("cannot blur focus: session has no current page");
        };
        let Some(previous_node_id) = self.entries[current_index]
            .focused_control
            .as_ref()
            .map(|focused| focused.node_id)
        else {
            return Ok(false);
        };

        {
            let entry = &mut self.entries[current_index];
            entry.focused_control = None;
            if previous_node_id < entry.page_state.dom.nodes.len() {
                entry.page_state.runtime.active_element = None;
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    previous_node_id,
                    "blur",
                    false,
                );
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    previous_node_id,
                    "focusout",
                    true,
                );
            }
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }

        self.persist_entry_runtime_storage(current_index);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(true)
    }

    fn focus_relative_control(&mut self, reverse: bool) -> Result<BrowserFocusedControl> {
        let Some(current_index) = self.current_index else {
            bail!("cannot move focus: session has no current page");
        };
        let controls = focusable_controls_for_render(&self.entries[current_index].render);
        ensure!(
            !controls.is_empty(),
            "cannot move focus: current page has no focusable form controls"
        );
        let current_position = self.entries[current_index]
            .focused_control
            .as_ref()
            .and_then(|focused| {
                controls.iter().position(|control| {
                    control.form_index == focused.form_index
                        && control.control_index == focused.control_index
                        && control.name == focused.name
                        && control.kind.eq_ignore_ascii_case(&focused.kind)
                })
            });
        let next_position = match (current_position, reverse) {
            (Some(position), false) => (position + 1) % controls.len(),
            (Some(0), true) => controls.len() - 1,
            (Some(position), true) => position - 1,
            (None, false) => 0,
            (None, true) => controls.len() - 1,
        };
        self.set_focused_control(current_index, controls[next_position].clone())?;
        self.focused_control()
            .context("focused control disappeared from current render")
    }

    fn set_focused_control(
        &mut self,
        current_index: usize,
        focused: BrowserFocusedFormControl,
    ) -> Result<()> {
        let previous = self.entries[current_index].focused_control.clone();
        if previous.as_ref() == Some(&focused) {
            self.entries[current_index]
                .page_state
                .runtime
                .active_element = Some(focused.node_id);
            return Ok(());
        }

        {
            let entry = &mut self.entries[current_index];
            if let Some(previous_node_id) = previous
                .as_ref()
                .map(|previous| previous.node_id)
                .filter(|&node_id| node_id < entry.page_state.dom.nodes.len())
            {
                entry.page_state.runtime.active_element = None;
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    previous_node_id,
                    "blur",
                    false,
                );
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    previous_node_id,
                    "focusout",
                    true,
                );
            }

            entry.focused_control = Some(focused.clone());
            if focused.node_id < entry.page_state.dom.nodes.len() {
                entry.page_state.runtime.active_element = Some(focused.node_id);
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    focused.node_id,
                    "focus",
                    false,
                );
                dispatch_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    focused.node_id,
                    "focusin",
                    true,
                );
            } else {
                entry.focused_control = None;
                entry.page_state.runtime.active_element = None;
            }
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }

        self.persist_entry_runtime_storage(current_index);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        ensure!(
            self.focused_control().is_some(),
            "focused control disappeared during focus event dispatch"
        );
        Ok(())
    }

    pub fn type_text(&mut self, text: &str) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot type text: session has no current page");
        };
        let focused = self.focused_text_control(current_index, "type text")?;
        let (node_id, kind) = self.focused_text_control_target(current_index, &focused)?;

        for ch in text.chars() {
            let key = ch.to_string();
            let keydown =
                self.dispatch_live_keyboard_event(current_index, node_id, "keydown", &key);
            if !keydown.default_prevented {
                let beforeinput = self.dispatch_live_beforeinput_event(
                    current_index,
                    node_id,
                    "insertText",
                    Some(&key),
                );
                if !beforeinput.default_prevented {
                    let mut value = self.live_form_control_value(current_index, node_id);
                    value.push(ch);
                    self.commit_focused_text_value(current_index, &focused, node_id, &kind, &value);
                    self.dispatch_live_form_events(current_index, node_id, &["input"]);
                }
            }
            self.dispatch_live_keyboard_event(current_index, node_id, "keyup", &key);
        }

        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    pub fn delete_text_backward(&mut self, count: usize) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot delete text: session has no current page");
        };
        let focused = self.focused_text_control(current_index, "delete text")?;
        let (node_id, kind) = self.focused_text_control_target(current_index, &focused)?;

        for _ in 0..count {
            let keydown =
                self.dispatch_live_keyboard_event(current_index, node_id, "keydown", "Backspace");
            if !keydown.default_prevented {
                let beforeinput = self.dispatch_live_beforeinput_event(
                    current_index,
                    node_id,
                    "deleteContentBackward",
                    None,
                );
                if !beforeinput.default_prevented {
                    let mut value = self.live_form_control_value(current_index, node_id);
                    if let Some((index, _)) = value.char_indices().next_back() {
                        value.truncate(index);
                        self.commit_focused_text_value(
                            current_index,
                            &focused,
                            node_id,
                            &kind,
                            &value,
                        );
                        self.dispatch_live_form_events(current_index, node_id, &["input"]);
                    }
                }
            }
            self.dispatch_live_keyboard_event(current_index, node_id, "keyup", "Backspace");
        }

        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    pub fn clear_focused_text(&mut self) -> Result<&BrowserRender> {
        self.edit_focused_text_control("clear text", |value| value.clear())
    }

    pub fn toggle_form_control(
        &mut self,
        form_index: usize,
        control_index: usize,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot toggle form control: session has no current page");
        };
        self.toggle_current_form_control_checked(current_index, form_index, control_index)
    }

    pub fn select_form_option(
        &mut self,
        form_index: usize,
        control_index: usize,
        value: &str,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot select form option: session has no current page");
        };
        let (name, node_id, kind) = {
            let entry = &self.entries[current_index];
            let Some(form) = entry.render.forms.get(form_index) else {
                bail!(
                    "form index {} not found; current page has {} form(s)",
                    form_index,
                    entry.render.forms.len()
                );
            };
            let Some(control) = form.controls.get(control_index) else {
                bail!(
                    "control index {} not found; form {} has {} control(s)",
                    control_index,
                    form_index,
                    form.controls.len()
                );
            };
            ensure!(
                !control.name.is_empty(),
                "select control {} in form {} has no name",
                control_index,
                form_index
            );
            ensure!(
                form_control_accepts_select_state(control),
                "control {} in form {} is not an enabled select control",
                control_index,
                form_index
            );
            ensure!(
                form_control_has_enabled_option(control, value),
                "select control {} in form {} has no enabled option value {:?}",
                control_index,
                form_index,
                value
            );
            (control.name.clone(), control.node_id, control.kind.clone())
        };
        self.entries[current_index]
            .form_state
            .insert(BrowserFormFieldKey { form_index, name }, value.to_owned());
        self.apply_live_form_value(current_index, node_id, &kind, value);
        self.dispatch_live_form_events(current_index, node_id, &["input", "change"]);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    pub fn select_focused_option(&mut self, value: &str) -> Result<&BrowserRender> {
        let Some(focused) = self.focused_control() else {
            bail!("cannot select focused option: no focused form control");
        };
        self.select_form_option(focused.form_index, focused.control_index, value)
    }

    pub fn dispatch_wheel_event(&mut self, delta_x: isize, delta_y: isize) -> Result<bool> {
        let Some(current_index) = self.current_index else {
            bail!("cannot dispatch wheel event: session has no current page");
        };
        let mut dispatch = BrowserEventDispatch {
            node_id: 0,
            default_prevented: false,
        };
        if let Some(entry) = self.entries.get_mut(current_index) {
            dispatch = dispatch_event_listeners_with_payload(
                &mut entry.page_state.dom,
                &mut entry.page_state.runtime,
                BrowserEventPayload::wheel(0, delta_x, delta_y),
                true,
            );
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }
        self.persist_entry_runtime_storage(current_index);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(dispatch.default_prevented)
    }

    fn edit_focused_text_control(
        &mut self,
        action: &str,
        edit: impl FnOnce(&mut String),
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot {action}: session has no current page");
        };
        let focused = self.focused_text_control(current_index, action)?;
        let (node_id, kind) = self.focused_text_control_target(current_index, &focused)?;
        let mut value = self.live_form_control_value(current_index, node_id);
        edit(&mut value);
        self.commit_focused_text_value(current_index, &focused, node_id, &kind, &value);
        self.dispatch_live_form_events(current_index, node_id, &["input"]);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    fn focused_text_control(
        &self,
        current_index: usize,
        action: &str,
    ) -> Result<BrowserFocusedFormControl> {
        let Some(focused) = self.entries[current_index].focused_control.clone() else {
            bail!("cannot {action}: no focused form control");
        };
        self.focused_text_control_target(current_index, &focused)?;
        Ok(focused)
    }

    fn focused_text_control_target(
        &self,
        current_index: usize,
        focused: &BrowserFocusedFormControl,
    ) -> Result<(usize, String)> {
        let entry = &self.entries[current_index];
        let control = entry
            .render
            .forms
            .get(focused.form_index)
            .and_then(|form| form.controls.get(focused.control_index))
            .with_context(|| {
                format!(
                    "focused control form={} control={} no longer exists",
                    focused.form_index, focused.control_index
                )
            })?;
        ensure!(
            control.node_id == focused.node_id
                && control.name == focused.name
                && control.kind.eq_ignore_ascii_case(&focused.kind)
                && form_control_accepts_text_edit_state(control),
            "focused control {:?} is no longer an editable text-like control",
            focused.name
        );
        Ok((control.node_id, control.kind.clone()))
    }

    fn live_form_control_value(&self, current_index: usize, node_id: usize) -> String {
        let Some(entry) = self.entries.get(current_index) else {
            return String::new();
        };
        match entry
            .page_state
            .dom
            .nodes
            .get(node_id)
            .map(|node| &node.kind)
        {
            Some(NodeKind::Element(element)) if element.tag == "textarea" => element
                .value
                .clone()
                .unwrap_or_else(|| text_content(&entry.page_state.dom, node_id)),
            _ => get_element_attribute(&entry.page_state.dom, node_id, "value").unwrap_or_default(),
        }
    }

    fn commit_focused_text_value(
        &mut self,
        current_index: usize,
        focused: &BrowserFocusedFormControl,
        node_id: usize,
        kind: &str,
        value: &str,
    ) {
        self.entries[current_index].form_state.insert(
            BrowserFormFieldKey {
                form_index: focused.form_index,
                name: focused.name.clone(),
            },
            value.to_owned(),
        );
        self.apply_live_form_value(current_index, node_id, kind, value);
    }

    fn toggle_current_form_control_checked(
        &mut self,
        current_index: usize,
        form_index: usize,
        control_index: usize,
    ) -> Result<&BrowserRender> {
        let (kind, name, checked, target_node_id) = {
            let entry = &self.entries[current_index];
            let Some(form) = entry.render.forms.get(form_index) else {
                bail!(
                    "form index {} not found; current page has {} form(s)",
                    form_index,
                    entry.render.forms.len()
                );
            };
            let Some(control) = form.controls.get(control_index) else {
                bail!(
                    "control index {} not found; form {} has {} control(s)",
                    control_index,
                    form_index,
                    form.controls.len()
                );
            };
            ensure!(
                form_control_accepts_checked_state(control),
                "control {} in form {} is not a checkable checkbox or radio",
                control_index,
                form_index
            );
            (
                control.kind.clone(),
                control.name.clone(),
                control.checked,
                control.node_id,
            )
        };
        let next_checked = if kind.eq_ignore_ascii_case("radio") {
            true
        } else {
            !checked
        };

        let mut checked_updates = Vec::new();
        if kind.eq_ignore_ascii_case("radio") && next_checked {
            let radio_group = self.entries[current_index]
                .render
                .forms
                .get(form_index)
                .map(|form| {
                    form.controls
                        .iter()
                        .enumerate()
                        .filter(|(_, control)| {
                            control.kind.eq_ignore_ascii_case("radio") && control.name == name
                        })
                        .map(|(index, control)| (index, control.node_id))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (index, node_id) in radio_group {
                self.entries[current_index].checked_state.insert(
                    BrowserFormControlKey {
                        form_index,
                        control_index: index,
                    },
                    false,
                );
                checked_updates.push((node_id, false));
            }
        }
        self.entries[current_index].checked_state.insert(
            BrowserFormControlKey {
                form_index,
                control_index,
            },
            next_checked,
        );
        checked_updates.push((target_node_id, next_checked));
        for (node_id, checked) in checked_updates {
            self.apply_live_form_checked(current_index, node_id, checked);
        }
        self.dispatch_live_form_events(current_index, target_node_id, &["input", "change"]);
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    pub async fn activate_link(&mut self, link_index: usize) -> Result<&BrowserRender> {
        let target = self.link_target(link_index)?;
        self.navigate(&target).await
    }

    pub fn link_target(&self, link_index: usize) -> Result<String> {
        let Some(current) = self.current() else {
            bail!("cannot activate link: session has no current page");
        };
        let Some(link) = current.links.get(link_index) else {
            bail!(
                "link index {} not found; current page has {} link(s)",
                link_index,
                current.links.len()
            );
        };
        Ok(link.resolved.clone())
    }

    pub fn focused_control(&self) -> Option<BrowserFocusedControl> {
        let current_index = self.current_index?;
        let entry = self.entries.get(current_index)?;
        let focused = entry.focused_control.as_ref()?;
        entry
            .render
            .forms
            .get(focused.form_index)
            .and_then(|form| form.controls.get(focused.control_index))
            .filter(|control| {
                control.name == focused.name && control.kind.eq_ignore_ascii_case(&focused.kind)
            })
            .map(|control| BrowserFocusedControl {
                form_index: focused.form_index,
                control_index: focused.control_index,
                name: control.name.clone(),
                kind: control.kind.clone(),
                value: control.value.clone(),
            })
    }

    pub async fn activate_link_text(&mut self, text: &str) -> Result<&BrowserRender> {
        let target = self.link_text_target(text)?;
        self.navigate(&target).await
    }

    pub fn link_text_target(&self, text: &str) -> Result<String> {
        let Some(current) = self.current() else {
            bail!("cannot activate link text: session has no current page");
        };
        let text = collapse_ascii_whitespace(text);
        ensure!(!text.is_empty(), "cannot activate empty link text");
        let matches = current
            .links
            .iter()
            .filter(|link| link.text == text)
            .collect::<Vec<_>>();
        let Some(link) = matches.first() else {
            bail!("link text {text:?} not found");
        };
        ensure!(
            matches.len() == 1,
            "link text {:?} is ambiguous; {} links match",
            text,
            matches.len()
        );
        Ok(link.resolved.clone())
    }

    pub async fn activate_link_selector(&mut self, selector: &str) -> Result<&BrowserRender> {
        let target = self.link_selector_target(selector)?;
        self.navigate(&target).await
    }

    pub fn link_selector_target(&self, selector: &str) -> Result<String> {
        let Some(current_index) = self.current_index else {
            bail!("cannot activate link selector: session has no current page");
        };
        let entry = &self.entries[current_index];
        let Some(node_id) = find_first_matching_selector(&entry.page_state.dom, selector) else {
            bail!("link selector not found: {selector}");
        };
        let Some(href) = anchor_href_for_node(&entry.page_state.dom, node_id) else {
            bail!("selector did not resolve to a link: {selector}");
        };
        Ok(resolve_browser_href(&entry.render.source, &href))
    }

    pub fn current_links(&self) -> &[BrowserLink] {
        self.current().map_or(&[], |render| render.links.as_slice())
    }

    pub fn link_target_at(&self, x: usize, y: usize) -> Option<String> {
        let current_index = self.current_index?;
        let entry = self.entries.get(current_index)?;
        let node_id = hit_test_target_node(&entry.render, x, y)?;
        let href = anchor_href_for_node(&entry.page_state.dom, node_id)?;
        Some(resolve_browser_href(&entry.render.source, &href))
    }

    pub fn link_target_at_viewport(
        &self,
        viewport: BrowserViewportState,
        x: usize,
        y: usize,
    ) -> Option<String> {
        let current_index = self.current_index?;
        let entry = self.entries.get(current_index)?;
        let node_id = hit_test_target_node_in_viewport(&entry.render, viewport, x, y)?;
        let href = anchor_href_for_node(&entry.page_state.dom, node_id)?;
        Some(resolve_browser_href(&entry.render.source, &href))
    }

    pub fn current_forms(&self) -> &[BrowserForm] {
        self.current().map_or(&[], |render| render.forms.as_slice())
    }

    pub fn back(&mut self) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot go back: session has no current page");
        };
        ensure!(current_index > 0, "cannot go back: already at first page");
        let next_index = current_index - 1;
        self.current_index = Some(next_index);
        Ok(&self.entries[next_index].render)
    }

    pub fn forward(&mut self) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot go forward: session has no current page");
        };
        let next_index = current_index + 1;
        ensure!(
            next_index < self.entries.len(),
            "cannot go forward: already at latest page"
        );
        self.current_index = Some(next_index);
        Ok(&self.entries[next_index].render)
    }

    pub fn current(&self) -> Option<&BrowserRender> {
        self.current_index
            .and_then(|index| self.entries.get(index))
            .map(|entry| &entry.render)
    }

    pub fn resolve_current_target(&self, target: &str) -> String {
        self.current().map_or_else(
            || target.to_owned(),
            |render| resolve_browser_href(&render.source, target),
        )
    }

    pub fn click_selector(&mut self, selector: &str) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot click: session has no current page");
        };
        let profiled =
            self.click_current_page_state(current_index, RenderClickTarget::Selector(selector))?;
        self.set_entry_render(current_index, profiled.render);
        Ok(&self.entries[current_index].render)
    }

    pub async fn click_selector_with_default_action(
        &mut self,
        selector: &str,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot click: session has no current page");
        };
        let profiled =
            self.click_current_page_state(current_index, RenderClickTarget::Selector(selector))?;
        let default_action = profiled
            .click_default_action
            .clone()
            .filter(|action| !action.default_prevented());
        let focused_control = self
            .focusable_control_for_selector(current_index, selector)
            .ok();
        self.set_entry_render(current_index, profiled.render);
        if let Some(focused_control) = focused_control {
            self.set_focused_control(current_index, focused_control)?;
        } else {
            self.blur_focused_control()?;
        }
        if let Some(action) = default_action {
            let render = self.entries[current_index].render.clone();
            return self
                .apply_click_default_action(current_index, render, action)
                .await;
        }
        Ok(&self.entries[current_index].render)
    }

    pub async fn click_at_with_default_action(
        &mut self,
        x: usize,
        y: usize,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot click coordinates: session has no current page");
        };
        let target_node = {
            let current = &self.entries[current_index].render;
            hit_test_target_node(current, x, y)
                .with_context(|| format!("click coordinates {x},{y} did not hit a DOM target"))?
        };
        let profiled = self.click_current_page_state(
            current_index,
            RenderClickTarget::Point {
                node_id: target_node,
                x,
                y,
            },
        )?;
        let default_action = profiled
            .click_default_action
            .clone()
            .filter(|action| !action.default_prevented());
        let focused_control = self.focusable_control_for_node(current_index, target_node);
        self.set_entry_render(current_index, profiled.render);
        if let Some(focused_control) = focused_control {
            self.set_focused_control(current_index, focused_control)?;
        } else {
            self.blur_focused_control()?;
        }
        if let Some(action) = default_action {
            let render = self.entries[current_index].render.clone();
            return self
                .apply_click_default_action(current_index, render, action)
                .await;
        }
        Ok(&self.entries[current_index].render)
    }

    pub async fn click_viewport_at_with_default_action(
        &mut self,
        viewport: BrowserViewportState,
        x: usize,
        y: usize,
    ) -> Result<&BrowserRender> {
        let Some(current_index) = self.current_index else {
            bail!("cannot click viewport coordinates: session has no current page");
        };
        let (target_node, page_x, page_y) = {
            let current = &self.entries[current_index].render;
            let viewport = browser_document_viewport(current, viewport, None).viewport;
            let page_x = viewport.x.saturating_add(x);
            let page_y = viewport.y.saturating_add(y);
            let target_node =
                hit_test_target_node_in_viewport(current, viewport, x, y).with_context(|| {
                    format!(
                        "click viewport coordinates {x},{y} at viewport {},{} did not hit a DOM target",
                        viewport.x, viewport.y
                    )
                })?;
            (target_node, page_x, page_y)
        };
        let profiled = self.click_current_page_state(
            current_index,
            RenderClickTarget::Point {
                node_id: target_node,
                x: page_x,
                y: page_y,
            },
        )?;
        let default_action = profiled
            .click_default_action
            .clone()
            .filter(|action| !action.default_prevented());
        let focused_control = self.focusable_control_for_node(current_index, target_node);
        self.set_entry_render(current_index, profiled.render);
        if let Some(focused_control) = focused_control {
            self.set_focused_control(current_index, focused_control)?;
        } else {
            self.blur_focused_control()?;
        }
        if let Some(action) = default_action {
            let render = self.entries[current_index].render.clone();
            return self
                .apply_click_default_action(current_index, render, action)
                .await;
        }
        Ok(&self.entries[current_index].render)
    }

    fn click_current_page_state(
        &mut self,
        current_index: usize,
        click_target: RenderClickTarget<'_>,
    ) -> Result<BrowserProfiledRender> {
        let page_source = self.entries[current_index].render.source.clone();
        let click_default_action = {
            let page_state = &mut self.entries[current_index].page_state;
            let dispatch = match click_target {
                RenderClickTarget::Selector(selector) => {
                    dispatch_click_selector(&mut page_state.dom, &mut page_state.runtime, selector)?
                }
                RenderClickTarget::Point { node_id, x, y } => dispatch_pointer_click_node(
                    &mut page_state.dom,
                    &mut page_state.runtime,
                    node_id,
                    x,
                    y,
                )?,
            };
            let action = click_default_action_for_node(&page_state.dom, &page_source, dispatch);
            if action
                .as_ref()
                .is_none_or(BrowserClickDefaultAction::drains_post_click_timers)
            {
                drain_timer_tasks(&mut page_state.dom, &mut page_state.runtime);
            }
            action
        };
        self.persist_entry_runtime_storage(current_index);
        Ok(self.render_entry_page_state_profiled(current_index, click_default_action))
    }

    async fn apply_click_default_action(
        &mut self,
        current_index: usize,
        render: BrowserRender,
        action: BrowserClickDefaultAction,
    ) -> Result<&BrowserRender> {
        match action {
            BrowserClickDefaultAction::Anchor { resolved, .. } => self.navigate(&resolved).await,
            BrowserClickDefaultAction::SubmitForm {
                form_index,
                submitter,
                ..
            } => {
                self.set_entry_render(current_index, render);
                self.submit_form_with_submitter(form_index, &[], &submitter)
                    .await
            }
            BrowserClickDefaultAction::ResetForm { form_index, .. } => {
                self.reset_current_form_state_with_render(current_index, form_index, render)
            }
            BrowserClickDefaultAction::ToggleFormControl {
                form_index,
                control_index,
                ..
            } => {
                self.set_entry_render(current_index, render);
                self.toggle_current_form_control_checked(current_index, form_index, control_index)
            }
        }
    }

    fn reset_current_form_state_with_render(
        &mut self,
        current_index: usize,
        form_index: usize,
        _render: BrowserRender,
    ) -> Result<&BrowserRender> {
        ensure!(
            form_index < self.entries[current_index].render.forms.len(),
            "form index {} not found; current page has {} form(s)",
            form_index,
            self.entries[current_index].render.forms.len()
        );
        let reset_event = self.dispatch_current_form_event(current_index, form_index, "reset")?;
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        if reset_event.default_prevented {
            return Ok(&self.entries[current_index].render);
        }
        self.reset_live_form_dom_to_defaults(current_index, form_index);
        clear_form_state_for_form(&mut self.entries[current_index].form_state, form_index);
        clear_form_checked_state_for_form(
            &mut self.entries[current_index].checked_state,
            form_index,
        );
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);
        Ok(&self.entries[current_index].render)
    }

    fn reset_live_form_dom_to_defaults(&mut self, current_index: usize, form_index: usize) {
        let Some(entry) = self.entries.get(current_index) else {
            return;
        };
        let parsed = parse_html(&entry.html);
        let default_forms = collect_forms(&parsed.dom, &entry.render.source);
        let Some(default_form) = default_forms.get(form_index) else {
            return;
        };
        let Some(current_form) = entry.render.forms.get(form_index) else {
            return;
        };
        let resets = current_form
            .controls
            .iter()
            .zip(default_form.controls.iter())
            .map(|(current, default)| {
                (
                    current.node_id,
                    current.kind.clone(),
                    default.value.clone(),
                    default.checked,
                )
            })
            .collect::<Vec<_>>();
        for (node_id, kind, value, checked) in resets {
            if form_control_kind_is_checkable(&kind) {
                self.apply_live_form_checked(current_index, node_id, checked);
            } else {
                self.apply_live_form_value(current_index, node_id, &kind, &value);
            }
        }
    }

    fn apply_live_form_value(
        &mut self,
        current_index: usize,
        node_id: usize,
        kind: &str,
        value: &str,
    ) {
        if let Some(entry) = self.entries.get_mut(current_index) {
            set_live_form_value(&mut entry.page_state.dom, node_id, kind, value);
        }
    }

    fn apply_live_form_checked(&mut self, current_index: usize, node_id: usize, checked: bool) {
        if let Some(entry) = self.entries.get_mut(current_index) {
            set_element_boolean_property(&mut entry.page_state.dom, node_id, "checked", checked);
        }
    }

    fn dispatch_live_form_events(&mut self, current_index: usize, node_id: usize, events: &[&str]) {
        if let Some(entry) = self.entries.get_mut(current_index) {
            for event_name in events {
                dispatch_bubbling_event_listeners(
                    &mut entry.page_state.dom,
                    &mut entry.page_state.runtime,
                    node_id,
                    event_name,
                );
            }
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }
        self.persist_entry_runtime_storage(current_index);
    }

    fn dispatch_live_keyboard_event(
        &mut self,
        current_index: usize,
        node_id: usize,
        event_name: &str,
        key: &str,
    ) -> BrowserEventDispatch {
        let mut dispatch = BrowserEventDispatch {
            node_id,
            default_prevented: false,
        };
        if let Some(entry) = self.entries.get_mut(current_index) {
            dispatch = dispatch_keyboard_event(
                &mut entry.page_state.dom,
                &mut entry.page_state.runtime,
                node_id,
                event_name,
                key,
            );
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }
        self.persist_entry_runtime_storage(current_index);
        dispatch
    }

    fn dispatch_live_beforeinput_event(
        &mut self,
        current_index: usize,
        node_id: usize,
        input_type: &str,
        data: Option<&str>,
    ) -> BrowserEventDispatch {
        let mut dispatch = BrowserEventDispatch {
            node_id,
            default_prevented: false,
        };
        if let Some(entry) = self.entries.get_mut(current_index) {
            dispatch = dispatch_beforeinput_event(
                &mut entry.page_state.dom,
                &mut entry.page_state.runtime,
                node_id,
                input_type,
                data,
            );
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
        }
        self.persist_entry_runtime_storage(current_index);
        dispatch
    }

    fn dispatch_current_form_event(
        &mut self,
        current_index: usize,
        form_index: usize,
        event_name: &str,
    ) -> Result<BrowserEventDispatch> {
        ensure!(
            current_index < self.entries.len(),
            "cannot dispatch form event: session entry {current_index} does not exist"
        );
        ensure!(
            form_index < self.entries[current_index].render.forms.len(),
            "form index {} not found; current page has {} form(s)",
            form_index,
            self.entries[current_index].render.forms.len()
        );
        let form_node_id =
            form_node_id_for_index(&self.entries[current_index].page_state.dom, form_index)
                .with_context(|| format!("form node for form index {form_index} not found"))?;
        let dispatch = {
            let entry = &mut self.entries[current_index];
            let dispatch = dispatch_bubbling_event_listeners(
                &mut entry.page_state.dom,
                &mut entry.page_state.runtime,
                form_node_id,
                event_name,
            );
            drain_timer_tasks(&mut entry.page_state.dom, &mut entry.page_state.runtime);
            dispatch
        };
        self.persist_entry_runtime_storage(current_index);
        Ok(dispatch)
    }

    fn render_entry_page_state_profiled(
        &self,
        current_index: usize,
        click_default_action: Option<BrowserClickDefaultAction>,
    ) -> BrowserProfiledRender {
        let entry = &self.entries[current_index];
        let cached_images = decoded_cached_images(&self.resource_cache);
        render_page_state_profiled_with_cached_images(
            &entry.render.source,
            self.options,
            &entry.page_state,
            &cached_images,
            click_default_action,
        )
    }

    fn render_entry_page_state(&self, current_index: usize) -> BrowserRender {
        self.render_entry_page_state_profiled(current_index, None)
            .render
    }

    fn persist_entry_runtime_storage(&mut self, current_index: usize) {
        let Some((source, local_values, session_values)) =
            self.entries.get(current_index).map(|entry| {
                (
                    entry.render.source.clone(),
                    entry.page_state.runtime.local_storage.clone(),
                    entry.page_state.runtime.session_storage.clone(),
                )
            })
        else {
            return;
        };
        let origin = storage_origin_key(&source);
        persist_web_storage_origin(&mut self.local_storage, &origin, &local_values);
        persist_web_storage_origin(&mut self.session_storage, &origin, &session_values);
    }

    pub fn snapshot(&self) -> BrowserHistorySnapshot {
        BrowserHistorySnapshot {
            current_index: self.current_index,
            retained_entry_limit: BROWSER_SESSION_HISTORY_MAX_ENTRIES,
            retained_entry_count: self.entries.len(),
            entries: self
                .entries
                .iter()
                .map(|entry| BrowserHistoryEntry {
                    target: entry.target.clone(),
                    source: entry.render.source.clone(),
                    title: entry.render.title.clone(),
                })
                .collect(),
        }
    }

    pub fn cookies_snapshot(&self) -> Vec<BrowserCookie> {
        self.cookie_jar.snapshot()
    }

    pub fn clear_cookies(&mut self) {
        self.cookie_jar.clear();
    }

    pub fn local_storage_snapshot(&self) -> BrowserLocalStorage {
        self.local_storage.clone()
    }

    pub fn local_storage_entries(&self) -> Vec<BrowserLocalStorageEntry> {
        browser_storage_entries(&self.local_storage)
    }

    pub fn session_storage_entries(&self) -> Vec<BrowserLocalStorageEntry> {
        browser_storage_entries(&self.session_storage)
    }

    pub fn clear_local_storage(&mut self) {
        self.local_storage.origins.clear();
        for entry in &mut self.entries {
            entry.page_state.runtime.local_storage.clear();
        }
    }

    pub fn clear_session_storage(&mut self) {
        self.session_storage.origins.clear();
        for entry in &mut self.entries {
            entry.page_state.runtime.session_storage.clear();
        }
    }

    fn focusable_control_for_selector(
        &self,
        current_index: usize,
        selector: &str,
    ) -> Result<BrowserFocusedFormControl> {
        let entry = &self.entries[current_index];
        let Some(node_id) = find_first_matching_selector(&entry.page_state.dom, selector) else {
            bail!("focus selector not found: {selector}");
        };
        focusable_form_control_for_node(&entry.page_state.dom, node_id).with_context(|| {
            format!("selector did not resolve to a focusable form control: {selector}")
        })
    }

    fn focusable_control_for_node(
        &self,
        current_index: usize,
        node_id: usize,
    ) -> Option<BrowserFocusedFormControl> {
        let entry = self.entries.get(current_index)?;
        focusable_form_control_for_node(&entry.page_state.dom, node_id)
    }

    pub async fn fetch_current_resources(
        &mut self,
        max_resource_bytes: usize,
    ) -> Result<BrowserResourceFetchReport> {
        let Some(current) = self.current() else {
            bail!("cannot fetch resources: session has no current page");
        };
        let page_source = current.source.clone();
        let resources = current.resources.clone();
        let mut fetched_resources = Vec::with_capacity(resources.len());

        for resource in resources {
            let fetch = fetch_resource_with_cache(
                resource,
                max_resource_bytes,
                &mut self.cookie_jar,
                &mut self.resource_cache,
            )
            .await;
            fetched_resources.push(fetch);
        }

        let fetched = fetched_resources
            .iter()
            .filter(|resource| resource.status == "fetched")
            .count();
        let cached = fetched_resources
            .iter()
            .filter(|resource| resource.status == "cached")
            .count();
        let failed = fetched_resources
            .iter()
            .filter(|resource| resource.status == "failed")
            .count();
        let skipped = fetched_resources
            .iter()
            .filter(|resource| resource.status == "skipped")
            .count();

        Ok(BrowserResourceFetchReport {
            page_source,
            total: fetched_resources.len(),
            fetched,
            cached,
            failed,
            skipped,
            cached_resource_count: self.resource_cache.len(),
            cached_resource_bytes: self.resource_cache.total_bytes(),
            resources: fetched_resources,
        })
    }

    pub async fn render_current_with_stylesheets(
        &mut self,
        max_resource_bytes: usize,
    ) -> Result<BrowserStylesheetRenderReport> {
        let Some(current_index) = self.current_index else {
            bail!("cannot render with stylesheets: session has no current page");
        };
        let page_source = self.entries[current_index].render.source.clone();
        let stylesheet_resources = self.entries[current_index]
            .render
            .resources
            .iter()
            .filter(|resource| resource.kind == "stylesheet")
            .cloned()
            .collect::<Vec<_>>();
        let mut fetches = Vec::with_capacity(stylesheet_resources.len());
        let mut stylesheet_text = Vec::new();

        for resource in stylesheet_resources {
            let fetch = fetch_resource_with_cache(
                resource,
                max_resource_bytes,
                &mut self.cookie_jar,
                &mut self.resource_cache,
            )
            .await;
            if matches!(fetch.status.as_str(), "fetched" | "cached")
                && let Some(bytes) = self.resource_cache.cached_bytes(&fetch.resource.resolved)
            {
                stylesheet_text.push(String::from_utf8_lossy(bytes).into_owned());
            }
            fetches.push(fetch);
        }

        let applied = stylesheet_text.len();
        let failed = fetches
            .iter()
            .filter(|fetch| matches!(fetch.status.as_str(), "failed" | "skipped"))
            .count();
        if applied > 0 {
            let css_text = &mut self.entries[current_index].page_state.css_text;
            for sheet in &stylesheet_text {
                css_text.push('\n');
                css_text.push_str(sheet);
            }
        }
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);

        Ok(BrowserStylesheetRenderReport {
            page_source,
            stylesheet_count: fetches.len(),
            applied,
            failed,
            cached_resource_count: self.resource_cache.len(),
            cached_resource_bytes: self.resource_cache.total_bytes(),
            fetches,
        })
    }

    pub async fn render_current_with_scripts(
        &mut self,
        max_resource_bytes: usize,
    ) -> Result<BrowserScriptRenderReport> {
        let Some(current_index) = self.current_index else {
            bail!("cannot render with scripts: session has no current page");
        };
        let page_source = self.entries[current_index].render.source.clone();
        let script_resources = self.entries[current_index]
            .render
            .resources
            .iter()
            .filter(|resource| resource.kind == "script")
            .cloned()
            .collect::<Vec<_>>();
        let mut fetches = Vec::with_capacity(script_resources.len());
        let mut script_text = Vec::new();

        for resource in script_resources {
            let fetch = fetch_resource_with_cache(
                resource,
                max_resource_bytes,
                &mut self.cookie_jar,
                &mut self.resource_cache,
            )
            .await;
            if matches!(fetch.status.as_str(), "fetched" | "cached")
                && let Some(bytes) = self.resource_cache.cached_bytes(&fetch.resource.resolved)
            {
                script_text.push(String::from_utf8_lossy(bytes).into_owned());
            }
            fetches.push(fetch);
        }

        let applied = script_text.len();
        let failed = fetches
            .iter()
            .filter(|fetch| matches!(fetch.status.as_str(), "failed" | "skipped"))
            .count();
        if applied > 0 {
            let page_state = &mut self.entries[current_index].page_state;
            execute_scripts_without_lifecycle_events(
                &mut page_state.dom,
                &mut page_state.runtime,
                &script_text,
            );
            self.persist_entry_runtime_storage(current_index);
        }
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);

        Ok(BrowserScriptRenderReport {
            page_source,
            script_count: fetches.len(),
            applied,
            failed,
            cached_resource_count: self.resource_cache.len(),
            cached_resource_bytes: self.resource_cache.total_bytes(),
            fetches,
        })
    }

    pub async fn render_current_with_images(
        &mut self,
        max_resource_bytes: usize,
    ) -> Result<BrowserImageRenderReport> {
        let Some(current_index) = self.current_index else {
            bail!("cannot render with images: session has no current page");
        };
        let page_source = self.entries[current_index].render.source.clone();
        let viewport_width_css_px = self.entries[current_index]
            .render
            .viewport_width
            .saturating_mul(8);
        let mut image_resources = collect_selected_image_resources(
            &self.entries[current_index].page_state.dom,
            &page_source,
            viewport_width_css_px,
        );
        let css_cascade = parse_css(&self.entries[current_index].page_state.css_text);
        image_resources.extend(collect_css_background_image_resources(
            &self.entries[current_index].page_state.dom,
            &page_source,
            &css_cascade,
        ));
        let mut seen_image_resources = HashSet::new();
        image_resources.retain(|resource| seen_image_resources.insert(resource.resolved.clone()));
        let mut fetches = Vec::with_capacity(image_resources.len());

        for resource in image_resources {
            let fetch = fetch_resource_with_cache(
                resource,
                max_resource_bytes,
                &mut self.cookie_jar,
                &mut self.resource_cache,
            )
            .await;
            fetches.push(fetch);
        }

        let cached_images = decoded_cached_images(&self.resource_cache);
        let decoded = cached_images.len();
        let decoded_image_bytes = cached_images
            .iter()
            .map(|entry| entry.image.pixels.len())
            .sum();
        let failed = fetches
            .iter()
            .filter(|fetch| matches!(fetch.status.as_str(), "failed" | "skipped"))
            .count();
        self.entries[current_index].page_state.cached_images = cached_images;
        let render = self.render_entry_page_state(current_index);
        self.set_entry_render(current_index, render);

        Ok(BrowserImageRenderReport {
            page_source,
            image_count: fetches.len(),
            decoded,
            failed,
            cached_resource_count: self.resource_cache.len(),
            cached_resource_bytes: self.resource_cache.total_bytes(),
            decoded_image_bytes,
            fetches,
        })
    }

    fn set_entry_render(&mut self, index: usize, mut render: BrowserRender) {
        if let Some(entry) = self.entries.get_mut(index) {
            apply_form_state_to_render(&mut render, &entry.form_state);
            apply_form_checked_state_to_render(&mut render, &entry.checked_state);
            entry.render = render;
        }
    }
}

fn set_live_form_value(dom: &mut Dom, node_id: usize, kind: &str, value: &str) {
    if kind.eq_ignore_ascii_case("select") {
        set_live_select_value(dom, node_id, value);
    } else {
        set_element_attribute(dom, node_id, "value", value);
    }
}

fn set_live_select_value(dom: &mut Dom, node_id: usize, value: &str) {
    set_element_attribute(dom, node_id, "value", value);
    let mut matched = false;
    set_live_select_option_values(dom, node_id, value, &mut matched);
}

fn set_live_select_option_values(dom: &mut Dom, node_id: usize, value: &str, matched: &mut bool) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    let is_option = matches!(&node.kind, NodeKind::Element(element) if element.tag == "option");
    let children = node.children.clone();
    if is_option {
        let option_value = option_dom_value(dom, node_id);
        let selected = !*matched && option_value == value;
        set_element_boolean_property(dom, node_id, "selected", selected);
        if selected {
            *matched = true;
        }
    }
    for child in children {
        set_live_select_option_values(dom, child, value, matched);
    }
}

fn option_dom_value(dom: &Dom, node_id: usize) -> String {
    match dom.nodes.get(node_id).map(|node| &node.kind) {
        Some(NodeKind::Element(element)) => element
            .value
            .clone()
            .unwrap_or_else(|| collapse_ascii_whitespace(&text_content(dom, node_id))),
        _ => String::new(),
    }
}

fn form_control_kind_is_checkable(kind: &str) -> bool {
    matches!(kind.to_ascii_lowercase().as_str(), "checkbox" | "radio")
}

fn storage_origin_key(source: &str) -> String {
    let Ok(url) = Url::parse(source) else {
        return "file://".to_owned();
    };
    match url.scheme() {
        "http" | "https" => {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            match url.port_or_known_default() {
                Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
                None => format!("{}://{}", url.scheme(), host),
            }
        }
        "file" => "file://".to_owned(),
        scheme => {
            if let Some(host) = url.host_str() {
                format!("{scheme}://{}", host.to_ascii_lowercase())
            } else {
                format!("{scheme}:")
            }
        }
    }
}

fn persist_runtime_web_storage(
    source: &str,
    runtime: &TinyJsRuntime,
    local_storage: Option<&mut BrowserLocalStorage>,
    session_storage: Option<&mut BrowserLocalStorage>,
) {
    let origin = storage_origin_key(source);
    if let Some(storage) = local_storage {
        persist_web_storage_origin(storage, &origin, &runtime.local_storage);
    }
    if let Some(storage) = session_storage {
        persist_web_storage_origin(storage, &origin, &runtime.session_storage);
    }
}

fn persist_web_storage_origin(
    storage: &mut BrowserLocalStorage,
    origin: &str,
    values: &HashMap<String, String>,
) {
    if values.is_empty() {
        storage.origins.remove(origin);
    } else {
        storage.origins.insert(origin.to_owned(), values.clone());
    }
}

pub fn render_html(source: &str, html: &[u8], options: BrowserRenderOptions) -> BrowserRender {
    render_html_with_external_css(source, html, options, &[])
}

pub fn render_html_with_external_css(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    external_css: &[String],
) -> BrowserRender {
    render_html_with_external_css_and_scripts(source, html, options, external_css, &[])
}

pub fn render_html_with_external_css_and_scripts(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    external_css: &[String],
    external_scripts: &[String],
) -> BrowserRender {
    render_html_prepared(
        source,
        html,
        options,
        external_css,
        external_scripts,
        None,
        None,
    )
    .expect("render without interaction should not fail")
}

pub fn render_html_with_click(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    selector: &str,
) -> Result<BrowserRender> {
    render_html_prepared(source, html, options, &[], &[], Some(selector), None)
}

fn render_html_prepared(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    external_css: &[String],
    external_scripts: &[String],
    click_selector: Option<&str>,
    local_storage: Option<&mut BrowserLocalStorage>,
) -> Result<BrowserRender> {
    render_html_prepared_with_inputs(
        source,
        html,
        options,
        RenderPreparation {
            external_css,
            external_scripts,
            click_target: click_selector.map(RenderClickTarget::Selector),
            local_storage,
            session_storage: None,
            cached_images: &[],
        },
    )
}

struct RenderPreparation<'a> {
    external_css: &'a [String],
    external_scripts: &'a [String],
    click_target: Option<RenderClickTarget<'a>>,
    local_storage: Option<&'a mut BrowserLocalStorage>,
    session_storage: Option<&'a mut BrowserLocalStorage>,
    cached_images: &'a [DecodedImageEntry],
}

enum RenderClickTarget<'a> {
    Selector(&'a str),
    Point { node_id: usize, x: usize, y: usize },
}

fn render_html_prepared_with_inputs(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    preparation: RenderPreparation<'_>,
) -> Result<BrowserRender> {
    Ok(render_html_prepared_profiled(source, html, options, preparation)?.render)
}

fn render_html_prepared_profiled(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    preparation: RenderPreparation<'_>,
) -> Result<BrowserProfiledRender> {
    Ok(render_html_prepared_with_state(source, html, options, preparation)?.1)
}

fn render_html_prepared_with_state(
    source: &str,
    html: &[u8],
    options: BrowserRenderOptions,
    preparation: RenderPreparation<'_>,
) -> Result<(BrowserPageState, BrowserProfiledRender)> {
    let total_start = Instant::now();
    let parse_start = Instant::now();
    let parsed = parse_html(html);
    let parse_us = parse_start.elapsed().as_micros();

    let mut css_text = parsed.style_text;
    for sheet in preparation.external_css {
        css_text.push('\n');
        css_text.push_str(sheet);
    }
    let mut dom = parsed.dom;
    let mut scripts = parsed.inline_scripts;
    scripts.extend(preparation.external_scripts.iter().cloned());

    let script_start = Instant::now();
    let storage_origin = (preparation.local_storage.is_some()
        || preparation.session_storage.is_some())
    .then(|| storage_origin_key(source));
    let initial_local_storage = match (
        preparation.local_storage.as_ref(),
        storage_origin.as_deref(),
    ) {
        (Some(storage), Some(origin)) => storage.origins.get(origin).cloned().unwrap_or_default(),
        _ => HashMap::new(),
    };
    let initial_session_storage = match (
        preparation.session_storage.as_ref(),
        storage_origin.as_deref(),
    ) {
        (Some(storage), Some(origin)) => storage.origins.get(origin).cloned().unwrap_or_default(),
        _ => HashMap::new(),
    };
    let mut runtime =
        TinyJsRuntime::with_web_storage(initial_local_storage, initial_session_storage);
    runtime.page_source = source.to_owned();
    execute_scripts_with_runtime(&mut dom, &mut runtime, &scripts);
    let click_default_action = match preparation.click_target {
        Some(RenderClickTarget::Selector(selector)) => {
            let dispatch = dispatch_click_selector(&mut dom, &mut runtime, selector)?;
            click_default_action_for_node(&dom, source, dispatch)
        }
        Some(RenderClickTarget::Point { node_id, x, y }) => {
            let dispatch = dispatch_pointer_click_node(&mut dom, &mut runtime, node_id, x, y)?;
            click_default_action_for_node(&dom, source, dispatch)
        }
        None => None,
    };
    if click_default_action
        .as_ref()
        .is_none_or(BrowserClickDefaultAction::drains_post_click_timers)
    {
        drain_timer_tasks(&mut dom, &mut runtime);
    }
    persist_runtime_web_storage(
        source,
        &runtime,
        preparation.local_storage,
        preparation.session_storage,
    );
    let script_us = script_start.elapsed().as_micros();

    let page_state = BrowserPageState {
        dom,
        css_text,
        runtime,
        cached_images: preparation.cached_images.to_vec(),
    };
    let profiled = render_page_state_with_timings(
        source,
        options,
        &page_state,
        &[],
        parse_us,
        script_us,
        total_start,
        click_default_action,
    );
    Ok((page_state, profiled))
}

fn render_page_state_profiled(
    source: &str,
    options: BrowserRenderOptions,
    page_state: &BrowserPageState,
    click_default_action: Option<BrowserClickDefaultAction>,
) -> BrowserProfiledRender {
    render_page_state_profiled_with_cached_images(
        source,
        options,
        page_state,
        &[],
        click_default_action,
    )
}

fn render_page_state_profiled_with_cached_images(
    source: &str,
    options: BrowserRenderOptions,
    page_state: &BrowserPageState,
    resource_cached_images: &[DecodedImageEntry],
    click_default_action: Option<BrowserClickDefaultAction>,
) -> BrowserProfiledRender {
    render_page_state_with_timings(
        source,
        options,
        page_state,
        resource_cached_images,
        0,
        0,
        Instant::now(),
        click_default_action,
    )
}

fn render_page_state_with_timings(
    source: &str,
    options: BrowserRenderOptions,
    page_state: &BrowserPageState,
    resource_cached_images: &[DecodedImageEntry],
    parse_us: u128,
    script_us: u128,
    total_start: Instant,
    click_default_action: Option<BrowserClickDefaultAction>,
) -> BrowserProfiledRender {
    let style_start = Instant::now();
    let css_cascade = parse_css(&page_state.css_text);
    let style_us = style_start.elapsed().as_micros();

    let collect_start = Instant::now();
    let title = dom_title(&page_state.dom);
    let links = collect_links(&page_state.dom, source);
    let forms = collect_forms(&page_state.dom, source);
    let mut resources = collect_resources(&page_state.dom, source);
    resources.extend(collect_css_background_image_resources(
        &page_state.dom,
        source,
        &css_cascade,
    ));
    let collect_us = collect_start.elapsed().as_micros();

    let layout_start = Instant::now();
    let mut renderer = FlowRenderer::new(options.width.max(20));
    renderer.seed_decoded_images(&page_state.cached_images);
    renderer.seed_decoded_images(resource_cached_images);
    let mut layout_box_count = 0usize;

    render_children(
        &page_state.dom,
        0,
        source,
        &css_cascade,
        &mut renderer,
        &mut layout_box_count,
        ChildLayout::default(),
    );

    let flow = renderer.finish();
    let paint_command_count = flow.display_list.len();
    let fragment_targets =
        collect_fragment_targets(&page_state.dom, &flow.display_list, &flow.hit_targets);
    let layout_boxes = collect_layout_boxes(
        &page_state.dom,
        &css_cascade,
        &flow.display_list,
        &flow.hit_targets,
    );
    let layout_us = layout_start.elapsed().as_micros();

    let render = BrowserRender {
        source: source.to_owned(),
        title,
        viewport_width: options.width.max(20),
        dom_node_count: page_state.dom.nodes.len(),
        css_rule_count: css_cascade.rules.len(),
        layout_box_count,
        layout_boxes,
        paint_command_count,
        links,
        forms,
        resources,
        fragment_targets,
        decoded_images: flow.decoded_images,
        hit_targets: flow.hit_targets,
        text: flow.text,
        display_list: flow.display_list,
    };
    let timings = BrowserRenderTimings {
        parse_us,
        script_us,
        style_us,
        collect_us,
        layout_us,
        total_us: total_start.elapsed().as_micros(),
    };

    BrowserProfiledRender {
        render,
        timings,
        click_default_action,
    }
}

pub fn layout_tree_render(render: &BrowserRender) -> BrowserLayoutTreeReport {
    BrowserLayoutTreeReport {
        source: render.source.clone(),
        viewport_width: render.viewport_width,
        layout_box_count: render.layout_box_count,
        retained_box_count: render.layout_boxes.len(),
        boxes: render.layout_boxes.clone(),
    }
}

pub fn hit_test_render(render: &BrowserRender, x: usize, y: usize) -> BrowserHitTestReport {
    let hit = render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            let bounds = display_command_bounds(command);
            bounds
                .contains(x, y)
                .then(|| browser_hit_test(command_index, command, bounds))
        });

    BrowserHitTestReport {
        source: render.source.clone(),
        x,
        y,
        hit,
    }
}

fn hit_test_target_node(render: &BrowserRender, x: usize, y: usize) -> Option<usize> {
    hit_test_text_target_node_by_text_row(render, x, y)
        .or_else(|| hit_test_text_target_node(render, x, y))
        .or_else(|| hit_test_nearby_text_target_node(render, x, y))
        .or_else(|| hit_test_visual_target_node(render, x, y))
        .or_else(|| hit_test_nearby_visual_target_node(render, x, y))
}

fn hit_test_target_node_in_viewport(
    render: &BrowserRender,
    viewport: BrowserViewportState,
    x: usize,
    y: usize,
) -> Option<usize> {
    let (viewport, page_x, page_y) = viewport_local_point_to_page(render, viewport, x, y)?;
    hit_test_visual_target_node_for_viewport(render, viewport, page_x, page_y)
        .or_else(|| hit_test_nearby_text_target_node_for_viewport(render, viewport, page_x, page_y))
        .or_else(|| {
            hit_test_nearby_visual_target_node_for_viewport(render, viewport, page_x, page_y)
        })
}

fn viewport_local_point_to_page(
    render: &BrowserRender,
    viewport: BrowserViewportState,
    x: usize,
    y: usize,
) -> Option<(RasterViewport, usize, usize)> {
    let viewport = browser_document_viewport(render, viewport, None).viewport;
    if x >= viewport.width || y >= viewport.height {
        return None;
    }
    let page_x = viewport.x.saturating_add(x);
    let page_y = viewport.y.saturating_add(y);
    Some((raster_viewport_from_state(viewport), page_x, page_y))
}

fn hit_test_text_target_node(render: &BrowserRender, x: usize, y: usize) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if !matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds(command);
            if !bounds.contains(x, y) {
                return None;
            }
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_at_column(x.saturating_sub(bounds.x)))
        })
}

fn hit_test_nearby_text_target_node(render: &BrowserRender, x: usize, y: usize) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if !matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds(command);
            if !bounds_contains_with_tolerance(bounds, x, y, 2, 1) {
                return None;
            }
            let column = clamped_bounds_column(bounds, x);
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_near_column(column, 2))
        })
}

fn hit_test_text_target_node_by_text_row(
    render: &BrowserRender,
    x: usize,
    text_row: usize,
) -> Option<usize> {
    let line = render.text.lines().nth(text_row)?;
    if line.trim().is_empty() {
        return None;
    }
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            let text = readable_display_text(display_command_text(command)?);
            if text.trim().is_empty() {
                return None;
            }
            let start = line.find(&text)?;
            let column_start = line[..start].chars().count();
            let column_end = column_start.saturating_add(text.chars().count());
            if x.saturating_add(2) < column_start || x > column_end.saturating_add(2) {
                return None;
            }
            let column = x.saturating_sub(column_start);
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_near_column(column, 2))
        })
}

fn hit_test_text_target_node_for_viewport(
    render: &BrowserRender,
    viewport: RasterViewport,
    x: usize,
    y: usize,
) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if !matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                display_command_viewport_fixed(render, command_index),
                display_command_viewport_sticky_top(render, command_index),
            );
            if !bounds.contains(x, y) {
                return None;
            }
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_at_column(x.saturating_sub(bounds.x)))
        })
}

fn hit_test_nearby_text_target_node_for_viewport(
    render: &BrowserRender,
    viewport: RasterViewport,
    x: usize,
    y: usize,
) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if !matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                display_command_viewport_fixed(render, command_index),
                display_command_viewport_sticky_top(render, command_index),
            );
            if !bounds_contains_with_tolerance(bounds, x, y, 2, 1) {
                return None;
            }
            let column = clamped_bounds_column(bounds, x);
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_near_column(column, 2))
        })
}

fn hit_test_visual_target_node(render: &BrowserRender, x: usize, y: usize) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            let bounds = display_command_bounds(command);
            if !bounds.contains(x, y) {
                return None;
            }
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_at_column(x.saturating_sub(bounds.x)))
        })
}

fn hit_test_nearby_visual_target_node(render: &BrowserRender, x: usize, y: usize) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds(command);
            if !bounds_contains_with_tolerance(bounds, x, y, 1, 1) {
                return None;
            }
            let column = clamped_bounds_column(bounds, x);
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_near_column(column, 1))
        })
}

fn hit_test_visual_target_node_for_viewport(
    render: &BrowserRender,
    viewport: RasterViewport,
    x: usize,
    y: usize,
) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            let bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                display_command_viewport_fixed(render, command_index),
                display_command_viewport_sticky_top(render, command_index),
            );
            if !bounds.contains(x, y) {
                return None;
            }
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_at_column(x.saturating_sub(bounds.x)))
        })
}

fn hit_test_nearby_visual_target_node_for_viewport(
    render: &BrowserRender,
    viewport: RasterViewport,
    x: usize,
    y: usize,
) -> Option<usize> {
    render
        .display_list
        .iter()
        .enumerate()
        .rev()
        .find_map(|(command_index, command)| {
            if matches!(
                command,
                DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. }
            ) {
                return None;
            }
            let bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                display_command_viewport_fixed(render, command_index),
                display_command_viewport_sticky_top(render, command_index),
            );
            if !bounds_contains_with_tolerance(bounds, x, y, 1, 1) {
                return None;
            }
            let column = clamped_bounds_column(bounds, x);
            render
                .hit_targets
                .get(command_index)
                .and_then(|target| target.target_near_column(column, 1))
        })
}

pub fn layer_tree_render(render: &BrowserRender) -> BrowserLayerTreeReport {
    let mut root_command_indices = Vec::new();
    let mut child_layers = Vec::new();
    let mut content_bounds = None;

    for (command_index, command) in render.display_list.iter().enumerate() {
        let bounds = display_command_bounds(command);
        content_bounds = Some(union_display_bounds(content_bounds, bounds));
        if matches!(
            command,
            DisplayCommand::Image { .. } | DisplayCommand::BackgroundImage { .. }
        ) {
            child_layers.push(browser_layer(
                child_layers.len() + 1,
                Some(0),
                "image",
                "image-replaced-element",
                bounds,
                child_layers.len() + 1,
                vec![command_index],
            ));
        } else {
            root_command_indices.push(command_index);
        }
    }

    let mut root_bounds = content_bounds.unwrap_or(DisplayCommandBounds {
        x: 0,
        y: 0,
        width: render.viewport_width.max(1),
        height: 1,
    });
    root_bounds.x = 0;
    root_bounds.y = 0;
    root_bounds.width = root_bounds.width.max(render.viewport_width.max(1));
    root_bounds.height = root_bounds.height.max(1);

    let mut layers = Vec::with_capacity(child_layers.len() + 1);
    layers.push(browser_layer(
        0,
        None,
        "root",
        "document-root",
        root_bounds,
        0,
        root_command_indices,
    ));
    layers.extend(child_layers);

    BrowserLayerTreeReport {
        source: render.source.clone(),
        viewport_width: render.viewport_width,
        paint_command_count: render.display_list.len(),
        layer_count: layers.len(),
        layers,
    }
}

pub fn browser_layer_metrics(render: &BrowserRender) -> BrowserLayerMetrics {
    let mut root_command_count = 0usize;
    let mut image_layer_count = 0usize;
    let mut max_image_layer_area = 0usize;
    let mut total_image_layer_area = 0usize;
    let mut content_bounds = None;

    for command in &render.display_list {
        let bounds = display_command_bounds(command);
        content_bounds = Some(union_display_bounds(content_bounds, bounds));
        if matches!(
            command,
            DisplayCommand::Image { .. } | DisplayCommand::BackgroundImage { .. }
        ) {
            image_layer_count += 1;
            let area = bounds.width.saturating_mul(bounds.height);
            max_image_layer_area = max_image_layer_area.max(area);
            total_image_layer_area = total_image_layer_area.saturating_add(area);
        } else {
            root_command_count += 1;
        }
    }

    let mut root_bounds = content_bounds.unwrap_or(DisplayCommandBounds {
        x: 0,
        y: 0,
        width: render.viewport_width.max(1),
        height: 1,
    });
    root_bounds.x = 0;
    root_bounds.y = 0;
    root_bounds.width = root_bounds.width.max(render.viewport_width.max(1));
    root_bounds.height = root_bounds.height.max(1);

    let root_layer_area = root_bounds.width.saturating_mul(root_bounds.height);
    BrowserLayerMetrics {
        layer_count: 1 + image_layer_count,
        root_command_count,
        image_layer_count,
        root_layer_width: root_bounds.width,
        root_layer_height: root_bounds.height,
        max_layer_area: root_layer_area.max(max_image_layer_area),
        total_layer_area: root_layer_area.saturating_add(total_image_layer_area),
    }
}

fn browser_layer(
    id: usize,
    parent: Option<usize>,
    kind: &str,
    reason: &str,
    bounds: DisplayCommandBounds,
    paint_order: usize,
    command_indices: Vec<usize>,
) -> BrowserLayer {
    BrowserLayer {
        id,
        parent,
        kind: kind.to_owned(),
        reason: reason.to_owned(),
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
        paint_order,
        command_indices,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisplayCommandBounds {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

impl DisplayCommandBounds {
    fn contains(self, x: usize, y: usize) -> bool {
        self.width > 0
            && self.height > 0
            && x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.width)
            && y < self.y.saturating_add(self.height)
    }
}

fn bounds_contains_with_tolerance(
    bounds: DisplayCommandBounds,
    x: usize,
    y: usize,
    x_tolerance: usize,
    y_tolerance: usize,
) -> bool {
    bounds.width > 0
        && bounds.height > 0
        && x.saturating_add(x_tolerance) >= bounds.x
        && y.saturating_add(y_tolerance) >= bounds.y
        && x < bounds
            .x
            .saturating_add(bounds.width)
            .saturating_add(x_tolerance)
        && y < bounds
            .y
            .saturating_add(bounds.height)
            .saturating_add(y_tolerance)
}

fn clamped_bounds_column(bounds: DisplayCommandBounds, x: usize) -> usize {
    if x <= bounds.x {
        return 0;
    }
    let column = x.saturating_sub(bounds.x);
    column.min(bounds.width.saturating_sub(1))
}

#[derive(Debug, Clone, Copy)]
struct RasterViewport {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    active: bool,
}

impl RasterViewport {
    fn end_x(self) -> usize {
        self.x.saturating_add(self.width)
    }

    fn end_y(self) -> usize {
        self.y.saturating_add(self.height)
    }
}

fn union_display_bounds(
    current: Option<DisplayCommandBounds>,
    next: DisplayCommandBounds,
) -> DisplayCommandBounds {
    let Some(current) = current else {
        return next;
    };
    let min_x = current.x.min(next.x);
    let min_y = current.y.min(next.y);
    let max_x = current
        .x
        .saturating_add(current.width)
        .max(next.x.saturating_add(next.width));
    let max_y = current
        .y
        .saturating_add(current.height)
        .max(next.y.saturating_add(next.height));
    DisplayCommandBounds {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x),
        height: max_y.saturating_sub(min_y),
    }
}

fn display_command_source_bounds_for_viewport(
    render: &BrowserRender,
    command_index: usize,
    fallback: DisplayCommandBounds,
    viewport: RasterViewport,
    viewport_fixed: bool,
    viewport_sticky_top: Option<usize>,
) -> DisplayCommandBounds {
    let source = render
        .hit_targets
        .get(command_index)
        .and_then(|target| target.source_bounds)
        .unwrap_or(DisplaySourceBounds {
            x: fallback.x,
            y: fallback.y,
            width: fallback.width,
            height: fallback.height,
        });
    let (x, y) = display_command_origin_for_viewport(
        source.x,
        source.y,
        viewport,
        viewport_fixed,
        viewport_sticky_top,
    );
    DisplayCommandBounds {
        x,
        y,
        width: source.width,
        height: source.height,
    }
}

fn intersect_display_bounds(
    left: DisplayCommandBounds,
    right: DisplayCommandBounds,
) -> Option<DisplayCommandBounds> {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let end_x = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let end_y = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    (end_x > x && end_y > y).then_some(DisplayCommandBounds {
        x,
        y,
        width: end_x.saturating_sub(x),
        height: end_y.saturating_sub(y),
    })
}

fn display_command_bounds(command: &DisplayCommand) -> DisplayCommandBounds {
    match command {
        DisplayCommand::Text { x, y, text } | DisplayCommand::StyledText { x, y, text, .. } => {
            DisplayCommandBounds {
                x: *x,
                y: *y,
                width: text.chars().count(),
                height: 1,
            }
        }
        DisplayCommand::Rect {
            x,
            y,
            width,
            height,
            ..
        }
        | DisplayCommand::ColorRect {
            x,
            y,
            width,
            height,
            ..
        }
        | DisplayCommand::Image {
            x,
            y,
            width,
            height,
            ..
        }
        | DisplayCommand::BackgroundImage {
            x,
            y,
            width,
            height,
            ..
        } => DisplayCommandBounds {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
    }
}

#[derive(Debug, Clone)]
struct LayoutBoxAccumulator {
    node_id: usize,
    tag: String,
    kind: String,
    bounds: DisplayCommandBounds,
    command_indices: Vec<usize>,
}

fn collect_layout_boxes(
    dom: &Dom,
    css_cascade: &CssCascade,
    display_list: &[DisplayCommand],
    hit_targets: &[DisplayHitTarget],
) -> Vec<BrowserLayoutBox> {
    let mut accumulators = Vec::<LayoutBoxAccumulator>::new();
    let mut node_to_index = HashMap::<usize, usize>::new();

    for (command_index, command) in display_list.iter().enumerate() {
        let Some(hit_target) = hit_targets.get(command_index) else {
            continue;
        };
        let command_bounds = display_command_bounds(command);
        for (node_id, bounds) in display_command_node_bounds(command, hit_target, command_bounds) {
            let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind)
            else {
                continue;
            };
            let style = computed_style(dom, node_id, element, css_cascade);
            if style.display == Display::Contents {
                continue;
            }
            let entry_index = *node_to_index.entry(node_id).or_insert_with(|| {
                let kind = layout_box_kind(dom, node_id, element, style);
                let index = accumulators.len();
                accumulators.push(LayoutBoxAccumulator {
                    node_id,
                    tag: element.tag.clone(),
                    kind,
                    bounds,
                    command_indices: Vec::new(),
                });
                index
            });
            let accumulator = &mut accumulators[entry_index];
            accumulator.bounds = union_display_bounds(Some(accumulator.bounds), bounds);
            if accumulator.command_indices.last() != Some(&command_index) {
                accumulator.command_indices.push(command_index);
            }
        }
    }

    let mut boxes: Vec<BrowserLayoutBox> = accumulators
        .into_iter()
        .enumerate()
        .map(|(id, accumulator)| BrowserLayoutBox {
            id,
            parent: None,
            node_id: accumulator.node_id,
            tag: accumulator.tag,
            kind: accumulator.kind,
            x: accumulator.bounds.x,
            y: accumulator.bounds.y,
            width: accumulator.bounds.width,
            height: accumulator.bounds.height,
            children: Vec::new(),
            command_indices: accumulator.command_indices,
        })
        .collect();

    let node_to_box: HashMap<usize, usize> = boxes
        .iter()
        .map(|layout_box| (layout_box.node_id, layout_box.id))
        .collect();
    let parents: Vec<Option<usize>> = boxes
        .iter()
        .map(|layout_box| nearest_retained_layout_parent(dom, layout_box.node_id, &node_to_box))
        .collect();
    for (box_id, parent) in parents.into_iter().enumerate() {
        boxes[box_id].parent = parent;
        if let Some(parent_id) = parent {
            boxes[parent_id].children.push(box_id);
        }
    }

    boxes
}

fn display_command_node_bounds(
    command: &DisplayCommand,
    hit_target: &DisplayHitTarget,
    command_bounds: DisplayCommandBounds,
) -> Vec<(usize, DisplayCommandBounds)> {
    if !hit_target.text_runs.is_empty() {
        return hit_target
            .text_runs
            .iter()
            .filter_map(|run| {
                let node_id = run.target_node?;
                (run.width > 0).then_some((
                    node_id,
                    DisplayCommandBounds {
                        x: command_bounds.x.saturating_add(run.start),
                        y: command_bounds.y,
                        width: run.width,
                        height: command_bounds.height.max(1),
                    },
                ))
            })
            .collect();
    }

    let Some(node_id) = hit_target.target_node else {
        return Vec::new();
    };
    if command_bounds.width == 0 || command_bounds.height == 0 {
        return Vec::new();
    }
    match command {
        DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. } => Vec::new(),
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => vec![(node_id, command_bounds)],
    }
}

fn layout_box_kind(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    style: ComputedStyle,
) -> String {
    if element.tag == "img" || is_replaced_media_element(&element.tag) {
        "replaced".to_owned()
    } else if form_control_render_text(dom, node_id, element).is_some() {
        "form-control".to_owned()
    } else {
        match style.display {
            Display::Block => "block".to_owned(),
            Display::Flex => "flex".to_owned(),
            Display::FlowRoot => "flow-root".to_owned(),
            Display::Grid => "grid".to_owned(),
            Display::InlineBlock => "inline-block".to_owned(),
            Display::InlineFlex => "inline-flex".to_owned(),
            Display::InlineGrid => "inline-grid".to_owned(),
            Display::ListItem => "list-item".to_owned(),
            Display::Table => "table".to_owned(),
            Display::TableRow => "table-row".to_owned(),
            Display::TableCell => "table-cell".to_owned(),
            Display::Contents => "contents".to_owned(),
            Display::Inline => "inline".to_owned(),
            Display::None => "none".to_owned(),
        }
    }
}

fn nearest_retained_layout_parent(
    dom: &Dom,
    node_id: usize,
    node_to_box: &HashMap<usize, usize>,
) -> Option<usize> {
    let mut current = dom.nodes.get(node_id).and_then(|node| node.parent);
    while let Some(parent_node_id) = current {
        if let Some(parent_box_id) = node_to_box.get(&parent_node_id) {
            return Some(*parent_box_id);
        }
        current = dom.nodes.get(parent_node_id).and_then(|node| node.parent);
    }
    None
}

fn raster_full_grid(render: &BrowserRender) -> (usize, usize) {
    let mut max_column = render.viewport_width.max(1);
    let mut max_row = 1usize;
    for (command_index, command) in render.display_list.iter().enumerate() {
        if !display_command_affects_scroll_extent(render, command_index) {
            continue;
        }
        let bounds = display_command_bounds(command);
        max_column = max_column.max(bounds.x.saturating_add(bounds.width));
        max_row = max_row.max(bounds.y.saturating_add(bounds.height));
    }
    for layout_box in &render.layout_boxes {
        if !layout_box_affects_scroll_extent(render, layout_box) {
            continue;
        }
        let bounds = layout_box_bounds(layout_box);
        max_column = max_column.max(bounds.x.saturating_add(bounds.width));
        max_row = max_row.max(bounds.y.saturating_add(bounds.height));
    }
    (max_column, max_row)
}

fn layout_box_affects_scroll_extent(render: &BrowserRender, layout_box: &BrowserLayoutBox) -> bool {
    !layout_box.command_indices.is_empty()
        && layout_box
            .command_indices
            .iter()
            .any(|command_index| display_command_affects_scroll_extent(render, *command_index))
}

fn display_command_affects_scroll_extent(render: &BrowserRender, command_index: usize) -> bool {
    !display_command_viewport_fixed(render, command_index)
}

fn layout_box_bounds(layout_box: &BrowserLayoutBox) -> DisplayCommandBounds {
    DisplayCommandBounds {
        x: layout_box.x,
        y: layout_box.y,
        width: layout_box.width,
        height: layout_box.height,
    }
}

fn effective_raster_viewport(
    render: &BrowserRender,
    options: BrowserRasterOptions,
) -> RasterViewport {
    let (full_width, full_height) = raster_full_grid(render);
    let requested_x = options.viewport_x.unwrap_or(0);
    let requested_y = options.viewport_y.unwrap_or(0);
    let active = options.viewport_x.is_some()
        || options.viewport_y.is_some()
        || options.viewport_width.is_some()
        || options.viewport_height.is_some();
    let width = options
        .viewport_width
        .unwrap_or_else(|| full_width.saturating_sub(requested_x).max(1))
        .max(1);
    let height = options
        .viewport_height
        .unwrap_or_else(|| full_height.saturating_sub(requested_y).max(1))
        .max(1);
    let x = requested_x.min(full_width.saturating_sub(width));
    let y = requested_y.min(full_height.saturating_sub(height));
    RasterViewport {
        x,
        y,
        width,
        height,
        active,
    }
}

pub fn browser_fixture_raster_options(
    fixture: &BrowserFixture,
    mut options: BrowserRasterOptions,
) -> BrowserRasterOptions {
    if let Some(viewport_x) = fixture.raster_viewport_x {
        options.viewport_x = Some(viewport_x);
    }
    if let Some(viewport_y) = fixture.raster_viewport_y {
        options.viewport_y = Some(viewport_y);
    }
    if let Some(viewport_width) = fixture.raster_viewport_width {
        options.viewport_width = Some(viewport_width);
    }
    if let Some(viewport_height) = fixture.raster_viewport_height {
        options.viewport_height = Some(viewport_height);
    }
    options
}

fn intersect_display_bounds_with_viewport(
    bounds: DisplayCommandBounds,
    viewport: RasterViewport,
) -> Option<DisplayCommandBounds> {
    let x = bounds.x.max(viewport.x);
    let y = bounds.y.max(viewport.y);
    let end_x = bounds.x.saturating_add(bounds.width).min(viewport.end_x());
    let end_y = bounds.y.saturating_add(bounds.height).min(viewport.end_y());
    (x < end_x && y < end_y).then_some(DisplayCommandBounds {
        x,
        y,
        width: end_x.saturating_sub(x),
        height: end_y.saturating_sub(y),
    })
}

fn display_command_viewport_fixed(render: &BrowserRender, command_index: usize) -> bool {
    render
        .hit_targets
        .get(command_index)
        .is_some_and(|target| target.viewport_fixed)
}

fn display_command_viewport_sticky_top(
    render: &BrowserRender,
    command_index: usize,
) -> Option<usize> {
    render
        .hit_targets
        .get(command_index)
        .and_then(|target| target.viewport_sticky_top)
}

fn display_command_origin_for_viewport(
    x: usize,
    y: usize,
    viewport: RasterViewport,
    viewport_fixed: bool,
    viewport_sticky_top: Option<usize>,
) -> (usize, usize) {
    if viewport_fixed {
        (x.saturating_add(viewport.x), y.saturating_add(viewport.y))
    } else if let Some(top) = viewport_sticky_top {
        (x, y.max(viewport.y.saturating_add(top)))
    } else {
        (x, y)
    }
}

fn display_command_bounds_for_viewport(
    command: &DisplayCommand,
    viewport: RasterViewport,
    viewport_fixed: bool,
    viewport_sticky_top: Option<usize>,
) -> DisplayCommandBounds {
    let mut bounds = display_command_bounds(command);
    (bounds.x, bounds.y) = display_command_origin_for_viewport(
        bounds.x,
        bounds.y,
        viewport,
        viewport_fixed,
        viewport_sticky_top,
    );
    bounds
}

fn raster_visibility_counts(render: &BrowserRender, viewport: RasterViewport) -> (usize, usize) {
    let visible = render
        .display_list
        .iter()
        .enumerate()
        .filter(|(command_index, command)| {
            let viewport_fixed = display_command_viewport_fixed(render, *command_index);
            let viewport_sticky_top = display_command_viewport_sticky_top(render, *command_index);
            let command_bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                viewport_fixed,
                viewport_sticky_top,
            );
            intersect_display_bounds_with_viewport(command_bounds, viewport).is_some()
        })
        .count();
    (visible, render.display_list.len().saturating_sub(visible))
}

fn layout_box_visibility(
    render: &BrowserRender,
    viewport: RasterViewport,
) -> (usize, usize, Vec<BrowserVisibleLayoutBox>) {
    let visible_layout_boxes = render
        .layout_boxes
        .iter()
        .filter_map(|layout_box| {
            let viewport_bounds = layout_box_viewport_bounds(render, layout_box, viewport);
            let visible_bounds = intersect_display_bounds_with_viewport(viewport_bounds, viewport)?;
            Some(BrowserVisibleLayoutBox {
                id: layout_box.id,
                parent: layout_box.parent,
                node_id: layout_box.node_id,
                tag: layout_box.tag.clone(),
                kind: layout_box.kind.clone(),
                x: layout_box.x,
                y: layout_box.y,
                width: layout_box.width,
                height: layout_box.height,
                visible_x: visible_bounds.x.saturating_sub(viewport.x),
                visible_y: visible_bounds.y.saturating_sub(viewport.y),
                visible_width: visible_bounds.width,
                visible_height: visible_bounds.height,
            })
        })
        .collect::<Vec<_>>();
    (
        visible_layout_boxes.len(),
        render
            .layout_boxes
            .len()
            .saturating_sub(visible_layout_boxes.len()),
        visible_layout_boxes,
    )
}

fn layout_box_viewport_bounds(
    render: &BrowserRender,
    layout_box: &BrowserLayoutBox,
    viewport: RasterViewport,
) -> DisplayCommandBounds {
    layout_box
        .command_indices
        .iter()
        .filter_map(|command_index| {
            let command = render.display_list.get(*command_index)?;
            Some(display_command_bounds_for_viewport(
                command,
                viewport,
                display_command_viewport_fixed(render, *command_index),
                display_command_viewport_sticky_top(render, *command_index),
            ))
        })
        .reduce(|current, next| union_display_bounds(Some(current), next))
        .unwrap_or_else(|| layout_box_bounds(layout_box))
}

pub fn browser_document_viewport(
    render: &BrowserRender,
    requested: BrowserViewportState,
    previous: Option<BrowserViewportState>,
) -> BrowserDocumentViewportReport {
    let (document_width, document_height) = raster_full_grid(render);
    let requested = normalize_browser_viewport_state(requested);
    let viewport_state = clamp_browser_viewport_state(document_width, document_height, requested);
    let previous = previous.map(|state| {
        clamp_browser_viewport_state(
            document_width,
            document_height,
            normalize_browser_viewport_state(state),
        )
    });
    let viewport = raster_viewport_from_state(viewport_state);
    let (visible_command_count, culled_command_count) = raster_visibility_counts(render, viewport);
    let (visible_layout_box_count, culled_layout_box_count, visible_layout_boxes) =
        layout_box_visibility(render, viewport);
    let (invalidated_regions, full_repaint) =
        browser_viewport_invalidated_regions(previous, viewport_state);
    let viewport_area = viewport_state.width.saturating_mul(viewport_state.height);
    let invalidated_area = invalidated_regions
        .iter()
        .map(|region| region.width.saturating_mul(region.height))
        .sum::<usize>()
        .min(viewport_area);
    let previous_state = previous.unwrap_or(viewport_state);

    BrowserDocumentViewportReport {
        source: render.source.clone(),
        title: render.title.clone(),
        document_width,
        document_height,
        requested,
        previous,
        viewport: viewport_state,
        max_scroll_x: document_width.saturating_sub(viewport_state.width),
        max_scroll_y: document_height.saturating_sub(viewport_state.height),
        scroll_delta_x: browser_viewport_signed_delta(viewport_state.x, previous_state.x),
        scroll_delta_y: browser_viewport_signed_delta(viewport_state.y, previous_state.y),
        display_command_count: render.display_list.len(),
        visible_command_count,
        culled_command_count,
        layout_box_count: render.layout_boxes.len(),
        visible_layout_box_count,
        culled_layout_box_count,
        visible_layout_boxes,
        invalidated_regions,
        invalidated_area,
        reused_area: viewport_area.saturating_sub(invalidated_area),
        full_repaint,
    }
}

pub fn browser_document_viewport_after_scroll(
    render: &BrowserRender,
    current: BrowserViewportState,
    delta_x: isize,
    delta_y: isize,
) -> BrowserDocumentViewportReport {
    let current = browser_document_viewport(render, current, None).viewport;
    let requested = BrowserViewportState {
        x: apply_signed_scroll_delta(current.x, delta_x),
        y: apply_signed_scroll_delta(current.y, delta_y),
        ..current
    };
    browser_document_viewport(render, requested, Some(current))
}

pub fn browser_document_viewport_after_page_scroll(
    render: &BrowserRender,
    current: BrowserViewportState,
    pages_x: isize,
    pages_y: isize,
) -> BrowserDocumentViewportReport {
    let current = browser_document_viewport(render, current, None).viewport;
    let delta_x = signed_scroll_unit_delta(pages_x, viewport_page_scroll_increment(current.width));
    let delta_y = signed_scroll_unit_delta(pages_y, viewport_page_scroll_increment(current.height));
    let requested = BrowserViewportState {
        x: apply_signed_scroll_delta(current.x, delta_x),
        y: apply_signed_scroll_delta(current.y, delta_y),
        ..current
    };
    browser_document_viewport(render, requested, Some(current))
}

pub fn browser_viewport_frame(
    render: &BrowserRender,
    requested: BrowserViewportState,
    previous: Option<BrowserViewportState>,
    mut raster_options: BrowserRasterOptions,
) -> Result<BrowserViewportFrame> {
    let viewport = browser_document_viewport(render, requested, previous);
    raster_options.viewport_x = Some(viewport.viewport.x);
    raster_options.viewport_y = Some(viewport.viewport.y);
    raster_options.viewport_width = Some(viewport.viewport.width);
    raster_options.viewport_height = Some(viewport.viewport.height);

    let raster = rasterize_render_rgba(render, raster_options)?;
    let frame = rgba_raster_report(render, &raster, raster_options);
    let dirty_pixel_regions =
        browser_viewport_frame_dirty_regions(render, &viewport, &frame, raster_options);
    let dirty_pixel_area = dirty_pixel_regions
        .iter()
        .map(|region| region.width.saturating_mul(region.height))
        .sum::<usize>()
        .min(frame.width.saturating_mul(frame.height));

    let report = BrowserViewportFrameReport {
        frame_width: frame.width,
        frame_height: frame.height,
        cell_width: frame.cell_width,
        cell_height: frame.cell_height,
        padding_x: raster_options.padding_x,
        padding_y: raster_options.padding_y,
        bytes_per_pixel: frame.bytes_per_pixel,
        pixel_hash: frame.pixel_hash.clone(),
        non_background_pixels: frame.non_background_pixels,
        artifact_format: frame.artifact_format.clone(),
        viewport,
        frame,
        dirty_pixel_regions,
        dirty_pixel_area,
    };

    Ok(BrowserViewportFrame { report, raster })
}

fn normalize_browser_viewport_state(state: BrowserViewportState) -> BrowserViewportState {
    BrowserViewportState {
        width: state.width.max(1),
        height: state.height.max(1),
        ..state
    }
}

fn clamp_browser_viewport_state(
    document_width: usize,
    document_height: usize,
    state: BrowserViewportState,
) -> BrowserViewportState {
    let max_scroll_x = document_width.saturating_sub(state.width);
    let max_scroll_y = document_height.saturating_sub(state.height);
    BrowserViewportState {
        x: state.x.min(max_scroll_x),
        y: state.y.min(max_scroll_y),
        ..state
    }
}

fn raster_viewport_from_state(state: BrowserViewportState) -> RasterViewport {
    RasterViewport {
        x: state.x,
        y: state.y,
        width: state.width,
        height: state.height,
        active: true,
    }
}

fn apply_signed_scroll_delta(value: usize, delta: isize) -> usize {
    if delta >= 0 {
        value.saturating_add(delta as usize)
    } else {
        value.saturating_sub(delta.saturating_abs() as usize)
    }
}

fn viewport_page_scroll_increment(size: usize) -> usize {
    size.saturating_sub(1).max(1)
}

fn signed_scroll_unit_delta(units: isize, increment: usize) -> isize {
    let magnitude = (units.saturating_abs() as usize)
        .saturating_mul(increment)
        .min(isize::MAX as usize) as isize;
    if units < 0 { -magnitude } else { magnitude }
}

fn browser_viewport_invalidated_regions(
    previous: Option<BrowserViewportState>,
    current: BrowserViewportState,
) -> (Vec<BrowserViewportRect>, bool) {
    let full_viewport = BrowserViewportRect {
        x: 0,
        y: 0,
        width: current.width,
        height: current.height,
    };
    let Some(previous) = previous else {
        return (vec![full_viewport], true);
    };
    if previous.width != current.width || previous.height != current.height {
        return (vec![full_viewport], true);
    }

    let dirty_width = current.x.abs_diff(previous.x).min(current.width);
    let dirty_height = current.y.abs_diff(previous.y).min(current.height);
    if dirty_width == 0 && dirty_height == 0 {
        return (Vec::new(), false);
    }
    if dirty_width == current.width || dirty_height == current.height {
        return (vec![full_viewport], true);
    }

    let mut regions = Vec::new();
    if dirty_width > 0 {
        let x = if current.x > previous.x {
            current.width.saturating_sub(dirty_width)
        } else {
            0
        };
        regions.push(BrowserViewportRect {
            x,
            y: 0,
            width: dirty_width,
            height: current.height,
        });
    }
    if dirty_height > 0 {
        let y = if current.y > previous.y {
            current.height.saturating_sub(dirty_height)
        } else {
            0
        };
        let x = if current.x < previous.x {
            dirty_width
        } else {
            0
        };
        let width = current.width.saturating_sub(dirty_width);
        if width > 0 {
            regions.push(BrowserViewportRect {
                x,
                y,
                width,
                height: dirty_height,
            });
        }
    }

    (regions, false)
}

fn append_viewport_positioned_invalidated_regions(
    render: &BrowserRender,
    previous: BrowserViewportState,
    current: BrowserViewportState,
    regions: &mut Vec<BrowserViewportRect>,
) {
    let previous_viewport = raster_viewport_from_state(previous);
    let current_viewport = raster_viewport_from_state(current);
    for (command_index, command) in render.display_list.iter().enumerate() {
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        if !viewport_fixed && viewport_sticky_top.is_none() {
            continue;
        }

        append_display_command_viewport_dirty_region(
            command,
            current_viewport,
            viewport_fixed,
            viewport_sticky_top,
            regions,
        );
        append_display_command_viewport_dirty_region(
            command,
            previous_viewport,
            viewport_fixed,
            viewport_sticky_top,
            regions,
        );
    }
}

fn append_display_command_viewport_dirty_region(
    command: &DisplayCommand,
    viewport: RasterViewport,
    viewport_fixed: bool,
    viewport_sticky_top: Option<usize>,
    regions: &mut Vec<BrowserViewportRect>,
) {
    let command_bounds =
        display_command_bounds_for_viewport(command, viewport, viewport_fixed, viewport_sticky_top);
    let Some(visible_bounds) = intersect_display_bounds_with_viewport(command_bounds, viewport)
    else {
        return;
    };
    append_non_overlapping_viewport_rect(
        regions,
        BrowserViewportRect {
            x: visible_bounds.x.saturating_sub(viewport.x),
            y: visible_bounds.y.saturating_sub(viewport.y),
            width: visible_bounds.width,
            height: visible_bounds.height,
        },
    );
}

fn append_non_overlapping_viewport_rect(
    regions: &mut Vec<BrowserViewportRect>,
    rect: BrowserViewportRect,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let mut fragments = vec![rect];
    for existing in regions.iter().copied() {
        fragments = fragments
            .into_iter()
            .flat_map(|fragment| subtract_viewport_rect(fragment, existing))
            .collect();
        if fragments.is_empty() {
            return;
        }
    }
    regions.extend(fragments);
}

fn subtract_viewport_rect(
    rect: BrowserViewportRect,
    covered: BrowserViewportRect,
) -> Vec<BrowserViewportRect> {
    let rect_end_x = rect.x.saturating_add(rect.width);
    let rect_end_y = rect.y.saturating_add(rect.height);
    let covered_end_x = covered.x.saturating_add(covered.width);
    let covered_end_y = covered.y.saturating_add(covered.height);
    let overlap_x = rect.x.max(covered.x);
    let overlap_y = rect.y.max(covered.y);
    let overlap_end_x = rect_end_x.min(covered_end_x);
    let overlap_end_y = rect_end_y.min(covered_end_y);
    if overlap_x >= overlap_end_x || overlap_y >= overlap_end_y {
        return vec![rect];
    }

    let mut fragments = Vec::new();
    if rect.y < overlap_y {
        fragments.push(BrowserViewportRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: overlap_y.saturating_sub(rect.y),
        });
    }
    if overlap_end_y < rect_end_y {
        fragments.push(BrowserViewportRect {
            x: rect.x,
            y: overlap_end_y,
            width: rect.width,
            height: rect_end_y.saturating_sub(overlap_end_y),
        });
    }
    if rect.x < overlap_x {
        fragments.push(BrowserViewportRect {
            x: rect.x,
            y: overlap_y,
            width: overlap_x.saturating_sub(rect.x),
            height: overlap_end_y.saturating_sub(overlap_y),
        });
    }
    if overlap_end_x < rect_end_x {
        fragments.push(BrowserViewportRect {
            x: overlap_end_x,
            y: overlap_y,
            width: rect_end_x.saturating_sub(overlap_end_x),
            height: overlap_end_y.saturating_sub(overlap_y),
        });
    }
    fragments
}

fn browser_viewport_frame_dirty_regions(
    render: &BrowserRender,
    viewport: &BrowserDocumentViewportReport,
    frame: &BrowserRgbaRasterReport,
    options: BrowserRasterOptions,
) -> Vec<BrowserViewportFrameDirtyRect> {
    if viewport.full_repaint {
        return vec![BrowserViewportFrameDirtyRect {
            x: 0,
            y: 0,
            width: frame.width,
            height: frame.height,
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: viewport.viewport.width,
            viewport_height: viewport.viewport.height,
        }];
    }

    let mut invalidated_regions = viewport.invalidated_regions.clone();
    if let Some(previous_state) = viewport.previous
        && (previous_state.x != viewport.viewport.x || previous_state.y != viewport.viewport.y)
    {
        append_viewport_positioned_invalidated_regions(
            render,
            previous_state,
            viewport.viewport,
            &mut invalidated_regions,
        );
    }

    invalidated_regions
        .iter()
        .filter(|region| region.width > 0 && region.height > 0)
        .map(|region| BrowserViewportFrameDirtyRect {
            x: options
                .padding_x
                .saturating_add(region.x.saturating_mul(options.cell_width)),
            y: options
                .padding_y
                .saturating_add(region.y.saturating_mul(options.cell_height)),
            width: region.width.saturating_mul(options.cell_width),
            height: region.height.saturating_mul(options.cell_height),
            viewport_x: region.x,
            viewport_y: region.y,
            viewport_width: region.width,
            viewport_height: region.height,
        })
        .collect()
}

fn browser_viewport_signed_delta(current: usize, previous: usize) -> isize {
    if current >= previous {
        current.saturating_sub(previous).min(isize::MAX as usize) as isize
    } else {
        -(previous.saturating_sub(current).min(isize::MAX as usize) as isize)
    }
}

fn append_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    png.extend_from_slice(&png_crc32(kind, data).to_be_bytes());
}

fn png_crc32(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in kind.iter().chain(data.iter()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn browser_hit_test(
    command_index: usize,
    command: &DisplayCommand,
    bounds: DisplayCommandBounds,
) -> BrowserHitTest {
    match command {
        DisplayCommand::Text { text, .. } => BrowserHitTest {
            command_index,
            kind: "text".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: Some(text.clone()),
            alt: None,
            url: None,
            shade: None,
        },
        DisplayCommand::StyledText { text, shade, .. } => BrowserHitTest {
            command_index,
            kind: "styled_text".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: Some(text.clone()),
            alt: None,
            url: None,
            shade: Some(*shade),
        },
        DisplayCommand::Rect { shade, .. } => BrowserHitTest {
            command_index,
            kind: "rect".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: None,
            alt: None,
            url: None,
            shade: Some(*shade),
        },
        DisplayCommand::ColorRect { shade, .. } => BrowserHitTest {
            command_index,
            kind: "rect".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: None,
            alt: None,
            url: None,
            shade: Some(*shade),
        },
        DisplayCommand::Image {
            alt, url, shade, ..
        } => BrowserHitTest {
            command_index,
            kind: "image".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: None,
            alt: alt.clone(),
            url: url.clone(),
            shade: Some(*shade),
        },
        DisplayCommand::BackgroundImage { url, shade, .. } => BrowserHitTest {
            command_index,
            kind: "image".to_owned(),
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            text: None,
            alt: None,
            url: url.clone(),
            shade: Some(*shade),
        },
    }
}

pub fn rasterize_render(
    render: &BrowserRender,
    options: BrowserRasterOptions,
) -> Result<BrowserRaster> {
    ensure!(
        options.cell_width > GLYPH_WIDTH,
        "cell_width must be at least {}",
        GLYPH_WIDTH + 1
    );
    ensure!(
        options.cell_height > GLYPH_HEIGHT,
        "cell_height must be at least {}",
        GLYPH_HEIGHT + 1
    );
    if let Some(viewport_width) = options.viewport_width {
        ensure!(
            viewport_width > 0,
            "viewport_width must be greater than zero"
        );
    }
    if let Some(viewport_height) = options.viewport_height {
        ensure!(
            viewport_height > 0,
            "viewport_height must be greater than zero"
        );
    }

    let viewport = effective_raster_viewport(render, options);
    let text_width = viewport
        .width
        .checked_mul(options.cell_width)
        .context("raster width overflow")?;
    let text_height = viewport
        .height
        .checked_mul(options.cell_height)
        .context("raster height overflow")?;
    let width = text_width
        .checked_add(options.padding_x.saturating_mul(2))
        .context("raster padded width overflow")?;
    let height = text_height
        .checked_add(options.padding_y.saturating_mul(2))
        .context("raster padded height overflow")?;
    let pixel_count = width.checked_mul(height).context("raster pixel overflow")?;
    ensure!(
        pixel_count <= options.max_pixels,
        "raster would allocate {pixel_count} pixels, over max {}",
        options.max_pixels
    );

    let background = 255u8;
    let mut pixels = vec![background; pixel_count];
    for (command_index, command) in render.display_list.iter().enumerate() {
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        let command_bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        let Some(visible_bounds) = intersect_display_bounds_with_viewport(command_bounds, viewport)
        else {
            continue;
        };
        match command {
            DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. } => {
                draw_raster_text_command(
                    &mut pixels,
                    width,
                    command,
                    render,
                    command_index,
                    viewport,
                    options,
                );
            }
            DisplayCommand::Rect {
                x: _,
                y: _,
                width: _,
                height: _,
                shade,
            }
            | DisplayCommand::ColorRect {
                x: _,
                y: _,
                width: _,
                height: _,
                shade,
                ..
            } => {
                let rect_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let rect_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let rect_width = visible_bounds.width.saturating_mul(options.cell_width);
                let rect_height = visible_bounds.height.saturating_mul(options.cell_height);
                fill_raster_rect(
                    &mut pixels,
                    width,
                    rect_x,
                    rect_y,
                    rect_width,
                    rect_height,
                    *shade,
                );
            }
            DisplayCommand::Image {
                x,
                y,
                width: image_width,
                height,
                shade,
                url,
                ..
            } => {
                let source_bounds = display_command_source_bounds_for_viewport(
                    render,
                    command_index,
                    DisplayCommandBounds {
                        x: *x,
                        y: *y,
                        width: *image_width,
                        height: *height,
                    },
                    viewport,
                    viewport_fixed,
                    viewport_sticky_top,
                );
                let image_width_cells = source_bounds.width;
                let image_height_cells = source_bounds.height;
                let image_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let image_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let clipped_image_width = visible_bounds.width.saturating_mul(options.cell_width);
                let clipped_image_height =
                    visible_bounds.height.saturating_mul(options.cell_height);
                let image_width = image_width_cells.saturating_mul(options.cell_width);
                let image_height = image_height_cells.saturating_mul(options.cell_height);
                let source_offset_x = visible_bounds
                    .x
                    .saturating_sub(source_bounds.x)
                    .saturating_mul(options.cell_width);
                let source_offset_y = visible_bounds
                    .y
                    .saturating_sub(source_bounds.y)
                    .saturating_mul(options.cell_height);
                if let Some(decoded) = url.as_deref().and_then(|url| render.decoded_image(url)) {
                    draw_decoded_image_region(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        image_width,
                        image_height,
                        decoded,
                    );
                } else if let Some(decoded) = url
                    .as_deref()
                    .and_then(|url| decode_image_reference(&render.source, url))
                {
                    draw_decoded_image_region(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        image_width,
                        image_height,
                        &decoded,
                    );
                } else {
                    fill_raster_rect(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        *shade,
                    );
                    let original_right = source_bounds.x.saturating_add(source_bounds.width);
                    let original_bottom = source_bounds.y.saturating_add(source_bounds.height);
                    if visible_bounds.y == source_bounds.y {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y,
                            clipped_image_width,
                            1,
                            96,
                        );
                    }
                    if visible_bounds.y.saturating_add(visible_bounds.height) == original_bottom {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y.saturating_add(clipped_image_height.saturating_sub(1)),
                            clipped_image_width,
                            1,
                            96,
                        );
                    }
                    if visible_bounds.x == source_bounds.x {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y,
                            1,
                            clipped_image_height,
                            96,
                        );
                    }
                    if visible_bounds.x.saturating_add(visible_bounds.width) == original_right {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x.saturating_add(clipped_image_width.saturating_sub(1)),
                            image_y,
                            1,
                            clipped_image_height,
                            96,
                        );
                    }
                }
            }
            DisplayCommand::BackgroundImage {
                x,
                y,
                width: image_width,
                height,
                shade,
                url,
                size,
                position,
                repeat,
                ..
            } => {
                let source_bounds = display_command_source_bounds_for_viewport(
                    render,
                    command_index,
                    DisplayCommandBounds {
                        x: *x,
                        y: *y,
                        width: *image_width,
                        height: *height,
                    },
                    viewport,
                    viewport_fixed,
                    viewport_sticky_top,
                );
                let image_width_cells = source_bounds.width;
                let image_height_cells = source_bounds.height;
                let image_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let image_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let clipped_image_width = visible_bounds.width.saturating_mul(options.cell_width);
                let clipped_image_height =
                    visible_bounds.height.saturating_mul(options.cell_height);
                let image_width = image_width_cells.saturating_mul(options.cell_width);
                let image_height = image_height_cells.saturating_mul(options.cell_height);
                let source_offset_x = visible_bounds
                    .x
                    .saturating_sub(source_bounds.x)
                    .saturating_mul(options.cell_width);
                let source_offset_y = visible_bounds
                    .y
                    .saturating_sub(source_bounds.y)
                    .saturating_mul(options.cell_height);
                if let Some(decoded) = url.as_deref().and_then(|url| render.decoded_image(url)) {
                    draw_background_image_region(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        image_width,
                        image_height,
                        *size,
                        *position,
                        *repeat,
                        decoded,
                    );
                } else if let Some(decoded) = url
                    .as_deref()
                    .and_then(|url| decode_image_reference(&render.source, url))
                {
                    draw_background_image_region(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        image_width,
                        image_height,
                        *size,
                        *position,
                        *repeat,
                        &decoded,
                    );
                } else {
                    fill_raster_rect(
                        &mut pixels,
                        width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        *shade,
                    );
                    let original_right = source_bounds.x.saturating_add(source_bounds.width);
                    let original_bottom = source_bounds.y.saturating_add(source_bounds.height);
                    if visible_bounds.y == source_bounds.y {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y,
                            clipped_image_width,
                            1,
                            96,
                        );
                    }
                    if visible_bounds.y.saturating_add(visible_bounds.height) == original_bottom {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y.saturating_add(clipped_image_height.saturating_sub(1)),
                            clipped_image_width,
                            1,
                            96,
                        );
                    }
                    if visible_bounds.x == source_bounds.x {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x,
                            image_y,
                            1,
                            clipped_image_height,
                            96,
                        );
                    }
                    if visible_bounds.x.saturating_add(visible_bounds.width) == original_right {
                        fill_raster_rect(
                            &mut pixels,
                            width,
                            image_x.saturating_add(clipped_image_width.saturating_sub(1)),
                            image_y,
                            1,
                            clipped_image_height,
                            96,
                        );
                    }
                }
            }
        }
    }
    if raster_viewport_needs_readable_context(render, viewport)
        && let Some(context) = nearby_visual_region_text_context(render, viewport)
    {
        let rows =
            raster_text_context_overlay_rows(render, viewport, context.bounds, context.lines.len());
        draw_raster_text_context_lines(
            &mut pixels,
            width,
            &context.lines,
            viewport,
            options,
            &rows,
        );
    }

    Ok(BrowserRaster {
        width,
        height,
        background,
        foreground: 0,
        pixels,
    })
}

fn draw_raster_text_command(
    pixels: &mut [u8],
    raster_width: usize,
    command: &DisplayCommand,
    render: &BrowserRender,
    command_index: usize,
    viewport: RasterViewport,
    options: BrowserRasterOptions,
) {
    let (x, y, text, ink) = match command {
        DisplayCommand::Text { x, y, text } => (*x, *y, text.as_str(), 0),
        DisplayCommand::StyledText { x, y, text, shade } => (*x, *y, text.as_str(), *shade),
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => return,
    };
    let viewport_fixed = display_command_viewport_fixed(render, command_index);
    let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
    let (x, y) =
        display_command_origin_for_viewport(x, y, viewport, viewport_fixed, viewport_sticky_top);
    if y < viewport.y || y >= viewport.end_y() {
        return;
    }
    if scaled_text_vertical_duplicate(render, command_index, command, viewport) {
        return;
    }
    let text = readable_display_text(text);
    draw_raster_text_run(pixels, raster_width, &text, x, y, viewport, options, ink);
}

fn draw_rgba_text_command(
    pixels: &mut [u8],
    raster_width: usize,
    command: &DisplayCommand,
    render: &BrowserRender,
    command_index: usize,
    viewport: RasterViewport,
    options: BrowserRasterOptions,
) {
    let (x, y, text, ink) = match command {
        DisplayCommand::Text { x, y, text } => (*x, *y, text.as_str(), 0),
        DisplayCommand::StyledText { x, y, text, shade } => (*x, *y, text.as_str(), *shade),
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => return,
    };
    let viewport_fixed = display_command_viewport_fixed(render, command_index);
    let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
    let (x, y) =
        display_command_origin_for_viewport(x, y, viewport, viewport_fixed, viewport_sticky_top);
    if y < viewport.y || y >= viewport.end_y() {
        return;
    }
    if scaled_text_vertical_duplicate(render, command_index, command, viewport) {
        return;
    }
    let text = readable_display_text(text);
    draw_rgba_text_run(pixels, raster_width, &text, x, y, viewport, options, ink);
}

pub fn rasterize_render_rgba(
    render: &BrowserRender,
    options: BrowserRasterOptions,
) -> Result<BrowserRgbaRaster> {
    ensure!(
        options.cell_width > GLYPH_WIDTH,
        "cell_width must be at least {}",
        GLYPH_WIDTH + 1
    );
    ensure!(
        options.cell_height > GLYPH_HEIGHT,
        "cell_height must be at least {}",
        GLYPH_HEIGHT + 1
    );
    if let Some(viewport_width) = options.viewport_width {
        ensure!(
            viewport_width > 0,
            "viewport_width must be greater than zero"
        );
    }
    if let Some(viewport_height) = options.viewport_height {
        ensure!(
            viewport_height > 0,
            "viewport_height must be greater than zero"
        );
    }

    let viewport = effective_raster_viewport(render, options);
    let text_width = viewport
        .width
        .checked_mul(options.cell_width)
        .context("raster width overflow")?;
    let text_height = viewport
        .height
        .checked_mul(options.cell_height)
        .context("raster height overflow")?;
    let width = text_width
        .checked_add(options.padding_x.saturating_mul(2))
        .context("raster padded width overflow")?;
    let height = text_height
        .checked_add(options.padding_y.saturating_mul(2))
        .context("raster padded height overflow")?;
    let pixel_count = width.checked_mul(height).context("raster pixel overflow")?;
    ensure!(
        pixel_count <= options.max_pixels,
        "raster would allocate {pixel_count} pixels, over max {}",
        options.max_pixels
    );

    let background = [255u8, 255u8, 255u8, 255u8];
    let mut rgba = BrowserRgbaRaster {
        width,
        height,
        background,
        pixels: vec![255u8; pixel_count.saturating_mul(4)],
    };
    for (command_index, command) in render.display_list.iter().enumerate() {
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        let command_bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        let Some(visible_bounds) = intersect_display_bounds_with_viewport(command_bounds, viewport)
        else {
            continue;
        };
        match command {
            DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. } => {
                draw_rgba_text_command(
                    &mut rgba.pixels,
                    rgba.width,
                    command,
                    render,
                    command_index,
                    viewport,
                    options,
                );
            }
            DisplayCommand::Image {
                x,
                y,
                width: image_width,
                height,
                url,
                ..
            } => {
                let source_bounds = display_command_source_bounds_for_viewport(
                    render,
                    command_index,
                    DisplayCommandBounds {
                        x: *x,
                        y: *y,
                        width: *image_width,
                        height: *height,
                    },
                    viewport,
                    viewport_fixed,
                    viewport_sticky_top,
                );
                let image_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let image_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let clipped_image_width = visible_bounds.width.saturating_mul(options.cell_width);
                let clipped_image_height =
                    visible_bounds.height.saturating_mul(options.cell_height);
                let full_width = source_bounds.width.saturating_mul(options.cell_width);
                let full_height = source_bounds.height.saturating_mul(options.cell_height);
                let source_offset_x = visible_bounds
                    .x
                    .saturating_sub(source_bounds.x)
                    .saturating_mul(options.cell_width);
                let source_offset_y = visible_bounds
                    .y
                    .saturating_sub(source_bounds.y)
                    .saturating_mul(options.cell_height);
                if let Some(decoded) = url.as_deref().and_then(|url| render.decoded_image(url)) {
                    draw_decoded_image_region_rgba(
                        &mut rgba.pixels,
                        rgba.width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        full_width,
                        full_height,
                        decoded,
                    );
                } else if let Some(decoded) = url
                    .as_deref()
                    .and_then(|url| decode_image_reference(&render.source, url))
                {
                    draw_decoded_image_region_rgba(
                        &mut rgba.pixels,
                        rgba.width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        full_width,
                        full_height,
                        &decoded,
                    );
                }
            }
            DisplayCommand::BackgroundImage {
                x,
                y,
                width: image_width,
                height,
                url,
                size,
                position,
                repeat,
                ..
            } => {
                let source_bounds = display_command_source_bounds_for_viewport(
                    render,
                    command_index,
                    DisplayCommandBounds {
                        x: *x,
                        y: *y,
                        width: *image_width,
                        height: *height,
                    },
                    viewport,
                    viewport_fixed,
                    viewport_sticky_top,
                );
                let image_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let image_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let clipped_image_width = visible_bounds.width.saturating_mul(options.cell_width);
                let clipped_image_height =
                    visible_bounds.height.saturating_mul(options.cell_height);
                let full_width = source_bounds.width.saturating_mul(options.cell_width);
                let full_height = source_bounds.height.saturating_mul(options.cell_height);
                let source_offset_x = visible_bounds
                    .x
                    .saturating_sub(source_bounds.x)
                    .saturating_mul(options.cell_width);
                let source_offset_y = visible_bounds
                    .y
                    .saturating_sub(source_bounds.y)
                    .saturating_mul(options.cell_height);
                if let Some(decoded) = url.as_deref().and_then(|url| render.decoded_image(url)) {
                    draw_background_image_region_rgba(
                        &mut rgba.pixels,
                        rgba.width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        full_width,
                        full_height,
                        *size,
                        *position,
                        *repeat,
                        decoded,
                    );
                } else if let Some(decoded) = url
                    .as_deref()
                    .and_then(|url| decode_image_reference(&render.source, url))
                {
                    draw_background_image_region_rgba(
                        &mut rgba.pixels,
                        rgba.width,
                        image_x,
                        image_y,
                        clipped_image_width,
                        clipped_image_height,
                        source_offset_x,
                        source_offset_y,
                        full_width,
                        full_height,
                        *size,
                        *position,
                        *repeat,
                        &decoded,
                    );
                }
            }
            DisplayCommand::Rect { shade, .. } => {
                let rect_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let rect_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let rect_width = visible_bounds.width.saturating_mul(options.cell_width);
                let rect_height = visible_bounds.height.saturating_mul(options.cell_height);
                fill_rgba_rect(
                    &mut rgba.pixels,
                    rgba.width,
                    rect_x,
                    rect_y,
                    rect_width,
                    rect_height,
                    [*shade, *shade, *shade, 255],
                );
            }
            DisplayCommand::ColorRect {
                red, green, blue, ..
            } => {
                let rect_x = options.padding_x.saturating_add(
                    visible_bounds
                        .x
                        .saturating_sub(viewport.x)
                        .saturating_mul(options.cell_width),
                );
                let rect_y = options.padding_y.saturating_add(
                    visible_bounds
                        .y
                        .saturating_sub(viewport.y)
                        .saturating_mul(options.cell_height),
                );
                let rect_width = visible_bounds.width.saturating_mul(options.cell_width);
                let rect_height = visible_bounds.height.saturating_mul(options.cell_height);
                fill_rgba_rect(
                    &mut rgba.pixels,
                    rgba.width,
                    rect_x,
                    rect_y,
                    rect_width,
                    rect_height,
                    [*red, *green, *blue, 255],
                );
            }
        }
    }
    if raster_viewport_needs_readable_context(render, viewport)
        && let Some(context) = nearby_visual_region_text_context(render, viewport)
    {
        let rows =
            raster_text_context_overlay_rows(render, viewport, context.bounds, context.lines.len());
        draw_rgba_text_context_lines(
            &mut rgba.pixels,
            rgba.width,
            &context.lines,
            viewport,
            options,
            &rows,
        );
    }
    Ok(rgba)
}

pub fn raster_report(
    render: &BrowserRender,
    raster: &BrowserRaster,
    options: BrowserRasterOptions,
) -> BrowserRasterReport {
    let viewport = effective_raster_viewport(render, options);
    let (visible_command_count, culled_command_count) = raster_visibility_counts(render, viewport);
    BrowserRasterReport {
        source: render.source.clone(),
        viewport_width: render.viewport_width,
        width: raster.width,
        height: raster.height,
        cell_width: options.cell_width,
        cell_height: options.cell_height,
        display_command_count: render.display_list.len(),
        visible_command_count,
        culled_command_count,
        raster_viewport_x: viewport.active.then_some(viewport.x),
        raster_viewport_y: viewport.active.then_some(viewport.y),
        raster_viewport_width: viewport.active.then_some(viewport.width),
        raster_viewport_height: viewport.active.then_some(viewport.height),
        non_background_pixels: raster.non_background_pixels(),
        pixel_hash: raster.pixel_hash(),
    }
}

pub fn rgba_raster_report(
    render: &BrowserRender,
    raster: &BrowserRgbaRaster,
    options: BrowserRasterOptions,
) -> BrowserRgbaRasterReport {
    let viewport = effective_raster_viewport(render, options);
    let (visible_command_count, culled_command_count) = raster_visibility_counts(render, viewport);
    BrowserRgbaRasterReport {
        source: render.source.clone(),
        viewport_width: render.viewport_width,
        width: raster.width,
        height: raster.height,
        cell_width: options.cell_width,
        cell_height: options.cell_height,
        bytes_per_pixel: 4,
        display_command_count: render.display_list.len(),
        visible_command_count,
        culled_command_count,
        raster_viewport_x: viewport.active.then_some(viewport.x),
        raster_viewport_y: viewport.active.then_some(viewport.y),
        raster_viewport_width: viewport.active.then_some(viewport.width),
        raster_viewport_height: viewport.active.then_some(viewport.height),
        non_background_pixels: raster.non_background_pixels(),
        pixel_hash: raster.pixel_hash(),
        artifact_format: "png-rgba8".to_owned(),
    }
}

pub fn browser_text_viewport(
    render: &BrowserRender,
    options: BrowserTextViewportOptions,
) -> BrowserTextViewportReport {
    let document_viewport = browser_document_viewport(
        render,
        BrowserViewportState {
            x: options.x,
            y: options.y,
            width: options.width,
            height: options.height,
        },
        None,
    );
    let viewport = raster_viewport_from_state(document_viewport.viewport);
    let width = document_viewport.viewport.width;
    let height = document_viewport.viewport.height;
    let mut cells = vec![vec![' '; width]; height];

    for (command_index, command) in render.display_list.iter().enumerate() {
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        let command_bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        let Some(visible_bounds) = intersect_display_bounds_with_viewport(command_bounds, viewport)
        else {
            continue;
        };
        match command {
            DisplayCommand::Text { .. } | DisplayCommand::StyledText { .. } => {
                draw_text_viewport_command(&mut cells, command, render, command_index, viewport);
            }
            DisplayCommand::Rect { .. } | DisplayCommand::ColorRect { .. } => {
                fill_text_viewport_visual_cells(&mut cells, viewport, visible_bounds, '#')
            }
            DisplayCommand::Image { alt, .. } => {
                fill_text_viewport_visual_cells(&mut cells, viewport, visible_bounds, '@');
                overlay_text_viewport_image_alt(
                    &mut cells,
                    viewport,
                    visible_bounds,
                    alt.as_deref(),
                );
            }
            DisplayCommand::BackgroundImage { .. } => {
                fill_text_viewport_visual_cells(&mut cells, viewport, visible_bounds, '@');
            }
        }
    }
    if text_viewport_needs_readable_context(&cells, render, viewport)
        && let Some(context) = nearby_visual_region_text_context(render, viewport)
    {
        overlay_text_viewport_context(&mut cells, viewport, context.bounds, &context.lines);
    }

    BrowserTextViewportReport {
        source: document_viewport.source,
        title: document_viewport.title,
        document_width: document_viewport.document_width,
        document_height: document_viewport.document_height,
        x: viewport.x,
        y: viewport.y,
        max_scroll_x: document_viewport.max_scroll_x,
        max_scroll_y: document_viewport.max_scroll_y,
        width,
        height,
        display_command_count: document_viewport.display_command_count,
        visible_command_count: document_viewport.visible_command_count,
        culled_command_count: document_viewport.culled_command_count,
        layout_box_count: document_viewport.layout_box_count,
        visible_layout_box_count: document_viewport.visible_layout_box_count,
        culled_layout_box_count: document_viewport.culled_layout_box_count,
        visible_layout_boxes: document_viewport.visible_layout_boxes,
        lines: cells
            .into_iter()
            .map(|line| trim_trailing_spaces(line.into_iter().collect::<String>()))
            .collect(),
    }
}

fn draw_text_viewport_command(
    cells: &mut [Vec<char>],
    command: &DisplayCommand,
    render: &BrowserRender,
    command_index: usize,
    viewport: RasterViewport,
) {
    let (x, y, text) = match command {
        DisplayCommand::Text { x, y, text } | DisplayCommand::StyledText { x, y, text, .. } => {
            (*x, *y, text.as_str())
        }
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => return,
    };
    let viewport_fixed = display_command_viewport_fixed(render, command_index);
    let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
    let (x, y) =
        display_command_origin_for_viewport(x, y, viewport, viewport_fixed, viewport_sticky_top);
    if y < viewport.y || y >= viewport.end_y() {
        return;
    }
    if scaled_text_vertical_duplicate(render, command_index, command, viewport) {
        return;
    }
    let row = y.saturating_sub(viewport.y);
    let text = readable_display_text(text);
    for (column_offset, ch) in text.chars().enumerate() {
        let document_column = x.saturating_add(column_offset);
        if document_column < viewport.x || document_column >= viewport.end_x() {
            continue;
        }
        let column = document_column.saturating_sub(viewport.x);
        if let Some(cell) = cells.get_mut(row).and_then(|line| line.get_mut(column)) {
            *cell = ch;
        }
    }
}

fn text_viewport_needs_readable_context(
    cells: &[Vec<char>],
    render: &BrowserRender,
    viewport: RasterViewport,
) -> bool {
    let visual_rows = text_viewport_visual_fill_row_count(cells);
    visual_rows > 0
        && viewport_needs_more_body_context(
            render,
            viewport,
            visual_rows,
            text_viewport_mixed_media_row_threshold(cells.len()),
        )
}

fn meaningful_text_row_threshold(width: usize) -> usize {
    width.saturating_div(4).clamp(6, 24)
}

fn text_viewport_visual_fill_row_count(cells: &[Vec<char>]) -> usize {
    cells
        .iter()
        .filter(|line| line.iter().any(|ch| matches!(*ch, '#' | '@')))
        .count()
}

fn text_viewport_mixed_media_row_threshold(height: usize) -> usize {
    height.saturating_div(2).max(1)
}

fn raster_viewport_needs_readable_context(
    render: &BrowserRender,
    viewport: RasterViewport,
) -> bool {
    let visual_rows = raster_viewport_visual_fill_row_count(render, viewport);
    visual_rows >= text_viewport_mixed_media_row_threshold(viewport.height)
        && raster_viewport_needs_more_body_context(render, viewport, visual_rows)
}

fn viewport_needs_more_body_context(
    render: &BrowserRender,
    viewport: RasterViewport,
    visual_rows: usize,
    mixed_media_row_threshold: usize,
) -> bool {
    let meaningful_rows = viewport_meaningful_visible_text_row_count(render, viewport, false);
    meaningful_rows == 0
        || (visual_rows >= mixed_media_row_threshold
            && meaningful_rows < viewport_minimum_readable_body_rows(viewport.height))
}

fn viewport_minimum_readable_body_rows(height: usize) -> usize {
    height.min(2)
}

fn raster_viewport_needs_more_body_context(
    render: &BrowserRender,
    viewport: RasterViewport,
    visual_rows: usize,
) -> bool {
    let meaningful_rows = viewport_meaningful_visible_text_row_count(render, viewport, false);
    meaningful_rows < raster_viewport_minimum_readable_body_rows(viewport.height, visual_rows)
}

fn raster_viewport_minimum_readable_body_rows(height: usize, visual_rows: usize) -> usize {
    if visual_rows >= height.saturating_sub(1).max(1) {
        height.min(3)
    } else {
        viewport_minimum_readable_body_rows(height)
    }
}

fn viewport_meaningful_visible_text_row_count(
    render: &BrowserRender,
    viewport: RasterViewport,
    include_pinned: bool,
) -> usize {
    let mut row_text_counts = vec![0usize; viewport.height];
    for (command_index, command) in render.display_list.iter().enumerate() {
        let Some(text) = display_command_text(command) else {
            continue;
        };
        let text = readable_display_text(text);
        if !text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
            continue;
        }
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        if !include_pinned && (viewport_fixed || viewport_sticky_top.is_some()) {
            continue;
        }
        let command_bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        if intersect_display_bounds_with_viewport(command_bounds, viewport).is_none() {
            continue;
        }
        let row = command_bounds.y.saturating_sub(viewport.y);
        if let Some(count) = row_text_counts.get_mut(row) {
            *count =
                count.saturating_add(text.chars().filter(|ch| ch.is_ascii_alphanumeric()).count());
        }
    }
    row_text_counts
        .into_iter()
        .filter(|count| *count >= meaningful_text_row_threshold(viewport.width))
        .count()
}

fn raster_viewport_visual_fill_row_count(
    render: &BrowserRender,
    viewport: RasterViewport,
) -> usize {
    let mut row_has_visual = vec![false; viewport.height];
    for (command_index, command) in render.display_list.iter().enumerate() {
        if !display_command_is_visual_fill(command) {
            continue;
        }
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        let bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        let Some(visible) = intersect_display_bounds_with_viewport(bounds, viewport) else {
            continue;
        };
        if !large_text_viewport_visual_fill(viewport, visible) {
            continue;
        }
        let start_row = visible.y.saturating_sub(viewport.y);
        let end_row = start_row
            .saturating_add(visible.height)
            .min(viewport.height);
        for row in start_row..end_row {
            if let Some(has_visual) = row_has_visual.get_mut(row) {
                *has_visual = true;
            }
        }
    }
    row_has_visual
        .into_iter()
        .filter(|has_visual| *has_visual)
        .count()
}

fn raster_text_context_overlay_rows(
    render: &BrowserRender,
    viewport: RasterViewport,
    context_bounds: DisplayCommandBounds,
    line_count: usize,
) -> Vec<usize> {
    let mut row_has_text = vec![false; viewport.height];
    let mut row_has_visual = vec![false; viewport.height];
    for (command_index, command) in render.display_list.iter().enumerate() {
        let viewport_fixed = display_command_viewport_fixed(render, command_index);
        let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
        let bounds = display_command_bounds_for_viewport(
            command,
            viewport,
            viewport_fixed,
            viewport_sticky_top,
        );
        let Some(visible) = intersect_display_bounds_with_viewport(bounds, viewport) else {
            continue;
        };
        let start_row = visible.y.saturating_sub(viewport.y);
        let end_row = start_row
            .saturating_add(visible.height)
            .min(viewport.height);
        if display_command_is_visual_fill(command) {
            for row in start_row..end_row {
                if let Some(has_visual) = row_has_visual.get_mut(row) {
                    *has_visual = true;
                }
            }
            continue;
        }
        if display_command_text(command)
            .is_some_and(|text| text.chars().any(|ch| ch.is_ascii_alphanumeric()))
        {
            for row in start_row..end_row {
                if let Some(has_text) = row_has_text.get_mut(row) {
                    *has_text = true;
                }
            }
        }
    }
    visual_context_overlay_rows(
        row_has_visual
            .iter()
            .zip(row_has_text.iter())
            .map(|(has_visual, has_text)| *has_visual && !*has_text),
        preferred_context_overlay_row(viewport, context_bounds),
        line_count,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisualTextContext {
    lines: Vec<String>,
    bounds: DisplayCommandBounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisualTextCandidate {
    rank: usize,
    viewport_distance: usize,
    visual_distance: usize,
    text: String,
    bounds: DisplayCommandBounds,
}

fn nearby_visual_region_text_context(
    render: &BrowserRender,
    viewport: RasterViewport,
) -> Option<VisualTextContext> {
    let visual_bounds = render
        .display_list
        .iter()
        .enumerate()
        .filter_map(|(command_index, command)| {
            if !display_command_is_visual_fill(command) {
                return None;
            }
            let viewport_fixed = display_command_viewport_fixed(render, command_index);
            let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
            let bounds = display_command_bounds_for_viewport(
                command,
                viewport,
                viewport_fixed,
                viewport_sticky_top,
            );
            let visible = intersect_display_bounds_with_viewport(bounds, viewport)?;
            large_text_viewport_visual_fill(viewport, visible).then_some(bounds)
        })
        .collect::<Vec<_>>();
    if visual_bounds.is_empty() {
        return None;
    }

    let selected = render
        .display_list
        .iter()
        .enumerate()
        .filter_map(|(command_index, command)| {
            visual_text_context_candidate(
                render,
                viewport,
                &visual_bounds,
                command_index,
                command,
                false,
            )
        })
        .min_by_key(|candidate| {
            (
                candidate.rank,
                candidate.viewport_distance,
                candidate.visual_distance,
            )
        })?;
    let lines =
        nearby_visual_region_text_context_lines(render, viewport, &visual_bounds, &selected);
    Some(VisualTextContext {
        lines,
        bounds: selected.bounds,
    })
}

fn visual_text_context_candidate(
    render: &BrowserRender,
    viewport: RasterViewport,
    visual_bounds: &[DisplayCommandBounds],
    command_index: usize,
    command: &DisplayCommand,
    include_visible_visual_text: bool,
) -> Option<VisualTextCandidate> {
    let text = display_command_text(command)?;
    let text = readable_display_text(text);
    let text = collapse_ascii_whitespace(&text);
    if text.is_empty() || !text.chars().any(|ch| ch.is_ascii_alphanumeric()) {
        return None;
    }
    let viewport_fixed = display_command_viewport_fixed(render, command_index);
    let viewport_sticky_top = display_command_viewport_sticky_top(render, command_index);
    let bounds =
        display_command_bounds_for_viewport(command, viewport, viewport_fixed, viewport_sticky_top);
    let inside_visual = visual_bounds
        .iter()
        .any(|visual| bounds_inside_visual_region(bounds, *visual));
    let visual_distance = visual_bounds
        .iter()
        .filter_map(|visual| bounds_visual_region_distance(bounds, *visual))
        .min();
    let near_visual = visual_distance
        .is_some_and(|distance| distance <= bounds_visual_region_search_rows(viewport));
    if intersect_display_bounds_with_viewport(bounds, viewport).is_some() {
        if !include_visible_visual_text || (!inside_visual && !near_visual) {
            return None;
        }
    }
    let near_viewport = bounds_near_visual_viewport(bounds, viewport);
    if !inside_visual && !near_visual && !near_viewport {
        return None;
    }
    let viewport_distance = bounds_viewport_distance(bounds, viewport);
    let rank = usize::from(!(inside_visual || near_visual));
    Some(VisualTextCandidate {
        rank,
        viewport_distance,
        visual_distance: visual_distance.unwrap_or(usize::MAX),
        text,
        bounds,
    })
}

fn nearby_visual_region_text_context_lines(
    render: &BrowserRender,
    viewport: RasterViewport,
    visual_bounds: &[DisplayCommandBounds],
    selected: &VisualTextCandidate,
) -> Vec<String> {
    let mut lines =
        render
            .display_list
            .iter()
            .enumerate()
            .filter_map(|(command_index, command)| {
                let candidate = visual_text_context_candidate(
                    render,
                    viewport,
                    visual_bounds,
                    command_index,
                    command,
                    true,
                )?;
                bounds_near_selected_context(candidate.bounds, selected.bounds, viewport)
                    .then_some((candidate.bounds.y, candidate.bounds.x, candidate.text))
            })
            .collect::<Vec<_>>();
    lines.sort_by_key(|(y, x, _)| (*y, *x));
    lines.dedup_by(|left, right| left.2 == right.2);
    if lines.is_empty() {
        return vec![selected.text.clone()];
    }
    lines.into_iter().take(3).map(|(_, _, text)| text).collect()
}

fn display_command_text(command: &DisplayCommand) -> Option<&str> {
    match command {
        DisplayCommand::Text { text, .. } | DisplayCommand::StyledText { text, .. } => {
            Some(text.as_str())
        }
        DisplayCommand::Rect { .. }
        | DisplayCommand::ColorRect { .. }
        | DisplayCommand::Image { .. }
        | DisplayCommand::BackgroundImage { .. } => None,
    }
}

fn readable_display_text(text: &str) -> String {
    collapse_repeated_glyph_runs(text).unwrap_or_else(|| text.to_owned())
}

fn scaled_text_vertical_duplicate(
    render: &BrowserRender,
    command_index: usize,
    command: &DisplayCommand,
    viewport: RasterViewport,
) -> bool {
    let Some(text) = display_command_text(command) else {
        return false;
    };
    let Some(readable) = collapse_repeated_glyph_runs(text) else {
        return false;
    };
    let current_bounds = display_command_bounds_for_viewport(
        command,
        viewport,
        display_command_viewport_fixed(render, command_index),
        display_command_viewport_sticky_top(render, command_index),
    );
    for previous_index in (0..command_index).rev() {
        let previous_command = &render.display_list[previous_index];
        let Some(previous_text) = display_command_text(previous_command) else {
            continue;
        };
        if collapse_repeated_glyph_runs(previous_text).as_deref() != Some(readable.as_str()) {
            continue;
        }
        let previous_bounds = display_command_bounds_for_viewport(
            previous_command,
            viewport,
            display_command_viewport_fixed(render, previous_index),
            display_command_viewport_sticky_top(render, previous_index),
        );
        return previous_bounds.x == current_bounds.x
            && previous_bounds.y.saturating_add(1) == current_bounds.y
            && previous_bounds.y >= viewport.y
            && previous_bounds.y < viewport.end_y();
    }
    false
}

fn collapse_repeated_glyph_runs(text: &str) -> Option<String> {
    let mut runs = Vec::new();
    let mut chars = text.chars();
    let mut current = chars.next()?;
    let mut current_len = 1usize;
    for ch in chars {
        if ch == current {
            current_len = current_len.saturating_add(1);
        } else {
            runs.push((current, current_len));
            current = ch;
            current_len = 1;
        }
    }
    runs.push((current, current_len));

    let repeated_non_space_runs = runs
        .iter()
        .filter(|(ch, len)| !ch.is_whitespace() && *len >= 2)
        .count();
    if repeated_non_space_runs < 3 {
        return None;
    }

    let scale = runs
        .iter()
        .filter(|(ch, _)| !ch.is_whitespace())
        .map(|(_, len)| *len)
        .reduce(gcd_usize)?;
    if !(2..=4).contains(&scale) {
        return None;
    }
    if runs
        .iter()
        .any(|(ch, len)| !ch.is_whitespace() && *len % scale != 0)
    {
        return None;
    }

    let mut collapsed = String::with_capacity(text.chars().count() / scale.max(1));
    for (ch, len) in runs {
        let count = if len % scale == 0 { len / scale } else { len };
        collapsed.extend(std::iter::repeat(ch).take(count.max(1)));
    }
    (collapsed.chars().count() < text.chars().count()).then_some(collapsed)
}

fn gcd_usize(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn display_command_is_visual_fill(command: &DisplayCommand) -> bool {
    matches!(
        command,
        DisplayCommand::Rect { .. }
            | DisplayCommand::ColorRect { .. }
            | DisplayCommand::Image { .. }
            | DisplayCommand::BackgroundImage { .. }
    )
}

fn bounds_inside_visual_region(text: DisplayCommandBounds, visual: DisplayCommandBounds) -> bool {
    let visual_end_x = visual.x.saturating_add(visual.width);
    let visual_end_y = visual.y.saturating_add(visual.height);
    text.y >= visual.y
        && text.y < visual_end_y
        && text.x < visual_end_x
        && text.x.saturating_add(text.width) > visual.x
}

fn bounds_near_visual_viewport(text: DisplayCommandBounds, viewport: RasterViewport) -> bool {
    let horizontal_overlap =
        text.x < viewport.end_x() && text.x.saturating_add(text.width) > viewport.x;
    if !horizontal_overlap {
        return false;
    }
    let search_rows = viewport.height.saturating_mul(2).max(4);
    bounds_viewport_distance(text, viewport) <= search_rows
}

fn bounds_viewport_distance(bounds: DisplayCommandBounds, viewport: RasterViewport) -> usize {
    if bounds.y < viewport.y {
        viewport
            .y
            .saturating_sub(bounds.y.saturating_add(bounds.height))
    } else {
        bounds.y.saturating_sub(viewport.end_y())
    }
}

fn bounds_visual_region_search_rows(viewport: RasterViewport) -> usize {
    viewport.height.saturating_mul(2).max(4)
}

fn bounds_visual_region_distance(
    text: DisplayCommandBounds,
    visual: DisplayCommandBounds,
) -> Option<usize> {
    let horizontal_overlap = text.x < visual.x.saturating_add(visual.width)
        && text.x.saturating_add(text.width) > visual.x;
    if !horizontal_overlap {
        return None;
    }
    let text_end_y = text.y.saturating_add(text.height);
    let visual_end_y = visual.y.saturating_add(visual.height);
    if text_end_y < visual.y {
        Some(visual.y.saturating_sub(text_end_y))
    } else if visual_end_y < text.y {
        Some(text.y.saturating_sub(visual_end_y))
    } else {
        Some(0)
    }
}

fn bounds_near_selected_context(
    candidate: DisplayCommandBounds,
    selected: DisplayCommandBounds,
    viewport: RasterViewport,
) -> bool {
    if bounds_visual_region_distance(candidate, selected)
        .is_some_and(|distance| distance <= viewport.height.saturating_mul(2).max(2))
    {
        return true;
    }
    candidate.y.abs_diff(selected.y) <= viewport.height
}

fn overlay_text_viewport_context(
    cells: &mut [Vec<char>],
    viewport: RasterViewport,
    context_bounds: DisplayCommandBounds,
    lines: &[String],
) {
    let rows = visual_context_overlay_rows(
        cells.iter().map(|line| {
            let has_text = line.iter().any(|ch| ch.is_ascii_alphanumeric());
            let has_visual = line.iter().any(|ch| matches!(*ch, '#' | '@'));
            has_visual && !has_text
        }),
        preferred_context_overlay_row(viewport, context_bounds),
        lines.len(),
    );
    for (text, row) in lines.iter().zip(rows) {
        let Some(line) = cells.get_mut(row) else {
            continue;
        };
        for (column, ch) in text.chars().take(line.len()).enumerate() {
            if let Some(cell) = line.get_mut(column) {
                *cell = ch;
            }
        }
    }
}

fn draw_raster_text_context_lines(
    pixels: &mut [u8],
    raster_width: usize,
    lines: &[String],
    viewport: RasterViewport,
    options: BrowserRasterOptions,
    rows: &[usize],
) {
    for (text, row) in lines.iter().zip(rows) {
        draw_raster_text_run(
            pixels,
            raster_width,
            text,
            viewport.x,
            viewport.y.saturating_add(*row),
            viewport,
            options,
            0,
        );
    }
}

fn draw_rgba_text_context_lines(
    pixels: &mut [u8],
    raster_width: usize,
    lines: &[String],
    viewport: RasterViewport,
    options: BrowserRasterOptions,
    rows: &[usize],
) {
    for (text, row) in lines.iter().zip(rows) {
        draw_rgba_text_run(
            pixels,
            raster_width,
            text,
            viewport.x,
            viewport.y.saturating_add(*row),
            viewport,
            options,
            0,
        );
    }
}

fn preferred_context_overlay_row(
    viewport: RasterViewport,
    context_bounds: DisplayCommandBounds,
) -> usize {
    if context_bounds.y < viewport.y {
        0
    } else if context_bounds.y >= viewport.end_y() {
        viewport.height.saturating_sub(1)
    } else {
        context_bounds
            .y
            .saturating_sub(viewport.y)
            .min(viewport.height.saturating_sub(1))
    }
}

fn visual_context_overlay_rows(
    rows: impl IntoIterator<Item = bool>,
    preferred_row: usize,
    line_count: usize,
) -> Vec<usize> {
    if line_count == 0 {
        return Vec::new();
    }
    let available = rows.into_iter().collect::<Vec<_>>();
    if available.is_empty() {
        return Vec::new();
    }
    let count = line_count.min(available.len());
    if count == 1 {
        return nearest_available_visual_row(available.iter().copied(), preferred_row)
            .into_iter()
            .collect();
    }

    let mut best_start = None;
    for start in 0..=available.len().saturating_sub(count) {
        if !available[start..start.saturating_add(count)]
            .iter()
            .all(|row| *row)
        {
            continue;
        }
        let end = start.saturating_add(count).saturating_sub(1);
        let distance = end.abs_diff(preferred_row);
        if best_start.is_none_or(|(_, best_distance)| distance < best_distance) {
            best_start = Some((start, distance));
        }
    }
    if let Some((start, _)) = best_start {
        return (start..start.saturating_add(count)).collect();
    }

    let mut chosen = Vec::new();
    let mut occupied = vec![false; available.len()];
    for offset in 0..count {
        let preferred = preferred_row
            .saturating_add(offset)
            .min(available.len().saturating_sub(1));
        let Some(row) = nearest_available_visual_row(
            available
                .iter()
                .zip(occupied.iter())
                .map(|(available, occupied)| *available && !*occupied),
            preferred,
        ) else {
            break;
        };
        occupied[row] = true;
        chosen.push(row);
    }
    chosen.sort_unstable();
    chosen
}

fn nearest_available_visual_row(
    rows: impl IntoIterator<Item = bool>,
    preferred_row: usize,
) -> Option<usize> {
    rows.into_iter()
        .enumerate()
        .filter(|(_, available)| *available)
        .min_by_key(|(row, _)| row.abs_diff(preferred_row))
        .map(|(row, _)| row)
}

fn fill_text_viewport_empty_cells(
    cells: &mut [Vec<char>],
    viewport: RasterViewport,
    bounds: DisplayCommandBounds,
    ch: char,
) {
    let start_y = bounds.y.saturating_sub(viewport.y);
    let end_y = start_y.saturating_add(bounds.height).min(cells.len());
    let start_x = bounds.x.saturating_sub(viewport.x);
    let end_x = start_x.saturating_add(bounds.width).min(viewport.width);
    for row in start_y..end_y {
        if let Some(line) = cells.get_mut(row) {
            for column in start_x..end_x {
                if let Some(cell) = line.get_mut(column) {
                    if *cell == ' ' {
                        *cell = ch;
                    }
                }
            }
        }
    }
}

fn fill_text_viewport_visual_cells(
    cells: &mut [Vec<char>],
    viewport: RasterViewport,
    bounds: DisplayCommandBounds,
    ch: char,
) {
    if large_text_viewport_visual_fill(viewport, bounds) {
        fill_text_viewport_sparse_cells(cells, viewport, bounds, ch);
    } else {
        fill_text_viewport_empty_cells(cells, viewport, bounds, ch);
    }
}

fn large_text_viewport_visual_fill(viewport: RasterViewport, bounds: DisplayCommandBounds) -> bool {
    bounds.width >= viewport.width.saturating_div(2).max(1)
        && bounds.height >= viewport.height.saturating_div(2).max(1)
}

fn fill_text_viewport_sparse_cells(
    cells: &mut [Vec<char>],
    viewport: RasterViewport,
    bounds: DisplayCommandBounds,
    ch: char,
) {
    let start_y = bounds.y.saturating_sub(viewport.y);
    let end_y = start_y.saturating_add(bounds.height).min(cells.len());
    let start_x = bounds.x.saturating_sub(viewport.x);
    let end_x = start_x.saturating_add(bounds.width).min(viewport.width);
    for row in start_y..end_y {
        if let Some(line) = cells.get_mut(row) {
            for column in start_x..end_x {
                if (row.saturating_add(column)) % 8 != 0 {
                    continue;
                }
                if let Some(cell) = line.get_mut(column)
                    && *cell == ' '
                {
                    *cell = ch;
                }
            }
        }
    }
}

fn overlay_text_viewport_image_alt(
    cells: &mut [Vec<char>],
    viewport: RasterViewport,
    bounds: DisplayCommandBounds,
    alt: Option<&str>,
) {
    let Some(alt) = alt
        .map(collapse_ascii_whitespace)
        .filter(|alt| !alt.is_empty())
    else {
        return;
    };
    if bounds.width == 0 || bounds.height == 0 {
        return;
    }

    let row = bounds
        .y
        .saturating_add(bounds.height / 2)
        .saturating_sub(viewport.y);
    let Some(line) = cells.get_mut(row) else {
        return;
    };

    let horizontal_padding = usize::from(bounds.width > 2);
    let available_width = bounds
        .width
        .saturating_sub(horizontal_padding.saturating_mul(2));
    if available_width == 0 {
        return;
    }
    let start_x = bounds
        .x
        .saturating_sub(viewport.x)
        .saturating_add(horizontal_padding);
    for (offset, ch) in alt.chars().take(available_width).enumerate() {
        if let Some(cell) = line.get_mut(start_x.saturating_add(offset)) {
            if *cell == ' ' || *cell == '@' {
                *cell = ch;
            }
        }
    }
}

fn trim_trailing_spaces(mut line: String) -> String {
    while line.ends_with(' ') {
        line.pop();
    }
    line
}

#[allow(clippy::too_many_arguments)]
fn draw_raster_text_run(
    pixels: &mut [u8],
    raster_width: usize,
    text: &str,
    document_x: usize,
    document_y: usize,
    viewport: RasterViewport,
    options: BrowserRasterOptions,
    ink: u8,
) {
    if document_y < viewport.y || document_y >= viewport.end_y() {
        return;
    }
    let viewport_pixel_x = viewport.x.saturating_mul(options.cell_width);
    let viewport_end_pixel_x = viewport.end_x().saturating_mul(options.cell_width);
    let mut cursor_x = document_x.saturating_mul(options.cell_width);
    let cell_y = options.padding_y.saturating_add(
        document_y
            .saturating_sub(viewport.y)
            .saturating_mul(options.cell_height),
    );
    for ch in text.chars() {
        let glyph_end = cursor_x.saturating_add(options.cell_width);
        if cursor_x < viewport_end_pixel_x && glyph_end > viewport_pixel_x {
            let cell_x = options.padding_x as isize + cursor_x as isize - viewport_pixel_x as isize;
            draw_glyph_clipped(pixels, raster_width, cell_x, cell_y, ch, options, ink);
        }
        cursor_x = cursor_x.saturating_add(raster_text_document_advance(options.cell_width));
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_rgba_text_run(
    pixels: &mut [u8],
    raster_width: usize,
    text: &str,
    document_x: usize,
    document_y: usize,
    viewport: RasterViewport,
    options: BrowserRasterOptions,
    ink: u8,
) {
    if document_y < viewport.y || document_y >= viewport.end_y() {
        return;
    }
    let viewport_pixel_x = viewport.x.saturating_mul(options.cell_width);
    let viewport_end_pixel_x = viewport.end_x().saturating_mul(options.cell_width);
    let mut cursor_x = document_x.saturating_mul(options.cell_width);
    let cell_y = options.padding_y.saturating_add(
        document_y
            .saturating_sub(viewport.y)
            .saturating_mul(options.cell_height),
    );
    for ch in text.chars() {
        let glyph_end = cursor_x.saturating_add(options.cell_width);
        if cursor_x < viewport_end_pixel_x && glyph_end > viewport_pixel_x {
            let cell_x = options.padding_x as isize + cursor_x as isize - viewport_pixel_x as isize;
            draw_rgba_glyph_clipped(pixels, raster_width, cell_x, cell_y, ch, options, ink);
        }
        cursor_x = cursor_x.saturating_add(raster_text_document_advance(options.cell_width));
    }
}

fn raster_text_document_advance(cell_width: usize) -> usize {
    cell_width.max(1)
}

fn raster_glyph_advance(ch: char, cell_width: usize) -> usize {
    let scale = raster_glyph_scale_for_width(cell_width);
    if ch.is_whitespace() {
        return cell_width.saturating_div(2).clamp(3, cell_width.max(3));
    }
    let Some((left, right)) = glyph_ink_bounds(ch) else {
        return cell_width.saturating_div(2).clamp(3, cell_width.max(3));
    };
    let ink_width = right.saturating_sub(left).saturating_add(1);
    let proportional_advance = ink_width
        .saturating_mul(scale)
        .saturating_add(2)
        .clamp(3, cell_width.max(3));
    proportional_advance
        .max(readable_raster_glyph_min_advance(cell_width))
        .min(cell_width.max(3))
}

fn readable_raster_glyph_min_advance(cell_width: usize) -> usize {
    cell_width
        .saturating_mul(5)
        .saturating_add(5)
        .checked_div(6)
        .unwrap_or(cell_width)
        .clamp(3, cell_width.max(3))
}

fn raster_glyph_scale(options: BrowserRasterOptions) -> usize {
    let horizontal = options
        .cell_width
        .saturating_sub(2)
        .checked_div(GLYPH_WIDTH)
        .unwrap_or(1);
    let vertical = options
        .cell_height
        .saturating_sub(4)
        .checked_div(GLYPH_HEIGHT)
        .unwrap_or(1);
    horizontal.min(vertical).max(1)
}

fn raster_glyph_scale_for_width(cell_width: usize) -> usize {
    cell_width
        .saturating_sub(2)
        .checked_div(GLYPH_WIDTH)
        .unwrap_or(1)
        .max(1)
}

fn glyph_ink_bounds(ch: char) -> Option<(usize, usize)> {
    let mut left = usize::MAX;
    let mut right = 0usize;
    for mask in glyph_rows(ch) {
        for column in 0..5 {
            if (mask & (1 << (4 - column))) == 0 {
                continue;
            }
            left = left.min(column);
            right = right.max(column);
        }
    }
    (left != usize::MAX).then_some((left, right))
}

fn draw_glyph_clipped(
    pixels: &mut [u8],
    width: usize,
    cell_x: isize,
    cell_y: usize,
    ch: char,
    options: BrowserRasterOptions,
    ink: u8,
) {
    let ink = if let Ok(cell_x) = usize::try_from(cell_x) {
        contrasting_glyph_ink(pixels, width, cell_x, cell_y, ch, ink)
    } else {
        ink
    };
    let scale = raster_glyph_scale(options);
    let glyph_width = GLYPH_WIDTH.saturating_mul(scale);
    let glyph_height = GLYPH_HEIGHT.saturating_mul(scale);
    let offset_x = options.cell_width.saturating_sub(glyph_width) / 2;
    let offset_y = options.cell_height.saturating_sub(glyph_height) / 2;
    for (row, mask) in glyph_rows(ch).iter().enumerate() {
        for column in 0usize..GLYPH_WIDTH {
            if (*mask & (1u8 << (GLYPH_WIDTH - 1 - column))) == 0 {
                continue;
            }
            let pixel_x = cell_x + offset_x as isize + column.saturating_mul(scale) as isize;
            for dy in 0..scale {
                for dx in 0..scale {
                    let Ok(pixel_x) = usize::try_from(pixel_x.saturating_add(dx as isize)) else {
                        continue;
                    };
                    let pixel_ink = raster_glyph_pixel_ink(ink, scale, dx, dy);
                    set_raster_pixel(
                        pixels,
                        width,
                        pixel_x,
                        cell_y
                            .saturating_add(offset_y)
                            .saturating_add(row.saturating_mul(scale))
                            .saturating_add(dy),
                        pixel_ink,
                    );
                }
            }
        }
    }
}

fn draw_rgba_glyph_clipped(
    pixels: &mut [u8],
    width: usize,
    cell_x: isize,
    cell_y: usize,
    ch: char,
    options: BrowserRasterOptions,
    ink: u8,
) {
    let ink = if let Ok(cell_x) = usize::try_from(cell_x) {
        contrasting_rgba_glyph_ink(pixels, width, cell_x, cell_y, ch, ink)
    } else {
        ink
    };
    let scale = raster_glyph_scale(options);
    let glyph_width = GLYPH_WIDTH.saturating_mul(scale);
    let glyph_height = GLYPH_HEIGHT.saturating_mul(scale);
    let offset_x = options.cell_width.saturating_sub(glyph_width) / 2;
    let offset_y = options.cell_height.saturating_sub(glyph_height) / 2;
    for (row, mask) in glyph_rows(ch).iter().enumerate() {
        for column in 0usize..GLYPH_WIDTH {
            if (*mask & (1u8 << (GLYPH_WIDTH - 1 - column))) == 0 {
                continue;
            }
            let pixel_x = cell_x + offset_x as isize + column.saturating_mul(scale) as isize;
            for dy in 0..scale {
                for dx in 0..scale {
                    let Ok(pixel_x) = usize::try_from(pixel_x.saturating_add(dx as isize)) else {
                        continue;
                    };
                    let pixel_ink = raster_glyph_pixel_ink(ink, scale, dx, dy);
                    set_rgba_pixel(
                        pixels,
                        width,
                        pixel_x,
                        cell_y
                            .saturating_add(offset_y)
                            .saturating_add(row.saturating_mul(scale))
                            .saturating_add(dy),
                        [pixel_ink, pixel_ink, pixel_ink, 255],
                    );
                }
            }
        }
    }
}

fn raster_glyph_pixel_ink(ink: u8, scale: usize, dx: usize, dy: usize) -> u8 {
    if scale <= 1 || (dx == 0 && dy == 0) {
        return ink;
    }
    if ink < 128 {
        ink.saturating_add(72)
    } else {
        ink.saturating_sub(72)
    }
}

fn contrasting_glyph_ink(
    pixels: &[u8],
    width: usize,
    cell_x: usize,
    cell_y: usize,
    ch: char,
    ink: u8,
) -> u8 {
    let mut total = 0usize;
    let mut count = 0usize;
    for (row, mask) in glyph_rows(ch).iter().enumerate() {
        for column in 0..5 {
            if (mask & (1 << (4 - column))) != 0 {
                continue;
            }
            let pixel_x = cell_x.saturating_add(1 + column);
            let pixel_y = cell_y.saturating_add(2 + row);
            let index = pixel_y.saturating_mul(width).saturating_add(pixel_x);
            if let Some(pixel) = pixels.get(index) {
                total = total.saturating_add(*pixel as usize);
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        return ink;
    }
    let background = total / count;
    if ink.abs_diff(background as u8) >= 96 {
        ink
    } else if background < 128 {
        255
    } else {
        0
    }
}

fn contrasting_rgba_glyph_ink(
    pixels: &[u8],
    width: usize,
    cell_x: usize,
    cell_y: usize,
    ch: char,
    ink: u8,
) -> u8 {
    let mut total = 0usize;
    let mut count = 0usize;
    for (row, mask) in glyph_rows(ch).iter().enumerate() {
        for column in 0..5 {
            if (mask & (1 << (4 - column))) != 0 {
                continue;
            }
            let pixel_x = cell_x.saturating_add(1 + column);
            let pixel_y = cell_y.saturating_add(2 + row);
            let index = pixel_y
                .saturating_mul(width)
                .saturating_add(pixel_x)
                .saturating_mul(4);
            let Some(pixel) = pixels.get(index..index.saturating_add(3)) else {
                continue;
            };
            total = total.saturating_add(rgb_to_luma(pixel[0], pixel[1], pixel[2]) as usize);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return ink;
    }
    let background = total / count;
    if ink.abs_diff(background as u8) >= 96 {
        ink
    } else if background < 128 {
        255
    } else {
        0
    }
}

fn fill_raster_rect(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    value: u8,
) {
    for row in y..y.saturating_add(height) {
        for column in x..x.saturating_add(width) {
            set_raster_pixel(pixels, raster_width, column, row, value);
        }
    }
}

fn fill_rgba_rect(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    value: [u8; 4],
) {
    for row in y..y.saturating_add(height) {
        for column in x..x.saturating_add(width) {
            set_rgba_pixel(pixels, raster_width, column, row, value);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_decoded_image_region_rgba(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    source_offset_x: usize,
    source_offset_y: usize,
    full_width: usize,
    full_height: usize,
    decoded: &DecodedImage,
) {
    if width == 0
        || height == 0
        || full_width == 0
        || full_height == 0
        || decoded.width == 0
        || decoded.height == 0
        || !decoded_image_has_rgb(decoded)
    {
        return;
    }
    for row in 0..height {
        let (source_y_start, source_y_end) = scaled_sample_range(
            source_offset_y.saturating_add(row),
            source_offset_y.saturating_add(row).saturating_add(1),
            decoded.height,
            full_height,
        );
        for column in 0..width {
            let (source_x_start, source_x_end) = scaled_sample_range(
                source_offset_x.saturating_add(column),
                source_offset_x.saturating_add(column).saturating_add(1),
                decoded.width,
                full_width,
            );
            let Some(rgb) = averaged_decoded_image_rgb_sample(
                decoded,
                source_x_start,
                source_x_end,
                source_y_start,
                source_y_end,
            ) else {
                continue;
            };
            set_rgba_pixel(
                pixels,
                raster_width,
                x.saturating_add(column),
                y.saturating_add(row),
                [rgb[0], rgb[1], rgb[2], 255],
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_decoded_image_region(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    source_offset_x: usize,
    source_offset_y: usize,
    full_width: usize,
    full_height: usize,
    decoded: &DecodedImage,
) {
    if width == 0
        || height == 0
        || full_width == 0
        || full_height == 0
        || decoded.width == 0
        || decoded.height == 0
    {
        return;
    }
    for row in 0..height {
        let (source_y_start, source_y_end) = scaled_sample_range(
            source_offset_y.saturating_add(row),
            source_offset_y.saturating_add(row).saturating_add(1),
            decoded.height,
            full_height,
        );
        for column in 0..width {
            let (source_x_start, source_x_end) = scaled_sample_range(
                source_offset_x.saturating_add(column),
                source_offset_x.saturating_add(column).saturating_add(1),
                decoded.width,
                full_width,
            );
            let value = averaged_decoded_image_sample(
                decoded,
                source_x_start,
                source_x_end,
                source_y_start,
                source_y_end,
            );
            set_raster_pixel(
                pixels,
                raster_width,
                x.saturating_add(column),
                y.saturating_add(row),
                value,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_background_image_region_rgba(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    visible_offset_x: usize,
    visible_offset_y: usize,
    container_width: usize,
    container_height: usize,
    size: BackgroundImageSize,
    position: BackgroundImagePosition,
    repeat: BackgroundImageRepeat,
    decoded: &DecodedImage,
) {
    if width == 0
        || height == 0
        || container_width == 0
        || container_height == 0
        || decoded.width == 0
        || decoded.height == 0
        || !decoded_image_has_rgb(decoded)
    {
        return;
    }
    let (tile_width, tile_height) =
        background_image_tile_size(container_width, container_height, size, decoded);
    if tile_width == 0 || tile_height == 0 {
        return;
    }
    let tile_x = background_position_offset(container_width, tile_width, position.x_percent);
    let tile_y = background_position_offset(container_height, tile_height, position.y_percent);
    for row in 0..height {
        let local_y = visible_offset_y.saturating_add(row) as i64;
        let Some(tile_local_y) =
            background_tile_local_coordinate(local_y, tile_y, tile_height, repeat)
        else {
            continue;
        };
        let (source_y_start, source_y_end) = scaled_sample_range(
            tile_local_y,
            tile_local_y.saturating_add(1),
            decoded.height,
            tile_height,
        );
        for column in 0..width {
            let local_x = visible_offset_x.saturating_add(column) as i64;
            let Some(tile_local_x) =
                background_tile_local_coordinate(local_x, tile_x, tile_width, repeat)
            else {
                continue;
            };
            let (source_x_start, source_x_end) = scaled_sample_range(
                tile_local_x,
                tile_local_x.saturating_add(1),
                decoded.width,
                tile_width,
            );
            let Some(rgb) = averaged_decoded_image_rgb_sample(
                decoded,
                source_x_start,
                source_x_end,
                source_y_start,
                source_y_end,
            ) else {
                continue;
            };
            set_rgba_pixel(
                pixels,
                raster_width,
                x.saturating_add(column),
                y.saturating_add(row),
                [rgb[0], rgb[1], rgb[2], 255],
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_background_image_region(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    visible_offset_x: usize,
    visible_offset_y: usize,
    container_width: usize,
    container_height: usize,
    size: BackgroundImageSize,
    position: BackgroundImagePosition,
    repeat: BackgroundImageRepeat,
    decoded: &DecodedImage,
) {
    if width == 0
        || height == 0
        || container_width == 0
        || container_height == 0
        || decoded.width == 0
        || decoded.height == 0
    {
        return;
    }
    let (tile_width, tile_height) =
        background_image_tile_size(container_width, container_height, size, decoded);
    if tile_width == 0 || tile_height == 0 {
        return;
    }
    let tile_x = background_position_offset(container_width, tile_width, position.x_percent);
    let tile_y = background_position_offset(container_height, tile_height, position.y_percent);
    for row in 0..height {
        let local_y = visible_offset_y.saturating_add(row) as i64;
        let Some(tile_local_y) =
            background_tile_local_coordinate(local_y, tile_y, tile_height, repeat)
        else {
            continue;
        };
        let (source_y_start, source_y_end) = scaled_sample_range(
            tile_local_y,
            tile_local_y.saturating_add(1),
            decoded.height,
            tile_height,
        );
        for column in 0..width {
            let local_x = visible_offset_x.saturating_add(column) as i64;
            let Some(tile_local_x) =
                background_tile_local_coordinate(local_x, tile_x, tile_width, repeat)
            else {
                continue;
            };
            let (source_x_start, source_x_end) = scaled_sample_range(
                tile_local_x,
                tile_local_x.saturating_add(1),
                decoded.width,
                tile_width,
            );
            let value = averaged_decoded_image_sample(
                decoded,
                source_x_start,
                source_x_end,
                source_y_start,
                source_y_end,
            );
            set_raster_pixel(
                pixels,
                raster_width,
                x.saturating_add(column),
                y.saturating_add(row),
                value,
            );
        }
    }
}

fn decoded_image_has_rgb(decoded: &DecodedImage) -> bool {
    decoded.rgb_pixels.as_ref().is_some_and(|pixels| {
        pixels.len()
            == decoded
                .width
                .saturating_mul(decoded.height)
                .saturating_mul(3)
    })
}

fn scaled_sample_range(
    start: usize,
    end: usize,
    source_extent: usize,
    target_extent: usize,
) -> (usize, usize) {
    if source_extent == 0 || target_extent == 0 {
        return (0, 0);
    }
    let start = start.saturating_mul(source_extent) / target_extent;
    let end = scale_ceil(end, source_extent, target_extent)
        .max(start.saturating_add(1))
        .min(source_extent);
    (start.min(source_extent.saturating_sub(1)), end)
}

fn averaged_decoded_image_rgb_sample(
    decoded: &DecodedImage,
    start_x: usize,
    end_x: usize,
    start_y: usize,
    end_y: usize,
) -> Option<[u8; 3]> {
    let rgb_pixels = decoded.rgb_pixels.as_ref()?;
    if !decoded_image_has_rgb(decoded) {
        return None;
    }
    let start_x = start_x.min(decoded.width.saturating_sub(1));
    let end_x = end_x.min(decoded.width).max(start_x.saturating_add(1));
    let start_y = start_y.min(decoded.height.saturating_sub(1));
    let end_y = end_y.min(decoded.height).max(start_y.saturating_add(1));
    let mut red = 0usize;
    let mut green = 0usize;
    let mut blue = 0usize;
    let mut count = 0usize;
    for source_y in start_y..end_y {
        for source_x in start_x..end_x {
            let index = source_y
                .saturating_mul(decoded.width)
                .saturating_add(source_x)
                .saturating_mul(3);
            let Some(pixel) = rgb_pixels.get(index..index.saturating_add(3)) else {
                continue;
            };
            red = red.saturating_add(pixel[0] as usize);
            green = green.saturating_add(pixel[1] as usize);
            blue = blue.saturating_add(pixel[2] as usize);
            count = count.saturating_add(1);
        }
    }
    (count > 0).then_some([
        (red / count).min(u8::MAX as usize) as u8,
        (green / count).min(u8::MAX as usize) as u8,
        (blue / count).min(u8::MAX as usize) as u8,
    ])
}

fn averaged_decoded_image_sample(
    decoded: &DecodedImage,
    start_x: usize,
    end_x: usize,
    start_y: usize,
    end_y: usize,
) -> u8 {
    let start_x = start_x.min(decoded.width.saturating_sub(1));
    let end_x = end_x.min(decoded.width).max(start_x.saturating_add(1));
    let start_y = start_y.min(decoded.height.saturating_sub(1));
    let end_y = end_y.min(decoded.height).max(start_y.saturating_add(1));
    let mut total = 0usize;
    let mut count = 0usize;
    for source_y in start_y..end_y {
        for source_x in start_x..end_x {
            if let Some(value) = decoded.pixels.get(
                source_y
                    .saturating_mul(decoded.width)
                    .saturating_add(source_x),
            ) {
                total = total.saturating_add(*value as usize);
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        255
    } else {
        (total / count).min(u8::MAX as usize) as u8
    }
}

fn background_image_tile_size(
    container_width: usize,
    container_height: usize,
    size: BackgroundImageSize,
    decoded: &DecodedImage,
) -> (usize, usize) {
    match size {
        BackgroundImageSize::Auto => (decoded.width.max(1), decoded.height.max(1)),
        BackgroundImageSize::Cover => {
            if container_width.saturating_mul(decoded.height)
                >= container_height.saturating_mul(decoded.width)
            {
                (
                    container_width.max(1),
                    scale_ceil(decoded.height, container_width, decoded.width).max(1),
                )
            } else {
                (
                    scale_ceil(decoded.width, container_height, decoded.height).max(1),
                    container_height.max(1),
                )
            }
        }
        BackgroundImageSize::Contain => {
            if container_width.saturating_mul(decoded.height)
                <= container_height.saturating_mul(decoded.width)
            {
                (
                    container_width.max(1),
                    scale_ceil(decoded.height, container_width, decoded.width).max(1),
                )
            } else {
                (
                    scale_ceil(decoded.width, container_height, decoded.height).max(1),
                    container_height.max(1),
                )
            }
        }
    }
}

fn scale_ceil(value: usize, numerator: usize, denominator: usize) -> usize {
    if denominator == 0 {
        return 0;
    }
    let value = value as u128;
    let numerator = numerator as u128;
    let denominator = denominator as u128;
    value
        .saturating_mul(numerator)
        .saturating_add(denominator.saturating_sub(1))
        .saturating_div(denominator)
        .min(usize::MAX as u128) as usize
}

fn background_position_offset(container: usize, tile: usize, percent: i32) -> i64 {
    let delta = container as i128 - tile as i128;
    delta
        .saturating_mul(percent as i128)
        .saturating_div(100)
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn background_tile_local_coordinate(
    local: i64,
    tile_offset: i64,
    tile_size: usize,
    repeat: BackgroundImageRepeat,
) -> Option<usize> {
    let tile_size = i64::try_from(tile_size).ok()?;
    let relative = local.saturating_sub(tile_offset);
    match repeat {
        BackgroundImageRepeat::Repeat => Some(relative.rem_euclid(tile_size) as usize),
        BackgroundImageRepeat::NoRepeat => {
            (relative >= 0 && relative < tile_size).then_some(relative as usize)
        }
    }
}

fn set_raster_pixel(pixels: &mut [u8], width: usize, x: usize, y: usize, value: u8) {
    let Some(index) = y.checked_mul(width).and_then(|row| row.checked_add(x)) else {
        return;
    };
    if let Some(pixel) = pixels.get_mut(index) {
        *pixel = value;
    }
}

fn set_rgba_pixel(pixels: &mut [u8], width: usize, x: usize, y: usize, value: [u8; 4]) {
    let Some(index) = y
        .checked_mul(width)
        .and_then(|row| row.checked_add(x))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    if let Some(pixel) = pixels.get_mut(index..index.saturating_add(4)) {
        pixel.copy_from_slice(&value);
    }
}

fn glyph_rows(ch: char) -> [u8; 7] {
    if let Some(rows) = lowercase_glyph_rows(ch) {
        return rows;
    }
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b01010, 0b10001,
        ],
        'Y' => [
            0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
        '6' => [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        ',' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110, 0b01100,
        ],
        ':' => [
            0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000,
        ],
        ';' => [
            0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b00100, 0b01000,
        ],
        '!' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100,
        ],
        '?' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100,
        ],
        '-' => [
            0b00000, 0b00000, 0b00000, 0b11110, 0b00000, 0b00000, 0b00000,
        ],
        '_' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b11111,
        ],
        '/' => [
            0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
        ],
        '\\' => [
            0b10000, 0b01000, 0b01000, 0b00100, 0b00010, 0b00010, 0b00001,
        ],
        '&' => [
            0b01100, 0b10010, 0b10100, 0b01000, 0b10101, 0b10010, 0b01101,
        ],
        '+' => [
            0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000,
        ],
        '=' => [
            0b00000, 0b00000, 0b11111, 0b00000, 0b11111, 0b00000, 0b00000,
        ],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '[' => [
            0b01110, 0b01000, 0b01000, 0b01000, 0b01000, 0b01000, 0b01110,
        ],
        ']' => [
            0b01110, 0b00010, 0b00010, 0b00010, 0b00010, 0b00010, 0b01110,
        ],
        '\'' => [
            0b00100, 0b00100, 0b01000, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '"' => [
            0b01010, 0b01010, 0b01010, 0b00000, 0b00000, 0b00000, 0b00000,
        ],
        '@' => [
            0b01110, 0b10001, 0b10111, 0b10101, 0b10111, 0b10000, 0b01110,
        ],
        '#' => [
            0b01010, 0b11111, 0b01010, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        '%' => [
            0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011,
        ],
        '*' => [
            0b00000, 0b10101, 0b01110, 0b11111, 0b01110, 0b10101, 0b00000,
        ],
        '|' => [
            0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        '<' => [
            0b00010, 0b00100, 0b01000, 0b10000, 0b01000, 0b00100, 0b00010,
        ],
        '>' => [
            0b01000, 0b00100, 0b00010, 0b00001, 0b00010, 0b00100, 0b01000,
        ],
        ' ' | '\t' | '\n' | '\r' => [0; 7],
        _ => [
            0b11111, 0b10001, 0b00010, 0b00100, 0b00010, 0b10001, 0b11111,
        ],
    }
}

fn lowercase_glyph_rows(ch: char) -> Option<[u8; 7]> {
    Some(match ch {
        'a' => [
            0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111,
        ],
        'b' => [
            0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b11110,
        ],
        'c' => [
            0b00000, 0b00000, 0b01110, 0b10000, 0b10000, 0b10000, 0b01110,
        ],
        'd' => [
            0b00001, 0b00001, 0b01101, 0b10011, 0b10001, 0b10001, 0b01111,
        ],
        'e' => [
            0b00000, 0b00000, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110,
        ],
        'f' => [
            0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000,
        ],
        'g' => [
            0b00000, 0b00000, 0b01111, 0b10001, 0b01111, 0b00001, 0b01110,
        ],
        'h' => [
            0b10000, 0b10000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001,
        ],
        'i' => [
            0b00100, 0b00000, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'j' => [
            0b00010, 0b00000, 0b00110, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'k' => [
            0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010,
        ],
        'l' => [
            0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'm' => [
            0b00000, 0b00000, 0b11010, 0b10101, 0b10101, 0b10101, 0b10101,
        ],
        'n' => [
            0b00000, 0b00000, 0b10110, 0b11001, 0b10001, 0b10001, 0b10001,
        ],
        'o' => [
            0b00000, 0b00000, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'p' => [
            0b00000, 0b00000, 0b11110, 0b10001, 0b11110, 0b10000, 0b10000,
        ],
        'q' => [
            0b00000, 0b00000, 0b01111, 0b10001, 0b01111, 0b00001, 0b00001,
        ],
        'r' => [
            0b00000, 0b00000, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000,
        ],
        's' => [
            0b00000, 0b00000, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110,
        ],
        't' => [
            0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110,
        ],
        'u' => [
            0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b10011, 0b01101,
        ],
        'v' => [
            0b00000, 0b00000, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'w' => [
            0b00000, 0b00000, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'x' => [
            0b00000, 0b00000, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001,
        ],
        'y' => [
            0b00000, 0b00000, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110,
        ],
        'z' => [
            0b00000, 0b00000, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        _ => return None,
    })
}

pub fn verify_browser_fixtures(manifest_path: &Path) -> Result<BrowserFixtureReport> {
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("read browser fixture manifest {}", manifest_path.display()))?;
    let manifest: BrowserFixtureManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut failures = Vec::new();

    for fixture in &manifest.fixtures {
        let path = if fixture.path.is_absolute() {
            fixture.path.clone()
        } else {
            base_dir.join(&fixture.path)
        };
        let name = fixture
            .name
            .clone()
            .unwrap_or_else(|| fixture.path.display().to_string());

        match verify_browser_fixture(&name, &path, fixture) {
            Ok(()) => {}
            Err(error) => failures.push(BrowserFixtureFailure {
                name,
                path: path.display().to_string(),
                reason: error.to_string(),
            }),
        }
    }

    Ok(BrowserFixtureReport {
        fixture_count: manifest.fixtures.len(),
        passed: manifest.fixtures.len().saturating_sub(failures.len()),
        failed: failures.len(),
        failures,
    })
}

fn verify_browser_fixture(name: &str, path: &Path, fixture: &BrowserFixture) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read browser fixture {name}"))?;
    let render = render_browser_fixture(path, &bytes, fixture)?;

    if let Some(expected_title) = &fixture.expected_title
        && render.title != *expected_title
    {
        bail!(
            "title mismatch: expected {:?}, got {:?}",
            expected_title,
            render.title
        );
    }

    if let Some(expected_text) = &fixture.expected_text
        && render.text != *expected_text
    {
        bail!(
            "text mismatch: expected {:?}, got {:?}",
            expected_text,
            render.text
        );
    }

    if let Some(expected_display_list) = &fixture.expected_display_list
        && render.display_list != *expected_display_list
    {
        bail!(
            "display list mismatch: expected {:?}, got {:?}",
            expected_display_list,
            render.display_list
        );
    }

    for expected_hit_test in &fixture.expected_hit_tests {
        let actual = hit_test_render(&render, expected_hit_test.x, expected_hit_test.y);
        if !hit_test_expectation_matches(actual.hit.as_ref(), expected_hit_test.expected.as_ref()) {
            bail!(
                "hit test mismatch at ({}, {}): expected {:?}, got {:?}",
                expected_hit_test.x,
                expected_hit_test.y,
                expected_hit_test.expected,
                actual.hit
            );
        }
    }

    if let Some(expected_layers) = &fixture.expected_layers {
        let actual = layer_tree_render(&render);
        if actual.layers != *expected_layers {
            bail!(
                "layer tree mismatch: expected {:?}, got {:?}",
                expected_layers,
                actual.layers
            );
        }
    }

    let needs_raster_check = fixture.expected_raster_hash.is_some()
        || fixture.expected_screenshot_hash.is_some()
        || fixture.expected_visible_command_count.is_some()
        || fixture.expected_culled_command_count.is_some();
    if needs_raster_check {
        let raster_options =
            browser_fixture_raster_options(fixture, BrowserRasterOptions::default());
        let raster = rasterize_render(&render, raster_options)?;
        let report = raster_report(&render, &raster, raster_options);
        if let Some(expected_raster_hash) = &fixture.expected_raster_hash {
            let actual = raster.pixel_hash();
            if actual != *expected_raster_hash {
                bail!(
                    "raster hash mismatch: expected {:?}, got {:?}",
                    expected_raster_hash,
                    actual
                );
            }
        }
        if let Some(expected_screenshot_hash) = &fixture.expected_screenshot_hash {
            let screenshot = BrowserRgbaRaster::from_grayscale(&raster);
            let actual = screenshot.pixel_hash();
            if actual != *expected_screenshot_hash {
                bail!(
                    "screenshot hash mismatch: expected {:?}, got {:?}",
                    expected_screenshot_hash,
                    actual
                );
            }
        }
        if let Some(expected_visible_command_count) = fixture.expected_visible_command_count
            && report.visible_command_count != expected_visible_command_count
        {
            bail!(
                "visible raster command count mismatch: expected {}, got {}",
                expected_visible_command_count,
                report.visible_command_count
            );
        }
        if let Some(expected_culled_command_count) = fixture.expected_culled_command_count
            && report.culled_command_count != expected_culled_command_count
        {
            bail!(
                "culled raster command count mismatch: expected {}, got {}",
                expected_culled_command_count,
                report.culled_command_count
            );
        }
    }

    Ok(())
}

pub(crate) fn hit_test_expectation_matches(
    actual: Option<&BrowserHitTest>,
    expected: Option<&BrowserHitTestExpectation>,
) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (Some(actual), Some(expected)) => {
            actual.kind == expected.kind
                && expected
                    .command_index
                    .is_none_or(|command_index| actual.command_index == command_index)
                && expected
                    .text
                    .as_ref()
                    .is_none_or(|text| actual.text.as_ref() == Some(text))
                && expected
                    .alt
                    .as_ref()
                    .is_none_or(|alt| actual.alt.as_ref() == Some(alt))
                && expected
                    .url
                    .as_ref()
                    .is_none_or(|url| actual.url.as_ref() == Some(url))
                && expected
                    .shade
                    .is_none_or(|shade| actual.shade == Some(shade))
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct BrowserVisualRunOptions<'a> {
    raster: BrowserRasterOptions,
    artifact_dir: Option<&'a Path>,
    baseline_dir: Option<&'a Path>,
    max_diff_pixels: Option<usize>,
    max_diff_ratio: Option<f64>,
}

pub fn verify_browser_visuals(
    manifest_path: &Path,
    artifact_dir: Option<&Path>,
    baseline_dir: Option<&Path>,
    require_all_baselines: bool,
    max_diff_pixels: Option<usize>,
    max_diff_ratio: Option<f64>,
) -> Result<BrowserVisualReport> {
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("read browser fixture manifest {}", manifest_path.display()))?;
    let manifest: BrowserFixtureManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    if let Some(dir) = artifact_dir {
        fs::create_dir_all(dir)
            .with_context(|| format!("create visual artifact dir {}", dir.display()))?;
    }

    let mut comparisons = Vec::new();
    let mut failures = Vec::new();
    let mut missing_baseline = 0usize;
    let options = BrowserVisualRunOptions {
        raster: BrowserRasterOptions::default(),
        artifact_dir,
        baseline_dir,
        max_diff_pixels,
        max_diff_ratio,
    };

    for (index, fixture) in manifest.fixtures.iter().enumerate() {
        let path = if fixture.path.is_absolute() {
            fixture.path.clone()
        } else {
            base_dir.join(&fixture.path)
        };
        let name = fixture
            .name
            .clone()
            .unwrap_or_else(|| fixture.path.display().to_string());
        let expected_raster_hash = fixture.expected_raster_hash.clone();
        if expected_raster_hash.is_none() {
            missing_baseline += 1;
            if require_all_baselines {
                failures.push(BrowserFixtureFailure {
                    name: name.clone(),
                    path: path.display().to_string(),
                    reason: "missing expected_raster_hash baseline".to_owned(),
                });
            }
        }

        match visual_comparison_for_fixture(
            index,
            &name,
            &path,
            fixture,
            expected_raster_hash.as_deref(),
            &options,
        ) {
            Ok(comparison) => {
                if comparison.matched == Some(false) {
                    failures.push(BrowserFixtureFailure {
                        name: name.clone(),
                        path: path.display().to_string(),
                        reason: format!(
                            "raster hash mismatch: expected {:?}, got {:?}",
                            expected_raster_hash.as_deref().unwrap_or_default(),
                            comparison.actual_raster_hash
                        ),
                    });
                }
                if comparison.diff_passed == Some(false) {
                    failures.push(BrowserFixtureFailure {
                        name: name.clone(),
                        path: path.display().to_string(),
                        reason: format!(
                            "visual pixel diff exceeded threshold: diff_pixels={} diff_ratio={:.6}",
                            comparison.diff_pixels.unwrap_or_default(),
                            comparison.diff_ratio.unwrap_or_default()
                        ),
                    });
                }
                comparisons.push(comparison);
            }
            Err(error) => failures.push(BrowserFixtureFailure {
                name,
                path: path.display().to_string(),
                reason: error.to_string(),
            }),
        }
    }

    Ok(BrowserVisualReport {
        fixture_count: manifest.fixtures.len(),
        checked: comparisons
            .iter()
            .filter(|comparison| comparison.expected_raster_hash.is_some())
            .count(),
        passed: comparisons
            .iter()
            .filter(|comparison| comparison.matched == Some(true))
            .count(),
        failed: failures.len(),
        missing_baseline,
        artifact_dir: artifact_dir.map(|path| path.display().to_string()),
        baseline_dir: baseline_dir.map(|path| path.display().to_string()),
        diff_checked: comparisons
            .iter()
            .filter(|comparison| comparison.diff_pixels.is_some())
            .count(),
        diff_passed: comparisons
            .iter()
            .filter(|comparison| comparison.diff_passed == Some(true))
            .count(),
        diff_failed: comparisons
            .iter()
            .filter(|comparison| comparison.diff_passed == Some(false))
            .count(),
        max_diff_pixels: baseline_dir.map(|_| max_diff_pixels.unwrap_or(0)),
        max_diff_ratio: baseline_dir.map(|_| max_diff_ratio.unwrap_or(0.0)),
        comparisons,
        failures,
    })
}

fn visual_comparison_for_fixture(
    index: usize,
    name: &str,
    path: &Path,
    fixture: &BrowserFixture,
    expected_raster_hash: Option<&str>,
    options: &BrowserVisualRunOptions<'_>,
) -> Result<BrowserVisualComparison> {
    let bytes = fs::read(path).with_context(|| format!("read browser fixture {name}"))?;
    let render = render_browser_fixture(path, &bytes, fixture)?;
    let raster_options = browser_fixture_raster_options(fixture, options.raster);
    let raster = rasterize_render(&render, raster_options)?;
    let raster_summary = raster_report(&render, &raster, raster_options);
    let actual_raster_hash = raster.pixel_hash();
    let artifact_name = format!("{index:03}-{}.pgm", artifact_slug(name));
    let artifact = if let Some(dir) = options.artifact_dir {
        let path = dir.join(&artifact_name);
        fs::write(&path, raster.encode_pgm())
            .with_context(|| format!("write visual artifact {}", path.display()))?;
        Some(path.display().to_string())
    } else {
        None
    };
    let baseline_path = options.baseline_dir.map(|dir| dir.join(&artifact_name));
    let diff = if let Some(path) = baseline_path.as_ref() {
        Some(
            compare_raster_with_pgm(&raster, path)
                .with_context(|| format!("compare visual baseline artifact {}", path.display()))?,
        )
    } else {
        None
    };
    let diff_artifact = match (options.artifact_dir, diff.as_ref()) {
        (Some(dir), Some(diff)) => {
            let path = dir.join(format!("{index:03}-{}-diff.pgm", artifact_slug(name)));
            fs::write(&path, encode_diff_pgm(diff))
                .with_context(|| format!("write visual diff artifact {}", path.display()))?;
            Some(path.display().to_string())
        }
        _ => None,
    };
    let diff_passed = diff
        .as_ref()
        .map(|diff| diff_within_threshold(diff, options.max_diff_pixels, options.max_diff_ratio));

    Ok(BrowserVisualComparison {
        name: name.to_owned(),
        path: path.display().to_string(),
        width: raster.width,
        height: raster.height,
        display_command_count: raster_summary.display_command_count,
        visible_command_count: raster_summary.visible_command_count,
        culled_command_count: raster_summary.culled_command_count,
        raster_viewport_x: raster_summary.raster_viewport_x,
        raster_viewport_y: raster_summary.raster_viewport_y,
        raster_viewport_width: raster_summary.raster_viewport_width,
        raster_viewport_height: raster_summary.raster_viewport_height,
        non_background_pixels: raster.non_background_pixels(),
        expected_raster_hash: expected_raster_hash.map(str::to_owned),
        matched: expected_raster_hash.map(|expected| actual_raster_hash == expected),
        actual_raster_hash,
        artifact,
        baseline_artifact: baseline_path.map(|path| path.display().to_string()),
        diff_artifact,
        diff_pixels: diff.as_ref().map(|diff| diff.diff_pixels),
        diff_ratio: diff.as_ref().map(|diff| diff.diff_ratio),
        diff_passed,
    })
}

fn artifact_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "fixture".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub fn compare_browser_fixtures_with_chromium(
    manifest_path: &Path,
) -> Result<BrowserChromiumParityReport> {
    let chrome = chrome_program().context("Chrome/Chromium executable not found")?;
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("read browser fixture manifest {}", manifest_path.display()))?;
    let manifest: BrowserFixtureManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut comparisons = Vec::with_capacity(manifest.fixtures.len());
    let mut failures = Vec::new();

    for fixture in &manifest.fixtures {
        let path = if fixture.path.is_absolute() {
            fixture.path.clone()
        } else {
            base_dir.join(&fixture.path)
        };
        let name = fixture
            .name
            .clone()
            .unwrap_or_else(|| fixture.path.display().to_string());

        match compare_browser_fixture_with_chromium(&chrome, &name, &path, fixture) {
            Ok(comparison) => {
                if !comparison.title_match || !comparison.text_match {
                    failures.push(BrowserFixtureFailure {
                        name: name.clone(),
                        path: path.display().to_string(),
                        reason: parity_failure_reason(&comparison),
                    });
                }
                comparisons.push(comparison);
            }
            Err(error) => failures.push(BrowserFixtureFailure {
                name,
                path: path.display().to_string(),
                reason: error.to_string(),
            }),
        }
    }

    Ok(BrowserChromiumParityReport {
        fixture_count: manifest.fixtures.len(),
        passed: manifest.fixtures.len().saturating_sub(failures.len()),
        failed: failures.len(),
        chrome: chrome_version(),
        comparisons,
        failures,
    })
}

fn compare_browser_fixture_with_chromium(
    chrome: &str,
    name: &str,
    path: &Path,
    fixture: &BrowserFixture,
) -> Result<BrowserChromiumParityComparison> {
    let bytes = fs::read(path).with_context(|| format!("read browser fixture {name}"))?;
    let render = render_browser_fixture(path, &bytes, fixture)?;
    let chromium = chromium_static_render(chrome, path, &bytes, fixture.click_selector.as_deref())?;
    let brutal_text = normalize_browser_parity_text(&render.text);
    let chromium_text = normalize_browser_parity_text(&chromium.text);
    let brutal_title = render.title.trim().to_owned();
    let chromium_title = chromium.title.trim().to_owned();

    Ok(BrowserChromiumParityComparison {
        name: name.to_owned(),
        path: path.display().to_string(),
        title_match: brutal_title == chromium_title,
        text_match: brutal_text == chromium_text,
        brutal_title,
        chromium_title,
        brutal_text,
        chromium_text,
    })
}

pub(crate) fn render_browser_fixture(
    path: &Path,
    bytes: &[u8],
    fixture: &BrowserFixture,
) -> Result<BrowserRender> {
    Ok(render_browser_fixture_profiled(path, bytes, fixture)?.render)
}

pub(crate) fn render_browser_fixture_profiled(
    path: &Path,
    bytes: &[u8],
    fixture: &BrowserFixture,
) -> Result<BrowserProfiledRender> {
    let source = path.display().to_string();
    let options = BrowserRenderOptions {
        width: fixture.width,
        ..BrowserRenderOptions::default()
    };
    if !fixture.external_scripts {
        if let Some(selector) = fixture.click_selector.as_deref() {
            return render_html_prepared_profiled(
                &source,
                bytes,
                options,
                RenderPreparation {
                    external_css: &[],
                    external_scripts: &[],
                    click_target: Some(RenderClickTarget::Selector(selector)),
                    local_storage: None,
                    session_storage: None,
                    cached_images: &[],
                },
            );
        }
        return render_html_prepared_profiled(
            &source,
            bytes,
            options,
            RenderPreparation {
                external_css: &[],
                external_scripts: &[],
                click_target: None,
                local_storage: None,
                session_storage: None,
                cached_images: &[],
            },
        );
    }

    let first_pass = render_html_prepared_profiled(
        &source,
        bytes,
        options,
        RenderPreparation {
            external_css: &[],
            external_scripts: &[],
            click_target: None,
            local_storage: None,
            session_storage: None,
            cached_images: &[],
        },
    )?;
    let script_text = local_external_script_texts(&first_pass.render)?;
    let mut second_pass = render_html_prepared_profiled(
        &source,
        bytes,
        options,
        RenderPreparation {
            external_css: &[],
            external_scripts: &script_text,
            click_target: fixture
                .click_selector
                .as_deref()
                .map(RenderClickTarget::Selector),
            local_storage: None,
            session_storage: None,
            cached_images: &[],
        },
    )?;
    second_pass.timings = first_pass.timings.add(second_pass.timings);
    Ok(second_pass)
}

fn local_external_script_texts(render: &BrowserRender) -> Result<Vec<String>> {
    render
        .resources
        .iter()
        .filter(|resource| resource.kind == "script")
        .map(read_local_script_resource)
        .collect()
}

fn read_local_script_resource(resource: &BrowserResource) -> Result<String> {
    if resource.resolved.starts_with("http://") || resource.resolved.starts_with("https://") {
        bail!(
            "fixture external script must be local, got {}",
            resource.resolved
        );
    }
    let bytes = if resource.resolved.starts_with("file://") {
        let url = Url::parse(&resource.resolved)
            .with_context(|| format!("parse script URL {}", resource.resolved))?;
        let path = url.to_file_path().map_err(|_| {
            anyhow::anyhow!(
                "script file URL cannot be converted to a local path: {}",
                resource.resolved
            )
        })?;
        fs::read(&path).with_context(|| format!("read script {}", path.display()))?
    } else {
        let path = local_path_without_url_parts(&resource.resolved);
        fs::read(path).with_context(|| format!("read script {}", resource.resolved))?
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parity_failure_reason(comparison: &BrowserChromiumParityComparison) -> String {
    match (comparison.title_match, comparison.text_match) {
        (false, false) => "title and text mismatch".to_owned(),
        (false, true) => "title mismatch".to_owned(),
        (true, false) => "text mismatch".to_owned(),
        (true, true) => "matched".to_owned(),
    }
}

fn chromium_static_render(
    chrome: &str,
    fixture_path: &Path,
    html: &[u8],
    click_selector: Option<&str>,
) -> Result<ChromiumStaticRender> {
    let html = String::from_utf8_lossy(html);
    let base_href = chromium_fixture_base_href(fixture_path);
    let wrapper = chromium_static_wrapper_html(&html, base_href.as_deref(), click_selector)?;
    let path = std::env::temp_dir().join(format!(
        "brutal-browser-chromium-static-{}.html",
        std::process::id()
    ));
    fs::write(&path, wrapper).with_context(|| format!("write {}", path.display()))?;

    let output = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-background-networking")
        .arg("--disable-default-apps")
        .arg("--disable-extensions")
        .arg("--run-all-compositor-stages-before-draw")
        .arg("--virtual-time-budget=1000")
        .arg("--dump-dom")
        .arg(format!("file://{}", path.display()))
        .output()
        .with_context(|| format!("run Chromium static parity {}", path.display()))?;
    let _ = fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(json) = extract_chromium_result_json(&stdout) else {
        ensure!(
            output.status.success(),
            "Chromium static parity failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        bail!("Chromium static parity output did not contain JSON");
    };
    Ok(serde_json::from_str(json)?)
}

fn chromium_fixture_base_href(fixture_path: &Path) -> Option<String> {
    let parent = fixture_path.parent()?;
    let parent = fs::canonicalize(parent).ok()?;
    Url::from_directory_path(parent)
        .ok()
        .map(|url| url.to_string())
}

fn chromium_static_wrapper_html(
    html: &str,
    base_href: Option<&str>,
    click_selector: Option<&str>,
) -> Result<String> {
    let base = base_href
        .map(|href| {
            format!(
                r#"<base href="{}">"#,
                html_escape::encode_double_quoted_attribute(href)
            )
        })
        .unwrap_or_default();
    let click_selector_json = serde_json::to_string(&click_selector)?;
    Ok(format!(
        r#"{base}{html}
<script>
const CLICK_SELECTOR = {click_selector_json};
if (CLICK_SELECTOR) {{
  const clickTarget = document.querySelector(CLICK_SELECTOR);
  if (clickTarget) clickTarget.click();
}}
setTimeout(() => {{
  const result = {{
    title: document.title || "",
    text: document.body ? document.body.innerText : ""
  }};
  const out = document.createElement("script");
  out.type = "application/json";
  out.id = "brutal-chromium-result";
  out.textContent = JSON.stringify(result).replaceAll("</", "<\\/");
  document.documentElement.appendChild(out);
}}, 0);
</script>"#
    ))
}

fn extract_chromium_result_json(dump: &str) -> Option<&str> {
    let marker = "id=\"brutal-chromium-result\"";
    let marker_index = dump.find(marker)?;
    let after_marker = &dump[marker_index..];
    let start = after_marker.find('>')? + marker_index + 1;
    let after_start = &dump[start..];
    let end = after_start.find("</script>")? + start;
    dump.get(start..end)
}

fn normalize_browser_parity_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
struct ParsedHtml {
    dom: Dom,
    style_text: String,
    inline_scripts: Vec<String>,
}

fn parse_html(html: &[u8]) -> ParsedHtml {
    let mut dom = Dom {
        nodes: vec![Node {
            kind: NodeKind::Document,
            parent: None,
            children: Vec::new(),
        }],
    };
    let mut stack = vec![0usize];
    let mut style_text = String::new();
    let mut inline_scripts = Vec::new();
    let mut cursor = 0usize;

    while cursor < html.len() {
        if let Some(raw_text_tag) = stack
            .last()
            .and_then(|&node_id| current_raw_text_tag(&dom, node_id))
        {
            let Some(closing_start) = find_closing_tag(html, cursor, raw_text_tag) else {
                push_text(
                    &mut dom,
                    &stack,
                    &html[cursor..],
                    &mut style_text,
                    &mut inline_scripts,
                );
                break;
            };
            push_text(
                &mut dom,
                &stack,
                &html[cursor..closing_start],
                &mut style_text,
                &mut inline_scripts,
            );
            let Some(tag_end_offset) = memchr(b'>', &html[closing_start + 1..]) else {
                break;
            };
            let tag_end = closing_start + 1 + tag_end_offset;
            if let Some(tag) = parse_tag(&html[closing_start + 1..tag_end])
                && matches!(tag.kind, TagKind::Closing)
            {
                pop_until(&mut stack, &dom, &tag.name);
            }
            cursor = tag_end + 1;
            continue;
        }

        let Some(offset) = memchr(b'<', &html[cursor..]) else {
            push_text(
                &mut dom,
                &stack,
                &html[cursor..],
                &mut style_text,
                &mut inline_scripts,
            );
            break;
        };
        let tag_start = cursor + offset;
        push_text(
            &mut dom,
            &stack,
            &html[cursor..tag_start],
            &mut style_text,
            &mut inline_scripts,
        );

        let Some(tag_end) = find_tag_end(html, tag_start) else {
            break;
        };
        let raw_tag = &html[tag_start + 1..tag_end];
        if let Some(tag) = parse_tag(raw_tag) {
            match tag.kind {
                TagKind::Opening => {
                    let parent = *stack.last().unwrap_or(&0);
                    let attrs = parse_attributes(raw_tag);
                    let element = element_data_from_attrs(tag.name.clone(), attrs);
                    let node_id = push_node(&mut dom, parent, NodeKind::Element(Box::new(element)));
                    if !tag.self_closing && !is_void_tag(&tag.name) {
                        stack.push(node_id);
                    }
                }
                TagKind::Closing => pop_until(&mut stack, &dom, &tag.name),
            }
        }

        cursor = tag_end + 1;
    }

    ParsedHtml {
        dom,
        style_text,
        inline_scripts,
    }
}

fn push_text(
    dom: &mut Dom,
    stack: &[usize],
    raw: &[u8],
    style_text: &mut String,
    inline_scripts: &mut Vec<String>,
) {
    if raw.is_empty() {
        return;
    }

    let parent = *stack.last().unwrap_or(&0);
    if current_element_is(dom, parent, "template") {
        return;
    }

    let lossy = String::from_utf8_lossy(raw);
    let decoded = decode_html_entities(&lossy).into_owned();
    if current_element_is(dom, parent, "script") {
        inline_scripts.push(decoded);
        return;
    }
    if current_element_is(dom, parent, "style") {
        style_text.push_str(&decoded);
        return;
    }

    if decoded.is_empty() {
        return;
    }
    push_node(dom, parent, NodeKind::Text(decoded));
}

fn push_node(dom: &mut Dom, parent: usize, kind: NodeKind) -> usize {
    let node_id = dom.nodes.len();
    dom.nodes.push(Node {
        kind,
        parent: Some(parent),
        children: Vec::new(),
    });
    if let Some(parent) = dom.nodes.get_mut(parent) {
        parent.children.push(node_id);
    }
    node_id
}

fn push_detached_node(dom: &mut Dom, kind: NodeKind) -> usize {
    let node_id = dom.nodes.len();
    dom.nodes.push(Node {
        kind,
        parent: None,
        children: Vec::new(),
    });
    node_id
}

fn current_element_is(dom: &Dom, node_id: usize, tag: &str) -> bool {
    matches!(
        dom.nodes.get(node_id).map(|node| &node.kind),
        Some(NodeKind::Element(element)) if element.tag == tag
    )
}

fn current_raw_text_tag(dom: &Dom, node_id: usize) -> Option<&'static str> {
    match dom.nodes.get(node_id).map(|node| &node.kind) {
        Some(NodeKind::Element(element)) if element.tag == "script" => Some("script"),
        Some(NodeKind::Element(element)) if element.tag == "style" => Some("style"),
        _ => None,
    }
}

fn find_closing_tag(html: &[u8], start: usize, tag: &str) -> Option<usize> {
    let needle = format!("</{tag}");
    html.get(start..)?
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
        .map(|offset| start + offset)
}

fn find_tag_end(html: &[u8], tag_start: usize) -> Option<usize> {
    let mut index = tag_start.saturating_add(1);
    while index < html.len() {
        match html[index] {
            b'\'' | b'"' => {
                index = skip_quoted_tag_attribute(html, index)?;
                continue;
            }
            b'>' => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_quoted_tag_attribute(html: &[u8], quote_start: usize) -> Option<usize> {
    let quote = *html.get(quote_start)?;
    let mut index = quote_start.saturating_add(1);
    while index < html.len() {
        if html[index] == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

const MAX_TIMER_TASKS_PER_RENDER: usize = 1024;

#[derive(Debug, Clone)]
struct TinyJsRuntime {
    page_source: String,
    bindings: HashMap<String, usize>,
    node_list_bindings: HashMap<String, Vec<usize>>,
    string_bindings: HashMap<String, String>,
    function_bindings: HashMap<String, String>,
    event_listeners: HashMap<(BrowserEventTarget, String), Vec<JsEventListener>>,
    lifecycle_event_listeners: HashMap<String, Vec<String>>,
    local_storage: HashMap<String, String>,
    session_storage: HashMap<String, String>,
    timer_tasks: VecDeque<TimerTask>,
    cancelled_timer_ids: HashSet<u64>,
    next_timer_id: u64,
    this_node: Option<usize>,
    this_target: Option<BrowserEventTarget>,
    active_element: Option<usize>,
    current_event: Option<TinyJsEvent>,
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_propagation_stopped: bool,
    return_false_prevents_default: bool,
}

#[derive(Debug, Clone)]
struct TimerTask {
    id: u64,
    handler: String,
}

impl Default for TinyJsRuntime {
    fn default() -> Self {
        Self {
            page_source: String::new(),
            bindings: HashMap::new(),
            node_list_bindings: HashMap::new(),
            string_bindings: HashMap::new(),
            function_bindings: HashMap::new(),
            event_listeners: HashMap::new(),
            lifecycle_event_listeners: HashMap::new(),
            local_storage: HashMap::new(),
            session_storage: HashMap::new(),
            timer_tasks: VecDeque::new(),
            cancelled_timer_ids: HashSet::new(),
            next_timer_id: 1,
            this_node: None,
            this_target: None,
            active_element: None,
            current_event: None,
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
            return_false_prevents_default: false,
        }
    }
}

impl TinyJsRuntime {
    fn with_web_storage(
        local_storage: HashMap<String, String>,
        session_storage: HashMap<String, String>,
    ) -> Self {
        Self {
            local_storage,
            session_storage,
            ..Self::default()
        }
    }
}

fn execute_scripts_with_runtime(dom: &mut Dom, runtime: &mut TinyJsRuntime, scripts: &[String]) {
    for script in scripts {
        for statement in split_js_statements(script) {
            execute_js_statement(dom, runtime, statement);
        }
    }
    dispatch_lifecycle_event(dom, runtime, "DOMContentLoaded");
    dispatch_lifecycle_event(dom, runtime, "load");
    drain_timer_tasks(dom, runtime);
}

fn execute_scripts_without_lifecycle_events(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    scripts: &[String],
) {
    for script in scripts {
        for statement in split_js_statements(script) {
            execute_js_statement(dom, runtime, statement);
        }
    }
    drain_timer_tasks(dom, runtime);
}

fn dispatch_lifecycle_event(dom: &mut Dom, runtime: &mut TinyJsRuntime, event_name: &str) {
    let Some(listeners) = runtime.lifecycle_event_listeners.get(event_name).cloned() else {
        return;
    };
    for listener in listeners {
        let previous_this = runtime.this_node.take();
        let previous_this_target = runtime.this_target.take();
        for statement in split_js_statements(&listener) {
            execute_js_statement(dom, runtime, statement);
        }
        runtime.this_node = previous_this;
        runtime.this_target = previous_this_target;
    }
}

fn dispatch_click_selector(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    selector: &str,
) -> Result<BrowserClickDispatch> {
    let Some(node_id) = resolve_event_target(dom, selector) else {
        bail!("click target not found: {selector}");
    };
    Ok(dispatch_click(dom, runtime, node_id))
}

fn dispatch_click(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
) -> BrowserClickDispatch {
    dispatch_click_with_payload(
        dom,
        runtime,
        node_id,
        BrowserEventPayload::new("click", node_id),
    )
}

fn dispatch_pointer_click_node(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    x: usize,
    y: usize,
) -> Result<BrowserClickDispatch> {
    ensure!(
        node_id < dom.nodes.len(),
        "click target node {node_id} is outside the current DOM"
    );
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::pointer("pointerdown", node_id, x, y),
        true,
    );
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::mouse("mousedown", node_id, x, y),
        true,
    );
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::pointer("pointerup", node_id, x, y),
        true,
    );
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::mouse("mouseup", node_id, x, y),
        true,
    );
    Ok(dispatch_click_with_payload(
        dom,
        runtime,
        node_id,
        BrowserEventPayload::mouse("click", node_id, x, y),
    ))
}

fn dispatch_click_with_payload(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    payload: BrowserEventPayload,
) -> BrowserClickDispatch {
    let snapshot = begin_click_dispatch(runtime, payload);
    let event_path = event_path_to_window(dom, node_id);
    for &current_target in event_path.iter().skip(1).rev() {
        dispatch_event_listener_group(
            dom,
            runtime,
            current_target,
            "click",
            true,
            BrowserEventPhase::Capture,
        );
        if runtime.propagation_stopped {
            break;
        }
    }
    if !runtime.propagation_stopped {
        dispatch_click_target_phase(dom, runtime, node_id);
    }
    if !runtime.propagation_stopped {
        for &current_target in event_path.iter().skip(1) {
            dispatch_click_bubble_target(dom, runtime, current_target);
            if runtime.propagation_stopped {
                break;
            }
        }
    }
    let default_prevented = restore_event_dispatch(runtime, snapshot);
    BrowserClickDispatch {
        node_id,
        default_prevented,
    }
}

fn dispatch_click_target_phase(dom: &mut Dom, runtime: &mut TinyJsRuntime, node_id: usize) {
    let previous_this = runtime.this_node.replace(node_id);
    let previous_this_target = runtime
        .this_target
        .replace(BrowserEventTarget::Node(node_id));
    dispatch_event_listener_group(
        dom,
        runtime,
        BrowserEventTarget::Node(node_id),
        "click",
        true,
        BrowserEventPhase::Target,
    );
    if !runtime.immediate_propagation_stopped
        && let Some(onclick) = onclick_handler(dom, node_id)
    {
        set_current_event_phase(runtime, BrowserEventPhase::Target);
        execute_click_handler(dom, runtime, &onclick, true);
    }
    if !runtime.immediate_propagation_stopped {
        dispatch_event_listener_group(
            dom,
            runtime,
            BrowserEventTarget::Node(node_id),
            "click",
            false,
            BrowserEventPhase::Target,
        );
    }
    runtime.this_node = previous_this;
    runtime.this_target = previous_this_target;
}

fn dispatch_click_bubble_target(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    target: BrowserEventTarget,
) {
    let previous_this = set_runtime_this_target(runtime, target);
    if let BrowserEventTarget::Node(node_id) = target
        && let Some(onclick) = onclick_handler(dom, node_id)
    {
        set_current_event_phase(runtime, BrowserEventPhase::Bubble);
        execute_click_handler(dom, runtime, &onclick, true);
    }
    if !runtime.immediate_propagation_stopped {
        dispatch_event_listener_group(
            dom,
            runtime,
            target,
            "click",
            false,
            BrowserEventPhase::Bubble,
        );
    }
    restore_runtime_this_target(runtime, previous_this);
}

fn onclick_handler(dom: &Dom, node_id: usize) -> Option<String> {
    match dom.nodes.get(node_id).map(|node| &node.kind) {
        Some(NodeKind::Element(element)) => element.onclick.clone(),
        _ => None,
    }
}

fn dispatch_bubbling_event_listeners(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    event_name: &str,
) -> BrowserEventDispatch {
    dispatch_event_listeners(dom, runtime, node_id, event_name, true)
}

fn dispatch_keyboard_event(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    event_name: &str,
    key: &str,
) -> BrowserEventDispatch {
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::keyboard(event_name, node_id, key),
        true,
    )
}

fn dispatch_event_listeners(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    event_name: &str,
    bubbles: bool,
) -> BrowserEventDispatch {
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::new(event_name, node_id),
        bubbles,
    )
}

fn dispatch_beforeinput_event(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    node_id: usize,
    input_type: &str,
    data: Option<&str>,
) -> BrowserEventDispatch {
    dispatch_event_listeners_with_payload(
        dom,
        runtime,
        BrowserEventPayload::beforeinput(node_id, input_type, data),
        true,
    )
}

fn dispatch_event_listeners_with_payload(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    payload: BrowserEventPayload,
    bubbles: bool,
) -> BrowserEventDispatch {
    let node_id = payload.target_node;
    if node_id >= dom.nodes.len() {
        return BrowserEventDispatch {
            node_id,
            default_prevented: false,
        };
    }
    let event_name = payload.type_name.clone();
    let snapshot = begin_event_dispatch(runtime, payload);

    let event_path = event_path_to_window(dom, node_id);
    for &current_target in event_path.iter().skip(1).rev() {
        dispatch_event_listener_group(
            dom,
            runtime,
            current_target,
            &event_name,
            true,
            BrowserEventPhase::Capture,
        );
        if runtime.propagation_stopped {
            break;
        }
    }
    if !runtime.propagation_stopped {
        dispatch_event_listener_group(
            dom,
            runtime,
            BrowserEventTarget::Node(node_id),
            &event_name,
            true,
            BrowserEventPhase::Target,
        );
        if !runtime.immediate_propagation_stopped {
            dispatch_event_listener_group(
                dom,
                runtime,
                BrowserEventTarget::Node(node_id),
                &event_name,
                false,
                BrowserEventPhase::Target,
            );
        }
    }
    if !runtime.propagation_stopped && bubbles {
        for &current_target in event_path.iter().skip(1) {
            dispatch_event_listener_group(
                dom,
                runtime,
                current_target,
                &event_name,
                false,
                BrowserEventPhase::Bubble,
            );
            if runtime.propagation_stopped {
                break;
            }
        }
    }
    let default_prevented = restore_event_dispatch(runtime, snapshot);
    BrowserEventDispatch {
        node_id,
        default_prevented,
    }
}

fn dispatch_event_listener_group(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    target: BrowserEventTarget,
    event_name: &str,
    capture: bool,
    phase: BrowserEventPhase,
) {
    dispatch_event_listener_group_core(
        runtime,
        target,
        event_name,
        capture,
        phase,
        |runtime, handler| execute_click_handler(dom, runtime, handler, false),
    );
}

fn execute_click_handler(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    handler: &str,
    return_false_prevents_default: bool,
) {
    let previous = runtime.return_false_prevents_default;
    runtime.return_false_prevents_default = return_false_prevents_default;
    for statement in split_js_statements(handler) {
        execute_js_statement(dom, runtime, statement);
    }
    runtime.return_false_prevents_default = previous;
}

fn resolve_event_target(dom: &Dom, selector: &str) -> Option<usize> {
    find_first_matching_selector(dom, selector)
}

fn split_js_statements(script: &str) -> Vec<&str> {
    let mut statements = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (index, ch) in script.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
        } else if ch == '(' {
            paren_depth += 1;
        } else if ch == ')' {
            paren_depth = paren_depth.saturating_sub(1);
        } else if ch == '{' {
            brace_depth += 1;
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        } else if ch == '[' {
            bracket_depth += 1;
        } else if ch == ']' {
            bracket_depth = bracket_depth.saturating_sub(1);
        } else if ch == ';' && paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 {
            statements.push(&script[start..index]);
            start = index + ch.len_utf8();
        }
    }

    if start < script.len() {
        statements.push(&script[start..]);
    }
    statements
}

fn execute_js_statement(dom: &mut Dom, runtime: &mut TinyJsRuntime, statement: &str) {
    let statement = statement.trim();
    if statement.is_empty() || statement.starts_with("//") {
        return;
    }

    if execute_js_default_prevention(runtime, statement) {
        return;
    }

    if execute_js_declaration(dom, runtime, statement) {
        return;
    }

    if execute_js_tree_mutation(dom, runtime, statement) {
        return;
    }

    if execute_js_set_attribute(dom, runtime, statement) {
        return;
    }

    if execute_js_class_list_mutation(dom, runtime, statement) {
        return;
    }

    if execute_js_style_mutation(dom, runtime, statement) {
        return;
    }

    if execute_js_web_storage(dom, runtime, statement) {
        return;
    }

    if execute_js_timer(dom, runtime, statement) {
        return;
    }

    if execute_js_remove_event_listener(dom, runtime, statement) {
        return;
    }

    if execute_js_add_event_listener(dom, runtime, statement) {
        return;
    }

    let Some((operator_index, append)) = find_js_assignment(statement) else {
        return;
    };
    let mut left = statement[..operator_index].trim();
    if append {
        left = left.trim_end_matches('+').trim_end();
    }
    let right = statement[operator_index + 1..].trim();
    if is_js_identifier(left)
        && let Some(timer_id) = schedule_js_timeout(runtime, right)
    {
        runtime
            .string_bindings
            .insert(left.to_owned(), timer_id.to_string());
        runtime.bindings.remove(left);
        runtime.function_bindings.remove(left);
        return;
    }
    let Some(value) = evaluate_js_string_expression(dom, runtime, right) else {
        return;
    };

    if left == "document.title" {
        set_document_title(dom, &value, append);
        return;
    }

    if let Some(target) = left.strip_suffix(".innerHTML")
        && let Some(node_id) = resolve_js_node_ref(dom, runtime, target.trim())
    {
        set_inner_html(dom, node_id, &value, append);
        return;
    }

    for property in [".textContent", ".innerText"] {
        if let Some(target) = left.strip_suffix(property)
            && let Some(node_id) = resolve_js_node_ref(dom, runtime, target.trim())
        {
            set_text_content(dom, node_id, &value, append);
            return;
        }
    }

    for property in [".checked", ".disabled", ".hidden", ".selected"] {
        if let Some(target) = left.strip_suffix(property)
            && let Some(node_id) = resolve_js_node_ref(dom, runtime, target.trim())
        {
            set_element_boolean_property(dom, node_id, &property[1..], js_truthy_string(&value));
            return;
        }
    }

    for property in [
        ".id",
        ".className",
        ".href",
        ".value",
        ".name",
        ".type",
        ".src",
        ".alt",
        ".action",
        ".method",
    ] {
        if let Some(target) = left.strip_suffix(property)
            && let Some(node_id) = resolve_js_node_ref(dom, runtime, target.trim())
        {
            set_element_string_property(dom, node_id, &property[1..], &value, append);
            return;
        }
    }

    if let Some((target, property)) = parse_js_style_property_ref(left)
        && let Some(node_id) = resolve_js_node_ref(dom, runtime, target)
    {
        set_element_style_property(dom, node_id, property, &value, append);
    }
}

fn execute_js_default_prevention(runtime: &mut TinyJsRuntime, statement: &str) -> bool {
    let statement = statement.trim().trim_end_matches(';').trim();
    if statement == "return false" {
        if runtime.return_false_prevents_default {
            runtime.default_prevented = true;
        }
        return true;
    }
    if statement == "preventDefault()" || statement.ends_with(".preventDefault()") {
        runtime.default_prevented = true;
        return true;
    }
    if statement == "stopImmediatePropagation()"
        || statement.ends_with(".stopImmediatePropagation()")
    {
        runtime.propagation_stopped = true;
        runtime.immediate_propagation_stopped = true;
        return true;
    }
    if statement == "stopPropagation()" || statement.ends_with(".stopPropagation()") {
        runtime.propagation_stopped = true;
        return true;
    }
    false
}

fn execute_js_declaration(dom: &mut Dom, runtime: &mut TinyJsRuntime, statement: &str) -> bool {
    let Some((name, expression)) = parse_js_declaration(statement) else {
        return false;
    };

    if let Some(node_id) = create_js_node_from_expression(dom, expression) {
        runtime.bindings.insert(name.to_owned(), node_id);
        runtime.node_list_bindings.remove(name);
        runtime.string_bindings.remove(name);
        runtime.function_bindings.remove(name);
        return true;
    }
    if let Some(node_id) = resolve_js_node_ref(dom, runtime, expression) {
        runtime.bindings.insert(name.to_owned(), node_id);
        runtime.node_list_bindings.remove(name);
        runtime.string_bindings.remove(name);
        runtime.function_bindings.remove(name);
        return true;
    }
    if let Some(node_ids) = resolve_js_node_list_ref(dom, runtime, expression) {
        runtime.node_list_bindings.insert(name.to_owned(), node_ids);
        runtime.bindings.remove(name);
        runtime.string_bindings.remove(name);
        runtime.function_bindings.remove(name);
        return true;
    }
    if let Some(handler) = parse_js_callable_handler_body(expression) {
        runtime.function_bindings.insert(name.to_owned(), handler);
        runtime.bindings.remove(name);
        runtime.node_list_bindings.remove(name);
        runtime.string_bindings.remove(name);
        return true;
    }
    if let Some(timer_id) = schedule_js_timeout(runtime, expression) {
        runtime
            .string_bindings
            .insert(name.to_owned(), timer_id.to_string());
        runtime.bindings.remove(name);
        runtime.node_list_bindings.remove(name);
        runtime.function_bindings.remove(name);
        return true;
    }
    if let Some(value) = evaluate_js_string_expression(dom, runtime, expression) {
        runtime.string_bindings.insert(name.to_owned(), value);
        runtime.bindings.remove(name);
        runtime.node_list_bindings.remove(name);
        runtime.function_bindings.remove(name);
        return true;
    }
    false
}

fn parse_js_declaration(statement: &str) -> Option<(&str, &str)> {
    for prefix in ["const ", "let ", "var "] {
        let Some(rest) = statement.strip_prefix(prefix) else {
            continue;
        };
        let (name, expression) = rest.split_once('=')?;
        let name = name.trim();
        if is_js_identifier(name) {
            return Some((name, expression.trim()));
        }
    }
    None
}

fn is_js_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || matches!(first, '_' | '$'))
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
}

fn create_js_node_from_expression(dom: &mut Dom, expression: &str) -> Option<usize> {
    if let Some(tag) = parse_js_call_string_arg(expression, "document.createElement") {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.is_empty() {
            return None;
        }
        return Some(push_detached_node(
            dom,
            NodeKind::Element(Box::new(empty_element_data(&tag))),
        ));
    }
    if let Some(text) = parse_js_call_string_arg(expression, "document.createTextNode") {
        return Some(push_detached_node(dom, NodeKind::Text(text)));
    }
    if matches!(
        expression.trim(),
        "document.createDocumentFragment()" | "new DocumentFragment()"
    ) {
        return Some(push_detached_node(dom, NodeKind::DocumentFragment));
    }
    None
}

fn execute_js_tree_mutation(dom: &mut Dom, runtime: &TinyJsRuntime, statement: &str) -> bool {
    if let Some((target, child)) = parse_js_method_call(statement, ".appendChild") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        if let Some(child_id) = resolve_or_create_js_node(dom, runtime, child) {
            append_child(dom, parent_id, child_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".insertBefore") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        let Some(child_id) = args
            .first()
            .and_then(|arg| resolve_or_create_js_node(dom, runtime, arg))
        else {
            return true;
        };
        let reference_id = args
            .get(1)
            .and_then(|arg| resolve_optional_js_node_ref(dom, runtime, arg));
        insert_child_before(dom, parent_id, child_id, reference_id);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".replaceChildren") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let child_ids = resolve_js_insert_nodes(dom, runtime, args);
        detach_children(dom, parent_id);
        for child_id in child_ids {
            append_child(dom, parent_id, child_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".append") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        for child_id in resolve_js_insert_nodes(dom, runtime, args) {
            append_child(dom, parent_id, child_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".prepend") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let reference_id = dom
            .nodes
            .get(parent_id)
            .and_then(|node| node.children.first().copied());
        for child_id in resolve_js_insert_nodes(dom, runtime, args) {
            insert_child_before(dom, parent_id, child_id, reference_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".before") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let Some(parent_id) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
            return true;
        };
        for child_id in resolve_js_insert_nodes(dom, runtime, args) {
            insert_child_before(dom, parent_id, child_id, Some(node_id));
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".after") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let Some(parent_id) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
            return true;
        };
        let next_id = next_child_after(dom, parent_id, node_id);
        for child_id in resolve_js_insert_nodes(dom, runtime, args) {
            insert_child_before(dom, parent_id, child_id, next_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".replaceWith") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let Some(parent_id) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
            return true;
        };
        for child_id in resolve_js_insert_nodes(dom, runtime, args) {
            insert_child_before(dom, parent_id, child_id, Some(node_id));
        }
        remove_child(dom, parent_id, node_id);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".replaceChild") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        let Some(new_child_id) = args
            .first()
            .and_then(|arg| resolve_or_create_js_node(dom, runtime, arg))
        else {
            return true;
        };
        let Some(old_child_id) = args
            .get(1)
            .and_then(|arg| resolve_js_node_ref(dom, runtime, arg))
        else {
            return true;
        };
        replace_child(dom, parent_id, new_child_id, old_child_id);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".removeChild") {
        let Some(parent_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        if let Some(child_id) = args
            .first()
            .and_then(|arg| resolve_js_node_ref(dom, runtime, arg))
        {
            remove_child(dom, parent_id, child_id);
        }
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".remove") {
        if !args.trim().is_empty() {
            return false;
        }
        if let Some(node_id) = resolve_js_node_ref(dom, runtime, target) {
            remove_node(dom, node_id);
        }
        return true;
    }

    false
}

fn resolve_js_insert_nodes(dom: &mut Dom, runtime: &TinyJsRuntime, args: &str) -> Vec<usize> {
    split_js_arguments(args)
        .into_iter()
        .filter_map(|arg| resolve_or_create_js_insert_node(dom, runtime, arg))
        .collect()
}

fn resolve_or_create_js_node(
    dom: &mut Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<usize> {
    resolve_js_node_ref(dom, runtime, expression)
        .or_else(|| create_js_node_from_expression(dom, expression))
}

fn resolve_or_create_js_insert_node(
    dom: &mut Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<usize> {
    if let Some(node_id) = resolve_or_create_js_node(dom, runtime, expression) {
        return Some(node_id);
    }
    let text = evaluate_js_string_expression(dom, runtime, expression)?;
    Some(push_detached_node(dom, NodeKind::Text(text)))
}

fn resolve_optional_js_node_ref(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<usize> {
    let expression = expression.trim();
    if matches!(expression, "null" | "undefined" | "") {
        None
    } else {
        resolve_js_node_ref(dom, runtime, expression)
    }
}

fn execute_js_set_attribute(dom: &mut Dom, runtime: &TinyJsRuntime, statement: &str) -> bool {
    let Some((target, args)) = parse_js_method_call(statement, ".setAttribute") else {
        return false;
    };
    let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
        return true;
    };
    let args = split_js_arguments(args);
    let Some(name) = args
        .first()
        .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
    else {
        return true;
    };
    let Some(value) = args
        .get(1)
        .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
    else {
        return true;
    };
    set_element_attribute(dom, node_id, &name, &value);
    true
}

fn execute_js_class_list_mutation(dom: &mut Dom, runtime: &TinyJsRuntime, statement: &str) -> bool {
    if let Some((target, args)) = parse_js_method_call(statement, ".classList.add") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let tokens = evaluate_js_class_tokens(dom, runtime, args);
        add_element_class_tokens(dom, node_id, &tokens);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".classList.remove") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let tokens = evaluate_js_class_tokens(dom, runtime, args);
        remove_element_class_tokens(dom, node_id, &tokens);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".classList.toggle") {
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        let Some(token) = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
            .and_then(|token| normalize_class_token(&token))
        else {
            return true;
        };
        let force = args
            .get(1)
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
            .and_then(|value| parse_js_boolish(&value));
        toggle_element_class_token(dom, node_id, &token, force);
        return true;
    }

    false
}

fn execute_js_style_mutation(dom: &mut Dom, runtime: &TinyJsRuntime, statement: &str) -> bool {
    if let Some((target, args)) = parse_js_method_call(statement, ".setProperty") {
        let Some(target) = target.strip_suffix(".style").map(str::trim) else {
            return false;
        };
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        let Some(property) = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        else {
            return true;
        };
        let Some(value) = args
            .get(1)
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        else {
            return true;
        };
        set_element_style_property(dom, node_id, &property, &value, false);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".removeProperty") {
        let Some(target) = target.strip_suffix(".style").map(str::trim) else {
            return false;
        };
        let Some(node_id) = resolve_js_node_ref(dom, runtime, target) else {
            return true;
        };
        let args = split_js_arguments(args);
        if let Some(property) = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        {
            remove_element_style_property(dom, node_id, &property);
        }
        return true;
    }

    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserWebStorageKind {
    Local,
    Session,
}

fn execute_js_web_storage(dom: &Dom, runtime: &mut TinyJsRuntime, statement: &str) -> bool {
    if let Some((target, args)) = parse_js_method_call(statement, ".setItem") {
        let Some(kind) = web_storage_kind(target) else {
            return false;
        };
        let args = split_js_arguments(args);
        let Some(key) = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        else {
            return true;
        };
        let Some(value) = args
            .get(1)
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        else {
            return true;
        };
        web_storage_mut(runtime, kind).insert(key, value);
        return true;
    }

    if let Some((target, args)) = parse_js_method_call(statement, ".removeItem") {
        let Some(kind) = web_storage_kind(target) else {
            return false;
        };
        let args = split_js_arguments(args);
        if let Some(key) = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))
        {
            web_storage_mut(runtime, kind).remove(&key);
        }
        return true;
    }

    if let Some((target, _args)) = parse_js_method_call(statement, ".clear") {
        let Some(kind) = web_storage_kind(target) else {
            return false;
        };
        web_storage_mut(runtime, kind).clear();
        return true;
    }

    false
}

fn is_local_storage_ref(target: &str) -> bool {
    matches!(target.trim(), "localStorage" | "window.localStorage")
}

fn is_session_storage_ref(target: &str) -> bool {
    matches!(target.trim(), "sessionStorage" | "window.sessionStorage")
}

fn web_storage_kind(target: &str) -> Option<BrowserWebStorageKind> {
    if is_local_storage_ref(target) {
        Some(BrowserWebStorageKind::Local)
    } else if is_session_storage_ref(target) {
        Some(BrowserWebStorageKind::Session)
    } else {
        None
    }
}

fn web_storage(runtime: &TinyJsRuntime, kind: BrowserWebStorageKind) -> &HashMap<String, String> {
    match kind {
        BrowserWebStorageKind::Local => &runtime.local_storage,
        BrowserWebStorageKind::Session => &runtime.session_storage,
    }
}

fn web_storage_mut(
    runtime: &mut TinyJsRuntime,
    kind: BrowserWebStorageKind,
) -> &mut HashMap<String, String> {
    match kind {
        BrowserWebStorageKind::Local => &mut runtime.local_storage,
        BrowserWebStorageKind::Session => &mut runtime.session_storage,
    }
}

fn execute_js_timer(dom: &Dom, runtime: &mut TinyJsRuntime, statement: &str) -> bool {
    if schedule_js_timeout(runtime, statement).is_some() {
        return true;
    }

    let Some(args) = parse_js_named_call(statement, &["clearTimeout", "window.clearTimeout"])
    else {
        return false;
    };
    let args = split_js_arguments(args);
    if let Some(timer_id) = args
        .first()
        .and_then(|arg| evaluate_js_timer_id(dom, runtime, arg))
    {
        runtime.cancelled_timer_ids.insert(timer_id);
    }
    true
}

fn schedule_js_timeout(runtime: &mut TinyJsRuntime, expression: &str) -> Option<u64> {
    let args = parse_js_named_call(expression, &["setTimeout", "window.setTimeout"])?;
    let args = split_js_arguments(args);
    let handler = parse_js_handler_body(runtime, args.first()?.trim())?;
    let timer_id = runtime.next_timer_id;
    runtime.next_timer_id = runtime.next_timer_id.saturating_add(1).max(1);
    runtime.timer_tasks.push_back(TimerTask {
        id: timer_id,
        handler,
    });
    Some(timer_id)
}

fn evaluate_js_timer_id(dom: &Dom, runtime: &TinyJsRuntime, expression: &str) -> Option<u64> {
    let expression = expression.trim();
    expression.parse::<u64>().ok().or_else(|| {
        evaluate_js_string_expression(dom, runtime, expression)
            .and_then(|value| value.trim().parse::<u64>().ok())
    })
}

fn drain_timer_tasks(dom: &mut Dom, runtime: &mut TinyJsRuntime) {
    let mut executed = 0usize;
    while let Some(task) = runtime.timer_tasks.pop_front() {
        if runtime.cancelled_timer_ids.remove(&task.id) {
            continue;
        }
        let previous_this = runtime.this_node.take();
        let previous_this_target = runtime.this_target.take();
        for statement in split_js_statements(&task.handler) {
            execute_js_statement(dom, runtime, statement);
        }
        runtime.this_node = previous_this;
        runtime.this_target = previous_this_target;
        executed += 1;
        if executed >= MAX_TIMER_TASKS_PER_RENDER {
            runtime.timer_tasks.clear();
            break;
        }
    }
}

fn execute_js_add_event_listener(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    statement: &str,
) -> bool {
    let Some((target, args)) = parse_js_method_call(statement, ".addEventListener") else {
        return false;
    };
    let Some((event_name, listener)) = parse_event_listener_args(runtime, args) else {
        return true;
    };
    if is_js_lifecycle_event_target(target) && is_supported_lifecycle_event(&event_name) {
        runtime
            .lifecycle_event_listeners
            .entry(event_name)
            .or_default()
            .push(listener.handler);
        return true;
    }
    let Some(target) = resolve_js_event_target(dom, runtime, target) else {
        return true;
    };
    runtime
        .event_listeners
        .entry((target, event_name))
        .or_default()
        .push(listener);
    true
}

fn execute_js_remove_event_listener(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    statement: &str,
) -> bool {
    let Some((target, args)) = parse_js_method_call(statement, ".removeEventListener") else {
        return false;
    };
    let Some((event_name, listener)) = parse_event_listener_args(runtime, args) else {
        return true;
    };
    if is_js_lifecycle_event_target(target) && is_supported_lifecycle_event(&event_name) {
        if let Some(listeners) = runtime.lifecycle_event_listeners.get_mut(&event_name) {
            listeners.retain(|handler| handler != &listener.handler);
        }
        return true;
    }
    let Some(target) = resolve_js_event_target(dom, runtime, target) else {
        return true;
    };
    if let Some(listeners) = runtime.event_listeners.get_mut(&(target, event_name)) {
        listeners.retain(|registered| {
            registered.handler != listener.handler || registered.capture != listener.capture
        });
    }
    true
}

fn resolve_js_event_target(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<BrowserEventTarget> {
    match expression.trim() {
        "window" => Some(BrowserEventTarget::Window),
        _ => resolve_js_node_ref(dom, runtime, expression).map(BrowserEventTarget::Node),
    }
}

fn is_js_lifecycle_event_target(target: &str) -> bool {
    matches!(target.trim(), "document" | "window")
}

fn is_supported_lifecycle_event(event_name: &str) -> bool {
    matches!(event_name, "DOMContentLoaded" | "load")
}

fn parse_event_listener_args(
    runtime: &TinyJsRuntime,
    args: &str,
) -> Option<(String, JsEventListener)> {
    let args = split_js_arguments(args);
    let event_name = parse_js_string_value(args.first()?.trim())?;
    let handler = parse_js_handler_body(runtime, args.get(1)?.trim())?;
    let (capture, once) = args
        .get(2)
        .map(|options| parse_js_event_listener_options(options))
        .unwrap_or((false, false));
    Some((
        event_name,
        JsEventListener {
            handler,
            capture,
            once,
        },
    ))
}

fn parse_js_event_listener_options(options: &str) -> (bool, bool) {
    let options = options.trim();
    if parse_js_boolish(options) == Some(true) {
        return (true, false);
    }
    let Some(inner) = options
        .strip_prefix('{')
        .and_then(|options| options.strip_suffix('}'))
    else {
        return (false, false);
    };
    let mut capture = false;
    let mut once = false;
    for property in split_js_arguments(inner) {
        let Some((name, value)) = property.split_once(':') else {
            continue;
        };
        let name = parse_js_string_value(name.trim()).unwrap_or_else(|| name.trim().to_owned());
        let value = parse_js_boolish(value.trim()) == Some(true);
        match name.as_str() {
            "capture" => capture = value,
            "once" => once = value,
            _ => {}
        }
    }
    (capture, once)
}

fn split_js_arguments(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;

    for (index, ch) in args.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
        } else if ch == '(' {
            paren_depth += 1;
        } else if ch == ')' {
            paren_depth = paren_depth.saturating_sub(1);
        } else if ch == '{' {
            brace_depth += 1;
        } else if ch == '}' {
            brace_depth = brace_depth.saturating_sub(1);
        } else if ch == '[' {
            bracket_depth += 1;
        } else if ch == ']' {
            bracket_depth = bracket_depth.saturating_sub(1);
        } else if ch == ',' && paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 {
            out.push(args[start..index].trim());
            start = index + ch.len_utf8();
        }
    }
    if start < args.len() {
        out.push(args[start..].trim());
    }
    out
}

fn parse_js_handler_body(runtime: &TinyJsRuntime, expression: &str) -> Option<String> {
    if let Some(body) = parse_js_string_value(expression) {
        return Some(body);
    }
    let expression = expression.trim();
    if let Some(handler) = runtime.function_bindings.get(expression) {
        return Some(handler.clone());
    }
    parse_js_callable_handler_body(expression)
}

fn parse_js_callable_handler_body(expression: &str) -> Option<String> {
    let expression = expression.trim();
    if expression.starts_with("function") {
        let body = parse_js_callable_body(expression)?;
        return Some(normalize_js_event_parameter(
            parse_function_first_parameter(expression).as_deref(),
            &body,
        ));
    }
    if let Some(arrow_index) = expression.find("=>") {
        let parameter = parse_arrow_first_parameter(&expression[..arrow_index]);
        let body_expression = expression[arrow_index + 2..].trim();
        let body = parse_js_callable_body(body_expression)
            .unwrap_or_else(|| body_expression.trim_end_matches(';').trim().to_owned());
        return Some(normalize_js_event_parameter(parameter.as_deref(), &body));
    }
    None
}

fn parse_function_first_parameter(expression: &str) -> Option<String> {
    let open = expression.find('(')?;
    let close = expression[open + 1..].find(')')? + open + 1;
    split_js_arguments(&expression[open + 1..close])
        .first()
        .and_then(|parameter| parse_js_parameter_name(parameter))
}

fn parse_arrow_first_parameter(parameters: &str) -> Option<String> {
    let parameters = parameters.trim();
    let first = if let Some(inner) = parameters
        .strip_prefix('(')
        .and_then(|parameters| parameters.strip_suffix(')'))
    {
        split_js_arguments(inner).first().copied()?
    } else {
        parameters
    };
    parse_js_parameter_name(first)
}

fn parse_js_parameter_name(parameter: &str) -> Option<String> {
    let parameter = parameter.trim();
    is_js_identifier(parameter).then(|| parameter.to_owned())
}

fn normalize_js_event_parameter(parameter: Option<&str>, body: &str) -> String {
    let Some(parameter) = parameter.filter(|parameter| *parameter != "event") else {
        return body.to_owned();
    };
    body.replace(&format!("{parameter}."), "event.")
}

fn parse_js_callable_body(expression: &str) -> Option<String> {
    let expression = expression.trim();
    if let Some(body) = expression.strip_prefix('{') {
        let close = body.rfind('}')?;
        return Some(body[..close].trim().to_owned());
    }
    let open = expression.find('{')?;
    let close = expression.rfind('}')?;
    (close > open).then(|| expression[open + 1..close].trim().to_owned())
}

fn parse_js_method_call<'a>(statement: &'a str, method: &str) -> Option<(&'a str, &'a str)> {
    let (target, rest) = statement.split_once(method)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(')?.trim();
    let close = rest.rfind(')')?;
    Some((target.trim(), rest[..close].trim()))
}

fn parse_js_named_call<'a>(expression: &'a str, names: &[&str]) -> Option<&'a str> {
    let expression = expression.trim();
    for name in names {
        let Some(rest) = expression.strip_prefix(name) else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('(')?.trim();
        let close = rest.rfind(')')?;
        return Some(rest[..close].trim());
    }
    None
}

fn find_js_assignment(statement: &str) -> Option<(usize, bool)> {
    let mut quote = None;
    let mut escaped = false;

    for (index, ch) in statement.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
            continue;
        }
        if ch != '=' {
            continue;
        }

        let previous = previous_non_ws_char(&statement[..index]);
        let next = statement[index + 1..]
            .chars()
            .find(|ch| !ch.is_whitespace());
        if matches!(previous, Some('=' | '!' | '<' | '>')) || next == Some('=') {
            continue;
        }
        return Some((index, previous == Some('+')));
    }
    None
}

fn previous_non_ws_char(text: &str) -> Option<char> {
    text.chars().rev().find(|ch| !ch.is_whitespace())
}

fn parse_js_string_value(expression: &str) -> Option<String> {
    let expression = expression.trim();
    let quote = expression.chars().next()?;
    if !matches!(quote, '"' | '\'' | '`') {
        return None;
    }

    let mut out = String::new();
    let mut escaped = false;
    for ch in expression[quote.len_utf8()..].chars() {
        if escaped {
            match ch {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                '\'' => out.push('\''),
                '`' => out.push('`'),
                other => out.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn evaluate_js_string_expression(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<String> {
    let expression = expression.trim();
    if let Some(value) = parse_js_string_value(expression) {
        return Some(value);
    }
    if matches!(expression, "true" | "false") || expression.parse::<i64>().is_ok() {
        return Some(expression.to_owned());
    }
    if let Some(value) = runtime.string_bindings.get(expression) {
        return Some(value.clone());
    }
    if let Some(value) = evaluate_js_event_expression(runtime, expression) {
        return Some(value);
    }
    if let Some(value) = evaluate_js_event_target_expression(runtime, expression) {
        return Some(value);
    }
    if let Some(value) = evaluate_js_location_expression(&runtime.page_source, expression) {
        return Some(value);
    }
    for (suffix, property) in [
        (".textContent", "textContent"),
        (".innerText", "innerText"),
        (".innerHTML", "innerHTML"),
        (".id", "id"),
        (".className", "className"),
        (".href", "href"),
        (".value", "value"),
        (".name", "name"),
        (".type", "type"),
        (".src", "src"),
        (".alt", "alt"),
        (".action", "action"),
        (".method", "method"),
        (".checked", "checked"),
        (".disabled", "disabled"),
        (".hidden", "hidden"),
        (".selected", "selected"),
        (".tagName", "tagName"),
        (".nodeName", "nodeName"),
        (".nodeType", "nodeType"),
    ] {
        if let Some(target) = expression.strip_suffix(suffix) {
            let node_id = resolve_js_node_ref(dom, runtime, target)?;
            return get_element_string_property(dom, node_id, property);
        }
    }
    if let Some(target) = expression.strip_suffix(".childElementCount") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return Some(element_child_ids(dom, node_id).len().to_string());
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".classList.contains") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        let args = split_js_arguments(args);
        let token = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return Some(get_element_class_contains(dom, node_id, &token).to_string());
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".matches") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        let selector = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return Some(node_matches_selector(dom, node_id, &selector).to_string());
    }
    if let Some(target) = expression.strip_suffix(".classList.length") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return Some(get_element_class_list_len(dom, node_id).to_string());
    }
    if let Some(target) = expression.strip_suffix(".length")
        && let Some(node_ids) = resolve_js_node_list_ref(dom, runtime, target)
    {
        return Some(node_ids.len().to_string());
    }
    if let Some(kind) = web_storage_length_expression_kind(expression) {
        return Some(web_storage(runtime, kind).len().to_string());
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".getItem")
        && let Some(kind) = web_storage_kind(target)
    {
        let args = split_js_arguments(args);
        let key = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return Some(
            web_storage(runtime, kind)
                .get(&key)
                .cloned()
                .unwrap_or_default(),
        );
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".getAttribute") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        let args = split_js_arguments(args);
        let name = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return get_element_attribute(dom, node_id, &name);
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".getPropertyValue")
        && let Some(target) = target.strip_suffix(".style").map(str::trim)
    {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        let args = split_js_arguments(args);
        let property = args
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return get_element_style_property(dom, node_id, &property);
    }
    if let Some((target, property)) = parse_js_style_property_ref(expression) {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return get_element_style_property(dom, node_id, property);
    }
    None
}

fn evaluate_js_event_expression(runtime: &TinyJsRuntime, expression: &str) -> Option<String> {
    let event = runtime.current_event.as_ref()?;
    match expression.trim() {
        "event.type" => Some(event.type_name.clone()),
        "event.key" => Some(event.key.clone().unwrap_or_default()),
        "event.data" => Some(event.data.clone().unwrap_or_default()),
        "event.inputType" => Some(event.input_type.clone().unwrap_or_default()),
        "event.clientX" => Some(
            event
                .client_x
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.clientY" => Some(
            event
                .client_y
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.button" => Some(
            event
                .button
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.pointerId" => Some(
            event
                .pointer_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.pointerType" => Some(event.pointer_type.clone().unwrap_or_default()),
        "event.isPrimary" => Some(
            event
                .is_primary
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.deltaX" => Some(
            event
                .delta_x
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.deltaY" => Some(
            event
                .delta_y
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        "event.eventPhase" => Some(event.phase.as_dom_event_phase().to_owned()),
        "event.defaultPrevented" => Some(runtime.default_prevented.to_string()),
        _ => None,
    }
}

fn evaluate_js_event_target_expression(
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<String> {
    let expression = expression.trim();
    match expression {
        "event.currentTarget === window"
        | "event.currentTarget == window"
        | "window === event.currentTarget"
        | "window == event.currentTarget"
        | "this === window"
        | "this == window"
        | "window === this"
        | "window == this" => {
            return Some((runtime.this_target == Some(BrowserEventTarget::Window)).to_string());
        }
        "event.currentTarget === document"
        | "event.currentTarget == document"
        | "document === event.currentTarget"
        | "document == event.currentTarget"
        | "this === document"
        | "this == document"
        | "document === this"
        | "document == this" => {
            return Some((runtime.this_target == Some(BrowserEventTarget::Node(0))).to_string());
        }
        "event.currentTarget.location"
        | "event.currentTarget.location.href"
        | "this.location"
        | "this.location.href" => {
            if runtime.this_target == Some(BrowserEventTarget::Window) {
                return Some(location_property(&runtime.page_source, "href"));
            }
        }
        _ => {}
    }
    for prefix in ["event.currentTarget.location.", "this.location."] {
        if let Some(property) = expression.strip_prefix(prefix)
            && runtime.this_target == Some(BrowserEventTarget::Window)
            && is_supported_location_property(property)
        {
            return Some(location_property(&runtime.page_source, property));
        }
    }
    None
}

fn web_storage_length_expression_kind(expression: &str) -> Option<BrowserWebStorageKind> {
    match expression.trim() {
        "localStorage.length" | "window.localStorage.length" => Some(BrowserWebStorageKind::Local),
        "sessionStorage.length" | "window.sessionStorage.length" => {
            Some(BrowserWebStorageKind::Session)
        }
        _ => None,
    }
}

fn evaluate_js_location_expression(source: &str, expression: &str) -> Option<String> {
    let expression = expression.trim();
    match expression {
        "location"
        | "window.location"
        | "document.location"
        | "location.href"
        | "window.location.href"
        | "document.location.href"
        | "document.URL"
        | "document.documentURI"
        | "document.baseURI" => {
            return Some(location_property(source, "href"));
        }
        _ => {}
    }
    if let Some((target, _args)) = parse_js_method_call(expression, ".toString")
        && is_location_object_ref(target)
    {
        return Some(location_property(source, "href"));
    }
    for prefix in ["location.", "window.location.", "document.location."] {
        if let Some(property) = expression.strip_prefix(prefix)
            && is_supported_location_property(property)
        {
            return Some(location_property(source, property));
        }
    }
    None
}

fn is_location_object_ref(target: &str) -> bool {
    matches!(
        target.trim(),
        "location" | "window.location" | "document.location"
    )
}

fn is_supported_location_property(property: &str) -> bool {
    matches!(
        property,
        "href"
            | "protocol"
            | "host"
            | "hostname"
            | "port"
            | "pathname"
            | "search"
            | "hash"
            | "origin"
    )
}

fn location_property(source: &str, property: &str) -> String {
    if let Ok(url) = Url::parse(source) {
        return parsed_url_location_property(&url, property);
    }
    match property {
        "href" => source.to_owned(),
        "pathname" => source_pathname(source),
        "search" => source_search(source),
        "hash" => source_hash(source),
        _ => String::new(),
    }
}

fn parsed_url_location_property(url: &Url, property: &str) -> String {
    match property {
        "href" => url.to_string(),
        "protocol" => format!("{}:", url.scheme()),
        "host" => location_host(url),
        "hostname" => url.host_str().unwrap_or_default().to_owned(),
        "port" => url.port().map(|port| port.to_string()).unwrap_or_default(),
        "pathname" => url.path().to_owned(),
        "search" => url
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default(),
        "hash" => url
            .fragment()
            .map(|fragment| format!("#{fragment}"))
            .unwrap_or_default(),
        "origin" => location_origin(url),
        _ => String::new(),
    }
}

fn location_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

fn location_origin(url: &Url) -> String {
    match url.scheme() {
        "http" | "https" => format!("{}://{}", url.scheme(), location_host(url)),
        "file" => "file://".to_owned(),
        scheme => url
            .host_str()
            .map(|host| format!("{scheme}://{host}"))
            .unwrap_or_else(|| "null".to_owned()),
    }
}

fn source_pathname(source: &str) -> String {
    let without_hash = source
        .split_once('#')
        .map_or(source, |(before_hash, _)| before_hash);
    without_hash
        .split_once('?')
        .map_or(without_hash, |(before_query, _)| before_query)
        .to_owned()
}

fn source_search(source: &str) -> String {
    let without_hash = source
        .split_once('#')
        .map_or(source, |(before_hash, _)| before_hash);
    without_hash
        .split_once('?')
        .map(|(_, query)| format!("?{query}"))
        .unwrap_or_default()
}

fn source_hash(source: &str) -> String {
    source
        .split_once('#')
        .map(|(_, fragment)| format!("#{fragment}"))
        .unwrap_or_default()
}

fn resolve_js_node_ref(dom: &Dom, runtime: &TinyJsRuntime, expression: &str) -> Option<usize> {
    let expression = expression.trim();
    if expression == "document"
        && matches!(
            dom.nodes.first().map(|node| &node.kind),
            Some(NodeKind::Document)
        )
    {
        return Some(0);
    }
    if let Some(&node_id) = runtime.bindings.get(expression)
        && node_id < dom.nodes.len()
    {
        return Some(node_id);
    }
    if let Some(node_id) = resolve_js_node_list_item_ref(dom, runtime, expression) {
        return Some(node_id);
    }
    if expression == "this" {
        if let Some(BrowserEventTarget::Node(node_id)) = runtime.this_target
            && node_id < dom.nodes.len()
        {
            return Some(node_id);
        }
        return runtime
            .this_node
            .filter(|&node_id| node_id < dom.nodes.len());
    }
    if expression == "document.activeElement" {
        return runtime
            .active_element
            .filter(|&node_id| node_id < dom.nodes.len());
    }
    if expression == "event.target" {
        return runtime
            .current_event
            .as_ref()
            .map(|event| event.target_node)
            .filter(|&node_id| node_id < dom.nodes.len());
    }
    if expression == "event.currentTarget" {
        return runtime.this_target.and_then(|target| match target {
            BrowserEventTarget::Node(node_id) if node_id < dom.nodes.len() => Some(node_id),
            _ => None,
        });
    }
    if let Some(target) = expression
        .strip_suffix(".parentNode")
        .or_else(|| expression.strip_suffix(".parentElement"))
    {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return dom.nodes.get(node_id).and_then(|node| node.parent);
    }
    if let Some(target) = expression.strip_suffix(".firstChild") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return dom.nodes.get(node_id)?.children.first().copied();
    }
    if let Some(target) = expression.strip_suffix(".lastChild") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return dom.nodes.get(node_id)?.children.last().copied();
    }
    if let Some(target) = expression.strip_suffix(".firstElementChild") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return element_child_ids(dom, node_id).first().copied();
    }
    if let Some(target) = expression.strip_suffix(".lastElementChild") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return element_child_ids(dom, node_id).last().copied();
    }
    if let Some(target) = expression.strip_suffix(".nextElementSibling") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return element_sibling(dom, node_id, 1);
    }
    if let Some(target) = expression.strip_suffix(".previousElementSibling") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return element_sibling(dom, node_id, -1);
    }
    if expression == "document.body" {
        return find_first_element_by_tag(dom, "body");
    }
    if expression == "document.head" {
        return find_first_element_by_tag(dom, "head");
    }
    if expression == "document.documentElement" {
        return find_first_element_by_tag(dom, "html");
    }
    if let Some(id) = parse_js_call_string_arg(expression, "document.getElementById") {
        return find_element_by_id(dom, &id);
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".closest") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        let selector = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        return closest_matching_selector(dom, node_id, &selector);
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".querySelector") {
        let selector = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        let scope = js_query_scope(dom, runtime, target)?;
        return find_first_matching_selector_in_scope(dom, &selector, scope);
    }
    None
}

fn resolve_js_node_list_item_ref(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<usize> {
    let expression = expression.trim();
    if let Some((target, args)) = parse_js_method_call(expression, ".item") {
        let index = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_index_expression(dom, runtime, arg))?;
        return resolve_js_node_list_ref(dom, runtime, target)
            .and_then(|nodes| nodes.get(index).copied());
    }
    let (target, index) = parse_js_index_ref(expression)?;
    let index = evaluate_js_index_expression(dom, runtime, index)?;
    resolve_js_node_list_ref(dom, runtime, target).and_then(|nodes| nodes.get(index).copied())
}

fn parse_js_index_ref(expression: &str) -> Option<(&str, &str)> {
    let expression = expression.trim();
    let inner = expression.strip_suffix(']')?;
    let open = inner.rfind('[')?;
    Some((inner[..open].trim(), inner[open + 1..].trim()))
}

fn evaluate_js_index_expression(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<usize> {
    expression.trim().parse::<usize>().ok().or_else(|| {
        evaluate_js_string_expression(dom, runtime, expression)?
            .parse()
            .ok()
    })
}

fn resolve_js_node_list_ref(
    dom: &Dom,
    runtime: &TinyJsRuntime,
    expression: &str,
) -> Option<Vec<usize>> {
    let expression = expression.trim();
    if let Some(node_ids) = runtime.node_list_bindings.get(expression) {
        return Some(
            node_ids
                .iter()
                .copied()
                .filter(|&node_id| node_id < dom.nodes.len())
                .collect(),
        );
    }
    if let Some(target) = expression.strip_suffix(".children") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return Some(element_child_ids(dom, node_id));
    }
    if let Some(target) = expression.strip_suffix(".childNodes") {
        let node_id = resolve_js_node_ref(dom, runtime, target)?;
        return dom.nodes.get(node_id).map(|node| node.children.clone());
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".querySelectorAll") {
        let selector = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        let scope = js_query_scope(dom, runtime, target)?;
        return Some(find_all_matching_selector_in_scope(dom, &selector, scope));
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".getElementsByClassName") {
        let class_names = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        let scope = js_query_scope(dom, runtime, target)?;
        return Some(find_elements_by_class_name_in_scope(
            dom,
            &class_names,
            scope,
        ));
    }
    if let Some((target, args)) = parse_js_method_call(expression, ".getElementsByTagName") {
        let tag = split_js_arguments(args)
            .first()
            .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg))?;
        let scope = js_query_scope(dom, runtime, target)?;
        return Some(find_elements_by_tag_name_in_scope(dom, &tag, scope));
    }
    None
}

fn js_query_scope(dom: &Dom, runtime: &TinyJsRuntime, target: &str) -> Option<usize> {
    if target.trim() == "document" {
        Some(0)
    } else {
        resolve_js_node_ref(dom, runtime, target)
    }
}

fn parse_js_call_string_arg(expression: &str, function_name: &str) -> Option<String> {
    let rest = expression.strip_prefix(function_name)?.trim_start();
    let rest = rest.strip_prefix('(')?.trim();
    let close = rest.rfind(')')?;
    parse_js_string_value(rest[..close].trim())
}

fn find_element_by_id(dom: &Dom, id: &str) -> Option<usize> {
    dom.nodes
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element) if element.id.as_deref() == Some(id) => Some(node_id),
            _ => None,
        })
}

fn find_first_element_by_tag(dom: &Dom, tag: &str) -> Option<usize> {
    dom.nodes
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element) if element.tag == tag => Some(node_id),
            _ => None,
        })
}

fn find_first_matching_selector(dom: &Dom, selector: &str) -> Option<usize> {
    find_first_matching_selector_in_scope(dom, selector, 0)
}

fn node_matches_selector(dom: &Dom, node_id: usize, selector: &str) -> bool {
    parse_selector(selector.trim())
        .as_ref()
        .is_some_and(|selector| selector_matches(selector, dom, node_id))
}

fn closest_matching_selector(dom: &Dom, node_id: usize, selector: &str) -> Option<usize> {
    let selector = parse_selector(selector.trim())?;
    let mut current = Some(node_id);
    while let Some(current_id) = current {
        if selector_matches(&selector, dom, current_id) {
            return Some(current_id);
        }
        current = dom.nodes.get(current_id).and_then(|node| node.parent);
    }
    None
}

fn find_first_matching_selector_in_scope(dom: &Dom, selector: &str, scope: usize) -> Option<usize> {
    let selector = parse_selector(selector.trim())?;
    dom.nodes.iter().enumerate().find_map(|(node_id, _)| {
        (node_in_query_scope(dom, node_id, scope) && selector_matches(&selector, dom, node_id))
            .then_some(node_id)
    })
}

fn find_all_matching_selector_in_scope(dom: &Dom, selector: &str, scope: usize) -> Vec<usize> {
    let Some(selector) = parse_selector(selector.trim()) else {
        return Vec::new();
    };
    dom.nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, _)| {
            (node_in_query_scope(dom, node_id, scope) && selector_matches(&selector, dom, node_id))
                .then_some(node_id)
        })
        .collect()
}

fn find_elements_by_class_name_in_scope(dom: &Dom, class_names: &str, scope: usize) -> Vec<usize> {
    let classes = class_names
        .split_ascii_whitespace()
        .filter(|class| !class.is_empty())
        .collect::<Vec<_>>();
    if classes.is_empty() {
        return Vec::new();
    }
    dom.nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element)
                if node_in_query_scope(dom, node_id, scope)
                    && classes.iter().all(|class| {
                        element
                            .classes
                            .iter()
                            .any(|element_class| element_class == class)
                    }) =>
            {
                Some(node_id)
            }
            _ => None,
        })
        .collect()
}

fn find_elements_by_tag_name_in_scope(dom: &Dom, tag: &str, scope: usize) -> Vec<usize> {
    let tag = tag.trim().to_ascii_lowercase();
    if tag.is_empty() {
        return Vec::new();
    }
    dom.nodes
        .iter()
        .enumerate()
        .filter_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element)
                if node_in_query_scope(dom, node_id, scope)
                    && (tag == "*" || element.tag == tag) =>
            {
                Some(node_id)
            }
            _ => None,
        })
        .collect()
}

fn node_in_query_scope(dom: &Dom, node_id: usize, scope: usize) -> bool {
    node_id != scope && node_id < dom.nodes.len() && is_descendant_of(dom, node_id, scope)
}

fn element_child_ids(dom: &Dom, node_id: usize) -> Vec<usize> {
    let Some(node) = dom.nodes.get(node_id) else {
        return Vec::new();
    };
    node.children
        .iter()
        .copied()
        .filter(|&child_id| {
            matches!(
                dom.nodes.get(child_id).map(|node| &node.kind),
                Some(NodeKind::Element(_))
            )
        })
        .collect()
}

fn element_sibling(dom: &Dom, node_id: usize, direction: isize) -> Option<usize> {
    let parent_id = dom.nodes.get(node_id)?.parent?;
    let siblings = element_child_ids(dom, parent_id);
    let index = siblings
        .iter()
        .position(|&sibling_id| sibling_id == node_id)?;
    let next_index = index.checked_add_signed(direction)?;
    siblings.get(next_index).copied()
}

fn set_document_title(dom: &mut Dom, value: &str, append: bool) {
    let title_id = find_first_element_by_tag(dom, "title").unwrap_or_else(|| {
        let parent = find_first_element_by_tag(dom, "head").unwrap_or(0);
        push_node(
            dom,
            parent,
            NodeKind::Element(Box::new(empty_element_data("title"))),
        )
    });
    set_text_content(dom, title_id, value, append);
}

fn set_text_content(dom: &mut Dom, node_id: usize, value: &str, append: bool) {
    if node_id >= dom.nodes.len() {
        return;
    }
    let text = if append {
        let mut current = text_content(dom, node_id);
        current.push_str(value);
        current
    } else {
        value.to_owned()
    };
    detach_children(dom, node_id);
    if !text.is_empty() {
        push_node(dom, node_id, NodeKind::Text(text));
    }
}

fn set_inner_html(dom: &mut Dom, node_id: usize, value: &str, append: bool) {
    if node_id >= dom.nodes.len() {
        return;
    }
    if !append {
        detach_children(dom, node_id);
    }
    let parsed = parse_html(value.as_bytes());
    for child_id in parsed
        .dom
        .nodes
        .first()
        .map(|node| node.children.clone())
        .unwrap_or_default()
    {
        clone_dom_subtree(dom, &parsed.dom, child_id, node_id);
    }
}

fn clone_dom_subtree(
    dest: &mut Dom,
    source: &Dom,
    source_id: usize,
    parent_id: usize,
) -> Option<usize> {
    let source_node = source.nodes.get(source_id)?;
    let node_id = push_node(dest, parent_id, source_node.kind.clone());
    for child_id in source_node.children.iter().copied() {
        clone_dom_subtree(dest, source, child_id, node_id);
    }
    Some(node_id)
}

fn detach_children(dom: &mut Dom, node_id: usize) {
    let children = std::mem::take(&mut dom.nodes[node_id].children);
    for child in children {
        if let Some(node) = dom.nodes.get_mut(child)
            && node.parent == Some(node_id)
        {
            node.parent = None;
        }
    }
}

fn set_element_string_property(
    dom: &mut Dom,
    node_id: usize,
    property: &str,
    value: &str,
    append: bool,
) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };

    match property {
        "id" => {
            let mut next = attr_string(element.id.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "id", &next);
        }
        "className" => {
            let current = element.classes.join(" ");
            let mut next = attr_string((!current.is_empty()).then_some(current.as_str()), append);
            next.push_str(value);
            set_element_attribute_data(element, "class", &next);
        }
        "href" => {
            let mut next = attr_string(element.href.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "href", &next);
        }
        "value" => {
            let mut next = attr_string(element.value.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "value", &next);
        }
        "name" => {
            let mut next = attr_string(element.name.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "name", &next);
        }
        "type" => {
            let mut next = attr_string(element.type_hint.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "type", &next);
        }
        "src" => {
            let mut next = attr_string(element.src.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "src", &next);
        }
        "alt" => {
            let mut next = attr_string(element.alt.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "alt", &next);
        }
        "action" => {
            let mut next = attr_string(element.action.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "action", &next);
        }
        "method" => {
            let mut next = attr_string(element.method.as_deref(), append);
            next.push_str(value);
            set_element_attribute_data(element, "method", &next);
        }
        _ => {}
    }
}

fn set_element_boolean_property(dom: &mut Dom, node_id: usize, property: &str, value: bool) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    set_element_boolean_attribute_data(element, property, value);
}

fn js_truthy_string(value: &str) -> bool {
    parse_js_boolish(value).unwrap_or_else(|| !value.is_empty())
}

fn evaluate_js_class_tokens(dom: &Dom, runtime: &TinyJsRuntime, args: &str) -> Vec<String> {
    split_js_arguments(args)
        .into_iter()
        .filter_map(|arg| evaluate_js_string_expression(dom, runtime, arg))
        .filter_map(|token| normalize_class_token(&token))
        .collect()
}

fn add_element_class_tokens(dom: &mut Dom, node_id: usize, tokens: &[String]) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    for token in tokens {
        if !element.classes.iter().any(|class| class == token) {
            element.classes.push(token.clone());
        }
    }
    sync_element_class_attribute(element);
}

fn remove_element_class_tokens(dom: &mut Dom, node_id: usize, tokens: &[String]) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    element
        .classes
        .retain(|class| !tokens.iter().any(|token| token == class));
    sync_element_class_attribute(element);
}

fn toggle_element_class_token(dom: &mut Dom, node_id: usize, token: &str, force: Option<bool>) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    let contains = element.classes.iter().any(|class| class == token);
    match (contains, force) {
        (true, Some(false) | None) => element.classes.retain(|class| class != token),
        (false, Some(true) | None) => element.classes.push(token.to_owned()),
        _ => {}
    }
    sync_element_class_attribute(element);
}

fn get_element_class_contains(dom: &Dom, node_id: usize, token: &str) -> bool {
    let Some(token) = normalize_class_token(token) else {
        return false;
    };
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    element.classes.iter().any(|class| class == &token)
}

fn get_element_class_list_len(dom: &Dom, node_id: usize) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return 0;
    };
    element.classes.len()
}

fn sync_element_class_attribute(element: &mut ElementData) {
    let class_attr = element.classes.join(" ");
    set_element_attribute_data(element, "class", &class_attr);
}

fn normalize_class_token(token: &str) -> Option<String> {
    let token = token.trim();
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then(|| token.to_owned())
}

fn parse_js_boolish(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "1" => Some(true),
        "false" | "0" | "" => Some(false),
        _ => None,
    }
}

fn parse_js_style_property_ref(expression: &str) -> Option<(&str, &str)> {
    let (target, property) = expression.trim().rsplit_once(".style.")?;
    let property = property.trim();
    if property.is_empty()
        || property
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$')))
    {
        return None;
    }
    Some((target.trim(), property))
}

fn set_element_style_property(
    dom: &mut Dom,
    node_id: usize,
    property: &str,
    value: &str,
    append: bool,
) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    set_element_style_property_data(element, property, value, append);
}

fn remove_element_style_property(dom: &mut Dom, node_id: usize, property: &str) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    set_element_style_property_data(element, property, "", false);
}

fn get_element_style_property(dom: &Dom, node_id: usize, property: &str) -> Option<String> {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return None;
    };
    get_element_style_property_data(element, property)
}

fn get_element_string_property(dom: &Dom, node_id: usize, property: &str) -> Option<String> {
    match property {
        "textContent" | "innerText" => Some(text_content(dom, node_id)),
        "innerHTML" => Some(inner_html(dom, node_id)),
        "id" | "className" | "href" | "value" | "name" | "type" | "src" | "alt" | "action"
        | "method" => get_element_attribute(dom, node_id, property).or_else(|| Some(String::new())),
        "checked" | "disabled" | "hidden" | "selected" => {
            Some(get_element_boolean_property(dom, node_id, property).to_string())
        }
        "tagName" => match dom.nodes.get(node_id).map(|node| &node.kind) {
            Some(NodeKind::Element(element)) => Some(element.tag.to_ascii_uppercase()),
            _ => None,
        },
        "nodeName" => match dom.nodes.get(node_id).map(|node| &node.kind) {
            Some(NodeKind::Document) => Some("#document".to_owned()),
            Some(NodeKind::DocumentFragment) => Some("#document-fragment".to_owned()),
            Some(NodeKind::Text(_)) => Some("#text".to_owned()),
            Some(NodeKind::Element(element)) => Some(element.tag.to_ascii_uppercase()),
            None => None,
        },
        "nodeType" => match dom.nodes.get(node_id).map(|node| &node.kind) {
            Some(NodeKind::Element(_)) => Some("1".to_owned()),
            Some(NodeKind::Text(_)) => Some("3".to_owned()),
            Some(NodeKind::Document) => Some("9".to_owned()),
            Some(NodeKind::DocumentFragment) => Some("11".to_owned()),
            None => None,
        },
        _ => None,
    }
}

fn inner_html(dom: &Dom, node_id: usize) -> String {
    let mut html = String::new();
    for child_id in dom
        .nodes
        .get(node_id)
        .map(|node| node.children.as_slice())
        .unwrap_or_default()
    {
        serialize_html_node(dom, *child_id, &mut html);
    }
    html
}

fn serialize_html_node(dom: &Dom, node_id: usize, out: &mut String) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.kind {
        NodeKind::Document | NodeKind::DocumentFragment => {
            for child_id in node.children.iter().copied() {
                serialize_html_node(dom, child_id, out);
            }
        }
        NodeKind::Text(text) => escape_html_text(text, out),
        NodeKind::Element(element) => {
            out.push('<');
            out.push_str(&element.tag);
            let mut attrs = element.attrs.iter().collect::<Vec<_>>();
            attrs.sort_by(|left, right| left.0.cmp(right.0));
            for (name, value) in attrs {
                out.push(' ');
                out.push_str(name);
                out.push_str("=\"");
                escape_html_attr(value, out);
                out.push('"');
            }
            out.push('>');
            if is_void_tag(&element.tag) {
                return;
            }
            for child_id in node.children.iter().copied() {
                serialize_html_node(dom, child_id, out);
            }
            out.push_str("</");
            out.push_str(&element.tag);
            out.push('>');
        }
    }
}

fn escape_html_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn escape_html_attr(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn set_element_style_property_data(
    element: &mut ElementData,
    property: &str,
    value: &str,
    append: bool,
) {
    let Some(property) = normalize_css_style_property(property) else {
        return;
    };
    let mut declarations = parse_style_attribute_declarations(element.style.as_deref());
    let index = declarations
        .iter()
        .position(|(name, _)| normalize_css_style_property(name).as_deref() == Some(&property));
    let mut next_value = String::new();
    if append && let Some(existing) = index.and_then(|index| declarations.get(index)) {
        next_value.push_str(existing.1.as_str());
    }
    next_value.push_str(value);
    if next_value.trim().is_empty() {
        if let Some(index) = index {
            declarations.remove(index);
        }
    } else if let Some(index) = index {
        declarations[index].0 = property;
        declarations[index].1 = next_value;
    } else {
        declarations.push((property, next_value));
    }
    let style = serialize_style_attribute_declarations(&declarations);
    set_element_attribute_data(element, "style", &style);
}

fn get_element_style_property_data(element: &ElementData, property: &str) -> Option<String> {
    let property = normalize_css_style_property(property)?;
    parse_style_attribute_declarations(element.style.as_deref())
        .into_iter()
        .find_map(|(name, value)| {
            (normalize_css_style_property(&name).as_deref() == Some(&property))
                .then(|| serialize_css_style_property_value(&property, &value))
        })
}

fn parse_style_attribute_declarations(style: Option<&str>) -> Vec<(String, String)> {
    style
        .unwrap_or_default()
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn serialize_style_attribute_declarations(declarations: &[(String, String)]) -> String {
    declarations
        .iter()
        .filter(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
        .map(|(name, value)| format!("{}: {}", name.trim(), value.trim()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn normalize_css_style_property(property: &str) -> Option<String> {
    let property = property.trim();
    if property.is_empty() {
        return None;
    }
    let mut out = String::new();
    for ch in property.chars() {
        if ch.is_ascii_uppercase() {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch == '_' {
            out.push('-');
        } else if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' {
            out.push(ch);
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn serialize_css_style_property_value(property: &str, value: &str) -> String {
    if matches!(
        property,
        "color" | "background" | "background-color" | "border-color"
    ) && let Some((red, green, blue)) = parse_hex_color_rgb(value.trim())
    {
        return format!("rgb({red}, {green}, {blue})");
    }
    value.to_owned()
}

fn attr_string(current: Option<&str>, append: bool) -> String {
    if append {
        current.unwrap_or_default().to_owned()
    } else {
        String::new()
    }
}

fn set_element_attribute(dom: &mut Dom, node_id: usize, name: &str, value: &str) {
    let Some(NodeKind::Element(element)) = dom.nodes.get_mut(node_id).map(|node| &mut node.kind)
    else {
        return;
    };
    set_element_attribute_data(element, name, value);
}

fn get_element_attribute(dom: &Dom, node_id: usize, name: &str) -> Option<String> {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return None;
    };
    get_element_attribute_data(element, name)
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.is_empty()).cloned()
}

fn non_empty_owned(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn boolean_attribute(value: bool) -> Option<String> {
    value.then(String::new)
}

fn get_element_attribute_data(element: &ElementData, name: &str) -> Option<String> {
    let name = normalize_attribute_name(name);
    if let Some(value) = element.attrs.get(&name) {
        return Some(value.clone());
    }
    match name.as_str() {
        "id" => non_empty(element.id.as_ref()),
        "class" => non_empty_owned(element.classes.join(" ")),
        "style" => non_empty(element.style.as_ref()),
        "href" => non_empty(element.href.as_ref()),
        "src" => non_empty(element.src.as_ref()),
        "srcset" => non_empty(element.srcset.as_ref()),
        "rel" => non_empty(element.rel.as_ref()),
        "media" => non_empty(element.media.as_ref()),
        "alt" => non_empty(element.alt.as_ref()),
        "data" => non_empty(element.data.as_ref()),
        "name" => non_empty(element.name.as_ref()),
        "value" => non_empty(element.value.as_ref()),
        "type" => non_empty(element.type_hint.as_ref()),
        "poster" => non_empty(element.poster.as_ref()),
        "action" => non_empty(element.action.as_ref()),
        "method" => non_empty(element.method.as_ref()),
        "onclick" => non_empty(element.onclick.as_ref()),
        "hidden" => boolean_attribute(element.hidden),
        "disabled" => boolean_attribute(element.disabled),
        "checked" => boolean_attribute(element.checked),
        "selected" => boolean_attribute(element.selected),
        _ => None,
    }
}

fn normalize_attribute_name(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "classname" => "class".to_owned(),
        other => other.to_owned(),
    }
}

fn set_optional_attribute(slot: &mut Option<String>, value: &str) {
    *slot = if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    };
}

fn set_element_attribute_data(element: &mut ElementData, name: &str, value: &str) {
    let name = normalize_attribute_name(name);
    element.attrs.insert(name.clone(), value.to_owned());
    match name.as_str() {
        "id" => set_optional_attribute(&mut element.id, value),
        "class" => {
            element.classes = value.split_ascii_whitespace().map(str::to_owned).collect();
        }
        "style" => set_optional_attribute(&mut element.style, value),
        "href" => set_optional_attribute(&mut element.href, value),
        "src" => set_optional_attribute(&mut element.src, value),
        "srcset" => set_optional_attribute(&mut element.srcset, value),
        "rel" => set_optional_attribute(&mut element.rel, value),
        "media" => set_optional_attribute(&mut element.media, value),
        "alt" => set_optional_attribute(&mut element.alt, value),
        "data" => set_optional_attribute(&mut element.data, value),
        "name" => set_optional_attribute(&mut element.name, value),
        "value" => set_optional_attribute(&mut element.value, value),
        "type" => {
            set_optional_attribute(&mut element.type_hint, value);
            element.input_type = if value.is_empty() {
                None
            } else {
                Some(value.to_ascii_lowercase())
            };
        }
        "poster" => set_optional_attribute(&mut element.poster, value),
        "action" => set_optional_attribute(&mut element.action, value),
        "method" => {
            element.method = if value.is_empty() {
                None
            } else {
                Some(value.to_ascii_uppercase())
            };
        }
        "onclick" => set_optional_attribute(&mut element.onclick, value),
        "hidden" => element.hidden = true,
        "disabled" => element.disabled = true,
        "checked" => element.checked = true,
        "selected" => element.selected = true,
        _ => {}
    }
}

fn set_element_boolean_attribute_data(element: &mut ElementData, name: &str, value: bool) {
    let name = normalize_attribute_name(name);
    if value {
        element.attrs.insert(name.clone(), String::new());
    } else {
        element.attrs.remove(&name);
    }
    match name.as_str() {
        "hidden" => element.hidden = value,
        "disabled" => element.disabled = value,
        "checked" => element.checked = value,
        "selected" => element.selected = value,
        _ => {}
    }
}

fn get_element_boolean_property(dom: &Dom, node_id: usize, property: &str) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    match normalize_attribute_name(property).as_str() {
        "hidden" => element.hidden,
        "disabled" => element.disabled,
        "checked" => element.checked,
        "selected" => element.selected,
        _ => false,
    }
}

fn append_child(dom: &mut Dom, parent_id: usize, child_id: usize) {
    if parent_id >= dom.nodes.len()
        || child_id >= dom.nodes.len()
        || parent_id == child_id
        || is_descendant_of(dom, parent_id, child_id)
    {
        return;
    }

    if matches!(
        dom.nodes.get(child_id).map(|node| &node.kind),
        Some(NodeKind::DocumentFragment)
    ) {
        append_document_fragment_children(dom, parent_id, child_id);
        return;
    }

    if let Some(old_parent) = dom.nodes[child_id].parent
        && let Some(parent) = dom.nodes.get_mut(old_parent)
    {
        parent.children.retain(|&id| id != child_id);
    }
    dom.nodes[child_id].parent = Some(parent_id);
    if !dom.nodes[parent_id].children.contains(&child_id) {
        dom.nodes[parent_id].children.push(child_id);
    }
}

fn insert_child_before(
    dom: &mut Dom,
    parent_id: usize,
    child_id: usize,
    reference_id: Option<usize>,
) {
    if parent_id >= dom.nodes.len()
        || child_id >= dom.nodes.len()
        || parent_id == child_id
        || is_descendant_of(dom, parent_id, child_id)
    {
        return;
    }

    if matches!(
        dom.nodes.get(child_id).map(|node| &node.kind),
        Some(NodeKind::DocumentFragment)
    ) {
        insert_document_fragment_children_before(dom, parent_id, child_id, reference_id);
        return;
    }

    let insert_index = match reference_id {
        Some(reference_id) => {
            if reference_id == child_id {
                return;
            }
            let Some(index) = dom.nodes[parent_id]
                .children
                .iter()
                .position(|&id| id == reference_id)
            else {
                return;
            };
            index
        }
        None => dom.nodes[parent_id].children.len(),
    };
    let old_index = (dom.nodes[child_id].parent == Some(parent_id))
        .then(|| {
            dom.nodes[parent_id]
                .children
                .iter()
                .position(|&id| id == child_id)
        })
        .flatten();

    detach_from_parent(dom, child_id);
    dom.nodes[child_id].parent = Some(parent_id);
    let adjusted_index = if old_index.is_some_and(|old_index| old_index < insert_index) {
        insert_index.saturating_sub(1)
    } else {
        insert_index
    }
    .min(dom.nodes[parent_id].children.len());
    dom.nodes[parent_id]
        .children
        .insert(adjusted_index, child_id);
}

fn replace_child(dom: &mut Dom, parent_id: usize, new_child_id: usize, old_child_id: usize) {
    if parent_id >= dom.nodes.len()
        || new_child_id >= dom.nodes.len()
        || old_child_id >= dom.nodes.len()
        || parent_id == new_child_id
        || is_descendant_of(dom, parent_id, new_child_id)
    {
        return;
    }
    if matches!(
        dom.nodes.get(new_child_id).map(|node| &node.kind),
        Some(NodeKind::DocumentFragment)
    ) {
        insert_document_fragment_children_before(dom, parent_id, new_child_id, Some(old_child_id));
        remove_child(dom, parent_id, old_child_id);
        return;
    }
    let Some(index) = dom.nodes[parent_id]
        .children
        .iter()
        .position(|&id| id == old_child_id)
    else {
        return;
    };

    detach_from_parent(dom, new_child_id);
    remove_child(dom, parent_id, old_child_id);
    let index = index.min(dom.nodes[parent_id].children.len());
    dom.nodes[new_child_id].parent = Some(parent_id);
    dom.nodes[parent_id].children.insert(index, new_child_id);
}

fn append_document_fragment_children(dom: &mut Dom, parent_id: usize, fragment_id: usize) {
    let children = dom
        .nodes
        .get(fragment_id)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    for child_id in children {
        append_child(dom, parent_id, child_id);
    }
}

fn insert_document_fragment_children_before(
    dom: &mut Dom,
    parent_id: usize,
    fragment_id: usize,
    reference_id: Option<usize>,
) {
    let children = dom
        .nodes
        .get(fragment_id)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    for child_id in children {
        insert_child_before(dom, parent_id, child_id, reference_id);
    }
}

fn remove_node(dom: &mut Dom, node_id: usize) {
    if node_id >= dom.nodes.len() {
        return;
    }
    if let Some(parent_id) = dom.nodes[node_id].parent {
        remove_child(dom, parent_id, node_id);
    }
}

fn remove_child(dom: &mut Dom, parent_id: usize, child_id: usize) {
    if parent_id >= dom.nodes.len() || child_id >= dom.nodes.len() {
        return;
    }
    if dom.nodes[child_id].parent != Some(parent_id) {
        return;
    }
    dom.nodes[parent_id].children.retain(|&id| id != child_id);
    dom.nodes[child_id].parent = None;
}

fn next_child_after(dom: &Dom, parent_id: usize, child_id: usize) -> Option<usize> {
    let children = dom.nodes.get(parent_id)?.children.as_slice();
    let index = children.iter().position(|&id| id == child_id)?;
    children.get(index + 1).copied()
}

fn detach_from_parent(dom: &mut Dom, child_id: usize) {
    if let Some(old_parent) = dom.nodes.get(child_id).and_then(|node| node.parent)
        && let Some(parent) = dom.nodes.get_mut(old_parent)
    {
        parent.children.retain(|&id| id != child_id);
    }
}

fn is_descendant_of(dom: &Dom, node_id: usize, possible_ancestor: usize) -> bool {
    let mut current = dom.nodes.get(node_id).and_then(|node| node.parent);
    while let Some(parent) = current {
        if parent == possible_ancestor {
            return true;
        }
        current = dom.nodes.get(parent).and_then(|node| node.parent);
    }
    false
}

fn empty_element_data(tag: &str) -> ElementData {
    ElementData {
        tag: tag.to_owned(),
        attrs: HashMap::new(),
        id: None,
        classes: Vec::new(),
        style: None,
        href: None,
        src: None,
        srcset: None,
        rel: None,
        media: None,
        alt: None,
        data: None,
        name: None,
        value: None,
        input_type: None,
        type_hint: None,
        poster: None,
        action: None,
        method: None,
        onclick: None,
        hidden: false,
        disabled: false,
        checked: false,
        selected: false,
    }
}

fn element_data_from_attrs(tag: String, attrs: HashMap<String, String>) -> ElementData {
    let type_hint = attr_from_attrs(&attrs, "type");

    ElementData {
        tag,
        id: attr_from_attrs(&attrs, "id"),
        classes: attr_from_attrs(&attrs, "class")
            .map(|value| value.split_ascii_whitespace().map(str::to_owned).collect())
            .unwrap_or_default(),
        style: attr_from_attrs(&attrs, "style"),
        href: attr_from_attrs(&attrs, "href"),
        src: attr_from_attrs(&attrs, "src"),
        srcset: attr_from_attrs(&attrs, "srcset"),
        rel: attr_from_attrs(&attrs, "rel"),
        media: attr_from_attrs(&attrs, "media"),
        alt: attr_from_attrs(&attrs, "alt"),
        data: attr_from_attrs(&attrs, "data"),
        name: attr_from_attrs(&attrs, "name"),
        value: attr_from_attrs(&attrs, "value"),
        input_type: type_hint.as_ref().map(|value| value.to_ascii_lowercase()),
        type_hint,
        poster: attr_from_attrs(&attrs, "poster"),
        action: attr_from_attrs(&attrs, "action"),
        method: attr_from_attrs(&attrs, "method").map(|value| value.to_ascii_uppercase()),
        onclick: attr_from_attrs(&attrs, "onclick"),
        hidden: attrs.contains_key("hidden"),
        disabled: attrs.contains_key("disabled"),
        checked: attrs.contains_key("checked"),
        selected: attrs.contains_key("selected"),
        attrs,
    }
}

fn attr_from_attrs(attrs: &HashMap<String, String>, name: &str) -> Option<String> {
    attrs.get(name).cloned()
}

fn pop_until(stack: &mut Vec<usize>, dom: &Dom, tag: &str) {
    while stack.len() > 1 {
        let node_id = stack.pop().unwrap_or(0);
        if current_element_is(dom, node_id, tag) {
            return;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TagKind {
    Opening,
    Closing,
}

#[derive(Debug)]
struct Tag {
    kind: TagKind,
    name: String,
    self_closing: bool,
}

fn parse_tag(raw: &[u8]) -> Option<Tag> {
    let mut i = 0;
    while i < raw.len() && raw[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= raw.len() || matches!(raw[i], b'!' | b'?') {
        return None;
    }

    let kind = if raw[i] == b'/' {
        i += 1;
        TagKind::Closing
    } else {
        TagKind::Opening
    };

    while i < raw.len() && raw[i].is_ascii_whitespace() {
        i += 1;
    }

    let name_start = i;
    while i < raw.len() && (raw[i].is_ascii_alphanumeric() || matches!(raw[i], b':' | b'-' | b'_'))
    {
        i += 1;
    }
    if i == name_start {
        return None;
    }

    let mut name = String::from_utf8_lossy(&raw[name_start..i]).into_owned();
    name.make_ascii_lowercase();
    let self_closing = raw.iter().rposition(|byte| !byte.is_ascii_whitespace())
        == Some(raw.len() - 1)
        && raw.last() == Some(&b'/');

    Some(Tag {
        kind,
        name,
        self_closing,
    })
}

fn parse_attributes(raw_tag: &[u8]) -> HashMap<String, String> {
    let mut attrs = HashMap::new();
    let mut i = 0;

    while i < raw_tag.len() && raw_tag[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < raw_tag.len() && raw_tag[i] == b'/' {
        i += 1;
    }
    while i < raw_tag.len() && raw_tag[i].is_ascii_whitespace() {
        i += 1;
    }
    while i < raw_tag.len()
        && (raw_tag[i].is_ascii_alphanumeric() || matches!(raw_tag[i], b':' | b'-' | b'_'))
    {
        i += 1;
    }

    while i < raw_tag.len() {
        while i < raw_tag.len()
            && !raw_tag[i].is_ascii_alphabetic()
            && raw_tag[i] != b'_'
            && raw_tag[i] != b'-'
        {
            i += 1;
        }
        let name_start = i;
        while i < raw_tag.len()
            && (raw_tag[i].is_ascii_alphanumeric() || matches!(raw_tag[i], b':' | b'-' | b'_'))
        {
            i += 1;
        }
        if name_start == i {
            break;
        }
        let name = normalize_attribute_name(&String::from_utf8_lossy(&raw_tag[name_start..i]));

        while i < raw_tag.len() && raw_tag[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < raw_tag.len() && raw_tag[i] == b'=' {
            i += 1;
            while i < raw_tag.len() && raw_tag[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= raw_tag.len() {
                String::new()
            } else {
                let raw_value = if matches!(raw_tag[i], b'\'' | b'"') {
                    let quote = raw_tag[i];
                    i += 1;
                    let value_start = i;
                    while i < raw_tag.len() && raw_tag[i] != quote {
                        i += 1;
                    }
                    let value = &raw_tag[value_start..i];
                    i += usize::from(i < raw_tag.len());
                    value
                } else {
                    let value_start = i;
                    while i < raw_tag.len()
                        && !raw_tag[i].is_ascii_whitespace()
                        && raw_tag[i] != b'>'
                    {
                        i += 1;
                    }
                    &raw_tag[value_start..i]
                };
                let lossy = String::from_utf8_lossy(raw_value);
                decode_html_entities(&lossy).into_owned()
            }
        } else {
            String::new()
        };

        attrs.insert(name, value);
    }

    attrs
}

fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn parse_css(css: &str) -> CssCascade {
    let css = strip_css_comments(css);
    let custom_properties = collect_root_css_custom_properties(&css);
    let mut cascade = CssCascade {
        rules: Vec::new(),
        id_rules: HashMap::new(),
        class_rules: HashMap::new(),
        tag_rules: HashMap::new(),
        attr_rules: HashMap::new(),
        universal_rules: Vec::new(),
        custom_properties,
    };

    for (selectors, declarations) in top_level_css_rule_blocks(&css) {
        let resolved_declarations = substitute_css_vars(declarations, &cascade.custom_properties);
        let declarations = parse_css_declarations(&resolved_declarations);
        if declarations == CssDeclarations::default() {
            continue;
        }
        for selector in split_css_selector_list(selectors) {
            if let Some(selector) = parse_selector(selector) {
                cascade.push(CssRule {
                    selector,
                    declarations: declarations.clone(),
                });
            }
        }
    }

    cascade
}

fn collect_root_css_custom_properties(css: &str) -> HashMap<String, String> {
    let mut custom_properties = HashMap::new();
    for (selectors, declarations) in top_level_css_rule_blocks(css) {
        if !split_css_selector_list(selectors)
            .iter()
            .any(|selector| css_selector_defines_root_custom_properties(selector))
        {
            continue;
        }
        for declaration in declarations.split(';') {
            let Some((name, value)) = declaration.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if !name.starts_with("--") || name.len() <= 2 {
                continue;
            }
            let value = css_declaration_value(value);
            if !value.is_empty() {
                custom_properties.insert(name.to_ascii_lowercase(), value.to_owned());
            }
        }
    }
    custom_properties
}

fn top_level_css_rule_blocks(css: &str) -> Vec<(&str, &str)> {
    let bytes = css.as_bytes();
    let mut blocks = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        index = skip_css_whitespace(bytes, index);
        if index >= bytes.len() {
            break;
        }
        if bytes[index] == b'@' {
            index = skip_css_at_rule(css, index);
            continue;
        }
        let selector_start = index;
        let Some(open_brace) = find_css_rule_open_brace(css, index) else {
            break;
        };
        let Some(close_brace) = find_matching_css_brace(css, open_brace) else {
            break;
        };
        blocks.push((
            &css[selector_start..open_brace],
            &css[open_brace + 1..close_brace],
        ));
        index = close_brace + 1;
    }
    blocks
}

fn skip_css_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn skip_css_at_rule(css: &str, start: usize) -> usize {
    let bytes = css.as_bytes();
    let mut quote = None;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b';' => return index + 1,
            b'{' => {
                return find_matching_css_brace(css, index)
                    .map(|close| close + 1)
                    .unwrap_or(bytes.len());
            }
            _ => index += 1,
        }
    }
    bytes.len()
}

fn find_css_rule_open_brace(css: &str, start: usize) -> Option<usize> {
    let bytes = css.as_bytes();
    let mut quote = None;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'{' => return Some(index),
            b'}' => return None,
            _ => {}
        }
        index += 1;
    }
    None
}

fn find_matching_css_brace(css: &str, open_brace: usize) -> Option<usize> {
    let bytes = css.as_bytes();
    let mut quote = None;
    let mut depth = 0usize;
    let mut index = open_brace;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'{' => depth = depth.saturating_add(1),
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn css_selector_defines_root_custom_properties(selector: &str) -> bool {
    matches!(
        selector.trim().to_ascii_lowercase().as_str(),
        ":root" | "html"
    )
}

fn substitute_css_vars(value: &str, custom_properties: &HashMap<String, String>) -> String {
    let mut resolved = value.to_owned();
    for _ in 0..8 {
        let next = substitute_css_vars_once(&resolved, custom_properties);
        if next == resolved {
            break;
        }
        resolved = next;
    }
    resolved
}

fn substitute_css_vars_once(value: &str, custom_properties: &HashMap<String, String>) -> String {
    let mut resolved = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(var_start) = rest.find("var(") {
        resolved.push_str(&rest[..var_start]);
        let content_start = var_start + "var(".len();
        let Some(close) = css_function_close(rest, content_start) else {
            resolved.push_str(&rest[var_start..]);
            return resolved;
        };
        let content = &rest[content_start..close];
        if let Some(replacement) = css_var_replacement(content, custom_properties) {
            resolved.push_str(&replacement);
        } else {
            resolved.push_str(&rest[var_start..=close]);
        }
        rest = &rest[close + 1..];
    }
    resolved.push_str(rest);
    resolved
}

fn css_var_replacement(
    content: &str,
    custom_properties: &HashMap<String, String>,
) -> Option<String> {
    let (name, fallback) = split_css_var_arguments(content);
    let name = name.trim().to_ascii_lowercase();
    if !name.starts_with("--") {
        return None;
    }
    custom_properties
        .get(&name)
        .cloned()
        .or_else(|| fallback.map(|fallback| fallback.trim().to_owned()))
}

fn split_css_var_arguments(content: &str) -> (&str, Option<&str>) {
    let bytes = content.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return (&content[..i], Some(&content[i + 1..])),
            _ => {}
        }
        i += 1;
    }
    (content, None)
}

fn css_function_close(value: &str, content_start: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut depth = 1usize;
    let mut quote = None;
    let mut i = content_start;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_css_comments(css: &str) -> String {
    let mut stripped = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("*/") else {
            return stripped;
        };
        rest = &after_start[end + 2..];
    }
    stripped.push_str(rest);
    stripped
}

fn split_css_selector_list(selectors: &str) -> Vec<&str> {
    let bytes = selectors.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut quote = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'[' => bracket_depth = bracket_depth.saturating_add(1),
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => paren_depth = paren_depth.saturating_add(1),
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b',' if bracket_depth == 0 && paren_depth == 0 => {
                parts.push(&selectors[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&selectors[start..]);
    parts
}

impl CssCascade {
    fn push(&mut self, rule: CssRule) {
        let rule_index = self.rules.len();
        let bucket = rule.selector.target_bucket();
        match bucket {
            SelectorBucket::Id(id) => self
                .id_rules
                .entry(id.to_owned())
                .or_default()
                .push(rule_index),
            SelectorBucket::Class(class) => self
                .class_rules
                .entry(class.to_owned())
                .or_default()
                .push(rule_index),
            SelectorBucket::Tag(tag) => self
                .tag_rules
                .entry(tag.to_owned())
                .or_default()
                .push(rule_index),
            SelectorBucket::Attr(attr) => self
                .attr_rules
                .entry(attr.to_owned())
                .or_default()
                .push(rule_index),
            SelectorBucket::Universal => self.universal_rules.push(rule_index),
        }
        self.rules.push(rule);
    }

    fn candidate_rule_indices(&self, element: &ElementData) -> Vec<usize> {
        let mut indices = Vec::new();
        if let Some(id) = &element.id
            && let Some(bucket) = self.id_rules.get(id)
        {
            indices.extend(bucket);
        }
        for class in &element.classes {
            if let Some(bucket) = self.class_rules.get(class) {
                indices.extend(bucket);
            }
        }
        if let Some(bucket) = self.tag_rules.get(&element.tag) {
            indices.extend(bucket);
        }
        for attr in element.attrs.keys() {
            if let Some(bucket) = self.attr_rules.get(attr) {
                indices.extend(bucket);
            }
        }
        indices.extend(&self.universal_rules);
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

#[derive(Debug, Clone, Copy)]
enum SelectorBucket<'a> {
    Id(&'a str),
    Class(&'a str),
    Tag(&'a str),
    Attr(&'a str),
    Universal,
}

impl CssSelector {
    fn target_bucket(&self) -> SelectorBucket<'_> {
        let Some(target) = self.steps.last().map(|step| &step.compound) else {
            return SelectorBucket::Universal;
        };
        if let Some(id) = &target.id {
            SelectorBucket::Id(id)
        } else if let Some(class) = target.classes.first() {
            SelectorBucket::Class(class)
        } else if let Some(tag) = &target.tag {
            SelectorBucket::Tag(tag)
        } else if let Some(attribute) = target.attributes.first() {
            SelectorBucket::Attr(&attribute.name)
        } else {
            SelectorBucket::Universal
        }
    }
}

fn parse_selector(selector: &str) -> Option<CssSelector> {
    let bytes = selector.as_bytes();
    let mut steps = Vec::new();
    let mut pending_combinator = None;
    let mut i = 0usize;

    while i < bytes.len() {
        let mut saw_space = false;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            saw_space = true;
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'>' {
            pending_combinator = Some(SelectorCombinator::Child);
            i += 1;
            continue;
        }
        if saw_space && !steps.is_empty() && pending_combinator.is_none() {
            pending_combinator = Some(SelectorCombinator::Descendant);
        }

        let start = i;
        i = compound_selector_end(selector, start);
        let compound = parse_compound_selector(&selector[start..i])?;
        let combinator = if steps.is_empty() {
            None
        } else {
            Some(
                pending_combinator
                    .take()
                    .unwrap_or(SelectorCombinator::Descendant),
            )
        };
        steps.push(SelectorStep {
            compound,
            combinator,
        });
    }

    (!steps.is_empty()).then_some(CssSelector { steps })
}

fn compound_selector_end(selector: &str, start: usize) -> usize {
    let bytes = selector.as_bytes();
    let mut i = start;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut quote = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if byte == active_quote {
                quote = None;
            }
            i += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'[' => bracket_depth = bracket_depth.saturating_add(1),
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'(' => paren_depth = paren_depth.saturating_add(1),
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'>' if bracket_depth == 0 && paren_depth == 0 => break,
            _ if byte.is_ascii_whitespace() && bracket_depth == 0 && paren_depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    i
}

fn parse_compound_selector(selector: &str) -> Option<CompoundSelector> {
    let bytes = selector.as_bytes();
    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();
    let mut attributes = Vec::new();
    let mut not_selectors = Vec::new();
    let mut first_child = false;
    let mut universal = false;
    let mut i = 0usize;

    if bytes.get(i) == Some(&b'*') {
        universal = true;
        i += 1;
    } else if bytes
        .get(i)
        .is_some_and(|byte| is_selector_ident_start(*byte))
    {
        let start = i;
        i += 1;
        while i < bytes.len() && is_selector_ident_continue(bytes[i]) {
            i += 1;
        }
        tag = Some(selector[start..i].to_ascii_lowercase());
    }

    while i < bytes.len() {
        let prefix = bytes[i];
        if prefix == b'[' {
            let (attribute, next) = parse_attribute_selector(selector, i)?;
            attributes.push(attribute);
            i = next;
            continue;
        }
        if selector[i..].starts_with(":not(") {
            let (negated, next) = parse_not_selector(selector, i)?;
            not_selectors.extend(negated);
            i = next;
            continue;
        }
        if selector[i..].starts_with(":first-child") {
            first_child = true;
            i += ":first-child".len();
            continue;
        }
        if !matches!(prefix, b'#' | b'.') {
            return None;
        }
        i += 1;
        let start = i;
        if !bytes
            .get(i)
            .is_some_and(|byte| is_selector_ident_start(*byte))
        {
            return None;
        }
        i += 1;
        while i < bytes.len() && is_selector_ident_continue(bytes[i]) {
            i += 1;
        }
        let value = selector[start..i].to_owned();
        if prefix == b'#' {
            if id.replace(value).is_some() {
                return None;
            }
        } else {
            classes.push(value);
        }
    }

    (universal
        || tag.is_some()
        || id.is_some()
        || !classes.is_empty()
        || !attributes.is_empty()
        || !not_selectors.is_empty()
        || first_child)
        .then_some(CompoundSelector {
            tag,
            id,
            classes,
            attributes,
            not_selectors,
            first_child,
            universal,
        })
}

fn parse_not_selector(selector: &str, start: usize) -> Option<(Vec<CompoundSelector>, usize)> {
    let bytes = selector.as_bytes();
    if !selector[start..].starts_with(":not(") {
        return None;
    }
    let mut depth = 1usize;
    let mut quote = None;
    let mut i = start + ":not(".len();
    let content_start = i;
    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            }
        } else {
            match byte {
                b'\'' | b'"' => quote = Some(byte),
                b'(' => depth = depth.saturating_add(1),
                b')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let content = &selector[content_start..i];
                        let selectors = parse_not_selector_list(content)?;
                        return Some((selectors, i + 1));
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn parse_not_selector_list(content: &str) -> Option<Vec<CompoundSelector>> {
    let mut selectors = Vec::new();
    for selector in split_css_selector_list(content) {
        let selector = selector.trim();
        if selector.is_empty() || selector.contains(char::is_whitespace) || selector.contains('>') {
            return None;
        }
        selectors.push(parse_compound_selector(selector)?);
    }
    (!selectors.is_empty()).then_some(selectors)
}

fn parse_attribute_selector(selector: &str, start: usize) -> Option<(AttributeSelector, usize)> {
    let bytes = selector.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut i = start + 1;
    let content_start = i;
    let mut quote = None;
    while i < bytes.len() {
        if let Some(active_quote) = quote {
            if bytes[i] == active_quote {
                quote = None;
            }
        } else if matches!(bytes[i], b'\'' | b'"') {
            quote = Some(bytes[i]);
        } else if bytes[i] == b']' {
            let content = selector[content_start..i].trim();
            let attribute = parse_attribute_selector_content(content)?;
            return Some((attribute, i + 1));
        }
        i += 1;
    }
    None
}

fn parse_attribute_selector_content(content: &str) -> Option<AttributeSelector> {
    let (name, value) = content
        .split_once('=')
        .map_or((content.trim(), None), |(name, value)| {
            (name.trim(), Some(unquote_css_attribute_value(value.trim())))
        });
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| is_selector_ident_continue(byte) || byte == b':')
    {
        return None;
    }
    Some(AttributeSelector {
        name: normalize_attribute_name(name),
        value,
    })
}

fn unquote_css_attribute_value(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && matches!(bytes[0], b'\'' | b'"') && bytes.last() == Some(&bytes[0]) {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn is_selector_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'-')
}

fn is_selector_ident_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn parse_css_declarations(style: &str) -> CssDeclarations {
    let style = strip_css_comments(style);
    let mut declarations = CssDeclarations::default();
    let mut border_width = None;
    let mut border_shade = None;
    let mut border_enabled = false;
    let mut padding = PartialBoxSpacing::default();
    let mut margin = PartialBoxSpacing::default();
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = css_declaration_value(value);
        match name.trim().to_ascii_lowercase().as_str() {
            "display" => {
                declarations.display = parse_css_display(value).or(declarations.display);
            }
            "flex-direction" => {
                declarations.flex_direction =
                    parse_css_flex_direction(value).or(declarations.flex_direction);
            }
            "flex-wrap" => {
                declarations.flex_wrap = parse_css_flex_wrap(value).or(declarations.flex_wrap);
            }
            "flex-basis" => {
                declarations.flex_basis =
                    parse_css_dimension(value, CssAxis::Horizontal).or(declarations.flex_basis);
            }
            "flex-flow" => {
                declarations.flex_direction =
                    parse_css_flex_direction(value).or(declarations.flex_direction);
                declarations.flex_wrap = parse_css_flex_wrap(value).or(declarations.flex_wrap);
            }
            "flex" => {
                declarations.flex_basis =
                    parse_css_flex_basis_shorthand(value).or(declarations.flex_basis);
            }
            "justify-content" => {
                declarations.justify_content =
                    parse_css_justify_content(value).or(declarations.justify_content);
            }
            "align-items" => {
                declarations.align_items =
                    parse_css_align_items(value).or(declarations.align_items);
            }
            "grid-template-columns" => {
                declarations.grid_columns =
                    parse_css_grid_template_columns(value).or(declarations.grid_columns);
                declarations.grid_auto_min_column_width =
                    parse_css_grid_auto_min_column_width(value)
                        .or(declarations.grid_auto_min_column_width);
            }
            "float" => {
                declarations.float = parse_css_float(value).or(declarations.float);
            }
            "clear" => {
                declarations.clear = parse_css_clear(value).or(declarations.clear);
            }
            "background" | "background-color" => {
                declarations.background_shade =
                    parse_css_color_shade(value).or(declarations.background_shade);
                declarations.background_image_url =
                    parse_css_image_url(value).or(declarations.background_image_url);
            }
            "background-image" => {
                declarations.background_image_url =
                    parse_css_image_url(value).or(declarations.background_image_url);
            }
            "background-size" => {
                declarations.background_image_size =
                    parse_css_background_image_size(value).or(declarations.background_image_size);
            }
            "background-position" => {
                declarations.background_image_position = parse_css_background_image_position(value)
                    .or(declarations.background_image_position);
            }
            "background-repeat" => {
                declarations.background_image_repeat = parse_css_background_image_repeat(value)
                    .or(declarations.background_image_repeat);
            }
            "color" => {
                declarations.text_shade = parse_css_color_shade(value).or(declarations.text_shade);
            }
            "text-align" => {
                declarations.text_align = parse_css_text_align(value).or(declarations.text_align);
            }
            "visibility" => {
                declarations.visibility = parse_css_visibility(value).or(declarations.visibility);
            }
            "opacity" => {
                declarations.opacity = parse_css_opacity(value).or(declarations.opacity);
            }
            "animation-fill-mode" | "-webkit-animation-fill-mode" => {
                declarations.animation_reveals_opacity = parse_css_animation_reveals_opacity(value)
                    .or(declarations.animation_reveals_opacity);
            }
            "animation" | "-webkit-animation" => {
                declarations.animation_reveals_opacity = parse_css_animation_reveals_opacity(value)
                    .or(declarations.animation_reveals_opacity);
            }
            "overflow" => {
                if let Some(overflow) = parse_css_overflow(value) {
                    declarations.overflow_x = Some(overflow);
                    declarations.overflow_y = Some(overflow);
                }
            }
            "overflow-x" | "overflow-inline" => {
                declarations.overflow_x = parse_css_overflow(value).or(declarations.overflow_x);
            }
            "overflow-y" | "overflow-block" => {
                declarations.overflow_y = parse_css_overflow(value).or(declarations.overflow_y);
            }
            "position" => {
                declarations.position = parse_css_position(value).or(declarations.position);
            }
            "top" => {
                declarations.position_top = parse_css_position_offset(value, CssAxis::Vertical)
                    .or(declarations.position_top);
            }
            "bottom" | "inset-block-end" => {
                declarations.position_bottom = parse_css_position_offset(value, CssAxis::Vertical)
                    .or(declarations.position_bottom);
            }
            "left" | "inset-inline-start" => {
                declarations.position_left = parse_css_position_offset(value, CssAxis::Horizontal)
                    .or(declarations.position_left);
            }
            "right" | "inset-inline-end" => {
                declarations.position_right = parse_css_position_offset(value, CssAxis::Horizontal)
                    .or(declarations.position_right);
            }
            "inset-block-start" => {
                declarations.position_top = parse_css_position_offset(value, CssAxis::Vertical)
                    .or(declarations.position_top);
            }
            "inset-block" => {
                let (start, end) = parse_css_inset_axis_offsets(value, CssAxis::Vertical);
                declarations.position_top = start.or(declarations.position_top);
                declarations.position_bottom = end.or(declarations.position_bottom);
            }
            "inset-inline" => {
                let (start, end) = parse_css_inset_axis_offsets(value, CssAxis::Horizontal);
                declarations.position_left = start.or(declarations.position_left);
                declarations.position_right = end.or(declarations.position_right);
            }
            "inset" => {
                let inset = parse_css_inset_offsets(value);
                declarations.position_top = inset.top.or(declarations.position_top);
                declarations.position_bottom = inset.bottom.or(declarations.position_bottom);
                declarations.position_right = inset.right.or(declarations.position_right);
                declarations.position_left = inset.left.or(declarations.position_left);
            }
            "transform" | "-webkit-transform" => {
                declarations.transform_translate =
                    parse_css_transform_translate(value).or(declarations.transform_translate);
            }
            "translate" => {
                declarations.transform_translate =
                    parse_css_translate_property(value).or(declarations.transform_translate);
            }
            "z-index" => {
                declarations.z_index = parse_css_z_index(value).or(declarations.z_index);
            }
            "white-space" => {
                declarations.white_space =
                    parse_css_white_space(value).or(declarations.white_space);
            }
            "text-transform" => {
                declarations.text_transform =
                    parse_css_text_transform(value).or(declarations.text_transform);
            }
            "letter-spacing" => {
                declarations.letter_spacing =
                    parse_css_letter_spacing(value).or(declarations.letter_spacing);
            }
            "word-spacing" => {
                declarations.word_spacing =
                    parse_css_word_spacing(value).or(declarations.word_spacing);
            }
            "overflow-wrap" | "word-wrap" => {
                declarations.overflow_wrap =
                    parse_css_overflow_wrap(value).or(declarations.overflow_wrap);
            }
            "word-break" => {
                declarations.word_break = parse_css_word_break(value).or(declarations.word_break);
            }
            "text-indent" => {
                declarations.text_indent = parse_css_dimension_length(value, CssAxis::Horizontal)
                    .or(declarations.text_indent);
            }
            "line-height" => {
                declarations.line_height =
                    parse_css_line_height(value).or(declarations.line_height);
            }
            "font-size" => {
                declarations.font_scale = parse_css_font_scale(value).or(declarations.font_scale);
            }
            "font" => {
                declarations.font_scale =
                    parse_css_font_shorthand_scale(value).or(declarations.font_scale);
            }
            "gap" => {
                if let Some((row_gap, column_gap)) = parse_css_gap(value) {
                    declarations.row_gap = Some(row_gap);
                    declarations.column_gap = Some(column_gap);
                }
            }
            "row-gap" => {
                declarations.row_gap =
                    parse_css_axis_gap(value, CssAxis::Vertical).or(declarations.row_gap);
            }
            "column-gap" => {
                declarations.column_gap =
                    parse_css_axis_gap(value, CssAxis::Horizontal).or(declarations.column_gap);
            }
            "border-spacing" => {
                if let Some((horizontal_gap, vertical_gap)) = parse_css_border_spacing(value) {
                    declarations.column_gap = Some(horizontal_gap);
                    declarations.row_gap = Some(vertical_gap);
                }
            }
            "box-sizing" => {
                declarations.box_sizing = parse_css_box_sizing(value).or(declarations.box_sizing);
            }
            "list-style-type" => {
                if let Some(list_style_type) = parse_css_list_style_type(value) {
                    declarations.list_style_type = Some(list_style_type);
                }
            }
            "list-style" => {
                if let Some(list_style_type) = parse_css_list_style(value) {
                    declarations.list_style_type = Some(list_style_type);
                }
            }
            "border" => {
                let border = parse_css_border(value);
                border_enabled |= border.enabled;
                border_width = border.width.or(border_width);
                border_shade = border.shade.or(border_shade);
            }
            "border-style" => {
                border_enabled |= parse_css_border_style_enabled(value);
            }
            "border-width" => {
                border_width = parse_css_border_width(value).or(border_width);
            }
            "border-color" => {
                border_shade = parse_css_color_shade(value).or(border_shade);
            }
            "padding" => {
                if let Some(parsed) = parse_css_padding(value) {
                    padding.set_all(parsed);
                }
            }
            "padding-inline" => {
                if let Some((start, end)) = parse_css_box_spacing_pair(value, CssAxis::Horizontal) {
                    padding.left = Some(start);
                    padding.right = Some(end);
                }
            }
            "padding-block" => {
                if let Some((start, end)) = parse_css_box_spacing_pair(value, CssAxis::Vertical) {
                    padding.top = Some(start);
                    padding.bottom = Some(end);
                }
            }
            "padding-top" => {
                padding.top = parse_css_padding_length(value, CssAxis::Vertical);
            }
            "padding-right" | "padding-inline-end" => {
                padding.right = parse_css_padding_length(value, CssAxis::Horizontal);
            }
            "padding-bottom" | "padding-block-end" => {
                padding.bottom = parse_css_padding_length(value, CssAxis::Vertical);
            }
            "padding-left" | "padding-inline-start" => {
                padding.left = parse_css_padding_length(value, CssAxis::Horizontal);
            }
            "padding-block-start" => {
                padding.top = parse_css_padding_length(value, CssAxis::Vertical);
            }
            "margin" => {
                if let Some(parsed) = parse_css_margin(value) {
                    margin.set_partial(parsed.spacing);
                    declarations.margin_left_auto =
                        parsed.left_auto.or(declarations.margin_left_auto);
                    declarations.margin_right_auto =
                        parsed.right_auto.or(declarations.margin_right_auto);
                }
            }
            "margin-inline" => {
                if let Some(parsed) = parse_css_logical_margin_pair(value, CssAxis::Horizontal) {
                    margin.left = parsed.start;
                    margin.right = parsed.end;
                    declarations.margin_left_auto =
                        parsed.start_auto.or(declarations.margin_left_auto);
                    declarations.margin_right_auto =
                        parsed.end_auto.or(declarations.margin_right_auto);
                }
            }
            "margin-block" => {
                if let Some(parsed) = parse_css_logical_margin_pair(value, CssAxis::Vertical) {
                    margin.top = parsed.start;
                    margin.bottom = parsed.end;
                }
            }
            "margin-top" => {
                margin.top = parse_css_margin_length(value, CssAxis::Vertical);
            }
            "margin-right" | "margin-inline-end" => {
                if css_value_is_auto(value) {
                    declarations.margin_right_auto = Some(true);
                } else {
                    margin.right = parse_css_margin_length(value, CssAxis::Horizontal);
                    if margin.right.is_some() {
                        declarations.margin_right_auto = Some(false);
                    }
                }
            }
            "margin-bottom" => {
                margin.bottom = parse_css_margin_length(value, CssAxis::Vertical);
            }
            "margin-left" | "margin-inline-start" => {
                if css_value_is_auto(value) {
                    declarations.margin_left_auto = Some(true);
                } else {
                    margin.left = parse_css_margin_length(value, CssAxis::Horizontal);
                    if margin.left.is_some() {
                        declarations.margin_left_auto = Some(false);
                    }
                }
            }
            "margin-block-start" => {
                margin.top = parse_css_margin_length(value, CssAxis::Vertical);
            }
            "margin-block-end" => {
                margin.bottom = parse_css_margin_length(value, CssAxis::Vertical);
            }
            "width" | "inline-size" => {
                declarations.width =
                    parse_css_dimension(value, CssAxis::Horizontal).or(declarations.width);
            }
            "max-width" | "max-inline-size" => {
                declarations.max_width =
                    parse_css_dimension(value, CssAxis::Horizontal).or(declarations.max_width);
            }
            "min-width" | "min-inline-size" => {
                declarations.min_width =
                    parse_css_dimension(value, CssAxis::Horizontal).or(declarations.min_width);
            }
            "height" | "block-size" => {
                declarations.height =
                    parse_css_dimension(value, CssAxis::Vertical).or(declarations.height);
            }
            "max-height" | "max-block-size" => {
                declarations.max_height =
                    parse_css_dimension(value, CssAxis::Vertical).or(declarations.max_height);
            }
            "aspect-ratio" => {
                declarations.aspect_ratio =
                    parse_css_aspect_ratio(value).or(declarations.aspect_ratio);
            }
            "min-height" | "min-block-size" => {
                declarations.min_height =
                    parse_css_dimension(value, CssAxis::Vertical).or(declarations.min_height);
            }
            _ => {}
        }
    }
    if border_enabled {
        declarations.border = Some(BorderPaint {
            width: border_width.unwrap_or(1).clamp(1, 4),
            shade: border_shade.unwrap_or(0),
        });
    }
    declarations.padding = padding.finish();
    declarations.margin = margin.finish();
    declarations
}

fn css_declaration_value(value: &str) -> &str {
    let value = value.trim().trim_end_matches(';').trim();
    let important = "!important";
    if value.len() >= important.len()
        && value[value.len() - important.len()..].eq_ignore_ascii_case(important)
    {
        value[..value.len() - important.len()].trim_end()
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PartialBoxSpacing {
    top: Option<usize>,
    right: Option<usize>,
    bottom: Option<usize>,
    left: Option<usize>,
}

impl PartialBoxSpacing {
    fn set_all(&mut self, spacing: BoxSpacing) {
        self.top = Some(spacing.top);
        self.right = Some(spacing.right);
        self.bottom = Some(spacing.bottom);
        self.left = Some(spacing.left);
    }

    fn set_partial(&mut self, spacing: Self) {
        if let Some(top) = spacing.top {
            self.top = Some(top);
        }
        if let Some(right) = spacing.right {
            self.right = Some(right);
        }
        if let Some(bottom) = spacing.bottom {
            self.bottom = Some(bottom);
        }
        if let Some(left) = spacing.left {
            self.left = Some(left);
        }
    }

    fn finish(self) -> Option<BoxSpacing> {
        (self.top.is_some() || self.right.is_some() || self.bottom.is_some() || self.left.is_some())
            .then_some(BoxSpacing {
                top: self.top.unwrap_or(0),
                right: self.right.unwrap_or(0),
                bottom: self.bottom.unwrap_or(0),
                left: self.left.unwrap_or(0),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssAxis {
    Horizontal,
    Vertical,
}

fn css_axis_cell_px(axis: CssAxis) -> f32 {
    match axis {
        CssAxis::Horizontal => 8.0,
        CssAxis::Vertical => 12.0,
    }
}

const CSS_TEXT_CELL_UNITS: usize = 256;
const CSS_DEFAULT_VIEWPORT_WIDTH_CELLS: f32 = 100.0;
const CSS_DEFAULT_VIEWPORT_HEIGHT_CELLS: f32 = 44.0;

fn parse_css_length_pixels(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value.starts_with('-') {
        return None;
    }
    parse_css_signed_length_pixels(&value)
}

fn parse_css_signed_length_pixels(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value.contains('%') || value == "auto" || value == "inherit" || value == "initial" {
        return None;
    }
    let (numeric, multiplier) =
        if let Some((numeric, multiplier)) = parse_css_viewport_unit_length(&value) {
            (numeric, multiplier)
        } else if let Some(numeric) = value.strip_suffix("rem") {
            (numeric, 16.0)
        } else if let Some(numeric) = value.strip_suffix("em") {
            (numeric, 16.0)
        } else if let Some(numeric) = value.strip_suffix("ch") {
            (numeric, 8.0)
        } else if let Some(numeric) = value.strip_suffix("px") {
            (numeric, 1.0)
        } else {
            (value.as_str(), 1.0)
        };
    let pixels = numeric.parse::<f32>().ok()? * multiplier;
    pixels.is_finite().then_some(pixels)
}

fn parse_css_viewport_unit_length(value: &str) -> Option<(&str, f32)> {
    let viewport_width_px =
        CSS_DEFAULT_VIEWPORT_WIDTH_CELLS * css_axis_cell_px(CssAxis::Horizontal);
    let viewport_height_px =
        CSS_DEFAULT_VIEWPORT_HEIGHT_CELLS * css_axis_cell_px(CssAxis::Vertical);
    let viewport_min_px = viewport_width_px.min(viewport_height_px);
    let viewport_max_px = viewport_width_px.max(viewport_height_px);
    for (unit, unit_px) in [
        ("svw", viewport_width_px / 100.0),
        ("lvw", viewport_width_px / 100.0),
        ("dvw", viewport_width_px / 100.0),
        ("svh", viewport_height_px / 100.0),
        ("lvh", viewport_height_px / 100.0),
        ("dvh", viewport_height_px / 100.0),
        ("vmin", viewport_min_px / 100.0),
        ("vmax", viewport_max_px / 100.0),
        ("vw", viewport_width_px / 100.0),
        ("vh", viewport_height_px / 100.0),
    ] {
        if let Some(numeric) = value.strip_suffix(unit) {
            return Some((numeric, unit_px));
        }
    }
    None
}

fn css_length_cells(value: &str, axis: CssAxis, max_cells: usize) -> Option<usize> {
    let pixels = parse_css_length_pixels(value)?;
    if pixels == 0.0 {
        return Some(0);
    }
    let cells = (pixels / css_axis_cell_px(axis)).ceil() as usize;
    Some(cells.clamp(1, max_cells))
}

fn css_length_cell_units(value: &str, axis: CssAxis, max_cells: usize) -> Option<usize> {
    let pixels = parse_css_length_pixels(value)?;
    if pixels == 0.0 {
        return Some(0);
    }
    let units = ((pixels / css_axis_cell_px(axis)) * CSS_TEXT_CELL_UNITS as f32).round() as usize;
    Some(units.clamp(1, max_cells.saturating_mul(CSS_TEXT_CELL_UNITS)))
}

fn parse_css_gap(value: &str) -> Option<(usize, usize)> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        [gap] => {
            let row_gap = parse_css_axis_gap(gap, CssAxis::Vertical)?;
            let column_gap = parse_css_axis_gap(gap, CssAxis::Horizontal)?;
            Some((row_gap, column_gap))
        }
        [row_gap, column_gap, ..] => Some((
            parse_css_axis_gap(row_gap, CssAxis::Vertical)?,
            parse_css_axis_gap(column_gap, CssAxis::Horizontal)?,
        )),
        _ => None,
    }
}

fn parse_css_border_spacing(value: &str) -> Option<(usize, usize)> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        [gap] => {
            let horizontal_gap = parse_css_axis_gap(gap, CssAxis::Horizontal)?;
            let vertical_gap = parse_css_axis_gap(gap, CssAxis::Vertical)?;
            Some((horizontal_gap, vertical_gap))
        }
        [horizontal_gap, vertical_gap, ..] => Some((
            parse_css_axis_gap(horizontal_gap, CssAxis::Horizontal)?,
            parse_css_axis_gap(vertical_gap, CssAxis::Vertical)?,
        )),
        _ => None,
    }
}

fn parse_css_axis_gap(value: &str, axis: CssAxis) -> Option<usize> {
    let value = value.trim().trim_end_matches(';');
    if value.eq_ignore_ascii_case("normal") {
        return Some(0);
    }
    css_length_cells(value, axis, 64)
}

fn parse_css_flex_direction(value: &str) -> Option<FlexDirection> {
    value.split_ascii_whitespace().find_map(|token| {
        match token.trim().to_ascii_lowercase().as_str() {
            "row" => Some(FlexDirection::Row),
            "row-reverse" => Some(FlexDirection::RowReverse),
            "column" => Some(FlexDirection::Column),
            "column-reverse" => Some(FlexDirection::ColumnReverse),
            _ => None,
        }
    })
}

fn parse_css_flex_wrap(value: &str) -> Option<bool> {
    value.split_ascii_whitespace().find_map(|token| {
        match token.trim().to_ascii_lowercase().as_str() {
            "wrap" | "wrap-reverse" => Some(true),
            "nowrap" => Some(false),
            _ => None,
        }
    })
}

fn parse_css_justify_content(value: &str) -> Option<JustifyContent> {
    value.split_ascii_whitespace().find_map(|token| {
        match token.trim().to_ascii_lowercase().as_str() {
            "normal" | "start" | "flex-start" | "left" => Some(JustifyContent::Start),
            "center" => Some(JustifyContent::Center),
            "end" | "flex-end" | "right" => Some(JustifyContent::End),
            "space-between" => Some(JustifyContent::SpaceBetween),
            "space-around" => Some(JustifyContent::SpaceAround),
            "space-evenly" => Some(JustifyContent::SpaceEvenly),
            _ => None,
        }
    })
}

fn parse_css_align_items(value: &str) -> Option<AlignItems> {
    value.split_ascii_whitespace().find_map(|token| {
        match token.trim().to_ascii_lowercase().as_str() {
            "center" => Some(AlignItems::Center),
            "end" | "flex-end" => Some(AlignItems::End),
            "baseline" | "first baseline" | "last baseline" => Some(AlignItems::Baseline),
            "normal" | "start" | "flex-start" | "stretch" => Some(AlignItems::Start),
            _ => None,
        }
    })
}

fn parse_css_flex_basis_shorthand(value: &str) -> Option<CssDimension> {
    let mut parsed = None;
    for token in value.split_ascii_whitespace() {
        let token = token.trim();
        if token.eq_ignore_ascii_case("auto")
            || token.eq_ignore_ascii_case("none")
            || token.eq_ignore_ascii_case("initial")
            || token.eq_ignore_ascii_case("inherit")
            || parse_css_flex_wrap(token).is_some()
            || parse_css_flex_direction(token).is_some()
            || token.parse::<f32>().is_ok()
        {
            continue;
        }
        parsed = parse_css_dimension(token, CssAxis::Horizontal).or(parsed);
    }
    parsed
}

fn parse_css_grid_template_columns(value: &str) -> Option<usize> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("subgrid")
        || value.eq_ignore_ascii_case("masonry")
    {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    if let Some(repeat_index) = lower.find("repeat(") {
        let mut count_start = repeat_index.saturating_add("repeat(".len());
        while lower
            .as_bytes()
            .get(count_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            count_start = count_start.saturating_add(1);
        }
        let count_end = lower[count_start..]
            .find(',')
            .map(|offset| count_start.saturating_add(offset))?;
        let repeated = lower[count_start..count_end].trim();
        if matches!(repeated, "auto-fill" | "auto-fit") {
            return None;
        }
        return repeated
            .parse::<usize>()
            .ok()
            .map(|count| count.clamp(1, 12));
    }

    let mut count = 0usize;
    let mut depth = 0usize;
    let mut in_token = false;
    for ch in value.chars() {
        match ch {
            '(' => {
                depth = depth.saturating_add(1);
                in_token = true;
            }
            ')' => {
                depth = depth.saturating_sub(1);
                in_token = true;
            }
            ch if ch.is_whitespace() && depth == 0 => {
                if in_token {
                    count = count.saturating_add(1);
                    in_token = false;
                }
            }
            _ => {
                in_token = true;
            }
        }
    }
    if in_token {
        count = count.saturating_add(1);
    }
    (count > 0).then(|| count.clamp(1, 12))
}

fn parse_css_grid_auto_min_column_width(value: &str) -> Option<usize> {
    let repeat_args = css_first_function_arguments(value, &["repeat"])?;
    let mut args = split_css_top_level_arguments(repeat_args);
    if args.len() < 2 {
        return None;
    }
    let repeated = args.remove(0);
    if !matches!(
        repeated.to_ascii_lowercase().as_str(),
        "auto-fit" | "auto-fill"
    ) {
        return None;
    }
    let track = args.join(", ");
    let minmax_args = css_first_function_arguments(&track, &["minmax"])?;
    let min_track = split_css_top_level_arguments(minmax_args)
        .into_iter()
        .next()?
        .to_owned();
    parse_css_axis_gap(&min_track, CssAxis::Horizontal).map(|width| width.max(1))
}

fn auto_grid_column_count(available_width: usize, min_column_width: usize) -> usize {
    if min_column_width == 0 || available_width == 0 {
        return 1;
    }
    (available_width / min_column_width).clamp(1, 12)
}

fn split_css_top_level_arguments(value: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let arg = value[start..index].trim();
                if !arg.is_empty() {
                    args.push(arg);
                }
                start = index.saturating_add(ch.len_utf8());
            }
            _ => {}
        }
    }
    let arg = value[start..].trim();
    if !arg.is_empty() {
        args.push(arg);
    }
    args
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ParsedMargin {
    spacing: PartialBoxSpacing,
    left_auto: Option<bool>,
    right_auto: Option<bool>,
}

fn parse_css_padding(value: &str) -> Option<BoxSpacing> {
    parse_css_box_spacing(value)
}

fn parse_css_margin(value: &str) -> Option<ParsedMargin> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [top, right, bottom, left] = match tokens.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left, ..] => [*top, *right, *bottom, *left],
        [] => return None,
    };
    let mut parsed = ParsedMargin::default();
    set_parsed_margin_side(&mut parsed, CssBoxSide::Top, top)?;
    set_parsed_margin_side(&mut parsed, CssBoxSide::Right, right)?;
    set_parsed_margin_side(&mut parsed, CssBoxSide::Bottom, bottom)?;
    set_parsed_margin_side(&mut parsed, CssBoxSide::Left, left)?;
    Some(parsed)
}

fn parse_css_box_spacing(value: &str) -> Option<BoxSpacing> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [top, right, bottom, left] = match tokens.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left, ..] => [*top, *right, *bottom, *left],
        [] => return None,
    };
    Some(BoxSpacing {
        top: parse_css_padding_length(top, CssAxis::Vertical)?,
        right: parse_css_padding_length(right, CssAxis::Horizontal)?,
        bottom: parse_css_padding_length(bottom, CssAxis::Vertical)?,
        left: parse_css_padding_length(left, CssAxis::Horizontal)?,
    })
}

fn parse_css_box_spacing_pair(value: &str, axis: CssAxis) -> Option<(usize, usize)> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [start, end] = match tokens.as_slice() {
        [all] => [*all, *all],
        [start, end, ..] => [*start, *end],
        [] => return None,
    };
    Some((
        parse_css_padding_length(start, axis)?,
        parse_css_padding_length(end, axis)?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssBoxSide {
    Top,
    Right,
    Bottom,
    Left,
}

fn set_parsed_margin_side(parsed: &mut ParsedMargin, side: CssBoxSide, value: &str) -> Option<()> {
    if css_value_is_auto(value) {
        match side {
            CssBoxSide::Left => parsed.left_auto = Some(true),
            CssBoxSide::Right => parsed.right_auto = Some(true),
            CssBoxSide::Top | CssBoxSide::Bottom => {}
        }
        return Some(());
    }
    let axis = match side {
        CssBoxSide::Top | CssBoxSide::Bottom => CssAxis::Vertical,
        CssBoxSide::Left | CssBoxSide::Right => CssAxis::Horizontal,
    };
    let length = parse_css_margin_length(value, axis)?;
    match side {
        CssBoxSide::Top => parsed.spacing.top = Some(length),
        CssBoxSide::Right => {
            parsed.spacing.right = Some(length);
            parsed.right_auto = Some(false);
        }
        CssBoxSide::Bottom => parsed.spacing.bottom = Some(length),
        CssBoxSide::Left => {
            parsed.spacing.left = Some(length);
            parsed.left_auto = Some(false);
        }
    }
    Some(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ParsedLogicalMargin {
    start: Option<usize>,
    end: Option<usize>,
    start_auto: Option<bool>,
    end_auto: Option<bool>,
}

fn parse_css_logical_margin_pair(value: &str, axis: CssAxis) -> Option<ParsedLogicalMargin> {
    let tokens = value.split_ascii_whitespace().collect::<Vec<_>>();
    let [start, end] = match tokens.as_slice() {
        [all] => [*all, *all],
        [start, end, ..] => [*start, *end],
        [] => return None,
    };
    let mut parsed = ParsedLogicalMargin::default();
    set_parsed_logical_margin_side(&mut parsed, true, start, axis)?;
    set_parsed_logical_margin_side(&mut parsed, false, end, axis)?;
    Some(parsed)
}

fn set_parsed_logical_margin_side(
    parsed: &mut ParsedLogicalMargin,
    start_side: bool,
    value: &str,
    axis: CssAxis,
) -> Option<()> {
    if css_value_is_auto(value) {
        if axis == CssAxis::Horizontal {
            if start_side {
                parsed.start_auto = Some(true);
            } else {
                parsed.end_auto = Some(true);
            }
        }
        return Some(());
    }
    let length = parse_css_margin_length(value, axis)?;
    if start_side {
        parsed.start = Some(length);
        if axis == CssAxis::Horizontal {
            parsed.start_auto = Some(false);
        }
    } else {
        parsed.end = Some(length);
        if axis == CssAxis::Horizontal {
            parsed.end_auto = Some(false);
        }
    }
    Some(())
}

fn css_value_is_auto(value: &str) -> bool {
    value
        .trim()
        .trim_end_matches(';')
        .eq_ignore_ascii_case("auto")
}

fn parse_css_padding_length(value: &str, axis: CssAxis) -> Option<usize> {
    parse_css_box_spacing_length(value, axis)
}

fn parse_css_margin_length(value: &str, axis: CssAxis) -> Option<usize> {
    parse_css_box_spacing_length(value, axis)
}

fn parse_css_box_spacing_length(value: &str, axis: CssAxis) -> Option<usize> {
    css_length_cells(value, axis, 8)
}

fn parse_css_dimension_length(value: &str, axis: CssAxis) -> Option<usize> {
    let basis = match axis {
        CssAxis::Horizontal => default_horizontal_dimension_basis(),
        CssAxis::Vertical => default_vertical_dimension_basis(),
    };
    Some(parse_css_dimension(value, axis)?.resolve(basis))
}

fn parse_css_dimension(value: &str, axis: CssAxis) -> Option<CssDimension> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if let Some(args) = css_function_arguments(&value, "clamp").first() {
        let args = split_css_transform_arguments(args);
        let minimum = args
            .first()
            .and_then(|arg| parse_css_dimension_term(arg, axis))?;
        let preferred = args
            .get(1)
            .and_then(|arg| parse_css_dimension_term(arg, axis))?;
        let maximum = args
            .get(2)
            .and_then(|arg| parse_css_dimension_term(arg, axis))?;
        return Some(CssDimension::Clamp(minimum, preferred, maximum));
    }
    if let Some(args) = css_function_arguments(&value, "min").first() {
        return parse_css_dimension_function_terms(args, axis)
            .map(|(terms, len)| CssDimension::Min(terms, len));
    }
    if let Some(args) = css_function_arguments(&value, "max").first() {
        return parse_css_dimension_function_terms(args, axis)
            .map(|(terms, len)| CssDimension::Max(terms, len));
    }
    parse_css_dimension_term(&value, axis).map(CssDimensionTerm::into_dimension)
}

fn parse_css_dimension_function_terms(
    args: &str,
    axis: CssAxis,
) -> Option<([CssDimensionTerm; 4], usize)> {
    let mut terms = [CssDimensionTerm::zero(); 4];
    let mut len = 0usize;
    for arg in split_css_transform_arguments(args) {
        if len >= terms.len() {
            break;
        }
        terms[len] = parse_css_dimension_term(arg, axis)?;
        len = len.saturating_add(1);
    }
    (len > 0).then_some((terms, len))
}

fn parse_css_dimension_term(value: &str, axis: CssAxis) -> Option<CssDimensionTerm> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<f32>().ok()?;
        if !percent.is_finite() || percent < 0.0 {
            return None;
        }
        return Some(CssDimensionTerm::Percent((percent * 100.0).round() as i32));
    }
    css_length_cells(&value, axis, 512).map(CssDimensionTerm::Cells)
}

fn parse_css_aspect_ratio(value: &str) -> Option<CssAspectRatio> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "auto" || value == "inherit" || value == "initial" {
        return None;
    }
    let value = value.strip_prefix("auto ").unwrap_or(value.as_str()).trim();
    let (width, height) = value
        .split_once('/')
        .map(|(width, height)| (width.trim(), height.trim()))
        .unwrap_or((value, "1"));
    let width = parse_css_aspect_ratio_component(width)?;
    let height = parse_css_aspect_ratio_component(height)?;
    Some(CssAspectRatio { width, height })
}

fn parse_css_aspect_ratio_component(value: &str) -> Option<usize> {
    let component = value.trim().parse::<f32>().ok()?;
    if !component.is_finite() || component <= 0.0 {
        return None;
    }
    Some(((component * 1000.0).round() as usize).max(1))
}

fn default_horizontal_dimension_basis() -> usize {
    CSS_DEFAULT_VIEWPORT_WIDTH_CELLS as usize
}

fn default_vertical_dimension_basis() -> usize {
    CSS_DEFAULT_VIEWPORT_HEIGHT_CELLS as usize
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ParsedBorder {
    enabled: bool,
    width: Option<usize>,
    shade: Option<u8>,
}

fn parse_css_border(value: &str) -> ParsedBorder {
    let mut border = ParsedBorder::default();
    for token in value.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("none") || token.eq_ignore_ascii_case("hidden") {
            return ParsedBorder::default();
        }
        border.enabled |= parse_css_border_style_enabled(token);
        border.width = parse_css_border_width(token).or(border.width);
        border.shade = parse_css_color_shade(token).or(border.shade);
    }
    border
}

fn parse_css_border_style_enabled(value: &str) -> bool {
    value.split_ascii_whitespace().any(|token| {
        matches!(
            token.to_ascii_lowercase().as_str(),
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
        )
    })
}

fn parse_css_border_width(value: &str) -> Option<usize> {
    for token in value.split_ascii_whitespace() {
        let token = token.trim().to_ascii_lowercase();
        match token.as_str() {
            "thin" | "medium" => return Some(1),
            "thick" => return Some(2),
            _ => {}
        }
        if let Some(pixels) = parse_css_length_pixels(&token) {
            if pixels == 0.0 {
                continue;
            }
            return Some(((pixels / 8.0).ceil() as usize).clamp(1, 4));
        }
    }
    None
}

fn parse_css_color_shade(value: &str) -> Option<u8> {
    let (red, green, blue) = parse_css_color_rgb_value(value)?;
    Some(rgb_to_luma(red, green, blue))
}

fn parse_css_color_rgb_value(value: &str) -> Option<(u8, u8, u8)> {
    if let Some((red, green, blue)) = parse_css_rgb_function(value) {
        return Some((red, green, blue));
    }
    for token in value.split_ascii_whitespace() {
        let token = token.trim_matches(|ch: char| ch == ',' || ch == ';');
        if token.eq_ignore_ascii_case("transparent") {
            return None;
        }
        if let Some((red, green, blue)) = parse_css_rgb_function(token) {
            return Some((red, green, blue));
        }
        if let Some(rgb) = parse_hex_color_rgb(token) {
            return Some(rgb);
        }
        match token.to_ascii_lowercase().as_str() {
            "black" => return Some((0, 0, 0)),
            "white" => return Some((255, 255, 255)),
            "gray" | "grey" => return Some((128, 128, 128)),
            "silver" => return Some((192, 192, 192)),
            "red" => return Some((255, 0, 0)),
            "green" => return Some((0, 128, 0)),
            "blue" => return Some((0, 0, 255)),
            "yellow" => return Some((255, 255, 0)),
            _ => {}
        }
    }
    None
}

fn parse_css_rgb_function(value: &str) -> Option<(u8, u8, u8)> {
    let args = css_first_function_arguments(value, &["rgb", "rgba"])?;
    let args = args.split('/').next().unwrap_or(args);
    let normalized = args.replace(',', " ");
    let mut components = normalized.split_ascii_whitespace();
    let red = parse_css_rgb_component(components.next()?)?;
    let green = parse_css_rgb_component(components.next()?)?;
    let blue = parse_css_rgb_component(components.next()?)?;
    Some((red, green, blue))
}

fn parse_css_rgb_component(value: &str) -> Option<u8> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.parse::<f32>().ok()?.clamp(0.0, 100.0);
        return Some(((percent * 255.0 / 100.0).round() as u16).min(u8::MAX as u16) as u8);
    }
    Some(
        value
            .parse::<f32>()
            .ok()?
            .round()
            .clamp(0.0, u8::MAX as f32) as u8,
    )
}

fn parse_css_image_url(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches(';');
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    split_css_top_level_commas(value)
        .into_iter()
        .find_map(|layer| parse_css_image_layer_url(&layer))
}

fn parse_css_image_layer_url(value: &str) -> Option<String> {
    parse_css_image_set_url(value).or_else(|| parse_css_url_function(value))
}

fn parse_css_url_function(value: &str) -> Option<String> {
    parse_css_image_url_token(css_first_function_arguments(value, &["url"])?)
}

fn parse_css_image_set_url(value: &str) -> Option<String> {
    let args = css_first_function_arguments(value, &["image-set", "-webkit-image-set"])?;
    split_css_top_level_commas(args)
        .into_iter()
        .find_map(|candidate| parse_css_image_set_candidate_url(&candidate))
}

fn parse_css_image_set_candidate_url(candidate: &str) -> Option<String> {
    let url = parse_css_url_function(candidate).or_else(|| parse_css_quoted_url(candidate))?;
    css_image_url_supported(&url, candidate).then_some(url)
}

fn parse_css_quoted_url(value: &str) -> Option<String> {
    let value = value.trim_start();
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let end = value[1..]
        .find(quote as char)
        .map(|offset| 1usize.saturating_add(offset))?;
    parse_css_image_url_token(&value[..=end])
}

fn parse_css_image_url_token(value: &str) -> Option<String> {
    let url = value.trim();
    if url.is_empty() {
        return None;
    }
    let bytes = url.as_bytes();
    let url = if bytes.len() >= 2
        && matches!(bytes[0], b'\'' | b'"')
        && bytes.last() == Some(&bytes[0])
    {
        &url[1..url.len().saturating_sub(1)]
    } else {
        url
    };
    (!url.is_empty()).then(|| url.to_owned())
}

fn css_image_url_supported(url: &str, candidate: &str) -> bool {
    let candidate = candidate.to_ascii_lowercase();
    if candidate.contains("image/avif") || url_has_extension(url, "avif") {
        return false;
    }
    true
}

fn url_has_extension(url: &str, extension: &str) -> bool {
    let base = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    base.rsplit('.')
        .next()
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(extension))
}

fn parse_css_background_image_size(value: &str) -> Option<BackgroundImageSize> {
    let value = split_css_top_level_commas(value)
        .into_iter()
        .next()
        .unwrap_or_else(|| value.to_owned())
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase();
    match value.as_str() {
        "auto" | "initial" => Some(BackgroundImageSize::Auto),
        "cover" => Some(BackgroundImageSize::Cover),
        "contain" => Some(BackgroundImageSize::Contain),
        _ => None,
    }
}

fn parse_css_background_image_repeat(value: &str) -> Option<BackgroundImageRepeat> {
    let value = split_css_top_level_commas(value)
        .into_iter()
        .next()
        .unwrap_or_else(|| value.to_owned())
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase();
    if value
        .split_ascii_whitespace()
        .any(|token| token == "no-repeat")
    {
        Some(BackgroundImageRepeat::NoRepeat)
    } else if value
        .split_ascii_whitespace()
        .any(|token| matches!(token, "repeat" | "repeat-x" | "repeat-y"))
    {
        Some(BackgroundImageRepeat::Repeat)
    } else {
        None
    }
}

fn parse_css_background_image_position(value: &str) -> Option<BackgroundImagePosition> {
    let layer = split_css_top_level_commas(value)
        .into_iter()
        .next()
        .unwrap_or_else(|| value.to_owned());
    let tokens = layer
        .split_ascii_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| ch == ',' || ch == ';')
                .to_ascii_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return None;
    }

    let mut x_percent = None;
    let mut y_percent = None;
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "left" => {
                let offset = css_position_following_percentage(&tokens, index).unwrap_or(0);
                x_percent = Some(offset);
                index += usize::from(
                    index + 1 < tokens.len() && parse_css_percentage(&tokens[index + 1]).is_some(),
                );
            }
            "right" => {
                let offset = css_position_following_percentage(&tokens, index).unwrap_or(0);
                x_percent = Some(100 - offset);
                index += usize::from(
                    index + 1 < tokens.len() && parse_css_percentage(&tokens[index + 1]).is_some(),
                );
            }
            "top" => {
                let offset = css_position_following_percentage(&tokens, index).unwrap_or(0);
                y_percent = Some(offset);
                index += usize::from(
                    index + 1 < tokens.len() && parse_css_percentage(&tokens[index + 1]).is_some(),
                );
            }
            "bottom" => {
                let offset = css_position_following_percentage(&tokens, index).unwrap_or(0);
                y_percent = Some(100 - offset);
                index += usize::from(
                    index + 1 < tokens.len() && parse_css_percentage(&tokens[index + 1]).is_some(),
                );
            }
            "center" => {
                if x_percent.is_none() {
                    x_percent = Some(50);
                } else if y_percent.is_none() {
                    y_percent = Some(50);
                }
            }
            token => {
                if let Some(percent) = parse_css_percentage(token) {
                    if x_percent.is_none() {
                        x_percent = Some(percent);
                    } else if y_percent.is_none() {
                        y_percent = Some(percent);
                    }
                }
            }
        }
        index += 1;
    }

    if x_percent.is_some() && y_percent.is_none() {
        y_percent = Some(50);
    }
    if y_percent.is_some() && x_percent.is_none() {
        x_percent = Some(50);
    }

    Some(BackgroundImagePosition {
        x_percent: x_percent.unwrap_or(0),
        y_percent: y_percent.unwrap_or(0),
    })
}

fn css_position_following_percentage(tokens: &[String], index: usize) -> Option<i32> {
    tokens
        .get(index + 1)
        .and_then(|token| parse_css_percentage(token))
}

fn parse_css_percentage(value: &str) -> Option<i32> {
    let numeric = value.trim().strip_suffix('%')?;
    numeric
        .parse::<f32>()
        .ok()
        .map(|value| value.round() as i32)
}

fn parse_css_text_align(value: &str) -> Option<TextAlign> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "left" | "start" => Some(TextAlign::Start),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::End),
        _ => None,
    }
}

fn parse_css_visibility(value: &str) -> Option<Visibility> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "visible" => Some(Visibility::Visible),
        "hidden" | "collapse" => Some(Visibility::Hidden),
        _ => None,
    }
}

fn parse_css_opacity(value: &str) -> Option<PaintOpacity> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    let opacity = value.parse::<f32>().ok()?;
    if opacity <= 0.0 {
        Some(PaintOpacity::Transparent)
    } else if opacity >= 1.0 {
        Some(PaintOpacity::Opaque)
    } else {
        None
    }
}

fn parse_css_animation_reveals_opacity(value: &str) -> Option<bool> {
    let mut saw_fill_mode = None;
    for token in value.split_ascii_whitespace() {
        let token = token
            .trim_matches(|ch: char| ch == ',' || ch == ';')
            .to_ascii_lowercase();
        match token.as_str() {
            "forwards" | "both" => return Some(true),
            "none" | "backwards" => saw_fill_mode = Some(false),
            _ => {}
        }
    }
    saw_fill_mode
}

fn parse_css_overflow(value: &str) -> Option<Overflow> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    let mut saw_value = false;
    let mut clips = false;
    for token in value.split_whitespace().take(2) {
        saw_value = true;
        match token {
            "visible" => {}
            "hidden" | "clip" | "auto" | "scroll" => clips = true,
            _ => return None,
        }
    }
    saw_value.then_some(if clips {
        Overflow::Clip
    } else {
        Overflow::Visible
    })
}

fn parse_css_float(value: &str) -> Option<Option<FloatSide>> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "none" => Some(None),
        "left" | "inline-start" => Some(Some(FloatSide::Left)),
        "right" | "inline-end" => Some(Some(FloatSide::Right)),
        _ => None,
    }
}

fn parse_css_clear(value: &str) -> Option<Option<ClearSide>> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "none" => Some(None),
        "left" | "inline-start" => Some(Some(ClearSide::Left)),
        "right" | "inline-end" => Some(Some(ClearSide::Right)),
        "both" | "inline" => Some(Some(ClearSide::Both)),
        _ => None,
    }
}

fn parse_css_position(value: &str) -> Option<Position> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "static" => Some(Position::Static),
        "relative" => Some(Position::Relative),
        "absolute" => Some(Position::Absolute),
        "fixed" => Some(Position::Fixed),
        "sticky" | "-webkit-sticky" => Some(Position::Sticky),
        _ => None,
    }
}

fn parse_css_position_offset(value: &str, axis: CssAxis) -> Option<CssPositionOffset> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "auto" || value == "inherit" || value == "initial" {
        return None;
    }
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<f32>().ok()?;
        if !percent.is_finite() {
            return None;
        }
        return Some(CssPositionOffset {
            cells: 0,
            percent_basis_points: (percent * 100.0).round() as i32,
        });
    }
    let pixels = parse_css_signed_length_pixels(&value)?;
    let cell_px = css_axis_cell_px(axis);
    let cells = if pixels == 0.0 {
        0
    } else {
        let sign = if pixels < 0.0 { -1 } else { 1 };
        sign * (pixels.abs() / cell_px).ceil() as isize
    };
    Some(CssPositionOffset {
        cells,
        percent_basis_points: 0,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ParsedPositionOffsets {
    top: Option<CssPositionOffset>,
    right: Option<CssPositionOffset>,
    bottom: Option<CssPositionOffset>,
    left: Option<CssPositionOffset>,
}

fn parse_css_inset_offsets(value: &str) -> ParsedPositionOffsets {
    let parts = value.split_ascii_whitespace().collect::<Vec<_>>();
    let top = parts
        .first()
        .and_then(|top| parse_css_position_offset(top, CssAxis::Vertical));
    let right_token = match parts.len() {
        0 | 1 => parts.first(),
        _ => parts.get(1),
    };
    let bottom_token = match parts.len() {
        0 | 1 => parts.first(),
        2 => parts.first(),
        _ => parts.get(2),
    };
    let left_token = match parts.len() {
        0 | 1 => parts.first(),
        2 | 3 => parts.get(1),
        _ => parts.get(3),
    };
    ParsedPositionOffsets {
        top,
        right: right_token.and_then(|right| parse_css_position_offset(right, CssAxis::Horizontal)),
        bottom: bottom_token
            .and_then(|bottom| parse_css_position_offset(bottom, CssAxis::Vertical)),
        left: left_token.and_then(|left| parse_css_position_offset(left, CssAxis::Horizontal)),
    }
}

fn parse_css_inset_axis_offsets(
    value: &str,
    axis: CssAxis,
) -> (Option<CssPositionOffset>, Option<CssPositionOffset>) {
    let parts = value.split_ascii_whitespace().collect::<Vec<_>>();
    let start = parts
        .first()
        .and_then(|start| parse_css_position_offset(start, axis));
    let end = match parts.len() {
        0 => None,
        1 => parts.first(),
        _ => parts.get(1),
    }
    .and_then(|end| parse_css_position_offset(end, axis));
    (start, end)
}

fn parse_css_transform_translate(value: &str) -> Option<CssTranslate> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "inherit" || value == "initial" {
        return None;
    }
    if value == "none" {
        return Some(CssTranslate::default());
    }

    let mut translate = CssTranslate::default();
    let mut saw_transform = false;
    for args in css_function_arguments(&value, "translate") {
        saw_transform = true;
        let args = split_css_transform_arguments(args);
        if let Some(x) = args
            .first()
            .and_then(|x| parse_css_position_offset(x, CssAxis::Horizontal))
        {
            translate.add_x(x);
        }
        if let Some(y) = args
            .get(1)
            .and_then(|y| parse_css_position_offset(y, CssAxis::Vertical))
        {
            translate.add_y(y);
        }
    }
    for args in css_function_arguments(&value, "translatex") {
        saw_transform = true;
        if let Some(x) = split_css_transform_arguments(args)
            .first()
            .and_then(|x| parse_css_position_offset(x, CssAxis::Horizontal))
        {
            translate.add_x(x);
        }
    }
    for args in css_function_arguments(&value, "translatey") {
        saw_transform = true;
        if let Some(y) = split_css_transform_arguments(args)
            .first()
            .and_then(|y| parse_css_position_offset(y, CssAxis::Vertical))
        {
            translate.add_y(y);
        }
    }

    saw_transform.then_some(translate)
}

fn parse_css_translate_property(value: &str) -> Option<CssTranslate> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "none" {
        return Some(CssTranslate::default());
    }
    let args = split_css_transform_arguments(&value);
    let mut translate = CssTranslate::default();
    if let Some(x) = args
        .first()
        .and_then(|x| parse_css_position_offset(x, CssAxis::Horizontal))
    {
        translate.add_x(x);
    }
    if let Some(y) = args
        .get(1)
        .and_then(|y| parse_css_position_offset(y, CssAxis::Vertical))
    {
        translate.add_y(y);
    }
    (!args.is_empty()).then_some(translate)
}

fn css_function_arguments<'a>(value: &'a str, name: &str) -> Vec<&'a str> {
    let mut args = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = value[cursor..].find(name) {
        let name_start = cursor.saturating_add(relative_start);
        let open = name_start.saturating_add(name.len());
        if !value[open..].starts_with('(') {
            cursor = open;
            continue;
        }
        let args_start = open.saturating_add(1);
        let mut depth = 1usize;
        for (offset, ch) in value[args_start..].char_indices() {
            match ch {
                '(' => depth = depth.saturating_add(1),
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let args_end = args_start.saturating_add(offset);
                        args.push(value[args_start..args_end].trim());
                        cursor = args_end.saturating_add(1);
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth > 0 {
            break;
        }
    }
    args
}

fn css_first_function_arguments<'a>(value: &'a str, names: &[&str]) -> Option<&'a str> {
    let lower = value.to_ascii_lowercase();
    let mut best_match = None;
    for name in names {
        let name = name.to_ascii_lowercase();
        let mut cursor = 0usize;
        while let Some(relative_start) = lower[cursor..].find(&name) {
            let name_start = cursor.saturating_add(relative_start);
            let open = name_start.saturating_add(name.len());
            let previous = value[..name_start].chars().next_back();
            if previous
                .is_none_or(|ch| !matches!(ch, '-' | '_' | 'a'..='z' | 'A'..='Z' | '0'..='9'))
                && value[open..].starts_with('(')
                && best_match.is_none_or(|(best_start, _)| name_start < best_start)
            {
                best_match = Some((name_start, name.len()));
                break;
            }
            cursor = open;
        }
    }
    let (name_start, name_len) = best_match?;
    let args_start = name_start.saturating_add(name_len).saturating_add(1);
    let args_end = find_css_function_close(value, args_start)?;
    Some(value[args_start..args_end].trim())
}

fn find_css_function_close(value: &str, args_start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in value[args_start..].char_indices() {
        if let Some(quote_ch) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(args_start.saturating_add(offset));
                }
            }
            _ => {}
        }
    }
    None
}

fn split_css_transform_arguments(value: &str) -> Vec<&str> {
    if value.contains(',') {
        value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect()
    } else {
        value
            .split_ascii_whitespace()
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect()
    }
}

fn parse_css_z_index(value: &str) -> Option<i32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "auto" {
        return Some(0);
    }
    value.parse::<i32>().ok()
}

fn parse_css_box_sizing(value: &str) -> Option<BoxSizing> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "content-box" => Some(BoxSizing::ContentBox),
        "border-box" => Some(BoxSizing::BorderBox),
        _ => None,
    }
}

fn parse_css_display(value: &str) -> Option<Display> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    match value.as_str() {
        "none" => return Some(Display::None),
        "block" => return Some(Display::Block),
        "flex" => return Some(Display::Flex),
        "flow-root" => return Some(Display::FlowRoot),
        "grid" => return Some(Display::Grid),
        "inline" => return Some(Display::Inline),
        "inline-block" => return Some(Display::InlineBlock),
        "inline-flex" => return Some(Display::InlineFlex),
        "inline-grid" => return Some(Display::InlineGrid),
        "list-item" => return Some(Display::ListItem),
        "table" | "inline-table" => return Some(Display::Table),
        "table-row" => return Some(Display::TableRow),
        "table-cell" => return Some(Display::TableCell),
        "contents" => return Some(Display::Contents),
        _ => {}
    }

    let tokens: Vec<&str> = value.split_ascii_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let inline = tokens.contains(&"inline");
    if tokens.contains(&"none") {
        return Some(Display::None);
    }
    if tokens.contains(&"contents") {
        return Some(Display::Contents);
    }
    if tokens.contains(&"flex") {
        return Some(if inline {
            Display::InlineFlex
        } else {
            Display::Flex
        });
    }
    if tokens.contains(&"grid") {
        return Some(if inline {
            Display::InlineGrid
        } else {
            Display::Grid
        });
    }
    if tokens.contains(&"flow-root") {
        return Some(if inline {
            Display::InlineBlock
        } else {
            Display::FlowRoot
        });
    }
    if tokens.contains(&"table") {
        return Some(Display::Table);
    }
    if tokens.contains(&"list-item") {
        return Some(Display::ListItem);
    }
    if tokens.contains(&"block") {
        return Some(Display::Block);
    }
    inline.then_some(Display::Inline)
}

fn parse_css_white_space(value: &str) -> Option<WhiteSpace> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "normal" => Some(WhiteSpace::Normal),
        "nowrap" => Some(WhiteSpace::Nowrap),
        "pre" => Some(WhiteSpace::Pre),
        "pre-line" => Some(WhiteSpace::PreLine),
        "pre-wrap" => Some(WhiteSpace::PreWrap),
        "break-spaces" => Some(WhiteSpace::BreakSpaces),
        _ => None,
    }
}

fn parse_css_text_transform(value: &str) -> Option<TextTransform> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "none" => Some(TextTransform::None),
        "uppercase" => Some(TextTransform::Uppercase),
        "lowercase" => Some(TextTransform::Lowercase),
        "capitalize" => Some(TextTransform::Capitalize),
        _ => None,
    }
}

fn parse_css_letter_spacing(value: &str) -> Option<usize> {
    if value
        .trim()
        .trim_end_matches(';')
        .eq_ignore_ascii_case("normal")
    {
        return Some(0);
    }
    css_length_cell_units(value, CssAxis::Horizontal, 512)
}

fn parse_css_word_spacing(value: &str) -> Option<usize> {
    if value
        .trim()
        .trim_end_matches(';')
        .eq_ignore_ascii_case("normal")
    {
        return Some(0);
    }
    css_length_cell_units(value, CssAxis::Horizontal, 512).map(|units| units / CSS_TEXT_CELL_UNITS)
}

fn parse_css_overflow_wrap(value: &str) -> Option<OverflowWrap> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "normal" => Some(OverflowWrap::Normal),
        "break-word" => Some(OverflowWrap::BreakWord),
        "anywhere" => Some(OverflowWrap::Anywhere),
        _ => None,
    }
}

fn parse_css_word_break(value: &str) -> Option<WordBreak> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "normal" => Some(WordBreak::Normal),
        "break-all" => Some(WordBreak::BreakAll),
        "break-word" => Some(WordBreak::BreakWord),
        "keep-all" => Some(WordBreak::KeepAll),
        _ => None,
    }
}

fn parse_css_line_height(value: &str) -> Option<usize> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if value == "normal" {
        return Some(1);
    }
    if value.starts_with('-') || value == "inherit" || value == "initial" {
        return None;
    }
    if value.ends_with("px")
        || value.ends_with("rem")
        || value.ends_with("em")
        || value.ends_with("ch")
    {
        return parse_css_dimension_length(&value, CssAxis::Vertical).map(|rows| rows.clamp(1, 16));
    }
    let rows = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f32>().ok()? / 100.0
    } else {
        value.parse::<f32>().ok()?
    };
    (rows > 0.0)
        .then(|| rows.ceil() as usize)
        .map(|rows| rows.clamp(1, 16))
}

fn parse_css_font_scale(value: &str) -> Option<usize> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    match value.as_str() {
        "xx-small" | "x-small" | "small" | "medium" | "smaller" => return Some(1),
        "large" | "x-large" | "larger" => return Some(2),
        "xx-large" | "xxx-large" => return Some(3),
        "inherit" | "initial" | "unset" | "revert" | "normal" => return None,
        _ => {}
    }
    css_font_scale_from_pixels(parse_css_font_size_pixels(&value)?)
}

fn parse_css_font_shorthand_scale(value: &str) -> Option<usize> {
    for token in split_css_top_level_whitespace(value) {
        let size = token
            .split_once('/')
            .map(|(size, _)| size)
            .unwrap_or(token.as_str());
        if let Some(scale) = parse_css_font_scale(size) {
            return Some(scale);
        }
    }
    None
}

fn parse_css_font_size_pixels(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches(';').to_ascii_lowercase();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.parse::<f32>().ok()?;
        return percent.is_finite().then_some(16.0 * percent / 100.0);
    }
    if let Some(args) = css_function_arguments(&value, "clamp").first() {
        let args = split_css_transform_arguments(args);
        let minimum = args.first().and_then(|arg| parse_css_font_size_pixels(arg));
        let preferred = args.get(1).and_then(|arg| parse_css_font_size_pixels(arg));
        let maximum = args.get(2).and_then(|arg| parse_css_font_size_pixels(arg));
        return match (minimum, preferred, maximum) {
            (Some(minimum), Some(preferred), Some(maximum)) => {
                Some(preferred.clamp(minimum, maximum))
            }
            (_, Some(preferred), _) => Some(preferred),
            (_, _, Some(maximum)) => Some(maximum),
            (Some(minimum), _, _) => Some(minimum),
            _ => None,
        };
    }
    if let Some(args) = css_function_arguments(&value, "max").first() {
        return split_css_transform_arguments(args)
            .into_iter()
            .filter_map(parse_css_font_size_pixels)
            .reduce(f32::max);
    }
    if let Some(args) = css_function_arguments(&value, "min").first() {
        return split_css_transform_arguments(args)
            .into_iter()
            .filter_map(parse_css_font_size_pixels)
            .reduce(f32::min);
    }
    parse_css_length_pixels(&value)
}

fn css_font_scale_from_pixels(pixels: f32) -> Option<usize> {
    if !pixels.is_finite() || pixels <= 0.0 {
        return None;
    }
    Some(match pixels {
        pixels if pixels >= 64.0 => 4,
        pixels if pixels >= 40.0 => 3,
        pixels if pixels >= 22.0 => 2,
        _ => 1,
    })
}

fn split_css_top_level_whitespace(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in value.chars() {
        match ch {
            '(' => {
                depth = depth.saturating_add(1);
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ch if ch.is_ascii_whitespace() && depth == 0 => {
                if !current.trim().is_empty() {
                    tokens.push(current.trim().to_owned());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        tokens.push(current.trim().to_owned());
    }
    tokens
}

fn split_css_top_level_commas(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for ch in value.chars() {
        if let Some(quote_ch) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                depth = depth.saturating_add(1);
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    tokens.push(current.trim().to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        tokens.push(current.trim().to_owned());
    }
    tokens
}

fn parse_css_list_style(value: &str) -> Option<CssListStyleType> {
    value
        .split_ascii_whitespace()
        .find_map(parse_css_list_style_type)
}

fn parse_css_list_style_type(value: &str) -> Option<CssListStyleType> {
    match value
        .trim()
        .trim_end_matches(';')
        .to_ascii_lowercase()
        .as_str()
    {
        "none" => Some(CssListStyleType::NoMarker),
        "disc" => Some(CssListStyleType::Disc),
        "circle" => Some(CssListStyleType::Circle),
        "square" => Some(CssListStyleType::Square),
        "decimal" => Some(CssListStyleType::Decimal),
        "lower-alpha" | "lower-latin" => Some(CssListStyleType::LowerAlpha),
        "upper-alpha" | "upper-latin" => Some(CssListStyleType::UpperAlpha),
        "lower-roman" => Some(CssListStyleType::LowerRoman),
        "upper-roman" => Some(CssListStyleType::UpperRoman),
        _ => None,
    }
}

fn parse_hex_color_rgb(token: &str) -> Option<(u8, u8, u8)> {
    let hex = token.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let r = hex_nibble(hex.as_bytes()[0])? * 17;
            let g = hex_nibble(hex.as_bytes()[1])? * 17;
            let b = hex_nibble(hex.as_bytes()[2])? * 17;
            Some((r, g, b))
        }
        6 => {
            let r = hex_byte(&hex.as_bytes()[0..2])?;
            let g = hex_byte(&hex.as_bytes()[2..4])?;
            let b = hex_byte(&hex.as_bytes()[4..6])?;
            Some((r, g, b))
        }
        _ => None,
    }
}

fn hex_byte(bytes: &[u8]) -> Option<u8> {
    Some(hex_nibble(bytes[0])? * 16 + hex_nibble(bytes[1])?)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn rgb_to_luma(red: u8, green: u8, blue: u8) -> u8 {
    let red = red as u16;
    let green = green as u16;
    let blue = blue as u16;
    ((red * 77 + green * 150 + blue * 29) >> 8) as u8
}

fn dom_title(dom: &Dom) -> String {
    dom.nodes
        .iter()
        .enumerate()
        .find_map(|(node_id, node)| match &node.kind {
            NodeKind::Element(element) if element.tag == "title" => {
                Some(text_content(dom, node_id).trim().to_owned())
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn text_content(dom: &Dom, node_id: usize) -> String {
    let mut out = String::new();
    collect_text(dom, node_id, &mut out);
    out
}

fn collect_links(dom: &Dom, source: &str) -> Vec<BrowserLink> {
    let mut links = Vec::new();
    collect_links_at(dom, 0, source, &mut links);
    links
}

fn collect_links_at(dom: &Dom, node_id: usize, source: &str, links: &mut Vec<BrowserLink>) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    if let NodeKind::Element(element) = &node.kind
        && element.tag == "a"
        && let Some(href) = element.href.as_ref().map(|href| href.trim())
        && !href.is_empty()
    {
        links.push(BrowserLink {
            text: collapse_ascii_whitespace(&text_content(dom, node_id)),
            href: href.to_owned(),
            resolved: resolve_browser_href(source, href),
        });
    }

    for &child in &node.children {
        collect_links_at(dom, child, source, links);
    }
}

fn anchor_href_for_node(dom: &Dom, mut node_id: usize) -> Option<String> {
    loop {
        let node = dom.nodes.get(node_id)?;
        if let NodeKind::Element(element) = &node.kind
            && element.tag == "a"
            && let Some(href) = element.href.as_ref().map(|href| href.trim())
            && !href.is_empty()
        {
            return Some(href.to_owned());
        }
        node_id = node.parent?;
    }
}

fn click_default_action_for_node(
    dom: &Dom,
    source: &str,
    dispatch: BrowserClickDispatch,
) -> Option<BrowserClickDefaultAction> {
    if let Some(href) = anchor_href_for_node(dom, dispatch.node_id) {
        return Some(BrowserClickDefaultAction::Anchor {
            resolved: resolve_browser_href(source, &href),
            default_prevented: dispatch.default_prevented,
        });
    }
    form_default_action_for_node(dom, source, dispatch.node_id).map(|action| match action {
        FormControlDefaultAction::Submit {
            form_index,
            submitter,
        } => BrowserClickDefaultAction::SubmitForm {
            form_index,
            submitter,
            default_prevented: dispatch.default_prevented,
        },
        FormControlDefaultAction::Reset { form_index } => BrowserClickDefaultAction::ResetForm {
            form_index,
            default_prevented: dispatch.default_prevented,
        },
        FormControlDefaultAction::Toggle {
            form_index,
            control_index,
        } => BrowserClickDefaultAction::ToggleFormControl {
            form_index,
            control_index,
            default_prevented: dispatch.default_prevented,
        },
    })
}

enum FormControlDefaultAction {
    Submit {
        form_index: usize,
        submitter: BrowserFormSubmitter,
    },
    Reset {
        form_index: usize,
    },
    Toggle {
        form_index: usize,
        control_index: usize,
    },
}

fn form_default_action_for_node(
    dom: &Dom,
    source: &str,
    node_id: usize,
) -> Option<FormControlDefaultAction> {
    form_default_action_for_node_or_ancestor(dom, source, node_id).or_else(|| {
        associated_label_control_node(dom, node_id).and_then(|control_node_id| {
            form_default_action_for_node_or_ancestor(dom, source, control_node_id)
        })
    })
}

fn form_default_action_for_node_or_ancestor(
    dom: &Dom,
    source: &str,
    node_id: usize,
) -> Option<FormControlDefaultAction> {
    let mut current = Some(node_id);
    while let Some(current_node_id) = current {
        let node = dom.nodes.get(current_node_id)?;
        if let NodeKind::Element(element) = &node.kind {
            if element.tag == "form" {
                return None;
            }
            if let Some(control_action) = default_action_for_form_control(element, source) {
                let form_node_id = nearest_form_ancestor(dom, current_node_id)?;
                let form_index = form_index_for_node(dom, form_node_id)?;
                return match control_action {
                    FormControlElementAction::Submit(submitter) => {
                        Some(FormControlDefaultAction::Submit {
                            form_index,
                            submitter,
                        })
                    }
                    FormControlElementAction::Reset => {
                        Some(FormControlDefaultAction::Reset { form_index })
                    }
                    FormControlElementAction::Toggle => {
                        let control_index =
                            form_control_index_for_node(dom, form_node_id, current_node_id)?;
                        Some(FormControlDefaultAction::Toggle {
                            form_index,
                            control_index,
                        })
                    }
                };
            }
        }
        current = node.parent;
    }
    None
}

enum FormControlElementAction {
    Submit(BrowserFormSubmitter),
    Reset,
    Toggle,
}

fn default_action_for_form_control(
    element: &ElementData,
    source: &str,
) -> Option<FormControlElementAction> {
    if element.disabled {
        return None;
    }
    match element.tag.as_str() {
        "input" => {
            let kind = element.input_type.as_deref().unwrap_or("text");
            if kind.eq_ignore_ascii_case("submit") {
                return Some(FormControlElementAction::Submit(submitter_from_element(
                    element, source,
                )));
            }
            if kind.eq_ignore_ascii_case("reset") {
                return Some(FormControlElementAction::Reset);
            }
            if matches!(kind.to_ascii_lowercase().as_str(), "checkbox" | "radio") {
                return Some(FormControlElementAction::Toggle);
            }
            None
        }
        "button" => {
            let kind = element.input_type.as_deref().unwrap_or("submit");
            if kind.eq_ignore_ascii_case("submit") {
                return Some(FormControlElementAction::Submit(submitter_from_element(
                    element, source,
                )));
            }
            if kind.eq_ignore_ascii_case("reset") {
                return Some(FormControlElementAction::Reset);
            }
            None
        }
        _ => None,
    }
}

fn submitter_from_element(element: &ElementData, source: &str) -> BrowserFormSubmitter {
    let fields = match element.name.as_deref() {
        Some(name) if !name.is_empty() => vec![(
            name.to_owned(),
            element
                .value
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_default(),
        )],
        _ => Vec::new(),
    };
    BrowserFormSubmitter {
        fields,
        no_validate: element.attrs.contains_key("formnovalidate"),
        method: submitter_form_method(element),
        resolved_action: submitter_resolved_form_action(element, source),
    }
}

fn collapse_ascii_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn resolve_browser_href(source: &str, href: &str) -> String {
    if let Ok(url) = Url::parse(href) {
        return url.to_string();
    }
    if let Ok(base) = Url::parse(source)
        && let Ok(url) = base.join(href)
    {
        return url.to_string();
    }
    if href.starts_with('#') {
        let base = source
            .split_once('#')
            .map_or(source, |(without_fragment, _)| without_fragment);
        return format!("{base}{href}");
    }
    if href.starts_with('?') {
        let fragmentless = source
            .split_once('#')
            .map_or(source, |(without_fragment, _)| without_fragment);
        let base = fragmentless
            .split_once('?')
            .map_or(fragmentless, |(without_query, _)| without_query);
        return format!("{base}{href}");
    }

    let base = Path::new(source);
    let parent = if base.is_dir() {
        base
    } else {
        base.parent().unwrap_or_else(|| Path::new("."))
    };
    parent.join(href).display().to_string()
}

fn collect_text(dom: &Dom, node_id: usize, out: &mut String) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.kind {
        NodeKind::Text(text) => out.push_str(text),
        _ => {
            for &child in &node.children {
                collect_text(dom, child, out);
            }
        }
    }
}

fn collect_css_background_image_resources(
    dom: &Dom,
    source: &str,
    css_cascade: &CssCascade,
) -> Vec<BrowserResource> {
    let mut resources = Vec::new();
    let mut seen = HashSet::new();
    collect_css_background_image_resources_at(
        dom,
        0,
        source,
        css_cascade,
        false,
        &mut resources,
        &mut seen,
    );
    resources
}

fn collect_css_background_image_resources_at(
    dom: &Dom,
    node_id: usize,
    source: &str,
    css_cascade: &CssCascade,
    is_row_item: bool,
    resources: &mut Vec<BrowserResource>,
    seen: &mut HashSet<String>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };

    match &node.kind {
        NodeKind::Document | NodeKind::DocumentFragment => {
            for &child in &node.children {
                collect_css_background_image_resources_at(
                    dom,
                    child,
                    source,
                    css_cascade,
                    false,
                    resources,
                    seen,
                );
            }
        }
        NodeKind::Text(_) => {}
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            if style.display == Display::None {
                return;
            }
            let child_layout = style.child_layout();
            if style.display == Display::Contents {
                for &child in &node.children {
                    if child_layout.row_items
                        && !row_layout_child_participates(dom, child, css_cascade)
                    {
                        continue;
                    }
                    collect_css_background_image_resources_at(
                        dom,
                        child,
                        source,
                        css_cascade,
                        child_layout.row_items,
                        resources,
                        seen,
                    );
                }
                return;
            }
            if style.display.is_block_flow()
                && !is_row_item
                && let Some(url) = style.background_image_url.as_deref()
            {
                push_css_background_image_resource(resources, seen, source, url);
            }
            for &child in &node.children {
                if child_layout.row_items && !row_layout_child_participates(dom, child, css_cascade)
                {
                    continue;
                }
                collect_css_background_image_resources_at(
                    dom,
                    child,
                    source,
                    css_cascade,
                    child_layout.row_items,
                    resources,
                    seen,
                );
            }
        }
    }
}

fn push_css_background_image_resource(
    resources: &mut Vec<BrowserResource>,
    seen: &mut HashSet<String>,
    source: &str,
    url: &str,
) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    let resolved = resolve_browser_href(source, url);
    if !seen.insert(resolved.clone()) {
        return;
    }
    resources.push(BrowserResource {
        kind: "background_image".to_owned(),
        initiator: "css".to_owned(),
        url: url.to_owned(),
        resolved,
        rel: None,
        media: None,
        alt: None,
        type_hint: None,
    });
}

fn render_children(
    dom: &Dom,
    node_id: usize,
    source: &str,
    css_cascade: &CssCascade,
    renderer: &mut FlowRenderer,
    layout_box_count: &mut usize,
    child_layout: ChildLayout,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    let row_align_entered = child_layout.row_items && child_layout.align_items != AlignItems::Start;
    if row_align_entered {
        renderer.enter_row_align_items(child_layout.align_items);
    }
    let mut child_seen = false;
    let mut row_item_count = 0usize;
    let row_justification = row_layout_justification(
        dom,
        node_id,
        css_cascade,
        child_layout,
        renderer.available_width(),
    );
    if let Some(justification) = row_justification
        && justification.leading_space > 0
    {
        renderer.push_fixed_space_width(justification.leading_space, None);
    }
    let mut child_sequence: Vec<usize> = if child_layout.row_items || child_layout.flex_items {
        node.children
            .iter()
            .copied()
            .filter(|&child| row_layout_child_participates(dom, child, css_cascade))
            .collect()
    } else {
        node.children.clone()
    };
    if child_layout.reverse_items {
        child_sequence.reverse();
    }
    for (child_index, &child) in child_sequence.iter().enumerate() {
        let child_width_hint = child_layout
            .wrap_items
            .then(|| {
                row_layout_child_width_hint(dom, child, css_cascade, renderer.available_width())
            })
            .flatten();
        if child_seen {
            if child_layout.row_items {
                let column_gap = row_justification
                    .map(|justification| justification.column_gap)
                    .unwrap_or_else(|| child_layout.column_gap.unwrap_or(1));
                let wraps_by_count = child_layout
                    .wrap_after
                    .is_some_and(|wrap_after| row_item_count >= wrap_after);
                let wraps_by_width = child_layout.wrap_items
                    && child_width_hint.is_some_and(|width| {
                        renderer.current_inline_width() > 0
                            && renderer
                                .effective_current_width()
                                .saturating_add(column_gap)
                                .saturating_add(width)
                                > renderer.available_width()
                    });
                if wraps_by_count || wraps_by_width {
                    renderer.break_line();
                    if let Some(row_gap) = child_layout.row_gap {
                        renderer.push_vertical_space(row_gap);
                    }
                    row_item_count = 0;
                } else if row_justification.is_some() {
                    renderer.push_fixed_space_width(column_gap, None);
                } else {
                    match child_layout.column_gap {
                        Some(gap) => renderer.push_fixed_space_width(gap, None),
                        None => renderer.push_text(" ", None),
                    }
                }
            } else if let Some(row_gap) = child_layout.row_gap {
                renderer.push_vertical_space(row_gap);
            }
        }
        let item_start_width = renderer.current_inline_width();
        let item_start_row = renderer.current_row();
        let item_margin = child_layout
            .row_items
            .then(|| row_layout_child_horizontal_margin(dom, child, css_cascade))
            .unwrap_or_default();
        if item_margin.left > 0 {
            renderer.push_fixed_space_width(item_margin.left, None);
        }
        render_node(
            dom,
            child,
            source,
            css_cascade,
            renderer,
            layout_box_count,
            child_layout.row_items,
        );
        let has_following_row_item =
            child_layout.row_items && child_index.saturating_add(1) < child_sequence.len();
        if item_margin.right > 0 && has_following_row_item {
            renderer.push_fixed_space_width(item_margin.right, None);
        }
        let item_spanned_rows = child_layout.row_items && renderer.current_row() > item_start_row;
        if let Some(width_hint) = child_width_hint
            && !item_spanned_rows
        {
            let item_width = renderer
                .current_inline_width()
                .saturating_sub(item_start_width);
            if item_width < width_hint {
                renderer.push_fixed_space_width(width_hint.saturating_sub(item_width), None);
            }
        }
        child_seen = true;
        if child_layout.row_items {
            row_item_count = row_item_count.saturating_add(1);
            if item_spanned_rows {
                renderer.break_line();
                if let Some(row_gap) = child_layout.row_gap {
                    renderer.push_vertical_space(row_gap);
                }
                row_item_count = 0;
            }
        }
    }
    if row_align_entered {
        renderer.break_line();
        renderer.exit_row_align_items();
    }
}

#[derive(Debug, Clone, Copy)]
struct RowJustification {
    leading_space: usize,
    column_gap: usize,
}

fn row_layout_justification(
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
    let base_gap = child_layout.column_gap.unwrap_or(1);
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

fn row_layout_child_width_estimate(
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
            let horizontal_margin = style.margin.left.saturating_add(style.margin.right);
            if let Some(width) = style.flex_basis.or(style.width) {
                return Some(
                    width
                        .resolve(basis)
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
            let text_width = collapse_ascii_whitespace(&text_content(dom, node_id))
                .chars()
                .count();
            if text_width > 0 {
                return Some(
                    text_width
                        .saturating_add(horizontal_margin)
                        .clamp(1, basis.max(1)),
                );
            }
            let child_width = node
                .children
                .iter()
                .filter_map(|&child| {
                    row_layout_child_width_estimate(dom, child, css_cascade, basis)
                })
                .sum::<usize>();
            (child_width > 0).then_some(
                child_width
                    .saturating_add(horizontal_margin)
                    .clamp(1, basis.max(1)),
            )
        }
        NodeKind::Document | NodeKind::DocumentFragment => None,
    }
}

fn row_layout_child_width_hint(
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
    let horizontal_margin = style.margin.left.saturating_add(style.margin.right);
    style.flex_basis.or(style.width).map(|width| {
        width
            .resolve(basis)
            .saturating_add(horizontal_margin)
            .clamp(1, basis.max(1))
    })
}

fn row_layout_child_horizontal_margin(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> BoxSpacing {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return BoxSpacing::default();
    };
    let style = computed_style(dom, node_id, element, css_cascade);
    BoxSpacing {
        left: style.margin.left,
        right: style.margin.right,
        ..BoxSpacing::default()
    }
}

fn row_layout_child_participates(dom: &Dom, node_id: usize, css_cascade: &CssCascade) -> bool {
    let Some(node) = dom.nodes.get(node_id) else {
        return false;
    };
    match &node.kind {
        NodeKind::Text(text) => !text.trim().is_empty(),
        NodeKind::Element(element) => {
            computed_style(dom, node_id, element, css_cascade).display != Display::None
        }
        NodeKind::Document | NodeKind::DocumentFragment => true,
    }
}

fn render_node(
    dom: &Dom,
    node_id: usize,
    source: &str,
    css_cascade: &CssCascade,
    renderer: &mut FlowRenderer,
    layout_box_count: &mut usize,
    is_row_item: bool,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    match &node.kind {
        NodeKind::Document | NodeKind::DocumentFragment => render_children(
            dom,
            node_id,
            source,
            css_cascade,
            renderer,
            layout_box_count,
            ChildLayout::default(),
        ),
        NodeKind::Text(text) => renderer.push_text(text, element_target_for_node(dom, node_id)),
        NodeKind::Element(element) => {
            let style = computed_style(dom, node_id, element, css_cascade);
            if source.starts_with("mem://overflow-wrap") && element.tag == "p" {
                eprintln!("style for p: {:?}", style);
            }
            if style.display == Display::None {
                return;
            }
            if style.display == Display::Contents {
                render_contents_node(
                    dom,
                    node_id,
                    source,
                    css_cascade,
                    renderer,
                    layout_box_count,
                    style,
                    is_row_item,
                );
                return;
            }
            *layout_box_count += 1;

            let visibility_entered = style.visibility;
            if let Some(visibility) = visibility_entered {
                renderer.enter_visibility(visibility);
            }
            let opacity_entered = style.suppresses_paint();
            if opacity_entered {
                renderer.enter_transparent_opacity();
            }
            let containing_height = if style.position == Position::Fixed {
                Some(default_vertical_dimension_basis())
            } else {
                renderer.current_positioning_context_height()
            };
            let out_of_flow_y = style.position.is_out_of_flow().then(|| {
                style
                    .vertical_projection_offset(containing_height)
                    .map(|offset| {
                        saturating_add_signed(renderer.current_positioning_context_y(), offset)
                    })
            });
            let mut out_of_flow_entered = style
                .position
                .is_out_of_flow()
                .then(|| renderer.enter_out_of_flow(out_of_flow_y.flatten()));
            let mut horizontal_projection_entered = (style.position.is_out_of_flow()
                || matches!(style.position, Position::Relative | Position::Sticky))
            .then(|| {
                renderer.enter_horizontal_projection(
                    style.horizontal_projection_offset(renderer.available_width()),
                )
            })
            .flatten();
            let mut vertical_projection_entered = (style.position == Position::Relative)
                .then(|| {
                    style
                        .vertical_projection_offset(None)
                        .and_then(|offset| renderer.enter_vertical_projection(offset))
                })
                .flatten();
            let viewport_fixed_entered = style.position == Position::Fixed;
            if viewport_fixed_entered {
                renderer.enter_viewport_fixed();
            }
            let fixed_clip_escape_entered = if viewport_fixed_entered {
                renderer.enter_unclipped();
                true
            } else {
                false
            };
            let viewport_sticky_top_entered = if style.position == Position::Sticky {
                style
                    .vertical_projection_offset(Some(default_vertical_dimension_basis()))
                    .map(|offset| saturating_add_signed(0, offset))
            } else {
                None
            };
            if let Some(sticky_top) = viewport_sticky_top_entered {
                renderer.enter_viewport_sticky(sticky_top);
            }
            let paint_layer_z_index =
                if style.position != Position::Static && style.z_index_specified {
                    Some(style.z_index)
                } else if matches!(style.position, Position::Fixed | Position::Sticky) {
                    Some(0)
                } else {
                    None
                };
            if let Some(z_index) = paint_layer_z_index {
                renderer.enter_positive_z_layer(z_index);
            }
            let link_text_entered = element.tag == "a"
                && element
                    .href
                    .as_ref()
                    .is_some_and(|href| !href.trim().is_empty());
            if link_text_entered {
                renderer.enter_link_text();
            }
            let exit_outer_contexts = |renderer: &mut FlowRenderer,
                                       out_of_flow_entered: &mut Option<FlowOutOfFlowSnapshot>,
                                       horizontal_projection_entered: &mut Option<
                FlowHorizontalProjectionSnapshot,
            >,
                                       vertical_projection_entered: &mut Option<
                FlowVerticalProjectionSnapshot,
            >| {
                if link_text_entered {
                    renderer.exit_link_text();
                }
                if let Some(snapshot) = vertical_projection_entered.take() {
                    renderer.exit_vertical_projection(snapshot);
                }
                if let Some(snapshot) = horizontal_projection_entered.take() {
                    renderer.exit_horizontal_projection(snapshot);
                }
                if let Some(snapshot) = out_of_flow_entered.take() {
                    renderer.exit_out_of_flow(snapshot);
                }
                if paint_layer_z_index.is_some() {
                    renderer.exit_positive_z_layer();
                }
                if fixed_clip_escape_entered {
                    renderer.exit_unclipped();
                }
                if viewport_fixed_entered {
                    renderer.exit_viewport_fixed();
                }
                if viewport_sticky_top_entered.is_some() {
                    renderer.exit_viewport_sticky();
                }
                if opacity_entered {
                    renderer.exit_transparent_opacity();
                }
                if visibility_entered.is_some() {
                    renderer.exit_visibility();
                }
            };

            if element.tag == "br" {
                renderer.break_line();
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if element.tag == "wbr" {
                renderer.push_word_break_opportunity();
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if element.tag == "hr" {
                renderer.push_horizontal_rule(Some(node_id));
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if element.tag == "svg" {
                let (svg_width, svg_height) =
                    replaced_media_placeholder_extent(element, &style, renderer);
                let rgb = svg_paint_rgb(dom, node_id, element);
                let shade = rgb
                    .map(|(red, green, blue)| rgb_to_luma(red, green, blue))
                    .or_else(|| svg_paint_shade(dom, node_id, element))
                    .or(style.background_shade)
                    .unwrap_or(220);
                let shapes = svg_paint_shapes(dom, node_id, element, svg_width, svg_height);
                if is_row_item {
                    renderer.push_inline_svg_placeholder(
                        svg_width,
                        svg_height,
                        &shapes,
                        rgb,
                        shade,
                        Some(node_id),
                    );
                } else {
                    renderer.push_svg_placeholder(
                        svg_width,
                        svg_height,
                        &shapes,
                        rgb,
                        shade,
                        Some(node_id),
                    );
                }
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if element.tag == "img" {
                let image_source =
                    image_render_source(dom, node_id, element, renderer.viewport_width_css_px());
                let intrinsic_size =
                    renderer.decoded_image_intrinsic_size(source, image_source.as_deref());
                let (image_width, image_height) =
                    image_placeholder_extent(element, &style, renderer, intrinsic_size);
                if let Some(float_side) = style.float {
                    renderer.push_floating_image_placeholder(
                        float_side,
                        image_width,
                        image_height,
                        element.alt.clone(),
                        source,
                        image_source.as_deref(),
                        Some(node_id),
                    );
                } else if is_row_item {
                    renderer.push_inline_image_placeholder(
                        image_width,
                        image_height,
                        element.alt.clone(),
                        source,
                        image_source.as_deref(),
                        Some(node_id),
                    );
                } else {
                    renderer.push_image_placeholder(
                        image_width,
                        image_height,
                        element.alt.clone(),
                        source,
                        image_source.as_deref(),
                        Some(node_id),
                    );
                }
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if is_replaced_media_element(&element.tag) {
                let (media_width, media_height) =
                    replaced_media_placeholder_extent(element, &style, renderer);
                if is_row_item {
                    renderer.push_inline_image_placeholder(
                        media_width,
                        media_height,
                        replaced_media_alt(element),
                        source,
                        replaced_media_render_source(element),
                        Some(node_id),
                    );
                } else {
                    renderer.push_image_placeholder(
                        media_width,
                        media_height,
                        replaced_media_alt(element),
                        source,
                        replaced_media_render_source(element),
                        Some(node_id),
                    );
                }
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            if let Some(label) = form_control_render_text(dom, node_id, element) {
                if !label.is_empty() {
                    renderer.push_inline_widget(&label, Some(node_id));
                }
                exit_outer_contexts(
                    renderer,
                    &mut out_of_flow_entered,
                    &mut horizontal_projection_entered,
                    &mut vertical_projection_entered,
                );
                return;
            }
            let text_shade_entered = style.text_shade;
            if let Some(text_shade) = text_shade_entered {
                renderer.enter_text_shade(text_shade);
            }
            let text_align_entered = style.text_align;
            let white_space_entered = style.white_space;
            let text_transform_entered = style.text_transform;
            let letter_spacing_entered = style.letter_spacing;
            let word_spacing_entered = style.word_spacing;
            let overflow_wrap_entered = style.overflow_wrap;
            let word_break_entered = style.word_break;
            let text_indent_entered = style.text_indent;
            let line_height_entered = style.line_height;
            let font_scale_entered = style.font_scale;
            let block_flow = style.display.is_block_flow() && !is_row_item;
            let inline_margin = if block_flow || is_row_item {
                BoxSpacing::default()
            } else {
                BoxSpacing {
                    left: style.margin.left,
                    right: style.margin.right,
                    ..BoxSpacing::default()
                }
            };
            let margin = if block_flow {
                style.margin
            } else {
                BoxSpacing::default()
            };
            if block_flow {
                renderer.break_line();
                if let Some(clear) = style.clear {
                    renderer.clear_floats(clear);
                }
                if margin.top > 0 {
                    renderer.push_vertical_space(margin.top);
                }
                if margin.left > 0 || margin.right > 0 {
                    renderer.enter_insets(margin.left, margin.right);
                }
            }
            let width_inset = if block_flow {
                let available_width = renderer.available_width();
                let block_width = style.positioned_outer_width(available_width);
                let remaining_width = available_width.saturating_sub(block_width);
                let left_inset = if style.margin_left_auto && style.margin_right_auto {
                    remaining_width / 2
                } else if style.margin_left_auto {
                    remaining_width
                } else {
                    0
                };
                let right_inset = remaining_width.saturating_sub(left_inset);
                if left_inset > 0 || right_inset > 0 {
                    renderer.enter_insets(left_inset, right_inset);
                }
                (left_inset, right_inset)
            } else {
                (0, 0)
            };
            let block_box = block_flow.then(|| {
                (
                    renderer.box_x(),
                    renderer.available_width(),
                    renderer.current_row(),
                )
            });
            let positioning_context_entered = block_box
                .filter(|_| style.position != Position::Static)
                .map(|(_, _, start_y)| {
                    let context_height = style.resolved_height().or_else(|| {
                        (style.resolved_min_height() > 0).then(|| style.resolved_min_height())
                    });
                    renderer.enter_positioning_context(start_y, context_height);
                })
                .is_some();
            let border = block_box.and(style.border);
            if let (Some((box_x, box_width, _)), Some(border)) = (block_box, border) {
                renderer.push_block_border_top(box_x, box_width, border, Some(node_id));
                renderer.enter_inset(border.width);
            }
            let side_start_y = renderer.current_row();
            let padding = block_box
                .map(|_| style.padding)
                .unwrap_or_else(BoxSpacing::default);
            if padding.top > 0 {
                renderer.push_vertical_space(padding.top);
            }
            if !padding.is_empty() {
                renderer.enter_insets(padding.left, padding.right);
            }
            let underlay_insert = block_box.map(|_| renderer.underlay_insert_position());
            let background_start = block_box.and_then(|(box_x, box_width, start_y)| {
                style.background_shade.and_then(|shade| {
                    underlay_insert.map(|insert| (box_x, box_width, start_y, shade, insert))
                })
            });
            let background_image_start = block_box.and_then(|(box_x, box_width, start_y)| {
                style.background_image_url.clone().map(|url| {
                    let insert = underlay_insert
                        .map(|insert| insert.offset(usize::from(style.background_shade.is_some())))
                        .unwrap_or_else(|| renderer.underlay_insert_position());
                    (
                        box_x,
                        box_width,
                        start_y,
                        url,
                        style.background_image_size,
                        style.background_image_position,
                        style.background_image_repeat,
                        insert,
                    )
                })
            });
            let style_height = style.resolved_height();
            let style_max_height = style.resolved_max_height();
            let overflow_clip_height = match (style_height, style_max_height) {
                (Some(height), Some(max_height)) => Some(height.min(max_height)),
                (Some(height), None) => Some(height),
                (None, Some(max_height)) => Some(max_height),
                (None, None) => None,
            };
            let overflow_clip_entered = if block_flow && style.clips_overflow() {
                let clips_x = style.overflow_x.clips();
                let clips_y = style.overflow_y.clips();
                let clip_x = if clips_x {
                    renderer.box_x().saturating_sub(padding.left)
                } else {
                    0
                };
                let clip_y = if clips_y { side_start_y } else { 0 };
                let clip_width = if clips_x {
                    renderer
                        .available_width()
                        .saturating_add(padding.left)
                        .saturating_add(padding.right)
                } else {
                    usize::MAX
                };
                let clip_height = clips_y
                    .then(|| {
                        overflow_clip_height.map(|height| {
                            height
                                .saturating_add(padding.top)
                                .saturating_add(padding.bottom)
                        })
                    })
                    .flatten();
                renderer.enter_clip(DisplayCommandBounds {
                    x: clip_x,
                    y: clip_y,
                    width: clip_width,
                    height: clip_height.unwrap_or_else(|| usize::MAX.saturating_sub(clip_y)),
                });
                Some((clip_y, style_height.is_some(), clip_height))
            } else {
                None
            };
            let row_item_overflow_clip_entered = if is_row_item && style.clips_overflow() {
                match overflow_clip_height {
                    Some(clip_height) => {
                        let remaining_width = renderer
                            .available_width()
                            .saturating_sub(renderer.current_inline_width())
                            .max(1);
                        let clip_width = style.positioned_outer_width(remaining_width);
                        let clip_x = renderer
                            .box_x()
                            .saturating_add(renderer.current_inline_width());
                        let clip_y = renderer.current_row();
                        let clip_height = clip_height
                            .saturating_add(style.padding.top)
                            .saturating_add(style.padding.bottom)
                            .max(1);
                        let clip_bounds = DisplayCommandBounds {
                            x: clip_x,
                            y: clip_y,
                            width: clip_width.max(1),
                            height: clip_height,
                        };
                        renderer.enter_clip(clip_bounds);
                        if let Some(shade) = style.background_shade
                            && renderer.paint_visible()
                            && let Some(command) = renderer.clipped_rect_command(
                                clip_bounds.x,
                                clip_bounds.y,
                                clip_bounds.width,
                                clip_bounds.height,
                                shade,
                            )
                        {
                            let target = renderer.node_hit_target(Some(node_id));
                            renderer.push_underlay_command(command, target);
                        }
                        Some(clip_bounds)
                    }
                    None => None,
                }
            } else {
                None
            };
            let table_cell_flow = is_table_layout_cell_for_flow(element, &style);
            let text_background_entered =
                if block_flow || table_cell_flow || row_item_overflow_clip_entered.is_some() {
                    None
                } else {
                    style.background_shade
                };
            if let Some(background_shade) = text_background_entered {
                renderer.enter_text_background_shade(background_shade);
            }
            if let Some(text_align) = text_align_entered {
                renderer.enter_text_align(text_align);
            }
            if let Some(white_space) = white_space_entered {
                renderer.enter_white_space(white_space);
            }
            if let Some(text_transform) = text_transform_entered {
                renderer.enter_text_transform(text_transform);
            }
            if let Some(letter_spacing) = letter_spacing_entered {
                renderer.enter_letter_spacing(letter_spacing);
            }
            if let Some(word_spacing) = word_spacing_entered {
                renderer.enter_word_spacing(word_spacing);
            }
            if let Some(overflow_wrap) = overflow_wrap_entered {
                renderer.enter_overflow_wrap(overflow_wrap);
            }
            if let Some(word_break) = word_break_entered {
                renderer.enter_word_break(word_break);
            }
            if let Some(text_indent) = text_indent_entered {
                renderer.enter_text_indent(text_indent);
            }
            if let Some(line_height) = line_height_entered {
                renderer.enter_line_height(line_height);
            }
            if let Some(font_scale) = font_scale_entered {
                renderer.enter_font_scale(font_scale);
            }
            let text_indent_block_entered = if block_flow {
                renderer.enter_block_text_indent();
                true
            } else {
                false
            };
            let table_entered = if is_table_layout_container(element, &style) {
                renderer.enter_table(
                    table_column_widths(dom, node_id, css_cascade),
                    style.column_gap.unwrap_or(TABLE_COLUMN_GAP_CELLS),
                    style.row_gap.unwrap_or(0),
                    table_rows(dom, node_id, css_cascade).len(),
                );
                true
            } else {
                false
            };
            let table_row_entered = if is_table_layout_row(element, &style) {
                renderer.enter_table_row(table_row_cell_count(dom, node_id, css_cascade));
                true
            } else {
                false
            };
            let table_cell_entered = if table_cell_flow {
                renderer.enter_table_cell(
                    table_cell_colspan(dom, node_id),
                    table_cell_rowspan(dom, node_id),
                    style.background_shade,
                    Some(node_id),
                );
                true
            } else {
                false
            };
            let list_indent = if block_flow {
                nested_list_indent(dom, node_id)
            } else {
                0
            };
            if list_indent > 0 {
                renderer.enter_insets(list_indent, 0);
            }
            let list_marker_indent_entered = if style.display == Display::ListItem
                && let Some(marker) = list_item_marker(dom, node_id, css_cascade)
            {
                let marker_width = text_cell_width(&marker, renderer.font_scale);
                renderer.push_text(&marker, Some(node_id));
                if marker_width > 0 {
                    renderer.enter_line_start_indent(marker_width);
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if inline_margin.left > 0 {
                renderer.push_fixed_space_width(inline_margin.left, None);
            }

            if element.tag == "details" {
                let child_layout = style.child_layout_for_width(renderer.available_width());
                render_details_children(
                    dom,
                    node_id,
                    element,
                    source,
                    css_cascade,
                    renderer,
                    layout_box_count,
                    child_layout,
                );
            } else {
                let child_layout = style.child_layout_for_width(renderer.available_width());
                render_children(
                    dom,
                    node_id,
                    source,
                    css_cascade,
                    renderer,
                    layout_box_count,
                    child_layout,
                );
            }

            if let Some(clip_bounds) = row_item_overflow_clip_entered {
                renderer.cap_inline_replaced_height(clip_bounds.height);
                renderer.exit_clip();
            }
            if positioning_context_entered {
                renderer.exit_positioning_context();
            }
            if table_cell_entered {
                renderer.exit_table_cell();
            }
            if table_row_entered {
                renderer.exit_table_row();
            }
            if list_marker_indent_entered {
                renderer.exit_line_start_indent();
            }
            if list_indent > 0 {
                renderer.break_line();
                renderer.exit_insets(list_indent, 0);
            }
            if block_flow {
                renderer.break_line();
            }
            if let Some((clip_y, fixed_height, clip_height)) = overflow_clip_entered {
                if let Some(clip_height) = clip_height {
                    let clip_end = clip_y.saturating_add(clip_height);
                    if fixed_height {
                        renderer.set_current_row(clip_end);
                    } else {
                        renderer.cap_current_row(clip_end);
                    }
                }
                renderer.exit_clip();
            }
            if text_indent_block_entered {
                renderer.exit_block_text_indent();
            }
            if font_scale_entered.is_some() {
                renderer.exit_font_scale();
            }
            if line_height_entered.is_some() {
                renderer.exit_line_height();
            }
            if text_indent_entered.is_some() {
                renderer.exit_text_indent();
            }
            if word_break_entered.is_some() {
                renderer.exit_word_break();
            }
            if overflow_wrap_entered.is_some() {
                renderer.exit_overflow_wrap();
            }
            if word_spacing_entered.is_some() {
                renderer.exit_word_spacing();
            }
            if letter_spacing_entered.is_some() {
                renderer.exit_letter_spacing();
            }
            if text_transform_entered.is_some() {
                renderer.exit_text_transform();
            }
            if white_space_entered.is_some() {
                renderer.exit_white_space();
            }
            if text_align_entered.is_some() {
                renderer.exit_text_align();
            }
            if text_background_entered.is_some() {
                renderer.exit_text_background_shade();
            }
            if inline_margin.right > 0 {
                renderer.push_fixed_space_width(inline_margin.right, None);
            }
            if !padding.is_empty() {
                renderer.exit_insets(padding.left, padding.right);
            }
            if padding.bottom > 0 {
                renderer.push_vertical_space(padding.bottom);
            }
            if let Some((_, _, start_y)) = block_box {
                let block_height = style
                    .resolved_height()
                    .unwrap_or(0)
                    .max(style.resolved_min_height());
                renderer.ensure_current_row_at_least(start_y.saturating_add(block_height));
            }
            if let (Some((box_x, box_width, _)), Some(border)) = (block_box, border) {
                let content_height = renderer.current_row().saturating_sub(side_start_y);
                renderer.exit_inset(border.width);
                renderer.push_block_border_sides(
                    box_x,
                    box_width,
                    side_start_y,
                    content_height,
                    border,
                    Some(node_id),
                );
                renderer.push_block_border_bottom(box_x, box_width, border, Some(node_id));
            }
            if let Some((box_x, box_width, start_y, shade, insert)) = background_start {
                renderer.insert_block_background(
                    insert,
                    box_x,
                    box_width,
                    start_y,
                    shade,
                    Some(node_id),
                );
            }
            if let Some((box_x, box_width, start_y, url, size, position, repeat, insert)) =
                background_image_start
            {
                renderer.insert_block_background_image(
                    insert,
                    box_x,
                    box_width,
                    start_y,
                    source,
                    &url,
                    size,
                    position,
                    repeat,
                    Some(node_id),
                );
            }
            if block_flow {
                if width_inset.0 > 0 || width_inset.1 > 0 {
                    renderer.exit_insets(width_inset.0, width_inset.1);
                }
                if margin.left > 0 || margin.right > 0 {
                    renderer.exit_insets(margin.left, margin.right);
                }
                if margin.bottom > 0 {
                    renderer.push_vertical_space(margin.bottom);
                }
            }
            if text_shade_entered.is_some() {
                renderer.exit_text_shade();
            }
            if table_entered {
                renderer.exit_table();
            }
            exit_outer_contexts(
                renderer,
                &mut out_of_flow_entered,
                &mut horizontal_projection_entered,
                &mut vertical_projection_entered,
            );
        }
    }
}

fn render_contents_node(
    dom: &Dom,
    node_id: usize,
    source: &str,
    css_cascade: &CssCascade,
    renderer: &mut FlowRenderer,
    layout_box_count: &mut usize,
    style: ComputedStyle,
    children_are_row_items: bool,
) {
    let visibility_entered = style.visibility;
    if let Some(visibility) = visibility_entered {
        renderer.enter_visibility(visibility);
    }
    let opacity_entered = style.suppresses_paint();
    if opacity_entered {
        renderer.enter_transparent_opacity();
    }
    let text_shade_entered = style.text_shade;
    if let Some(text_shade) = text_shade_entered {
        renderer.enter_text_shade(text_shade);
    }
    let text_align_entered = style.text_align;
    if let Some(text_align) = text_align_entered {
        renderer.enter_text_align(text_align);
    }
    let white_space_entered = style.white_space;
    if let Some(white_space) = white_space_entered {
        renderer.enter_white_space(white_space);
    }
    let text_transform_entered = style.text_transform;
    if let Some(text_transform) = text_transform_entered {
        renderer.enter_text_transform(text_transform);
    }
    let letter_spacing_entered = style.letter_spacing;
    if let Some(letter_spacing) = letter_spacing_entered {
        renderer.enter_letter_spacing(letter_spacing);
    }
    let word_spacing_entered = style.word_spacing;
    if let Some(word_spacing) = word_spacing_entered {
        renderer.enter_word_spacing(word_spacing);
    }
    let overflow_wrap_entered = style.overflow_wrap;
    if let Some(overflow_wrap) = overflow_wrap_entered {
        renderer.enter_overflow_wrap(overflow_wrap);
    }
    let word_break_entered = style.word_break;
    if let Some(word_break) = word_break_entered {
        renderer.enter_word_break(word_break);
    }
    let text_indent_entered = style.text_indent;
    if let Some(text_indent) = text_indent_entered {
        renderer.enter_text_indent(text_indent);
    }
    let line_height_entered = style.line_height;
    if let Some(line_height) = line_height_entered {
        renderer.enter_line_height(line_height);
    }
    let font_scale_entered = style.font_scale;
    if let Some(font_scale) = font_scale_entered {
        renderer.enter_font_scale(font_scale);
    }

    render_children(
        dom,
        node_id,
        source,
        css_cascade,
        renderer,
        layout_box_count,
        ChildLayout {
            row_items: children_are_row_items,
            ..ChildLayout::default()
        },
    );

    if font_scale_entered.is_some() {
        renderer.exit_font_scale();
    }
    if line_height_entered.is_some() {
        renderer.exit_line_height();
    }
    if text_indent_entered.is_some() {
        renderer.exit_text_indent();
    }
    if word_break_entered.is_some() {
        renderer.exit_word_break();
    }
    if overflow_wrap_entered.is_some() {
        renderer.exit_overflow_wrap();
    }
    if word_spacing_entered.is_some() {
        renderer.exit_word_spacing();
    }
    if letter_spacing_entered.is_some() {
        renderer.exit_letter_spacing();
    }
    if text_transform_entered.is_some() {
        renderer.exit_text_transform();
    }
    if white_space_entered.is_some() {
        renderer.exit_white_space();
    }
    if text_align_entered.is_some() {
        renderer.exit_text_align();
    }
    if text_shade_entered.is_some() {
        renderer.exit_text_shade();
    }
    if opacity_entered {
        renderer.exit_transparent_opacity();
    }
    if visibility_entered.is_some() {
        renderer.exit_visibility();
    }
}

fn render_details_children(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    source: &str,
    css_cascade: &CssCascade,
    renderer: &mut FlowRenderer,
    layout_box_count: &mut usize,
    child_layout: ChildLayout,
) {
    let is_open = element.attrs.contains_key("open");
    let summary_child = first_details_summary_child(dom, node_id, css_cascade);
    if !is_open {
        if let Some(summary_child) = summary_child {
            render_node(
                dom,
                summary_child,
                source,
                css_cascade,
                renderer,
                layout_box_count,
                child_layout.row_items,
            );
        } else {
            renderer.push_text("> Details", Some(node_id));
        }
        return;
    }

    if summary_child.is_none() {
        renderer.push_text("v Details", Some(node_id));
        renderer.break_line();
    }
    render_children(
        dom,
        node_id,
        source,
        css_cascade,
        renderer,
        layout_box_count,
        child_layout,
    );
}

fn first_details_summary_child(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
) -> Option<usize> {
    let node = dom.nodes.get(node_id)?;
    node.children.iter().copied().find(|&child_id| {
        let Some(NodeKind::Element(element)) = dom.nodes.get(child_id).map(|node| &node.kind)
        else {
            return false;
        };
        element.tag == "summary"
            && computed_style(dom, child_id, element, css_cascade).display != Display::None
    })
}

fn element_target_for_node(dom: &Dom, node_id: usize) -> Option<usize> {
    let mut current = Some(node_id);
    while let Some(current_id) = current {
        let node = dom.nodes.get(current_id)?;
        if matches!(node.kind, NodeKind::Element(_)) {
            return Some(current_id);
        }
        current = node.parent;
    }
    None
}

fn image_placeholder_extent(
    element: &ElementData,
    style: &ComputedStyle,
    renderer: &FlowRenderer,
    intrinsic_size: Option<(usize, usize)>,
) -> (usize, usize) {
    let width_basis = renderer.available_width().max(1);
    let height_basis = default_vertical_dimension_basis();
    let attr_width = element
        .attrs
        .get("width")
        .and_then(|value| parse_css_dimension(value, CssAxis::Horizontal))
        .map(|width| width.resolve(width_basis));
    let attr_height = element
        .attrs
        .get("height")
        .and_then(|value| parse_css_dimension(value, CssAxis::Vertical))
        .map(|height| height.resolve(height_basis));
    let (decoded_width, decoded_height) = intrinsic_size.unzip();
    let style_ratio = style
        .aspect_ratio
        .map(|ratio| (Some(ratio.width), Some(ratio.height)));
    let (ratio_width, ratio_height) =
        style_ratio.unwrap_or_else(|| match (attr_width, attr_height) {
            (Some(width), Some(height)) => (Some(width), Some(height)),
            _ => (decoded_width, decoded_height),
        });
    let style_width = style.resolved_width(width_basis);
    let style_height = style.resolved_height();
    let (mut width, mut height) = match (style_width, style_height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => {
            let width = constrain_image_width(width, style, width_basis);
            let height = style
                .aspect_ratio
                .and_then(|ratio| ratio.height_for_width(width))
                .or_else(|| {
                    ratio_width
                        .zip(ratio_height)
                        .and_then(|(intrinsic_width, intrinsic_height)| {
                            scale_image_dimension(width, intrinsic_height, intrinsic_width)
                        })
                })
                .or(attr_height)
                .or(decoded_height)
                .unwrap_or(4);
            (width, height)
        }
        (None, Some(height)) => {
            let height = constrain_image_height(height, style);
            let width = style
                .aspect_ratio
                .and_then(|ratio| ratio.width_for_height(height))
                .or_else(|| {
                    ratio_width
                        .zip(ratio_height)
                        .and_then(|(intrinsic_width, intrinsic_height)| {
                            scale_image_dimension(height, intrinsic_width, intrinsic_height)
                        })
                })
                .or(attr_width)
                .or(decoded_width)
                .unwrap_or(10);
            (width, height)
        }
        (None, None) => match (attr_width, attr_height) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => {
                let height = style
                    .aspect_ratio
                    .and_then(|ratio| ratio.height_for_width(width))
                    .or_else(|| {
                        decoded_width.zip(decoded_height).and_then(
                            |(intrinsic_width, intrinsic_height)| {
                                scale_image_dimension(width, intrinsic_height, intrinsic_width)
                            },
                        )
                    })
                    .or(decoded_height)
                    .unwrap_or(4);
                (width, height)
            }
            (None, Some(height)) => {
                let width = style
                    .aspect_ratio
                    .and_then(|ratio| ratio.width_for_height(height))
                    .or_else(|| {
                        decoded_width.zip(decoded_height).and_then(
                            |(intrinsic_width, intrinsic_height)| {
                                scale_image_dimension(height, intrinsic_width, intrinsic_height)
                            },
                        )
                    })
                    .or(decoded_width)
                    .unwrap_or(10);
                (width, height)
            }
            (None, None) => (decoded_width.unwrap_or(10), decoded_height.unwrap_or(4)),
        },
    };
    width = constrain_image_width(width, style, width_basis);
    height = constrain_image_height(height, style);
    (width.clamp(1, width_basis), height)
}

fn constrain_image_width(width: usize, style: &ComputedStyle, basis: usize) -> usize {
    let width = if let Some(max_width) = style.resolved_max_width(basis) {
        width.min(max_width)
    } else {
        width
    };
    width.max(style.resolved_min_width(basis))
}

fn constrain_image_height(height: usize, style: &ComputedStyle) -> usize {
    let height = if let Some(max_height) = style.resolved_max_height() {
        height.min(max_height)
    } else {
        height
    };
    height.max(style.resolved_min_height()).clamp(1, 24)
}

fn scale_image_dimension(
    source_dimension: usize,
    target_intrinsic: usize,
    source_intrinsic: usize,
) -> Option<usize> {
    if source_intrinsic == 0 {
        return None;
    }
    let scaled = source_dimension
        .saturating_mul(target_intrinsic)
        .saturating_add(source_intrinsic.saturating_sub(1))
        / source_intrinsic;
    Some(scaled.max(1))
}

fn is_replaced_media_element(tag: &str) -> bool {
    matches!(tag, "audio" | "embed" | "iframe" | "object" | "video")
}

fn replaced_media_placeholder_extent(
    element: &ElementData,
    style: &ComputedStyle,
    renderer: &FlowRenderer,
) -> (usize, usize) {
    let (width, height) = image_placeholder_extent(element, style, renderer, None);
    let has_explicit_height = style.height.is_some() || element.attrs.contains_key("height");
    if element.tag == "audio" && !has_explicit_height {
        return (width, 1usize.max(style.resolved_min_height()).clamp(1, 24));
    }
    (width, height)
}

fn replaced_media_render_source(element: &ElementData) -> Option<&str> {
    match element.tag.as_str() {
        "object" => element.data.as_deref(),
        "video" => element.poster.as_deref().or(element.src.as_deref()),
        _ => element.src.as_deref(),
    }
}

fn replaced_media_alt(element: &ElementData) -> Option<String> {
    element
        .attrs
        .get("title")
        .or(element.alt.as_ref())
        .cloned()
        .or_else(|| Some(element.tag.clone()))
}

fn svg_paint_shade(dom: &Dom, node_id: usize, element: &ElementData) -> Option<u8> {
    element
        .attrs
        .get("fill")
        .and_then(|fill| parse_css_color_shade(fill))
        .or_else(|| element.style.as_deref().and_then(svg_style_paint_shade))
        .or_else(|| svg_child_paint_shade(dom, node_id))
}

fn svg_paint_rgb(dom: &Dom, node_id: usize, element: &ElementData) -> Option<(u8, u8, u8)> {
    let current_color = svg_element_current_color_rgb(element);
    svg_element_paint_rgb_with_current(element, current_color)
        .or_else(|| svg_child_paint_rgb(dom, node_id, None, current_color))
}

fn svg_child_paint_shade(dom: &Dom, node_id: usize) -> Option<u8> {
    let node = dom.nodes.get(node_id)?;
    for &child_id in &node.children {
        let Some(child) = dom.nodes.get(child_id) else {
            continue;
        };
        if let NodeKind::Element(element) = &child.kind {
            if let Some(shade) = element
                .attrs
                .get("fill")
                .and_then(|fill| parse_css_color_shade(fill))
                .or_else(|| element.style.as_deref().and_then(svg_style_paint_shade))
                .or_else(|| svg_child_paint_shade(dom, child_id))
            {
                return Some(shade);
            }
        }
    }
    None
}

fn svg_child_paint_rgb(
    dom: &Dom,
    node_id: usize,
    inherited: Option<(u8, u8, u8)>,
    current_color: Option<(u8, u8, u8)>,
) -> Option<(u8, u8, u8)> {
    let node = dom.nodes.get(node_id)?;
    for &child_id in &node.children {
        let Some(child) = dom.nodes.get(child_id) else {
            continue;
        };
        if let NodeKind::Element(element) = &child.kind {
            let current_color = svg_element_current_color_rgb(element).or(current_color);
            let fill = svg_element_paint_rgb_with_current(element, current_color).or(inherited);
            if fill.is_some() {
                return fill;
            }
            if let Some(fill) = svg_child_paint_rgb(dom, child_id, inherited, current_color) {
                return Some(fill);
            }
        }
    }
    None
}

fn svg_style_paint_shade(style: &str) -> Option<u8> {
    for declaration in strip_css_comments(style).split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        if matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "fill" | "color" | "background" | "background-color"
        ) && let Some(shade) = parse_css_color_shade(css_declaration_value(value))
        {
            return Some(shade);
        }
    }
    None
}

fn svg_style_paint_rgb_with_current(
    style: &str,
    current_color: Option<(u8, u8, u8)>,
) -> Option<(u8, u8, u8)> {
    for declaration in strip_css_comments(style).split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        if matches!(name.trim().to_ascii_lowercase().as_str(), "fill") {
            let value = css_declaration_value(value);
            if value.eq_ignore_ascii_case("currentColor") {
                return current_color.or(Some((0, 0, 0)));
            }
            if let Some(rgb) = parse_css_color_rgb_value(value) {
                return Some(rgb);
            }
        }
    }
    None
}

fn svg_element_paint_rgb_with_current(
    element: &ElementData,
    current_color: Option<(u8, u8, u8)>,
) -> Option<(u8, u8, u8)> {
    element
        .attrs
        .get("fill")
        .and_then(|fill| {
            if fill.trim().eq_ignore_ascii_case("currentColor") {
                current_color.or(Some((0, 0, 0)))
            } else {
                parse_css_color_rgb_value(fill)
            }
        })
        .or_else(|| {
            element
                .style
                .as_deref()
                .and_then(|style| svg_style_paint_rgb_with_current(style, current_color))
        })
}

fn svg_element_current_color_rgb(element: &ElementData) -> Option<(u8, u8, u8)> {
    element
        .attrs
        .get("color")
        .and_then(|color| parse_css_color_rgb_value(color))
        .or_else(|| element.style.as_deref().and_then(svg_style_color_rgb))
}

fn svg_style_color_rgb(style: &str) -> Option<(u8, u8, u8)> {
    for declaration in strip_css_comments(style).split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        if matches!(name.trim().to_ascii_lowercase().as_str(), "color")
            && let Some(rgb) = parse_css_color_rgb_value(css_declaration_value(value))
        {
            return Some(rgb);
        }
    }
    None
}

fn svg_element_fill_suppressed(element: &ElementData) -> bool {
    if element
        .attrs
        .get("fill")
        .is_some_and(|fill| svg_paint_value_suppressed(fill))
    {
        return true;
    }
    element
        .style
        .as_deref()
        .is_some_and(svg_style_fill_suppressed)
}

fn svg_style_fill_suppressed(style: &str) -> bool {
    for declaration in strip_css_comments(style).split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        if matches!(name.trim().to_ascii_lowercase().as_str(), "fill") {
            return svg_paint_value_suppressed(css_declaration_value(value));
        }
    }
    false
}

fn svg_paint_value_suppressed(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "none" | "transparent"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SvgPaintShape {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    red: u8,
    green: u8,
    blue: u8,
}

fn svg_paint_shapes(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    svg_width: usize,
    svg_height: usize,
) -> Vec<SvgPaintShape> {
    let mut shapes = Vec::new();
    let current_color = svg_element_current_color_rgb(element);
    let inherited = svg_element_paint_rgb_with_current(element, current_color);
    collect_svg_paint_shapes(
        dom,
        node_id,
        inherited,
        current_color,
        svg_width,
        svg_height,
        &mut shapes,
    );
    shapes
}

fn collect_svg_paint_shapes(
    dom: &Dom,
    node_id: usize,
    inherited: Option<(u8, u8, u8)>,
    current_color: Option<(u8, u8, u8)>,
    svg_width: usize,
    svg_height: usize,
    shapes: &mut Vec<SvgPaintShape>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    for &child_id in &node.children {
        let Some(child) = dom.nodes.get(child_id) else {
            continue;
        };
        let NodeKind::Element(element) = &child.kind else {
            continue;
        };
        let current_color = svg_element_current_color_rgb(element).or(current_color);
        let fill = if svg_element_fill_suppressed(element) {
            None
        } else {
            svg_element_paint_rgb_with_current(element, current_color).or(inherited)
        };
        if let Some((x, y, width, height)) = svg_shape_bounds(element, svg_width, svg_height) {
            let (red, green, blue) = fill.unwrap_or((0, 0, 0));
            shapes.push(SvgPaintShape {
                x,
                y,
                width,
                height,
                red,
                green,
                blue,
            });
        }
        collect_svg_paint_shapes(
            dom,
            child_id,
            fill,
            current_color,
            svg_width,
            svg_height,
            shapes,
        );
    }
}

fn svg_shape_bounds(
    element: &ElementData,
    svg_width: usize,
    svg_height: usize,
) -> Option<(usize, usize, usize, usize)> {
    match element.tag.as_str() {
        "rect" => {
            let x = svg_attr_pixels(element, "x").unwrap_or(0.0);
            let y = svg_attr_pixels(element, "y").unwrap_or(0.0);
            let width = svg_attr_pixels(element, "width")?;
            let height = svg_attr_pixels(element, "height")?;
            svg_pixels_to_bounds(x, y, width, height, svg_width, svg_height)
        }
        "circle" => {
            let cx = svg_attr_pixels(element, "cx").unwrap_or(0.0);
            let cy = svg_attr_pixels(element, "cy").unwrap_or(0.0);
            let r = svg_attr_pixels(element, "r")?;
            svg_pixels_to_bounds(cx - r, cy - r, r * 2.0, r * 2.0, svg_width, svg_height)
        }
        "ellipse" => {
            let cx = svg_attr_pixels(element, "cx").unwrap_or(0.0);
            let cy = svg_attr_pixels(element, "cy").unwrap_or(0.0);
            let rx = svg_attr_pixels(element, "rx")?;
            let ry = svg_attr_pixels(element, "ry")?;
            svg_pixels_to_bounds(cx - rx, cy - ry, rx * 2.0, ry * 2.0, svg_width, svg_height)
        }
        "path" | "polygon" | "polyline" => Some((0, 0, svg_width.max(1), svg_height.max(1))),
        _ => None,
    }
}

fn svg_attr_pixels(element: &ElementData, name: &str) -> Option<f32> {
    let pixels = element
        .attrs
        .get(name)
        .and_then(|value| parse_css_signed_length_pixels(value))?;
    pixels.is_finite().then_some(pixels)
}

fn svg_pixels_to_bounds(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    svg_width: usize,
    svg_height: usize,
) -> Option<(usize, usize, usize, usize)> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let start_x_px = x.max(0.0);
    let start_y_px = y.max(0.0);
    let end_x_px = (x + width).max(start_x_px);
    let end_y_px = (y + height).max(start_y_px);
    let start_x = (start_x_px / css_axis_cell_px(CssAxis::Horizontal)).floor() as usize;
    let start_y = (start_y_px / css_axis_cell_px(CssAxis::Vertical)).floor() as usize;
    let end_x = (end_x_px / css_axis_cell_px(CssAxis::Horizontal))
        .ceil()
        .max(start_x.saturating_add(1) as f32) as usize;
    let end_y = (end_y_px / css_axis_cell_px(CssAxis::Vertical))
        .ceil()
        .max(start_y.saturating_add(1) as f32) as usize;
    if start_x >= svg_width || start_y >= svg_height {
        return None;
    }
    let end_x = end_x.min(svg_width.max(1));
    let end_y = end_y.min(svg_height.max(1));
    (end_x > start_x && end_y > start_y).then_some((
        start_x,
        start_y,
        end_x - start_x,
        end_y - start_y,
    ))
}

fn form_control_render_text(dom: &Dom, node_id: usize, element: &ElementData) -> Option<String> {
    match element.tag.as_str() {
        "input" => input_render_text(element),
        "select" => Some(select_render_text(dom, node_id)),
        "textarea" => {
            Some(control_label_text(&element.value.clone().unwrap_or_else(
                || collapse_ascii_whitespace(&text_content(dom, node_id)),
            )))
        }
        "button" => Some(button_render_text(dom, node_id, element)),
        _ => None,
    }
}

fn button_render_text(dom: &Dom, node_id: usize, element: &ElementData) -> String {
    let label = element
        .value
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| collapse_ascii_whitespace(&text_content(dom, node_id)));
    if label.is_empty() {
        "Button".to_owned()
    } else {
        label
    }
}

fn input_render_text(element: &ElementData) -> Option<String> {
    let kind = element
        .input_type
        .as_deref()
        .unwrap_or("text")
        .to_ascii_lowercase();
    match kind.as_str() {
        "hidden" => Some(String::new()),
        "checkbox" => Some(if element.checked { "x" } else { " " }.to_owned()),
        "radio" => Some(if element.checked { "x" } else { " " }.to_owned()),
        "submit" => Some(control_label_text(
            element.value.as_deref().unwrap_or("Submit"),
        )),
        "reset" => Some(control_label_text(
            element.value.as_deref().unwrap_or("Reset"),
        )),
        "button" => Some(control_label_text(
            element.value.as_deref().unwrap_or("Button"),
        )),
        "image" => element
            .alt
            .as_deref()
            .or(element.value.as_deref())
            .map(control_label_text)
            .or_else(|| Some("image".to_owned())),
        "password" => Some(control_label_text(
            &"*".repeat(element.value.as_deref().unwrap_or_default().chars().count()),
        )),
        _ => Some(control_label_text(
            element
                .value
                .as_deref()
                .or_else(|| element.attrs.get("placeholder").map(String::as_str))
                .unwrap_or_default(),
        )),
    }
}

fn control_label_text(value: &str) -> String {
    let label = value.trim();
    if label.is_empty() {
        " ".to_owned()
    } else {
        label.to_owned()
    }
}

fn select_render_text(dom: &Dom, node_id: usize) -> String {
    let options = select_options(dom, node_id);
    let value = select_value(&options);
    let label = value
        .as_ref()
        .and_then(|value| options.iter().find(|option| &option.value == value))
        .map(|option| option.label.as_str())
        .or_else(|| options.first().map(|option| option.label.as_str()))
        .unwrap_or_default();
    control_label_text(label)
}

fn computed_style(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    css_cascade: &CssCascade,
) -> ComputedStyle {
    if element.hidden || default_display(&element.tag) == Display::None {
        return ComputedStyle {
            display: Display::None,
            float: None,
            clear: None,
            background_shade: None,
            background_image_url: None,
            background_image_size: BackgroundImageSize::default(),
            background_image_position: BackgroundImagePosition::default(),
            background_image_repeat: BackgroundImageRepeat::default(),
            text_shade: None,
            text_align: None,
            visibility: None,
            opacity: PaintOpacity::Opaque,
            animation_reveals_opacity: false,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            flex_direction: FlexDirection::Row,
            flex_wrap: false,
            flex_basis: None,
            justify_content: JustifyContent::Start,
            align_items: AlignItems::Start,
            grid_columns: None,
            grid_auto_min_column_width: None,
            position: Position::Static,
            position_top: None,
            position_bottom: None,
            position_left: None,
            position_right: None,
            transform_translate: CssTranslate::default(),
            z_index: 0,
            z_index_specified: false,
            white_space: None,
            text_transform: None,
            letter_spacing: None,
            word_spacing: None,
            overflow_wrap: None,
            word_break: None,
            text_indent: None,
            line_height: None,
            font_scale: None,
            row_gap: None,
            column_gap: None,
            box_sizing: BoxSizing::ContentBox,
            list_style_type: None,
            border: None,
            padding: BoxSpacing::default(),
            margin: BoxSpacing::default(),
            width: None,
            max_width: None,
            min_width: CssDimension::zero(),
            height: None,
            max_height: None,
            aspect_ratio: None,
            margin_left_auto: false,
            margin_right_auto: false,
            min_height: CssDimension::zero(),
        };
    }
    let mut display = default_display(&element.tag);
    let mut float = None;
    let mut clear = None;
    let mut background_shade = None;
    let mut background_image_url = None;
    let mut background_image_size = BackgroundImageSize::default();
    let mut background_image_position = BackgroundImagePosition::default();
    let mut background_image_repeat = BackgroundImageRepeat::default();
    let mut text_shade = default_text_shade(element);
    let mut text_align = None;
    let mut visibility = None;
    let mut opacity = PaintOpacity::Opaque;
    let mut animation_reveals_opacity = false;
    let mut overflow_x = Overflow::Visible;
    let mut overflow_y = Overflow::Visible;
    let mut flex_direction = FlexDirection::Row;
    let mut flex_wrap = false;
    let mut flex_basis = None;
    let mut justify_content = JustifyContent::Start;
    let mut align_items = AlignItems::Start;
    let mut grid_columns = None;
    let mut grid_auto_min_column_width = None;
    let mut position = Position::Static;
    let mut position_top = None;
    let mut position_bottom = None;
    let mut position_left = None;
    let mut position_right = None;
    let mut transform_translate = CssTranslate::default();
    let mut z_index = 0i32;
    let mut z_index_specified = false;
    let mut white_space = (element.tag == "pre").then_some(WhiteSpace::Pre);
    let mut text_transform = None;
    let mut letter_spacing = None;
    let mut word_spacing = None;
    let mut overflow_wrap = None;
    let mut word_break = None;
    let mut text_indent = None;
    let mut line_height = default_line_height(&element.tag);
    let mut font_scale = default_font_scale(&element.tag);
    let mut row_gap = None;
    let mut column_gap = None;
    let mut box_sizing = BoxSizing::ContentBox;
    let mut list_style_type = None;
    let mut border = None;
    let mut padding = BoxSpacing::default();
    let mut margin = default_margin(&element.tag);
    let mut width = None;
    let mut max_width = None;
    let mut min_width = CssDimension::zero();
    let mut height = None;
    let mut max_height = None;
    let mut aspect_ratio = None;
    let mut margin_left_auto = false;
    let mut margin_right_auto = false;
    let mut min_height = CssDimension::zero();
    let mut display_specificity = 0u32;
    let mut float_specificity = 0u32;
    let mut clear_specificity = 0u32;
    let mut background_specificity = 0u32;
    let mut background_image_specificity = 0u32;
    let mut background_image_size_specificity = 0u32;
    let mut background_image_position_specificity = 0u32;
    let mut background_image_repeat_specificity = 0u32;
    let mut text_specificity = 0u32;
    let mut text_align_specificity = 0u32;
    let mut visibility_specificity = 0u32;
    let mut opacity_specificity = 0u32;
    let mut animation_reveals_opacity_specificity = 0u32;
    let mut overflow_x_specificity = 0u32;
    let mut overflow_y_specificity = 0u32;
    let mut flex_direction_specificity = 0u32;
    let mut flex_wrap_specificity = 0u32;
    let mut flex_basis_specificity = 0u32;
    let mut justify_content_specificity = 0u32;
    let mut align_items_specificity = 0u32;
    let mut grid_columns_specificity = 0u32;
    let mut grid_auto_min_column_width_specificity = 0u32;
    let mut position_specificity = 0u32;
    let mut position_top_specificity = 0u32;
    let mut position_bottom_specificity = 0u32;
    let mut position_left_specificity = 0u32;
    let mut position_right_specificity = 0u32;
    let mut transform_translate_specificity = 0u32;
    let mut z_index_specificity = 0u32;
    let mut white_space_specificity = 0u32;
    let mut text_transform_specificity = 0u32;
    let mut letter_spacing_specificity = 0u32;
    let mut word_spacing_specificity = 0u32;
    let mut overflow_wrap_specificity = 0u32;
    let mut word_break_specificity = 0u32;
    let mut text_indent_specificity = 0u32;
    let mut line_height_specificity = 0u32;
    let mut font_scale_specificity = 0u32;
    let mut row_gap_specificity = 0u32;
    let mut column_gap_specificity = 0u32;
    let mut box_sizing_specificity = 0u32;
    let mut list_style_type_specificity = 0u32;
    let mut border_specificity = 0u32;
    let mut padding_specificity = 0u32;
    let mut margin_specificity = 0u32;
    let mut width_specificity = 0u32;
    let mut max_width_specificity = 0u32;
    let mut min_width_specificity = 0u32;
    let mut height_specificity = 0u32;
    let mut max_height_specificity = 0u32;
    let mut aspect_ratio_specificity = 0u32;
    let mut margin_left_auto_specificity = 0u32;
    let mut margin_right_auto_specificity = 0u32;
    let mut min_height_specificity = 0u32;
    for rule_index in css_cascade.candidate_rule_indices(element) {
        let Some(rule) = css_cascade.rules.get(rule_index) else {
            continue;
        };
        if selector_matches(&rule.selector, dom, node_id) {
            let rule_specificity = selector_specificity(&rule.selector);
            if let Some(rule_display) = rule.declarations.display
                && rule_specificity >= display_specificity
            {
                display = rule_display;
                display_specificity = rule_specificity;
            }
            if let Some(rule_float) = rule.declarations.float
                && rule_specificity >= float_specificity
            {
                float = rule_float;
                float_specificity = rule_specificity;
            }
            if let Some(rule_clear) = rule.declarations.clear
                && rule_specificity >= clear_specificity
            {
                clear = rule_clear;
                clear_specificity = rule_specificity;
            }
            if let Some(rule_background) = rule.declarations.background_shade
                && rule_specificity >= background_specificity
            {
                background_shade = Some(rule_background);
                background_specificity = rule_specificity;
            }
            if let Some(rule_background_image_url) = rule.declarations.background_image_url.as_ref()
                && rule_specificity >= background_image_specificity
            {
                background_image_url = Some(rule_background_image_url.clone());
                background_image_specificity = rule_specificity;
            }
            if let Some(rule_background_image_size) = rule.declarations.background_image_size
                && rule_specificity >= background_image_size_specificity
            {
                background_image_size = rule_background_image_size;
                background_image_size_specificity = rule_specificity;
            }
            if let Some(rule_background_image_position) =
                rule.declarations.background_image_position
                && rule_specificity >= background_image_position_specificity
            {
                background_image_position = rule_background_image_position;
                background_image_position_specificity = rule_specificity;
            }
            if let Some(rule_background_image_repeat) = rule.declarations.background_image_repeat
                && rule_specificity >= background_image_repeat_specificity
            {
                background_image_repeat = rule_background_image_repeat;
                background_image_repeat_specificity = rule_specificity;
            }
            if let Some(rule_text) = rule.declarations.text_shade
                && rule_specificity >= text_specificity
            {
                text_shade = Some(rule_text);
                text_specificity = rule_specificity;
            }
            if let Some(rule_text_align) = rule.declarations.text_align
                && rule_specificity >= text_align_specificity
            {
                text_align = Some(rule_text_align);
                text_align_specificity = rule_specificity;
            }
            if let Some(rule_visibility) = rule.declarations.visibility
                && rule_specificity >= visibility_specificity
            {
                visibility = Some(rule_visibility);
                visibility_specificity = rule_specificity;
            }
            if let Some(rule_opacity) = rule.declarations.opacity
                && rule_specificity >= opacity_specificity
            {
                opacity = rule_opacity;
                opacity_specificity = rule_specificity;
            }
            if let Some(rule_animation_reveals_opacity) =
                rule.declarations.animation_reveals_opacity
                && rule_specificity >= animation_reveals_opacity_specificity
            {
                animation_reveals_opacity = rule_animation_reveals_opacity;
                animation_reveals_opacity_specificity = rule_specificity;
            }
            if let Some(rule_overflow_x) = rule.declarations.overflow_x
                && rule_specificity >= overflow_x_specificity
            {
                overflow_x = rule_overflow_x;
                overflow_x_specificity = rule_specificity;
            }
            if let Some(rule_overflow_y) = rule.declarations.overflow_y
                && rule_specificity >= overflow_y_specificity
            {
                overflow_y = rule_overflow_y;
                overflow_y_specificity = rule_specificity;
            }
            if let Some(rule_flex_direction) = rule.declarations.flex_direction
                && rule_specificity >= flex_direction_specificity
            {
                flex_direction = rule_flex_direction;
                flex_direction_specificity = rule_specificity;
            }
            if let Some(rule_flex_wrap) = rule.declarations.flex_wrap
                && rule_specificity >= flex_wrap_specificity
            {
                flex_wrap = rule_flex_wrap;
                flex_wrap_specificity = rule_specificity;
            }
            if let Some(rule_flex_basis) = rule.declarations.flex_basis
                && rule_specificity >= flex_basis_specificity
            {
                flex_basis = Some(rule_flex_basis);
                flex_basis_specificity = rule_specificity;
            }
            if let Some(rule_justify_content) = rule.declarations.justify_content
                && rule_specificity >= justify_content_specificity
            {
                justify_content = rule_justify_content;
                justify_content_specificity = rule_specificity;
            }
            if let Some(rule_align_items) = rule.declarations.align_items
                && rule_specificity >= align_items_specificity
            {
                align_items = rule_align_items;
                align_items_specificity = rule_specificity;
            }
            if let Some(rule_grid_columns) = rule.declarations.grid_columns
                && rule_specificity >= grid_columns_specificity
            {
                grid_columns = Some(rule_grid_columns);
                grid_columns_specificity = rule_specificity;
            }
            if let Some(rule_grid_auto_min_column_width) =
                rule.declarations.grid_auto_min_column_width
                && rule_specificity >= grid_auto_min_column_width_specificity
            {
                grid_auto_min_column_width = Some(rule_grid_auto_min_column_width);
                grid_auto_min_column_width_specificity = rule_specificity;
            }
            if let Some(rule_position) = rule.declarations.position
                && rule_specificity >= position_specificity
            {
                position = rule_position;
                position_specificity = rule_specificity;
            }
            if let Some(rule_position_top) = rule.declarations.position_top
                && rule_specificity >= position_top_specificity
            {
                position_top = Some(rule_position_top);
                position_top_specificity = rule_specificity;
            }
            if let Some(rule_position_bottom) = rule.declarations.position_bottom
                && rule_specificity >= position_bottom_specificity
            {
                position_bottom = Some(rule_position_bottom);
                position_bottom_specificity = rule_specificity;
            }
            if let Some(rule_position_left) = rule.declarations.position_left
                && rule_specificity >= position_left_specificity
            {
                position_left = Some(rule_position_left);
                position_left_specificity = rule_specificity;
            }
            if let Some(rule_position_right) = rule.declarations.position_right
                && rule_specificity >= position_right_specificity
            {
                position_right = Some(rule_position_right);
                position_right_specificity = rule_specificity;
            }
            if let Some(rule_transform_translate) = rule.declarations.transform_translate
                && rule_specificity >= transform_translate_specificity
            {
                transform_translate = rule_transform_translate;
                transform_translate_specificity = rule_specificity;
            }
            if let Some(rule_z_index) = rule.declarations.z_index
                && rule_specificity >= z_index_specificity
            {
                z_index = rule_z_index;
                z_index_specified = true;
                z_index_specificity = rule_specificity;
            }
            if let Some(rule_white_space) = rule.declarations.white_space
                && rule_specificity >= white_space_specificity
            {
                white_space = Some(rule_white_space);
                white_space_specificity = rule_specificity;
            }
            if let Some(rule_text_transform) = rule.declarations.text_transform
                && rule_specificity >= text_transform_specificity
            {
                text_transform = Some(rule_text_transform);
                text_transform_specificity = rule_specificity;
            }
            if let Some(rule_letter_spacing) = rule.declarations.letter_spacing
                && rule_specificity >= letter_spacing_specificity
            {
                letter_spacing = Some(rule_letter_spacing);
                letter_spacing_specificity = rule_specificity;
            }
            if let Some(rule_word_spacing) = rule.declarations.word_spacing
                && rule_specificity >= word_spacing_specificity
            {
                word_spacing = Some(rule_word_spacing);
                word_spacing_specificity = rule_specificity;
            }
            if let Some(rule_overflow_wrap) = rule.declarations.overflow_wrap
                && rule_specificity >= overflow_wrap_specificity
            {
                overflow_wrap = Some(rule_overflow_wrap);
                overflow_wrap_specificity = rule_specificity;
            }
            if let Some(rule_word_break) = rule.declarations.word_break
                && rule_specificity >= word_break_specificity
            {
                word_break = Some(rule_word_break);
                word_break_specificity = rule_specificity;
            }
            if let Some(rule_text_indent) = rule.declarations.text_indent
                && rule_specificity >= text_indent_specificity
            {
                text_indent = Some(rule_text_indent);
                text_indent_specificity = rule_specificity;
            }
            if let Some(rule_line_height) = rule.declarations.line_height
                && rule_specificity >= line_height_specificity
            {
                line_height = Some(rule_line_height);
                line_height_specificity = rule_specificity;
            }
            if let Some(rule_font_scale) = rule.declarations.font_scale
                && rule_specificity >= font_scale_specificity
            {
                font_scale = Some(rule_font_scale);
                font_scale_specificity = rule_specificity;
            }
            if let Some(rule_row_gap) = rule.declarations.row_gap
                && rule_specificity >= row_gap_specificity
            {
                row_gap = Some(rule_row_gap);
                row_gap_specificity = rule_specificity;
            }
            if let Some(rule_column_gap) = rule.declarations.column_gap
                && rule_specificity >= column_gap_specificity
            {
                column_gap = Some(rule_column_gap);
                column_gap_specificity = rule_specificity;
            }
            if let Some(rule_box_sizing) = rule.declarations.box_sizing
                && rule_specificity >= box_sizing_specificity
            {
                box_sizing = rule_box_sizing;
                box_sizing_specificity = rule_specificity;
            }
            if let Some(rule_list_style_type) = rule.declarations.list_style_type
                && rule_specificity >= list_style_type_specificity
            {
                list_style_type = Some(rule_list_style_type);
                list_style_type_specificity = rule_specificity;
            }
            if let Some(rule_border) = rule.declarations.border
                && rule_specificity >= border_specificity
            {
                border = Some(rule_border);
                border_specificity = rule_specificity;
            }
            if let Some(rule_padding) = rule.declarations.padding
                && rule_specificity >= padding_specificity
            {
                padding = rule_padding;
                padding_specificity = rule_specificity;
            }
            if let Some(rule_margin) = rule.declarations.margin
                && rule_specificity >= margin_specificity
            {
                margin = rule_margin;
                margin_specificity = rule_specificity;
            }
            if let Some(rule_width) = rule.declarations.width
                && rule_specificity >= width_specificity
            {
                width = Some(rule_width);
                width_specificity = rule_specificity;
            }
            if let Some(rule_max_width) = rule.declarations.max_width
                && rule_specificity >= max_width_specificity
            {
                max_width = Some(rule_max_width);
                max_width_specificity = rule_specificity;
            }
            if let Some(rule_min_width) = rule.declarations.min_width
                && rule_specificity >= min_width_specificity
            {
                min_width = rule_min_width;
                min_width_specificity = rule_specificity;
            }
            if let Some(rule_height) = rule.declarations.height
                && rule_specificity >= height_specificity
            {
                height = Some(rule_height);
                height_specificity = rule_specificity;
            }
            if let Some(rule_max_height) = rule.declarations.max_height
                && rule_specificity >= max_height_specificity
            {
                max_height = Some(rule_max_height);
                max_height_specificity = rule_specificity;
            }
            if let Some(rule_aspect_ratio) = rule.declarations.aspect_ratio
                && rule_specificity >= aspect_ratio_specificity
            {
                aspect_ratio = Some(rule_aspect_ratio);
                aspect_ratio_specificity = rule_specificity;
            }
            if let Some(rule_margin_left_auto) = rule.declarations.margin_left_auto
                && rule_specificity >= margin_left_auto_specificity
            {
                margin_left_auto = rule_margin_left_auto;
                margin_left_auto_specificity = rule_specificity;
            }
            if let Some(rule_margin_right_auto) = rule.declarations.margin_right_auto
                && rule_specificity >= margin_right_auto_specificity
            {
                margin_right_auto = rule_margin_right_auto;
                margin_right_auto_specificity = rule_specificity;
            }
            if let Some(rule_min_height) = rule.declarations.min_height
                && rule_specificity >= min_height_specificity
            {
                min_height = rule_min_height;
                min_height_specificity = rule_specificity;
            }
        }
    }
    if let Some(inline_style) = element.style.as_deref() {
        let resolved_inline_style =
            substitute_css_vars(inline_style, &css_cascade.custom_properties);
        let inline = parse_css_declarations(&resolved_inline_style);
        if let Some(inline_display) = inline.display {
            display = inline_display;
        }
        if let Some(inline_float) = inline.float {
            float = inline_float;
        }
        if let Some(inline_clear) = inline.clear {
            clear = inline_clear;
        }
        if let Some(inline_background) = inline.background_shade {
            background_shade = Some(inline_background);
        }
        if let Some(inline_background_image_url) = inline.background_image_url {
            background_image_url = Some(inline_background_image_url);
        }
        if let Some(inline_background_image_size) = inline.background_image_size {
            background_image_size = inline_background_image_size;
        }
        if let Some(inline_background_image_position) = inline.background_image_position {
            background_image_position = inline_background_image_position;
        }
        if let Some(inline_background_image_repeat) = inline.background_image_repeat {
            background_image_repeat = inline_background_image_repeat;
        }
        if let Some(inline_text) = inline.text_shade {
            text_shade = Some(inline_text);
        }
        if let Some(inline_text_align) = inline.text_align {
            text_align = Some(inline_text_align);
        }
        if let Some(inline_visibility) = inline.visibility {
            visibility = Some(inline_visibility);
        }
        if let Some(inline_opacity) = inline.opacity {
            opacity = inline_opacity;
        }
        if let Some(inline_animation_reveals_opacity) = inline.animation_reveals_opacity {
            animation_reveals_opacity = inline_animation_reveals_opacity;
        }
        if let Some(inline_overflow_x) = inline.overflow_x {
            overflow_x = inline_overflow_x;
        }
        if let Some(inline_overflow_y) = inline.overflow_y {
            overflow_y = inline_overflow_y;
        }
        if let Some(inline_flex_direction) = inline.flex_direction {
            flex_direction = inline_flex_direction;
        }
        if let Some(inline_flex_wrap) = inline.flex_wrap {
            flex_wrap = inline_flex_wrap;
        }
        if let Some(inline_flex_basis) = inline.flex_basis {
            flex_basis = Some(inline_flex_basis);
        }
        if let Some(inline_justify_content) = inline.justify_content {
            justify_content = inline_justify_content;
        }
        if let Some(inline_align_items) = inline.align_items {
            align_items = inline_align_items;
        }
        if let Some(inline_grid_columns) = inline.grid_columns {
            grid_columns = Some(inline_grid_columns);
        }
        if let Some(inline_grid_auto_min_column_width) = inline.grid_auto_min_column_width {
            grid_auto_min_column_width = Some(inline_grid_auto_min_column_width);
        }
        if let Some(inline_position) = inline.position {
            position = inline_position;
        }
        if let Some(inline_position_top) = inline.position_top {
            position_top = Some(inline_position_top);
        }
        if let Some(inline_position_bottom) = inline.position_bottom {
            position_bottom = Some(inline_position_bottom);
        }
        if let Some(inline_position_left) = inline.position_left {
            position_left = Some(inline_position_left);
        }
        if let Some(inline_position_right) = inline.position_right {
            position_right = Some(inline_position_right);
        }
        if let Some(inline_transform_translate) = inline.transform_translate {
            transform_translate = inline_transform_translate;
        }
        if let Some(inline_z_index) = inline.z_index {
            z_index = inline_z_index;
            z_index_specified = true;
        }
        if let Some(inline_white_space) = inline.white_space {
            white_space = Some(inline_white_space);
        }
        if let Some(inline_text_transform) = inline.text_transform {
            text_transform = Some(inline_text_transform);
        }
        if let Some(inline_letter_spacing) = inline.letter_spacing {
            letter_spacing = Some(inline_letter_spacing);
        }
        if let Some(inline_word_spacing) = inline.word_spacing {
            word_spacing = Some(inline_word_spacing);
        }
        if let Some(inline_overflow_wrap) = inline.overflow_wrap {
            overflow_wrap = Some(inline_overflow_wrap);
        }
        if let Some(inline_word_break) = inline.word_break {
            word_break = Some(inline_word_break);
        }
        if let Some(inline_text_indent) = inline.text_indent {
            text_indent = Some(inline_text_indent);
        }
        if let Some(inline_line_height) = inline.line_height {
            line_height = Some(inline_line_height);
        }
        if let Some(inline_font_scale) = inline.font_scale {
            font_scale = Some(inline_font_scale);
        }
        if let Some(inline_row_gap) = inline.row_gap {
            row_gap = Some(inline_row_gap);
        }
        if let Some(inline_column_gap) = inline.column_gap {
            column_gap = Some(inline_column_gap);
        }
        if let Some(inline_box_sizing) = inline.box_sizing {
            box_sizing = inline_box_sizing;
        }
        if let Some(inline_list_style_type) = inline.list_style_type {
            list_style_type = Some(inline_list_style_type);
        }
        if let Some(inline_border) = inline.border {
            border = Some(inline_border);
        }
        if let Some(inline_padding) = inline.padding {
            padding = inline_padding;
        }
        if let Some(inline_margin) = inline.margin {
            margin = inline_margin;
        }
        if let Some(inline_width) = inline.width {
            width = Some(inline_width);
        }
        if let Some(inline_max_width) = inline.max_width {
            max_width = Some(inline_max_width);
        }
        if let Some(inline_min_width) = inline.min_width {
            min_width = inline_min_width;
        }
        if let Some(inline_height) = inline.height {
            height = Some(inline_height);
        }
        if let Some(inline_max_height) = inline.max_height {
            max_height = Some(inline_max_height);
        }
        if let Some(inline_aspect_ratio) = inline.aspect_ratio {
            aspect_ratio = Some(inline_aspect_ratio);
        }
        if let Some(inline_margin_left_auto) = inline.margin_left_auto {
            margin_left_auto = inline_margin_left_auto;
        }
        if let Some(inline_margin_right_auto) = inline.margin_right_auto {
            margin_right_auto = inline_margin_right_auto;
        }
        if let Some(inline_min_height) = inline.min_height {
            min_height = inline_min_height;
        }
    }
    if background_image_url.is_none()
        && let Some(lazy_background_image_url) = background_image_render_source(element)
    {
        background_image_url = Some(lazy_background_image_url);
    }
    ComputedStyle {
        display,
        float,
        clear,
        background_shade,
        background_image_url,
        background_image_size,
        background_image_position,
        background_image_repeat,
        text_shade,
        text_align,
        visibility,
        opacity,
        animation_reveals_opacity,
        overflow_x,
        overflow_y,
        flex_direction,
        flex_wrap,
        flex_basis,
        justify_content,
        align_items,
        grid_columns,
        grid_auto_min_column_width,
        position,
        position_top,
        position_bottom,
        position_left,
        position_right,
        transform_translate,
        z_index,
        z_index_specified,
        white_space,
        text_transform,
        letter_spacing,
        word_spacing,
        overflow_wrap,
        word_break,
        text_indent,
        line_height,
        font_scale,
        row_gap,
        column_gap,
        box_sizing,
        list_style_type,
        border,
        padding,
        margin,
        width,
        max_width,
        min_width,
        height,
        max_height,
        aspect_ratio,
        margin_left_auto,
        margin_right_auto,
        min_height,
    }
}

fn default_text_shade(element: &ElementData) -> Option<u8> {
    (element.tag == "a" && element.href.is_some()).then(|| rgb_to_luma(0, 0, 255))
}

fn default_display(tag: &str) -> Display {
    match tag {
        "area" | "base" | "basefont" | "datalist" | "head" | "link" | "meta" | "noembed"
        | "noframes" | "param" | "rp" | "script" | "source" | "style" | "template" | "title"
        | "track" | "canvas" | "noscript" => Display::None,
        "address" | "article" | "aside" | "blockquote" | "body" | "dd" | "details" | "div"
        | "dl" | "dt" | "figcaption" | "figure" | "footer" | "form" | "h1" | "h2" | "h3" | "h4"
        | "h5" | "h6" | "header" | "hgroup" | "hr" | "html" | "main" | "nav" | "ol" | "p"
        | "pre" | "search" | "section" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "ul" => {
            Display::Block
        }
        "li" | "summary" => Display::ListItem,
        "svg" => Display::InlineBlock,
        _ => Display::Inline,
    }
}

fn default_margin(tag: &str) -> BoxSpacing {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => BoxSpacing {
            top: 1,
            bottom: 1,
            ..BoxSpacing::default()
        },
        "p" => BoxSpacing {
            top: 1,
            bottom: 1,
            ..BoxSpacing::default()
        },
        "article" | "aside" | "details" | "figure" | "form" | "main" | "section" => BoxSpacing {
            top: 1,
            bottom: 1,
            ..BoxSpacing::default()
        },
        "blockquote" => BoxSpacing {
            left: 4,
            right: 4,
            ..BoxSpacing::default()
        },
        "dd" => BoxSpacing {
            left: 4,
            ..BoxSpacing::default()
        },
        _ => BoxSpacing::default(),
    }
}

fn default_line_height(tag: &str) -> Option<usize> {
    match tag {
        "h1" | "h2" | "h3" => Some(2),
        _ => None,
    }
}

fn default_font_scale(tag: &str) -> Option<usize> {
    match tag {
        "h1" | "h2" => Some(2),
        _ => None,
    }
}

fn is_table_cell_tag(tag: &str) -> bool {
    matches!(tag, "td" | "th")
}

fn is_table_layout_container(element: &ElementData, style: &ComputedStyle) -> bool {
    element.tag == "table" || style.display == Display::Table
}

fn is_table_layout_row(element: &ElementData, style: &ComputedStyle) -> bool {
    element.tag == "tr" || style.display == Display::TableRow
}

fn is_table_layout_cell_for_flow(element: &ElementData, style: &ComputedStyle) -> bool {
    (is_table_cell_tag(&element.tag) && style.display == Display::Inline)
        || style.display == Display::TableCell
}

fn is_table_layout_cell_for_collection(element: &ElementData, style: &ComputedStyle) -> bool {
    is_table_cell_tag(&element.tag) || style.display == Display::TableCell
}

fn table_column_widths(dom: &Dom, table_id: usize, css_cascade: &CssCascade) -> Vec<usize> {
    let mut widths = table_column_width_hints(dom, table_id, css_cascade);
    let mut rowspans = Vec::new();
    for row_id in table_rows(dom, table_id, css_cascade) {
        let mut column = 0usize;
        for cell_id in table_row_cells(dom, row_id, css_cascade) {
            while rowspans.get(column).copied().unwrap_or(0) > 0 {
                column = column.saturating_add(1);
            }
            let colspan = table_cell_colspan(dom, cell_id);
            let rowspan = table_cell_rowspan(dom, cell_id);
            let cell_width = table_cell_layout_width(dom, cell_id, css_cascade);
            let column_width = cell_width.div_ceil(colspan).max(1);
            let end_column = column.saturating_add(colspan);
            if widths.len() < end_column {
                widths.resize(end_column, 0);
            }
            if rowspans.len() < end_column {
                rowspans.resize(end_column, 0);
            }
            for width in &mut widths[column..end_column] {
                *width = (*width).max(column_width);
            }
            for active_rowspan in &mut rowspans[column..end_column] {
                *active_rowspan = (*active_rowspan).max(rowspan);
            }
            column = end_column;
        }
        decrement_table_rowspans(&mut rowspans);
    }
    widths
}

fn table_column_width_hints(dom: &Dom, table_id: usize, css_cascade: &CssCascade) -> Vec<usize> {
    let mut widths = Vec::new();
    let Some(table) = dom.nodes.get(table_id) else {
        return widths;
    };

    for &child_id in &table.children {
        let Some(NodeKind::Element(element)) = dom.nodes.get(child_id).map(|node| &node.kind)
        else {
            continue;
        };
        if computed_style(dom, child_id, element, css_cascade).display == Display::None {
            continue;
        }
        match element.tag.as_str() {
            "col" => push_table_column_width_hint(dom, child_id, css_cascade, None, &mut widths),
            "colgroup" => {
                collect_table_colgroup_width_hints(dom, child_id, css_cascade, &mut widths)
            }
            _ => {}
        }
    }
    widths
}

fn collect_table_colgroup_width_hints(
    dom: &Dom,
    colgroup_id: usize,
    css_cascade: &CssCascade,
    widths: &mut Vec<usize>,
) {
    let inherited_width = table_column_layout_width(dom, colgroup_id, css_cascade);
    let Some(colgroup) = dom.nodes.get(colgroup_id) else {
        return;
    };
    let mut saw_col = false;
    for &child_id in &colgroup.children {
        let Some(NodeKind::Element(element)) = dom.nodes.get(child_id).map(|node| &node.kind)
        else {
            continue;
        };
        if element.tag != "col"
            || computed_style(dom, child_id, element, css_cascade).display == Display::None
        {
            continue;
        }
        saw_col = true;
        push_table_column_width_hint(dom, child_id, css_cascade, inherited_width, widths);
    }

    if !saw_col {
        let width = inherited_width.unwrap_or(0);
        for _ in 0..table_column_span(dom, colgroup_id) {
            widths.push(width);
        }
    }
}

fn push_table_column_width_hint(
    dom: &Dom,
    column_id: usize,
    css_cascade: &CssCascade,
    inherited_width: Option<usize>,
    widths: &mut Vec<usize>,
) {
    let width = table_column_layout_width(dom, column_id, css_cascade)
        .or(inherited_width)
        .unwrap_or(0);
    for _ in 0..table_column_span(dom, column_id) {
        widths.push(width);
    }
}

fn table_rows(dom: &Dom, table_id: usize, css_cascade: &CssCascade) -> Vec<usize> {
    let mut rows = Vec::new();
    collect_table_rows(dom, table_id, table_id, css_cascade, &mut rows);
    rows
}

fn collect_table_rows(
    dom: &Dom,
    table_id: usize,
    node_id: usize,
    css_cascade: &CssCascade,
    rows: &mut Vec<usize>,
) {
    let Some(node) = dom.nodes.get(node_id) else {
        return;
    };
    if let NodeKind::Element(element) = &node.kind {
        let style = computed_style(dom, node_id, element, css_cascade);
        if style.display == Display::None {
            return;
        }
        if node_id != table_id && is_table_layout_container(element, &style) {
            return;
        }
        if is_table_layout_row(element, &style) {
            rows.push(node_id);
            return;
        }
    }

    for &child in &node.children {
        collect_table_rows(dom, table_id, child, css_cascade, rows);
    }
}

fn table_row_cell_count(dom: &Dom, row_id: usize, css_cascade: &CssCascade) -> usize {
    table_row_cells(dom, row_id, css_cascade).len()
}

fn table_row_cells(dom: &Dom, row_id: usize, css_cascade: &CssCascade) -> Vec<usize> {
    let Some(row) = dom.nodes.get(row_id) else {
        return Vec::new();
    };
    row.children
        .iter()
        .copied()
        .filter(|&child_id| {
            let Some(NodeKind::Element(element)) = dom.nodes.get(child_id).map(|node| &node.kind)
            else {
                return false;
            };
            let style = computed_style(dom, child_id, element, css_cascade);
            style.display != Display::None && is_table_layout_cell_for_collection(element, &style)
        })
        .collect()
}

fn table_cell_colspan(dom: &Dom, cell_id: usize) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(cell_id).map(|node| &node.kind) else {
        return 1;
    };
    element
        .attrs
        .get("colspan")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 16)
}

fn table_cell_rowspan(dom: &Dom, cell_id: usize) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(cell_id).map(|node| &node.kind) else {
        return 1;
    };
    element
        .attrs
        .get("rowspan")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 16)
}

fn table_column_span(dom: &Dom, column_id: usize) -> usize {
    let Some(NodeKind::Element(element)) = dom.nodes.get(column_id).map(|node| &node.kind) else {
        return 1;
    };
    element
        .attrs
        .get("span")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 64)
}

fn table_cell_layout_width(dom: &Dom, cell_id: usize, css_cascade: &CssCascade) -> usize {
    let text_width = table_cell_text_width(dom, cell_id, css_cascade);
    let visible_box_width = dom
        .nodes
        .get(cell_id)
        .and_then(|node| match &node.kind {
            NodeKind::Element(element) => {
                let style = computed_style(dom, cell_id, element, css_cascade);
                let border_width = style.border.map(|border| border.width).unwrap_or(0);
                Some(
                    text_width
                        .saturating_add(style.padding.left)
                        .saturating_add(style.padding.right)
                        .saturating_add(border_width.saturating_mul(2)),
                )
            }
            _ => None,
        })
        .unwrap_or(text_width);
    visible_box_width.max(table_column_layout_width(dom, cell_id, css_cascade).unwrap_or(0))
}

fn table_column_layout_width(
    dom: &Dom,
    column_id: usize,
    css_cascade: &CssCascade,
) -> Option<usize> {
    let Some(NodeKind::Element(element)) = dom.nodes.get(column_id).map(|node| &node.kind) else {
        return None;
    };
    let style_width = computed_style(dom, column_id, element, css_cascade)
        .width
        .map(|width| width.resolve(default_horizontal_dimension_basis()));
    let attr_width = element
        .attrs
        .get("width")
        .and_then(|value| parse_css_dimension(value, CssAxis::Horizontal))
        .map(|width| width.resolve(default_horizontal_dimension_basis()));
    style_width.or(attr_width)
}

fn decrement_table_rowspans(rowspans: &mut [usize]) {
    for rowspan in rowspans {
        *rowspan = rowspan.saturating_sub(1);
    }
}

fn table_cell_text_width(dom: &Dom, cell_id: usize, css_cascade: &CssCascade) -> usize {
    let mut text = String::new();
    collect_visible_layout_text(dom, cell_id, css_cascade, &mut text);
    collapse_ascii_whitespace(&text).chars().count()
}

fn collect_visible_layout_text(
    dom: &Dom,
    node_id: usize,
    css_cascade: &CssCascade,
    out: &mut String,
) {
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
                collect_visible_layout_text(dom, child, css_cascade, out);
            }
        }
        NodeKind::Element(element) => {
            if computed_style(dom, node_id, element, css_cascade).display == Display::None {
                return;
            }
            for &child in &node.children {
                collect_visible_layout_text(dom, child, css_cascade, out);
            }
        }
    }
}

fn selector_specificity(selector: &CssSelector) -> u32 {
    selector
        .steps
        .iter()
        .map(|step| compound_specificity(&step.compound))
        .sum()
}

fn compound_specificity(compound: &CompoundSelector) -> u32 {
    u32::from(compound.id.is_some()) * 100
        + (compound.classes.len() + compound.attributes.len() + usize::from(compound.first_child))
            as u32
            * 10
        + u32::from(compound.tag.is_some())
        + compound
            .not_selectors
            .iter()
            .map(compound_specificity)
            .max()
            .unwrap_or(0)
}

fn selector_matches(selector: &CssSelector, dom: &Dom, node_id: usize) -> bool {
    if selector.steps.is_empty() {
        return false;
    }
    selector_matches_step(selector, selector.steps.len() - 1, dom, node_id)
}

fn selector_matches_step(
    selector: &CssSelector,
    step_index: usize,
    dom: &Dom,
    node_id: usize,
) -> bool {
    let Some(step) = selector.steps.get(step_index) else {
        return false;
    };
    if !compound_selector_matches(&step.compound, dom, node_id) {
        return false;
    }
    if step_index == 0 {
        return true;
    }

    match step.combinator.unwrap_or(SelectorCombinator::Descendant) {
        SelectorCombinator::Child => dom
            .nodes
            .get(node_id)
            .and_then(|node| node.parent)
            .is_some_and(|parent| selector_matches_step(selector, step_index - 1, dom, parent)),
        SelectorCombinator::Descendant => {
            let mut current = dom.nodes.get(node_id).and_then(|node| node.parent);
            while let Some(parent) = current {
                if selector_matches_step(selector, step_index - 1, dom, parent) {
                    return true;
                }
                current = dom.nodes.get(parent).and_then(|node| node.parent);
            }
            false
        }
    }
}

fn compound_selector_matches(compound: &CompoundSelector, dom: &Dom, node_id: usize) -> bool {
    let Some(NodeKind::Element(element)) = dom.nodes.get(node_id).map(|node| &node.kind) else {
        return false;
    };
    if let Some(tag) = &compound.tag
        && element.tag != *tag
    {
        return false;
    }
    if let Some(id) = &compound.id
        && element.id.as_deref() != Some(id.as_str())
    {
        return false;
    }
    if !compound
        .classes
        .iter()
        .all(|class| element.classes.iter().any(|item| item == class))
    {
        return false;
    }
    for attribute in &compound.attributes {
        let Some(value) = get_element_attribute_data(element, &attribute.name) else {
            return false;
        };
        if attribute
            .value
            .as_deref()
            .is_some_and(|expected| value != expected)
        {
            return false;
        }
    }
    if compound
        .not_selectors
        .iter()
        .any(|selector| compound_selector_matches(selector, dom, node_id))
    {
        return false;
    }
    if compound.first_child && !element_is_first_child(dom, node_id) {
        return false;
    }
    compound.universal
        || compound.tag.is_some()
        || compound.id.is_some()
        || !compound.classes.is_empty()
        || !compound.attributes.is_empty()
        || !compound.not_selectors.is_empty()
        || compound.first_child
}

fn element_is_first_child(dom: &Dom, node_id: usize) -> bool {
    let Some(parent_id) = dom.nodes.get(node_id).and_then(|node| node.parent) else {
        return false;
    };
    dom.nodes
        .get(parent_id)
        .map(|parent| {
            parent.children.iter().copied().find(|&child_id| {
                matches!(
                    dom.nodes.get(child_id).map(|node| &node.kind),
                    Some(NodeKind::Element(_))
                )
            }) == Some(node_id)
        })
        .unwrap_or(false)
}

#[derive(Debug)]
struct TextRun {
    start_x: usize,
    text: String,
    shade: u8,
    font_scale: usize,
    background_shade: Option<u8>,
    link_underline: bool,
    visible: bool,
    target_runs: Vec<TextHitTargetRun>,
}

#[derive(Debug)]
struct FlowOutOfFlowSnapshot {
    lines_len: usize,
    current_runs: Vec<TextRun>,
    current_width: usize,
    inline_replaced_height: usize,
    pending_inter_word_space: Option<usize>,
    soft_break_opportunity: bool,
    next_y: usize,
    pending_text_indent: Option<usize>,
    line_start_indent: usize,
    table_stack: Vec<TableFlow>,
    active_floats: Vec<ActiveFloat>,
}

#[derive(Debug)]
struct FlowHorizontalProjectionSnapshot {
    left_inset: usize,
    right_inset: usize,
}

#[derive(Debug)]
struct FlowVerticalProjectionSnapshot {
    offset: isize,
}

#[derive(Debug, Clone, Copy)]
struct FlowPositioningContext {
    y: usize,
    height: Option<usize>,
}

#[derive(Debug)]
struct TableFlow {
    column_widths: Vec<usize>,
    column_gap: usize,
    row_gap: usize,
    remaining_rows: usize,
    row_stack: Vec<TableRowFlow>,
    rowspans: Vec<usize>,
}

#[derive(Debug)]
struct TableRowFlow {
    remaining_cells: usize,
    next_column_index: usize,
    active_cell: Option<TableCellFlow>,
}

#[derive(Debug, Clone, Copy)]
struct TableCellFlow {
    column_index: usize,
    colspan: usize,
    rowspan: usize,
    start_width: usize,
    start_y: usize,
    background_shade: Option<u8>,
    target_node: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveFloat {
    side: FloatSide,
    width: usize,
    bottom_y: usize,
}

#[derive(Debug)]
struct PaintLayerCommands {
    z_index: i32,
    underlay_list: Vec<DisplayCommand>,
    underlay_targets: Vec<DisplayHitTarget>,
    border_list: Vec<DisplayCommand>,
    border_targets: Vec<DisplayHitTarget>,
    display_list: Vec<DisplayCommand>,
    display_targets: Vec<DisplayHitTarget>,
}

#[derive(Debug, Clone, Copy)]
struct PaintUnderlayInsertion {
    z_index: Option<i32>,
    index: usize,
}

impl PaintUnderlayInsertion {
    fn offset(self, amount: usize) -> Self {
        Self {
            z_index: self.z_index,
            index: self.index.saturating_add(amount),
        }
    }
}

impl PaintLayerCommands {
    fn new(z_index: i32) -> Self {
        Self {
            z_index,
            underlay_list: Vec::new(),
            underlay_targets: Vec::new(),
            border_list: Vec::new(),
            border_targets: Vec::new(),
            display_list: Vec::new(),
            display_targets: Vec::new(),
        }
    }
}

fn table_spanned_column_width(
    column_widths: &[usize],
    column_gap: usize,
    active_cell: TableCellFlow,
    fallback_width: usize,
) -> usize {
    let span = active_cell.colspan.max(1);
    let Some(columns) =
        column_widths.get(active_cell.column_index..active_cell.column_index.saturating_add(span))
    else {
        return fallback_width;
    };
    if columns.is_empty() {
        return fallback_width;
    }
    columns
        .iter()
        .copied()
        .sum::<usize>()
        .saturating_add(span.saturating_sub(1).saturating_mul(column_gap))
}

fn table_skipped_column_padding(
    column_widths: &[usize],
    column_gap: usize,
    start: usize,
    span: usize,
) -> usize {
    if span == 0 {
        return 0;
    }
    let width = column_widths
        .get(start..start.saturating_add(span))
        .map(|columns| columns.iter().copied().sum::<usize>())
        .unwrap_or(0);
    width
        .saturating_add(span.saturating_mul(column_gap))
        .saturating_sub(1)
}

fn push_text_hit_target_run(
    runs: &mut Vec<TextHitTargetRun>,
    start: usize,
    piece: &str,
    target_node: Option<usize>,
    font_scale: usize,
) {
    let width = text_cell_width(piece, font_scale);
    if width == 0 {
        return;
    }
    if let Some(last) = runs.last_mut()
        && last.target_node == target_node
        && last.start.saturating_add(last.width) == start
    {
        last.width = last.width.saturating_add(width);
        return;
    }
    runs.push(TextHitTargetRun {
        start,
        width,
        target_node,
    });
}

fn text_char_slice(text: &str, start: usize, width: usize) -> String {
    text.chars().skip(start).take(width).collect()
}

fn text_cell_width(text: &str, font_scale: usize) -> usize {
    text.chars().count().saturating_mul(font_scale.max(1))
}

fn scaled_text_char_range(
    start: usize,
    width: usize,
    font_scale: usize,
    char_count: usize,
) -> (usize, usize) {
    let font_scale = font_scale.max(1);
    let start_char = start / font_scale;
    let end_char = start
        .saturating_add(width)
        .saturating_add(font_scale.saturating_sub(1))
        / font_scale;
    (start_char.min(char_count), end_char.min(char_count))
}

fn scaled_text_for_line(text: &str, font_scale: usize) -> String {
    let font_scale = font_scale.max(1);
    if font_scale == 1 {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text_cell_width(text, font_scale));
    for ch in text.chars() {
        out.extend(std::iter::repeat(ch).take(font_scale));
    }
    out
}

fn clip_text_hit_target_runs(
    runs: &[TextHitTargetRun],
    start: usize,
    width: usize,
) -> Vec<TextHitTargetRun> {
    let end = start.saturating_add(width);
    let mut clipped = Vec::new();
    for run in runs {
        let run_start = run.start;
        let run_end = run.start.saturating_add(run.width);
        let clipped_start = run_start.max(start);
        let clipped_end = run_end.min(end);
        if clipped_end > clipped_start {
            clipped.push(TextHitTargetRun {
                start: clipped_start.saturating_sub(start),
                width: clipped_end.saturating_sub(clipped_start),
                target_node: run.target_node,
            });
        }
    }
    clipped
}

#[derive(Debug)]
struct FlowRenderer {
    width: usize,
    left_inset: usize,
    right_inset: usize,
    lines: Vec<String>,
    current_runs: Vec<TextRun>,
    current_width: usize,
    inline_replaced_height: usize,
    pending_inter_word_space: Option<usize>,
    soft_break_opportunity: bool,
    next_y: usize,
    text_shade: u8,
    text_shade_stack: Vec<u8>,
    text_background_shade: Option<u8>,
    text_background_shade_stack: Vec<Option<u8>>,
    text_align: TextAlign,
    text_align_stack: Vec<TextAlign>,
    visibility: Visibility,
    visibility_stack: Vec<Visibility>,
    transparent_opacity_depth: usize,
    white_space: WhiteSpace,
    white_space_stack: Vec<WhiteSpace>,
    text_transform: TextTransform,
    text_transform_stack: Vec<TextTransform>,
    text_transform_capitalize_next: bool,
    text_transform_capitalize_next_stack: Vec<bool>,
    letter_spacing: usize,
    letter_spacing_stack: Vec<usize>,
    word_spacing: usize,
    word_spacing_stack: Vec<usize>,
    overflow_wrap: OverflowWrap,
    overflow_wrap_stack: Vec<OverflowWrap>,
    word_break: WordBreak,
    word_break_stack: Vec<WordBreak>,
    text_indent: usize,
    text_indent_stack: Vec<usize>,
    pending_text_indent: Option<usize>,
    pending_text_indent_stack: Vec<Option<usize>>,
    line_start_indent: usize,
    line_start_indent_stack: Vec<usize>,
    line_height: usize,
    line_height_stack: Vec<usize>,
    font_scale: usize,
    font_scale_stack: Vec<usize>,
    row_align_items: AlignItems,
    row_align_items_stack: Vec<AlignItems>,
    link_text_depth: usize,
    positioning_context_stack: Vec<FlowPositioningContext>,
    vertical_projection_offset: isize,
    viewport_fixed_depth: usize,
    viewport_sticky_top_stack: Vec<usize>,
    positive_z_layer_stack: Vec<i32>,
    table_stack: Vec<TableFlow>,
    active_floats: Vec<ActiveFloat>,
    underlay_list: Vec<DisplayCommand>,
    underlay_targets: Vec<DisplayHitTarget>,
    border_list: Vec<DisplayCommand>,
    border_targets: Vec<DisplayHitTarget>,
    display_list: Vec<DisplayCommand>,
    display_targets: Vec<DisplayHitTarget>,
    positive_z_layers: Vec<PaintLayerCommands>,
    active_clip: Option<DisplayCommandBounds>,
    clip_stack: Vec<Option<DisplayCommandBounds>>,
    decoded_images: Vec<DecodedImageEntry>,
    decoded_image_cache: HashMap<String, Option<usize>>,
}

impl FlowRenderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            left_inset: 0,
            right_inset: 0,
            lines: Vec::new(),
            current_runs: Vec::new(),
            current_width: 0,
            inline_replaced_height: 0,
            pending_inter_word_space: None,
            soft_break_opportunity: false,
            next_y: 0,
            text_shade: 0,
            text_shade_stack: Vec::new(),
            text_background_shade: None,
            text_background_shade_stack: Vec::new(),
            text_align: TextAlign::Start,
            text_align_stack: Vec::new(),
            visibility: Visibility::Visible,
            visibility_stack: Vec::new(),
            transparent_opacity_depth: 0,
            white_space: WhiteSpace::Normal,
            white_space_stack: Vec::new(),
            text_transform: TextTransform::None,
            text_transform_stack: Vec::new(),
            text_transform_capitalize_next: true,
            text_transform_capitalize_next_stack: Vec::new(),
            letter_spacing: 0,
            letter_spacing_stack: Vec::new(),
            word_spacing: 0,
            word_spacing_stack: Vec::new(),
            overflow_wrap: OverflowWrap::Normal,
            overflow_wrap_stack: Vec::new(),
            word_break: WordBreak::Normal,
            word_break_stack: Vec::new(),
            text_indent: 0,
            text_indent_stack: Vec::new(),
            pending_text_indent: None,
            pending_text_indent_stack: Vec::new(),
            line_start_indent: 0,
            line_start_indent_stack: Vec::new(),
            line_height: 1,
            line_height_stack: Vec::new(),
            font_scale: 1,
            font_scale_stack: Vec::new(),
            row_align_items: AlignItems::Start,
            row_align_items_stack: Vec::new(),
            link_text_depth: 0,
            positioning_context_stack: Vec::new(),
            vertical_projection_offset: 0,
            viewport_fixed_depth: 0,
            viewport_sticky_top_stack: Vec::new(),
            positive_z_layer_stack: Vec::new(),
            table_stack: Vec::new(),
            active_floats: Vec::new(),
            underlay_list: Vec::new(),
            underlay_targets: Vec::new(),
            border_list: Vec::new(),
            border_targets: Vec::new(),
            display_list: Vec::new(),
            display_targets: Vec::new(),
            positive_z_layers: Vec::new(),
            active_clip: None,
            clip_stack: Vec::new(),
            decoded_images: Vec::new(),
            decoded_image_cache: HashMap::new(),
        }
    }

    fn seed_decoded_images(&mut self, images: &[DecodedImageEntry]) {
        for image in images {
            let index = self.decoded_images.len();
            self.decoded_images.push(image.clone());
            self.decoded_image_cache
                .insert(image.url.clone(), Some(index));
        }
    }

    fn positive_z_layer_mut(&mut self, z_index: i32) -> &mut PaintLayerCommands {
        if let Some(index) = self
            .positive_z_layers
            .iter()
            .position(|layer| layer.z_index == z_index)
        {
            return &mut self.positive_z_layers[index];
        }
        self.positive_z_layers
            .push(PaintLayerCommands::new(z_index));
        self.positive_z_layers.last_mut().unwrap()
    }

    fn current_positive_z_index(&self) -> Option<i32> {
        self.positive_z_layer_stack.last().copied()
    }

    fn push_underlay_command(&mut self, command: DisplayCommand, target: DisplayHitTarget) {
        if let Some(z_index) = self.current_positive_z_index() {
            let layer = self.positive_z_layer_mut(z_index);
            layer.underlay_list.push(command);
            layer.underlay_targets.push(target);
        } else {
            self.underlay_list.push(command);
            self.underlay_targets.push(target);
        }
    }

    fn underlay_insert_position(&mut self) -> PaintUnderlayInsertion {
        if let Some(z_index) = self.current_positive_z_index() {
            let layer = self.positive_z_layer_mut(z_index);
            PaintUnderlayInsertion {
                z_index: Some(z_index),
                index: layer.underlay_list.len(),
            }
        } else {
            PaintUnderlayInsertion {
                z_index: None,
                index: self.underlay_list.len(),
            }
        }
    }

    fn insert_underlay_command(
        &mut self,
        position: PaintUnderlayInsertion,
        command: DisplayCommand,
        target: DisplayHitTarget,
    ) {
        if let Some(z_index) = position.z_index {
            let layer = self.positive_z_layer_mut(z_index);
            let index = position.index.min(layer.underlay_list.len());
            layer.underlay_list.insert(index, command);
            layer.underlay_targets.insert(index, target);
        } else {
            let index = position.index.min(self.underlay_list.len());
            self.underlay_list.insert(index, command);
            self.underlay_targets.insert(index, target);
        }
    }

    fn push_border_command(&mut self, command: DisplayCommand, target: DisplayHitTarget) {
        if let Some(z_index) = self.current_positive_z_index() {
            let layer = self.positive_z_layer_mut(z_index);
            layer.border_list.push(command);
            layer.border_targets.push(target);
        } else {
            self.border_list.push(command);
            self.border_targets.push(target);
        }
    }

    fn push_display_command(&mut self, command: DisplayCommand, target: DisplayHitTarget) {
        if let Some(z_index) = self.current_positive_z_index() {
            let layer = self.positive_z_layer_mut(z_index);
            layer.display_list.push(command);
            layer.display_targets.push(target);
        } else {
            self.display_list.push(command);
            self.display_targets.push(target);
        }
    }

    fn decoded_image_intrinsic_size(
        &mut self,
        source: &str,
        url: Option<&str>,
    ) -> Option<(usize, usize)> {
        let info = self.cached_decoded_image_info(source, url?)?;
        Some((
            info.width.div_ceil(8).max(1),
            info.height.div_ceil(12).max(1),
        ))
    }

    fn enter_clip(&mut self, clip: DisplayCommandBounds) {
        let active = self
            .active_clip
            .and_then(|current| intersect_display_bounds(current, clip))
            .or_else(|| {
                self.active_clip
                    .is_none()
                    .then_some(clip)
                    .or(Some(DisplayCommandBounds {
                        x: clip.x,
                        y: clip.y,
                        width: 0,
                        height: 0,
                    }))
            });
        self.clip_stack.push(self.active_clip);
        self.active_clip = active;
    }

    fn exit_clip(&mut self) {
        self.active_clip = self.clip_stack.pop().unwrap_or(None);
    }

    fn enter_unclipped(&mut self) {
        self.clip_stack.push(self.active_clip);
        self.active_clip = None;
    }

    fn exit_unclipped(&mut self) {
        self.active_clip = self.clip_stack.pop().unwrap_or(None);
    }

    fn clipped_bounds(&self, bounds: DisplayCommandBounds) -> Option<DisplayCommandBounds> {
        let bounds = self.project_display_bounds(bounds);
        match self.active_clip {
            Some(clip) => intersect_display_bounds(bounds, clip),
            None => Some(bounds),
        }
    }

    fn project_display_bounds(&self, bounds: DisplayCommandBounds) -> DisplayCommandBounds {
        DisplayCommandBounds {
            y: saturating_add_signed(bounds.y, self.vertical_projection_offset),
            ..bounds
        }
    }

    fn push_text_display_command(
        &mut self,
        x: usize,
        y: usize,
        text: String,
        shade: u8,
        font_scale: usize,
        target_runs: Vec<TextHitTargetRun>,
    ) {
        let font_scale = font_scale.max(1);
        let bounds = DisplayCommandBounds {
            x,
            y,
            width: text_cell_width(&text, font_scale),
            height: font_scale,
        };
        let Some(clipped) = self.clipped_bounds(bounds) else {
            return;
        };
        let visual_y = saturating_add_signed(y, self.vertical_projection_offset);
        let char_count = text.chars().count();
        let start = clipped.x.saturating_sub(x);
        let (start_char, end_char) =
            scaled_text_char_range(start, clipped.width, font_scale, char_count);
        let text = text_char_slice(&text, start_char, end_char.saturating_sub(start_char));
        if text.is_empty() {
            return;
        }
        let command_x = x.saturating_add(start_char.saturating_mul(font_scale));
        let command_width = text_cell_width(&text, font_scale);
        let target = self.text_hit_target(clip_text_hit_target_runs(
            &target_runs,
            start_char.saturating_mul(font_scale),
            command_width,
        ));
        let text = scaled_text_for_line(&text, font_scale);
        let row_start = clipped.y.saturating_sub(visual_y);
        let row_end = clipped
            .y
            .saturating_add(clipped.height)
            .saturating_sub(visual_y)
            .min(font_scale);
        for row_offset in row_start..row_end {
            let command = if shade == 0 {
                DisplayCommand::Text {
                    x: command_x,
                    y: visual_y.saturating_add(row_offset),
                    text: text.clone(),
                }
            } else {
                DisplayCommand::StyledText {
                    x: command_x,
                    y: visual_y.saturating_add(row_offset),
                    text: text.clone(),
                    shade,
                }
            };
            self.push_display_command(command, target.clone());
        }
    }

    fn clipped_rect_command(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        shade: u8,
    ) -> Option<DisplayCommand> {
        let clipped = self.clipped_bounds(DisplayCommandBounds {
            x,
            y,
            width,
            height,
        })?;
        Some(DisplayCommand::Rect {
            x: clipped.x,
            y: clipped.y,
            width: clipped.width,
            height: clipped.height,
            shade,
        })
    }

    fn clipped_color_rect_command(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        red: u8,
        green: u8,
        blue: u8,
    ) -> Option<DisplayCommand> {
        let clipped = self.clipped_bounds(DisplayCommandBounds {
            x,
            y,
            width,
            height,
        })?;
        Some(DisplayCommand::ColorRect {
            x: clipped.x,
            y: clipped.y,
            width: clipped.width,
            height: clipped.height,
            red,
            green,
            blue,
            shade: rgb_to_luma(red, green, blue),
        })
    }

    fn clipped_image_command(
        &self,
        command: DisplayCommand,
    ) -> Option<(DisplayCommand, Option<DisplaySourceBounds>)> {
        let DisplayCommand::Image {
            x,
            y,
            width,
            height,
            shade,
            alt,
            url,
            decoded_width,
            decoded_height,
            decoded_hash,
        } = command
        else {
            return None;
        };
        let visual_bounds = self.project_display_bounds(DisplayCommandBounds {
            x,
            y,
            width,
            height,
        });
        let source_bounds = DisplaySourceBounds {
            x: visual_bounds.x,
            y: visual_bounds.y,
            width: visual_bounds.width,
            height: visual_bounds.height,
        };
        let clipped = match self.active_clip {
            Some(clip) => intersect_display_bounds(visual_bounds, clip),
            None => Some(visual_bounds),
        }?;
        let clip_source_bounds = (clipped.x != visual_bounds.x
            || clipped.y != visual_bounds.y
            || clipped.width != visual_bounds.width
            || clipped.height != visual_bounds.height)
            .then_some(source_bounds);
        Some((
            DisplayCommand::Image {
                x: clipped.x,
                y: clipped.y,
                width: clipped.width,
                height: clipped.height,
                shade,
                alt,
                url,
                decoded_width,
                decoded_height,
                decoded_hash,
            },
            clip_source_bounds,
        ))
    }

    fn clipped_background_image_command(
        &self,
        command: DisplayCommand,
    ) -> Option<(DisplayCommand, Option<DisplaySourceBounds>)> {
        let DisplayCommand::BackgroundImage {
            x,
            y,
            width,
            height,
            shade,
            url,
            decoded_width,
            decoded_height,
            decoded_hash,
            size,
            position,
            repeat,
        } = command
        else {
            return None;
        };
        let visual_bounds = self.project_display_bounds(DisplayCommandBounds {
            x,
            y,
            width,
            height,
        });
        let source_bounds = DisplaySourceBounds {
            x: visual_bounds.x,
            y: visual_bounds.y,
            width: visual_bounds.width,
            height: visual_bounds.height,
        };
        let clipped = match self.active_clip {
            Some(clip) => intersect_display_bounds(visual_bounds, clip),
            None => Some(visual_bounds),
        }?;
        let clip_source_bounds = (clipped.x != visual_bounds.x
            || clipped.y != visual_bounds.y
            || clipped.width != visual_bounds.width
            || clipped.height != visual_bounds.height)
            .then_some(source_bounds);
        Some((
            DisplayCommand::BackgroundImage {
                x: clipped.x,
                y: clipped.y,
                width: clipped.width,
                height: clipped.height,
                shade,
                url,
                decoded_width,
                decoded_height,
                decoded_hash,
                size,
                position,
                repeat,
            },
            clip_source_bounds,
        ))
    }

    fn push_text(&mut self, text: &str, target_node: Option<usize>) {
        match self.white_space {
            WhiteSpace::Normal => self.push_wrapped_text(text, target_node),
            WhiteSpace::Nowrap => self.push_nowrap_text(text, target_node),
            WhiteSpace::Pre => self.push_preformatted_text(text, target_node),
            WhiteSpace::PreLine => self.push_pre_line_text(text, target_node),
            WhiteSpace::PreWrap => self.push_pre_wrap_text(text, target_node),
            WhiteSpace::BreakSpaces => self.push_break_spaces_text(text, target_node),
        }
    }

    fn push_wrapped_text(&mut self, text: &str, target_node: Option<usize>) {
        let mut token_start = None;
        for (index, ch) in text.char_indices() {
            if ch.is_whitespace() {
                if let Some(start) = token_start.take() {
                    self.push_wrapped_text_token(&text[start..index], target_node);
                }
                if self.current_width > 0 {
                    self.pending_inter_word_space = Some(self.inter_word_space_width());
                }
            } else if token_start.is_none() {
                token_start = Some(index);
            }
        }
        if let Some(start) = token_start {
            self.push_wrapped_text_token(&text[start..], target_node);
        }
    }

    fn push_wrapped_text_token(&mut self, word: &str, target_node: Option<usize>) {
        self.clear_narrow_float_column_for_text();
        let available_width = self.available_width();
        let word_width = self.letter_spaced_text_width(word);
        if self.current_width == 0 {
            self.pending_inter_word_space = None;
            self.soft_break_opportunity = false;
            self.push_wrapped_word(word, target_node);
        } else if self.soft_break_opportunity {
            self.soft_break_opportunity = false;
            self.pending_inter_word_space = None;
            if self.current_width.saturating_add(word_width) > available_width {
                self.break_line();
            }
            self.push_wrapped_word(word, target_node);
        } else if let Some(space_width) = self.pending_inter_word_space.take() {
            if self
                .effective_current_width()
                .saturating_add(space_width)
                .saturating_add(word_width)
                > available_width
            {
                if self.can_break_word()
                    && self.effective_current_width().saturating_add(space_width) < available_width
                {
                    self.push_inter_word_space_width(space_width, target_node);
                } else {
                    self.break_line();
                }
            } else {
                self.push_inter_word_space_width(space_width, target_node);
            }
            self.push_wrapped_word(word, target_node);
        } else {
            self.push_wrapped_word(word, target_node);
        }
    }

    fn push_wrapped_word(&mut self, word: &str, target_node: Option<usize>) {
        let word_fits = self
            .effective_current_width()
            .saturating_add(self.letter_spaced_text_width(word))
            <= self.available_width();
        if !self.can_break_word() || word_fits {
            self.push_text_run_piece(word, target_node);
            return;
        }
        self.push_breakable_text_segment(word, target_node);
    }

    fn can_break_word(&self) -> bool {
        self.overflow_wrap != OverflowWrap::Normal
            || matches!(self.word_break, WordBreak::BreakAll | WordBreak::BreakWord)
    }

    fn push_nowrap_text(&mut self, text: &str, target_node: Option<usize>) {
        self.soft_break_opportunity = false;
        let mut token_start = None;
        for (index, ch) in text.char_indices() {
            if ch.is_whitespace() {
                if let Some(start) = token_start.take() {
                    self.push_nowrap_text_token(&text[start..index], target_node);
                }
                if self.current_width > 0 {
                    self.pending_inter_word_space = Some(self.inter_word_space_width());
                }
            } else if token_start.is_none() {
                token_start = Some(index);
            }
        }
        if let Some(start) = token_start {
            self.push_nowrap_text_token(&text[start..], target_node);
        }
    }

    fn push_nowrap_text_token(&mut self, word: &str, target_node: Option<usize>) {
        self.clear_narrow_float_column_for_text();
        if self.current_width > 0 {
            if let Some(space_width) = self.pending_inter_word_space.take() {
                self.push_inter_word_space_width(space_width, target_node);
            }
        }
        self.pending_inter_word_space = None;
        self.push_text_run_piece(word, target_node);
    }

    fn inter_word_space_width(&self) -> usize {
        self.font_scale.max(1).saturating_add(self.word_spacing)
    }

    fn push_inter_word_space_width(&mut self, width: usize, target_node: Option<usize>) {
        self.push_fixed_space_width(width, target_node);
    }

    fn push_word_break_opportunity(&mut self) {
        if self.current_width > 0 {
            self.soft_break_opportunity = true;
        }
    }

    fn push_pre_line_text(&mut self, text: &str, target_node: Option<usize>) {
        let mut start = 0usize;
        for (index, ch) in text.char_indices() {
            if ch == '\n' {
                self.push_wrapped_text(&text[start..index], target_node);
                self.force_line_break();
                start = index + ch.len_utf8();
            }
        }
        if start < text.len() {
            self.push_wrapped_text(&text[start..], target_node);
        }
    }

    fn push_pre_wrap_text(&mut self, text: &str, target_node: Option<usize>) {
        let mut start = 0usize;
        for (index, ch) in text.char_indices() {
            if ch == '\n' {
                self.push_pre_wrap_segment(&text[start..index], target_node);
                self.force_line_break();
                start = index + ch.len_utf8();
            }
        }
        if start < text.len() {
            self.push_pre_wrap_segment(&text[start..], target_node);
        }
    }

    fn push_break_spaces_text(&mut self, text: &str, target_node: Option<usize>) {
        self.push_pre_wrap_text(text, target_node);
    }

    fn push_pre_wrap_segment(&mut self, text: &str, target_node: Option<usize>) {
        self.push_breakable_text_segment(text, target_node);
    }

    fn push_breakable_text_segment(&mut self, text: &str, target_node: Option<usize>) {
        self.clear_narrow_float_column_for_text();
        let mut start = 0usize;
        let mut char_count = 0usize;
        for (index, _) in text.char_indices() {
            let available_width = self.available_width();
            let next_width = self.letter_spaced_char_count_width(char_count.saturating_add(1));
            if self.effective_current_width().saturating_add(next_width) > available_width {
                if char_count > 0 {
                    self.push_text_run_piece(&text[start..index], target_node);
                }
                self.break_line();
                start = index;
                char_count = 1;
            } else {
                char_count += 1;
            }
        }
        if char_count > 0 {
            self.push_text_run_piece(&text[start..], target_node);
        }
    }

    fn push_preformatted_text(&mut self, text: &str, target_node: Option<usize>) {
        let mut start = 0usize;
        for (index, ch) in text.char_indices() {
            if ch == '\n' {
                self.push_text_run_piece(&text[start..index], target_node);
                self.force_line_break();
                start = index + ch.len_utf8();
            }
        }
        if start < text.len() {
            self.push_text_run_piece(&text[start..], target_node);
        }
    }

    fn push_inline_widget(&mut self, text: &str, target_node: Option<usize>) {
        let line_was_empty = self.current_width == 0;
        if line_was_empty {
            self.push_line_start_spacing();
        }
        let text_width = text_cell_width(text, self.font_scale);
        if text_width == 0 {
            return;
        }
        let padding = 1usize;
        let width = text_width.saturating_add(padding.saturating_mul(2));
        if !line_was_empty && self.current_width > 0 {
            if self.current_width.saturating_add(1).saturating_add(width) > self.available_width() {
                self.break_line();
            } else {
                self.push_text_run_piece_unspaced(" ", None);
            }
        }
        if self.current_width == 0 {
            self.push_line_start_spacing();
        }
        let widget_height = self.line_height.max(self.font_scale).max(2);
        if self.paint_visible() {
            let x = self.box_x().saturating_add(self.current_width);
            let target = self.node_hit_target(target_node);
            self.push_inline_widget_box(x, self.next_y, width, widget_height, target);
        }
        self.push_fixed_space_width(padding, target_node);
        self.push_text_run_piece(text, target_node);
        self.push_fixed_space_width(padding, target_node);
        self.inline_replaced_height = self.inline_replaced_height.max(widget_height);
    }

    fn push_inline_widget_box(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        target: DisplayHitTarget,
    ) {
        if let Some(command) =
            self.clipped_rect_command(x, y, width, height, INLINE_WIDGET_BACKGROUND_SHADE)
        {
            self.push_underlay_command(command, target.clone());
        }
        if width == 0 || height == 0 {
            return;
        }
        for (border_x, border_y, border_width, border_height) in [
            (x, y, width, 1),
            (x, y.saturating_add(height.saturating_sub(1)), width, 1),
            (x, y, 1, height),
            (x.saturating_add(width.saturating_sub(1)), y, 1, height),
        ] {
            if let Some(command) = self.clipped_rect_command(
                border_x,
                border_y,
                border_width,
                border_height,
                INLINE_WIDGET_BORDER_SHADE,
            ) {
                self.push_underlay_command(command, target.clone());
            }
        }
    }

    fn break_line(&mut self) {
        self.soft_break_opportunity = false;
        self.pending_inter_word_space = None;
        let line_height = self.line_height.max(1);
        let text_row_height = self
            .current_runs
            .iter()
            .map(|run| run.font_scale.max(1))
            .max()
            .unwrap_or(1);
        let row_height = line_height
            .max(self.inline_replaced_height)
            .max(text_row_height);
        if !self.current_runs.is_empty() {
            let y = self.next_y;
            let align_offset = self
                .text_align
                .offset(self.available_width(), self.current_width);
            let mut text = String::new();
            let runs = std::mem::take(&mut self.current_runs);
            for run in runs {
                let run_width = text_cell_width(&run.text, run.font_scale);
                if run.start_x > text.chars().count() {
                    text.push_str(&" ".repeat(run.start_x.saturating_sub(text.chars().count())));
                }
                let run_x = self
                    .box_x()
                    .saturating_add(align_offset)
                    .saturating_add(run.start_x);
                let run_height = run.font_scale.max(1);
                let row_align_offset = match self.row_align_items {
                    AlignItems::Start => 0,
                    AlignItems::Center => row_height.saturating_sub(run_height) / 2,
                    AlignItems::End | AlignItems::Baseline => row_height.saturating_sub(run_height),
                };
                let run_y = y.saturating_add(row_align_offset);
                let (background_y, background_height) = match self.row_align_items {
                    AlignItems::Start => (y, row_height),
                    AlignItems::Center | AlignItems::End | AlignItems::Baseline => {
                        (run_y, run_height)
                    }
                };
                if run.visible {
                    if let Some(background_shade) = run.background_shade
                        && let Some(command) = self.clipped_rect_command(
                            run_x,
                            background_y,
                            run_width,
                            background_height,
                            background_shade,
                        )
                    {
                        let target = self.node_hit_target(None);
                        self.push_underlay_command(command, target);
                    }
                    if run.link_underline
                        && run_width > 0
                        && let Some(command) = self.clipped_rect_command(
                            run_x,
                            run_y.saturating_add(run_height.saturating_sub(1)),
                            run_width,
                            1,
                            LINK_UNDERLINE_SHADE,
                        )
                    {
                        let target = self.text_hit_target(run.target_runs.clone());
                        self.push_underlay_command(command, target);
                    }
                    text.push_str(&scaled_text_for_line(&run.text, run.font_scale));
                    self.push_text_display_command(
                        run_x,
                        run_y,
                        run.text,
                        run.shade,
                        run.font_scale,
                        run.target_runs,
                    );
                } else {
                    text.push_str(&" ".repeat(run_width));
                }
            }
            self.lines.push(text);
            self.push_line_height_gaps(row_height);
            self.next_y = self.next_y.saturating_add(row_height);
        } else if self.inline_replaced_height > 0 {
            self.next_y = self.next_y.saturating_add(self.inline_replaced_height);
        }
        self.current_width = 0;
        self.inline_replaced_height = 0;
    }

    fn force_line_break(&mut self) {
        self.soft_break_opportunity = false;
        self.pending_inter_word_space = None;
        let line_height = self.line_height.max(1);
        if self.current_runs.is_empty() && self.inline_replaced_height == 0 {
            self.lines.push(String::new());
            self.push_line_height_gaps(line_height);
            self.next_y = self.next_y.saturating_add(line_height);
            self.current_width = 0;
            return;
        }
        self.break_line();
    }

    fn push_line_height_gaps(&mut self, line_height: usize) {
        for _ in 1..line_height {
            self.lines.push(String::new());
        }
    }

    fn push_text_run_piece(&mut self, piece: &str, target_node: Option<usize>) {
        self.push_text_run_piece_with_spacing(piece, target_node, true);
    }

    fn push_text_run_piece_unspaced(&mut self, piece: &str, target_node: Option<usize>) {
        self.push_text_run_piece_with_spacing(piece, target_node, false);
    }

    fn push_text_run_piece_with_spacing(
        &mut self,
        piece: &str,
        target_node: Option<usize>,
        apply_letter_spacing: bool,
    ) {
        if piece.is_empty() {
            return;
        }
        if apply_letter_spacing && self.current_width == 0 {
            self.push_line_start_spacing();
        }
        let piece = self.transform_text_piece(piece);
        let piece = if apply_letter_spacing {
            self.apply_letter_spacing(&piece)
        } else {
            piece
        };
        let piece = piece.as_str();
        let run_start = self.current_width;
        let piece_width = text_cell_width(piece, self.font_scale);
        self.current_width = self.current_width.saturating_add(piece_width);
        let visible = self.paint_visible();
        let link_underline = self.link_text_active();
        if let Some(last) = self.current_runs.last_mut()
            && last.shade == self.text_shade
            && last.font_scale == self.font_scale
            && last.background_shade == self.text_background_shade
            && last.link_underline == link_underline
            && last.visible == visible
            && last
                .start_x
                .saturating_add(text_cell_width(&last.text, last.font_scale))
                == run_start
        {
            let start = text_cell_width(&last.text, last.font_scale);
            last.text.push_str(piece);
            push_text_hit_target_run(
                &mut last.target_runs,
                start,
                piece,
                target_node,
                self.font_scale,
            );
            return;
        }
        let mut target_runs = Vec::new();
        push_text_hit_target_run(&mut target_runs, 0, piece, target_node, self.font_scale);
        self.current_runs.push(TextRun {
            start_x: run_start,
            text: piece.to_owned(),
            shade: self.text_shade,
            font_scale: self.font_scale,
            background_shade: self.text_background_shade,
            link_underline,
            visible,
            target_runs,
        });
    }

    fn push_pending_text_indent(&mut self) {
        let Some(indent) = self.pending_text_indent.take() else {
            return;
        };
        if indent > 0 {
            self.push_fixed_space_width(indent, None);
        }
    }

    fn push_line_start_spacing(&mut self) {
        if self.line_start_indent > 0 {
            self.push_fixed_space_width(self.line_start_indent, None);
        }
        self.push_pending_text_indent();
    }

    fn push_fixed_space_width(&mut self, width: usize, target_node: Option<usize>) {
        if width == 0 {
            return;
        }
        let run_start = self.current_width;
        self.current_width = self.current_width.saturating_add(width);
        let visible = self.paint_visible();
        let active_font_scale = self.font_scale.max(1);
        let font_scale = if width % active_font_scale == 0 {
            active_font_scale
        } else {
            1
        };
        let piece = " ".repeat(width.div_ceil(font_scale));
        let link_underline = self.link_text_active();
        if let Some(last) = self.current_runs.last_mut()
            && last.shade == self.text_shade
            && last.font_scale == font_scale
            && last.background_shade == self.text_background_shade
            && last.link_underline == link_underline
            && last.visible == visible
            && last
                .start_x
                .saturating_add(text_cell_width(&last.text, last.font_scale))
                == run_start
        {
            let start = text_cell_width(&last.text, last.font_scale);
            last.text.push_str(&piece);
            push_text_hit_target_run(
                &mut last.target_runs,
                start,
                &piece,
                target_node,
                font_scale,
            );
            return;
        }
        let mut target_runs = Vec::new();
        push_text_hit_target_run(&mut target_runs, 0, &piece, target_node, font_scale);
        self.current_runs.push(TextRun {
            start_x: run_start,
            text: piece,
            shade: self.text_shade,
            font_scale,
            background_shade: self.text_background_shade,
            link_underline,
            visible,
            target_runs,
        });
    }

    fn enter_positioning_context(&mut self, y: usize, height: Option<usize>) {
        self.positioning_context_stack
            .push(FlowPositioningContext { y, height });
    }

    fn exit_positioning_context(&mut self) {
        self.positioning_context_stack.pop();
    }

    fn current_positioning_context_y(&self) -> usize {
        self.positioning_context_stack
            .last()
            .map(|context| context.y)
            .unwrap_or(0)
    }

    fn current_positioning_context_height(&self) -> Option<usize> {
        self.positioning_context_stack
            .last()
            .and_then(|context| context.height)
    }

    fn enter_viewport_fixed(&mut self) {
        self.viewport_fixed_depth = self.viewport_fixed_depth.saturating_add(1);
    }

    fn exit_viewport_fixed(&mut self) {
        self.viewport_fixed_depth = self.viewport_fixed_depth.saturating_sub(1);
    }

    fn viewport_fixed(&self) -> bool {
        self.viewport_fixed_depth > 0
    }

    fn enter_viewport_sticky(&mut self, top: usize) {
        self.viewport_sticky_top_stack.push(top);
    }

    fn exit_viewport_sticky(&mut self) {
        self.viewport_sticky_top_stack.pop();
    }

    fn viewport_sticky_top(&self) -> Option<usize> {
        self.viewport_sticky_top_stack.last().copied()
    }

    fn enter_positive_z_layer(&mut self, z_index: i32) {
        self.positive_z_layer_stack.push(z_index);
    }

    fn exit_positive_z_layer(&mut self) {
        self.positive_z_layer_stack.pop();
    }

    fn enter_link_text(&mut self) {
        self.link_text_depth = self.link_text_depth.saturating_add(1);
    }

    fn exit_link_text(&mut self) {
        self.link_text_depth = self.link_text_depth.saturating_sub(1);
    }

    fn link_text_active(&self) -> bool {
        self.link_text_depth > 0
    }

    fn node_hit_target(&self, target_node: Option<usize>) -> DisplayHitTarget {
        DisplayHitTarget::node(target_node)
            .with_viewport_fixed(self.viewport_fixed())
            .with_viewport_sticky_top(
                (!self.viewport_fixed())
                    .then(|| self.viewport_sticky_top())
                    .flatten(),
            )
    }

    fn text_hit_target(&self, target_runs: Vec<TextHitTargetRun>) -> DisplayHitTarget {
        DisplayHitTarget::text(target_runs)
            .with_viewport_fixed(self.viewport_fixed())
            .with_viewport_sticky_top(
                (!self.viewport_fixed())
                    .then(|| self.viewport_sticky_top())
                    .flatten(),
            )
    }

    fn enter_out_of_flow(&mut self, y: Option<usize>) -> FlowOutOfFlowSnapshot {
        let snapshot = FlowOutOfFlowSnapshot {
            lines_len: self.lines.len(),
            current_runs: std::mem::take(&mut self.current_runs),
            current_width: std::mem::take(&mut self.current_width),
            inline_replaced_height: std::mem::take(&mut self.inline_replaced_height),
            pending_inter_word_space: self.pending_inter_word_space.take(),
            soft_break_opportunity: std::mem::take(&mut self.soft_break_opportunity),
            next_y: self.next_y,
            pending_text_indent: self.pending_text_indent.take(),
            line_start_indent: self.line_start_indent,
            table_stack: std::mem::take(&mut self.table_stack),
            active_floats: std::mem::take(&mut self.active_floats),
        };
        if let Some(y) = y {
            self.next_y = y;
        }
        snapshot
    }

    fn enter_horizontal_projection(
        &mut self,
        offset: isize,
    ) -> Option<FlowHorizontalProjectionSnapshot> {
        if offset == 0 {
            return None;
        }
        let snapshot = FlowHorizontalProjectionSnapshot {
            left_inset: self.left_inset,
            right_inset: self.right_inset,
        };
        if offset > 0 {
            self.left_inset = self.left_inset.saturating_add(offset as usize);
        } else {
            self.left_inset = self.left_inset.saturating_sub(offset.unsigned_abs());
        }
        Some(snapshot)
    }

    fn exit_horizontal_projection(&mut self, snapshot: FlowHorizontalProjectionSnapshot) {
        self.break_line();
        self.left_inset = snapshot.left_inset;
        self.right_inset = snapshot.right_inset;
    }

    fn enter_vertical_projection(
        &mut self,
        offset: isize,
    ) -> Option<FlowVerticalProjectionSnapshot> {
        if offset == 0 {
            return None;
        }
        let snapshot = FlowVerticalProjectionSnapshot {
            offset: self.vertical_projection_offset,
        };
        self.vertical_projection_offset = self.vertical_projection_offset.saturating_add(offset);
        Some(snapshot)
    }

    fn exit_vertical_projection(&mut self, snapshot: FlowVerticalProjectionSnapshot) {
        self.break_line();
        self.vertical_projection_offset = snapshot.offset;
    }

    fn exit_out_of_flow(&mut self, snapshot: FlowOutOfFlowSnapshot) {
        self.break_line();
        self.lines.truncate(snapshot.lines_len);
        self.current_runs = snapshot.current_runs;
        self.current_width = snapshot.current_width;
        self.inline_replaced_height = snapshot.inline_replaced_height;
        self.pending_inter_word_space = snapshot.pending_inter_word_space;
        self.soft_break_opportunity = snapshot.soft_break_opportunity;
        self.next_y = snapshot.next_y;
        self.pending_text_indent = snapshot.pending_text_indent;
        self.line_start_indent = snapshot.line_start_indent;
        self.table_stack = snapshot.table_stack;
        self.active_floats = snapshot.active_floats;
    }

    fn effective_current_width(&self) -> usize {
        self.current_width
            .saturating_add(self.pending_text_indent.unwrap_or(0))
    }

    fn letter_spaced_text_width(&self, text: &str) -> usize {
        self.letter_spaced_char_count_width(text.chars().count())
    }

    fn letter_spaced_char_count_width(&self, char_count: usize) -> usize {
        let gap_units = self
            .letter_spacing
            .saturating_mul(char_count.saturating_sub(1));
        char_count
            .saturating_add(gap_units / CSS_TEXT_CELL_UNITS)
            .saturating_mul(self.font_scale.max(1))
    }

    fn apply_letter_spacing(&self, text: &str) -> String {
        if self.letter_spacing == 0 {
            return text.to_owned();
        }
        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return String::new();
        };
        let mut spaced = String::with_capacity(self.letter_spaced_text_width(text));
        spaced.push(first);
        let mut pending_units = 0usize;
        for ch in chars {
            pending_units = pending_units.saturating_add(self.letter_spacing);
            let gap_cells = pending_units / CSS_TEXT_CELL_UNITS;
            pending_units %= CSS_TEXT_CELL_UNITS;
            if gap_cells > 0 {
                spaced.push_str(&" ".repeat(gap_cells));
            }
            spaced.push(ch);
        }
        spaced
    }

    fn transform_text_piece(&mut self, piece: &str) -> String {
        match self.text_transform {
            TextTransform::None => piece.to_owned(),
            TextTransform::Uppercase => piece.to_ascii_uppercase(),
            TextTransform::Lowercase => piece.to_ascii_lowercase(),
            TextTransform::Capitalize => self.capitalize_text_piece(piece),
        }
    }

    fn capitalize_text_piece(&mut self, piece: &str) -> String {
        let mut transformed = String::with_capacity(piece.len());
        for ch in piece.chars() {
            if ch.is_ascii_alphanumeric() {
                if self.text_transform_capitalize_next && ch.is_ascii_alphabetic() {
                    transformed.push(ch.to_ascii_uppercase());
                } else {
                    transformed.push(ch);
                }
                self.text_transform_capitalize_next = false;
            } else {
                transformed.push(ch);
                self.text_transform_capitalize_next = true;
            }
        }
        transformed
    }

    fn enter_text_shade(&mut self, shade: u8) {
        self.text_shade_stack.push(self.text_shade);
        self.text_shade = shade;
    }

    fn exit_text_shade(&mut self) {
        self.text_shade = self.text_shade_stack.pop().unwrap_or(0);
    }

    fn enter_text_background_shade(&mut self, shade: u8) {
        self.text_background_shade_stack
            .push(self.text_background_shade);
        self.text_background_shade = Some(shade);
    }

    fn exit_text_background_shade(&mut self) {
        self.text_background_shade = self.text_background_shade_stack.pop().unwrap_or(None);
    }

    fn enter_text_align(&mut self, align: TextAlign) {
        self.text_align_stack.push(self.text_align);
        self.text_align = align;
    }

    fn exit_text_align(&mut self) {
        self.text_align = self.text_align_stack.pop().unwrap_or(TextAlign::Start);
    }

    fn enter_visibility(&mut self, visibility: Visibility) {
        self.visibility_stack.push(self.visibility);
        self.visibility = visibility;
    }

    fn exit_visibility(&mut self) {
        self.visibility = self.visibility_stack.pop().unwrap_or(Visibility::Visible);
    }

    fn enter_transparent_opacity(&mut self) {
        self.transparent_opacity_depth = self.transparent_opacity_depth.saturating_add(1);
    }

    fn exit_transparent_opacity(&mut self) {
        self.transparent_opacity_depth = self.transparent_opacity_depth.saturating_sub(1);
    }

    fn paint_visible(&self) -> bool {
        self.visibility == Visibility::Visible && self.transparent_opacity_depth == 0
    }

    fn enter_white_space(&mut self, white_space: WhiteSpace) {
        self.white_space_stack.push(self.white_space);
        self.white_space = white_space;
    }

    fn exit_white_space(&mut self) {
        self.white_space = self.white_space_stack.pop().unwrap_or(WhiteSpace::Normal);
    }

    fn enter_text_transform(&mut self, text_transform: TextTransform) {
        self.text_transform_stack.push(self.text_transform);
        self.text_transform_capitalize_next_stack
            .push(self.text_transform_capitalize_next);
        self.text_transform = text_transform;
        if self.text_transform == TextTransform::Capitalize {
            self.text_transform_capitalize_next = true;
        }
    }

    fn exit_text_transform(&mut self) {
        self.text_transform = self
            .text_transform_stack
            .pop()
            .unwrap_or(TextTransform::None);
        self.text_transform_capitalize_next = self
            .text_transform_capitalize_next_stack
            .pop()
            .unwrap_or(true);
    }

    fn enter_letter_spacing(&mut self, letter_spacing: usize) {
        self.letter_spacing_stack.push(self.letter_spacing);
        self.letter_spacing = letter_spacing;
    }

    fn exit_letter_spacing(&mut self) {
        self.letter_spacing = self.letter_spacing_stack.pop().unwrap_or(0);
    }

    fn enter_word_spacing(&mut self, word_spacing: usize) {
        self.word_spacing_stack.push(self.word_spacing);
        self.word_spacing = word_spacing;
    }

    fn exit_word_spacing(&mut self) {
        self.word_spacing = self.word_spacing_stack.pop().unwrap_or(0);
    }

    fn enter_overflow_wrap(&mut self, overflow_wrap: OverflowWrap) {
        self.overflow_wrap_stack.push(self.overflow_wrap);
        self.overflow_wrap = overflow_wrap;
    }

    fn exit_overflow_wrap(&mut self) {
        self.overflow_wrap = self
            .overflow_wrap_stack
            .pop()
            .unwrap_or(OverflowWrap::Normal);
    }

    fn enter_word_break(&mut self, word_break: WordBreak) {
        self.word_break_stack.push(self.word_break);
        self.word_break = word_break;
    }

    fn exit_word_break(&mut self) {
        self.word_break = self.word_break_stack.pop().unwrap_or(WordBreak::Normal);
    }

    fn enter_text_indent(&mut self, text_indent: usize) {
        self.text_indent_stack.push(self.text_indent);
        self.text_indent = text_indent;
    }

    fn exit_text_indent(&mut self) {
        self.text_indent = self.text_indent_stack.pop().unwrap_or(0);
    }

    fn enter_block_text_indent(&mut self) {
        self.pending_text_indent_stack
            .push(self.pending_text_indent.take());
        self.pending_text_indent = (self.text_indent > 0).then_some(self.text_indent);
    }

    fn exit_block_text_indent(&mut self) {
        self.pending_text_indent = self.pending_text_indent_stack.pop().unwrap_or(None);
    }

    fn enter_line_start_indent(&mut self, indent: usize) {
        self.line_start_indent_stack.push(self.line_start_indent);
        self.line_start_indent = self.line_start_indent.saturating_add(indent);
    }

    fn exit_line_start_indent(&mut self) {
        self.line_start_indent = self.line_start_indent_stack.pop().unwrap_or(0);
    }

    fn enter_line_height(&mut self, line_height: usize) {
        self.line_height_stack.push(self.line_height);
        self.line_height = line_height.max(1);
    }

    fn exit_line_height(&mut self) {
        self.line_height = self.line_height_stack.pop().unwrap_or(1);
    }

    fn enter_font_scale(&mut self, font_scale: usize) {
        self.font_scale_stack.push(self.font_scale);
        self.font_scale = font_scale.clamp(1, 4);
    }

    fn exit_font_scale(&mut self) {
        self.font_scale = self.font_scale_stack.pop().unwrap_or(1);
    }

    fn enter_row_align_items(&mut self, align_items: AlignItems) {
        self.row_align_items_stack.push(self.row_align_items);
        self.row_align_items = align_items;
    }

    fn exit_row_align_items(&mut self) {
        self.row_align_items = self.row_align_items_stack.pop().unwrap_or_default();
    }

    fn enter_table(
        &mut self,
        column_widths: Vec<usize>,
        column_gap: usize,
        row_gap: usize,
        row_count: usize,
    ) {
        self.table_stack.push(TableFlow {
            column_widths,
            column_gap,
            row_gap,
            remaining_rows: row_count,
            row_stack: Vec::new(),
            rowspans: Vec::new(),
        });
    }

    fn exit_table(&mut self) {
        self.table_stack.pop();
    }

    fn enter_table_row(&mut self, cell_count: usize) {
        if let Some(table) = self.table_stack.last_mut() {
            table.row_stack.push(TableRowFlow {
                remaining_cells: cell_count,
                next_column_index: 0,
                active_cell: None,
            });
        }
    }

    fn exit_table_row(&mut self) {
        let row_gap = if let Some(table) = self.table_stack.last_mut() {
            table.row_stack.pop();
            decrement_table_rowspans(&mut table.rowspans);
            table.remaining_rows = table.remaining_rows.saturating_sub(1);
            (table.remaining_rows > 0).then_some(table.row_gap)
        } else {
            None
        };
        if let Some(row_gap) = row_gap
            && row_gap > 0
        {
            self.push_vertical_space(row_gap);
        }
    }

    fn enter_table_cell(
        &mut self,
        colspan: usize,
        rowspan: usize,
        background_shade: Option<u8>,
        target_node: Option<usize>,
    ) {
        let skipped_padding = {
            let Some(table) = self.table_stack.last_mut() else {
                return;
            };
            let Some(row) = table.row_stack.last_mut() else {
                return;
            };
            if row.active_cell.is_some() {
                return;
            }
            let start_column = row.next_column_index;
            while table
                .rowspans
                .get(row.next_column_index)
                .copied()
                .unwrap_or(0)
                > 0
            {
                row.next_column_index = row.next_column_index.saturating_add(1);
            }
            let skipped_columns = row.next_column_index.saturating_sub(start_column);
            let start_width = self
                .current_width
                .saturating_add(table_skipped_column_padding(
                    &table.column_widths,
                    table.column_gap,
                    start_column,
                    skipped_columns,
                ));
            row.active_cell = Some(TableCellFlow {
                column_index: row.next_column_index,
                colspan: colspan.clamp(1, 16),
                rowspan: rowspan.clamp(1, 16),
                start_width,
                start_y: self.next_y,
                background_shade,
                target_node,
            });
            table_skipped_column_padding(
                &table.column_widths,
                table.column_gap,
                start_column,
                skipped_columns,
            )
        };
        if skipped_padding > 0 {
            self.push_text_run_piece_unspaced(&" ".repeat(skipped_padding), None);
        }
    }

    fn exit_table_cell(&mut self) {
        let box_x = self.box_x();
        let text_row_height = self
            .current_runs
            .iter()
            .map(|run| run.font_scale.max(1))
            .max()
            .unwrap_or(1);
        let row_height = self
            .line_height
            .max(1)
            .max(self.inline_replaced_height)
            .max(text_row_height);
        let (padding, background) = {
            let Some(table) = self.table_stack.last_mut() else {
                return;
            };
            let Some(row) = table.row_stack.last_mut() else {
                return;
            };
            let Some(active_cell) = row.active_cell.take() else {
                return;
            };
            let cell_width = self.current_width.saturating_sub(active_cell.start_width);
            let column_width = table_spanned_column_width(
                &table.column_widths,
                table.column_gap,
                active_cell,
                cell_width,
            );
            let next_column_index = active_cell.column_index.saturating_add(active_cell.colspan);
            if table.rowspans.len() < next_column_index {
                table.rowspans.resize(next_column_index, 0);
            }
            for rowspan in &mut table.rowspans[active_cell.column_index..next_column_index] {
                *rowspan = (*rowspan).max(active_cell.rowspan);
            }
            let has_next_cell = row.remaining_cells > 1;
            row.remaining_cells = row.remaining_cells.saturating_sub(1);
            row.next_column_index = next_column_index;
            let background = active_cell.background_shade.map(|shade| {
                (
                    box_x.saturating_add(active_cell.start_width),
                    active_cell.start_y,
                    column_width.max(cell_width),
                    row_height,
                    shade,
                    active_cell.target_node,
                )
            });
            (
                has_next_cell.then_some(
                    column_width
                        .saturating_sub(cell_width)
                        .saturating_add(table.column_gap.saturating_sub(1)),
                ),
                background,
            )
        };
        if let Some((x, y, width, height, shade, target_node)) = background
            && self.paint_visible()
            && let Some(command) = self.clipped_rect_command(x, y, width, height, shade)
        {
            let target = self.node_hit_target(target_node);
            self.push_underlay_command(command, target);
        }
        if let Some(padding) = padding
            && padding > 0
        {
            self.push_text_run_piece_unspaced(&" ".repeat(padding), None);
        }
    }

    fn push_horizontal_rule(&mut self, target_node: Option<usize>) {
        self.break_line();
        if self.paint_visible() {
            if let Some(command) = self.clipped_rect_command(0, self.next_y, self.width, 1, 96) {
                let target = self.node_hit_target(target_node);
                self.push_display_command(command, target);
            }
        }
        self.next_y += 1;
    }

    fn push_inline_svg_placeholder(
        &mut self,
        width: usize,
        height: usize,
        shapes: &[SvgPaintShape],
        fallback_rgb: Option<(u8, u8, u8)>,
        shade: u8,
        target_node: Option<usize>,
    ) {
        self.soft_break_opportunity = false;
        let rect_width = width.min(self.available_width()).max(1);
        let rect_height = height.max(1);
        let pending_space = if self.current_width > 0 {
            self.pending_inter_word_space.take().unwrap_or(0)
        } else {
            self.pending_inter_word_space = None;
            0
        };
        if self.current_width > 0
            && self
                .effective_current_width()
                .saturating_add(pending_space)
                .saturating_add(rect_width)
                > self.available_width()
        {
            self.break_line();
        } else if pending_space > 0 {
            self.current_width = self.current_width.saturating_add(pending_space);
        }
        if self.current_width == 0 {
            self.push_line_start_spacing();
        }
        if self.current_width > 0
            && self.current_width.saturating_add(rect_width) > self.available_width()
        {
            self.break_line();
        }
        let remaining_width = self.available_width().saturating_sub(self.current_width);
        let rect_width = rect_width.min(remaining_width.max(1));
        let x = self.box_x().saturating_add(self.current_width);
        self.push_svg_paint_commands(
            x,
            self.next_y,
            rect_width,
            rect_height,
            shapes,
            fallback_rgb,
            shade,
            target_node,
        );
        self.current_width = self.current_width.saturating_add(rect_width);
        self.inline_replaced_height = self.inline_replaced_height.max(rect_height);
    }

    fn push_svg_placeholder(
        &mut self,
        width: usize,
        height: usize,
        shapes: &[SvgPaintShape],
        fallback_rgb: Option<(u8, u8, u8)>,
        shade: u8,
        target_node: Option<usize>,
    ) {
        self.break_line();
        let rect_width = width.min(self.available_width()).max(1);
        let rect_height = height.max(1);
        self.push_svg_paint_commands(
            self.left_inset,
            self.next_y,
            rect_width,
            rect_height,
            shapes,
            fallback_rgb,
            shade,
            target_node,
        );
        self.next_y = self.next_y.saturating_add(rect_height);
    }

    fn push_svg_paint_commands(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        shapes: &[SvgPaintShape],
        fallback_rgb: Option<(u8, u8, u8)>,
        shade: u8,
        target_node: Option<usize>,
    ) {
        if !self.paint_visible() {
            return;
        }
        let target = self.node_hit_target(target_node);
        if shapes.is_empty() {
            let command = if let Some((red, green, blue)) = fallback_rgb {
                self.clipped_color_rect_command(x, y, width, height, red, green, blue)
            } else {
                self.clipped_rect_command(x, y, width, height, shade)
            };
            if let Some(command) = command {
                self.push_display_command(command, target);
            }
            return;
        }
        for shape in shapes {
            let shape_x = x.saturating_add(shape.x.min(width.saturating_sub(1)));
            let shape_y = y.saturating_add(shape.y.min(height.saturating_sub(1)));
            let shape_width = shape.width.min(width.saturating_sub(shape.x).max(1));
            let shape_height = shape.height.min(height.saturating_sub(shape.y).max(1));
            if let Some(command) = self.clipped_color_rect_command(
                shape_x,
                shape_y,
                shape_width,
                shape_height,
                shape.red,
                shape.green,
                shape.blue,
            ) {
                self.push_display_command(command, target.clone());
            }
        }
    }

    fn push_inline_image_placeholder(
        &mut self,
        width: usize,
        height: usize,
        alt: Option<String>,
        source: &str,
        url: Option<&str>,
        target_node: Option<usize>,
    ) {
        self.soft_break_opportunity = false;
        let decoded_info = url.and_then(|url| self.cached_decoded_image_info(source, url));
        let placeholder_height = if decoded_info.is_some() {
            height.max(1)
        } else {
            height.min(MAX_UNRESOLVED_IMAGE_PLACEHOLDER_HEIGHT).max(1)
        };
        let image_width = width.min(self.available_width()).max(1);
        let pending_space = if self.current_width > 0 {
            self.pending_inter_word_space.take().unwrap_or(0)
        } else {
            self.pending_inter_word_space = None;
            0
        };
        if self.current_width > 0
            && self
                .effective_current_width()
                .saturating_add(pending_space)
                .saturating_add(image_width)
                > self.available_width()
        {
            self.break_line();
        } else if pending_space > 0 {
            self.current_width = self.current_width.saturating_add(pending_space);
        }
        if self.current_width == 0 {
            self.push_line_start_spacing();
        }
        if self.current_width > 0
            && self.current_width.saturating_add(image_width) > self.available_width()
        {
            self.break_line();
        }
        let remaining_width = self.available_width().saturating_sub(self.current_width);
        let image_width = image_width.min(remaining_width.max(1));
        if self.paint_visible() {
            let command = DisplayCommand::Image {
                x: self.box_x().saturating_add(self.current_width),
                y: self.next_y,
                width: image_width,
                height: placeholder_height,
                shade: 220,
                alt,
                url: decoded_info.as_ref().map(|image| image.url.clone()),
                decoded_width: decoded_info.as_ref().map(|image| image.width),
                decoded_height: decoded_info.as_ref().map(|image| image.height),
                decoded_hash: decoded_info.map(|image| image.pixel_hash),
            };
            if let Some((command, source_bounds)) = self.clipped_image_command(command) {
                let target = self
                    .node_hit_target(target_node)
                    .with_source_bounds(source_bounds);
                self.push_display_command(command, target);
            }
        }
        self.current_width = self.current_width.saturating_add(image_width);
        self.inline_replaced_height = self.inline_replaced_height.max(placeholder_height);
    }

    fn push_image_placeholder(
        &mut self,
        width: usize,
        height: usize,
        alt: Option<String>,
        source: &str,
        url: Option<&str>,
        target_node: Option<usize>,
    ) {
        self.break_line();
        let decoded_info = url.and_then(|url| self.cached_decoded_image_info(source, url));
        let placeholder_height = if decoded_info.is_some() {
            height.max(1)
        } else {
            height.min(MAX_UNRESOLVED_IMAGE_PLACEHOLDER_HEIGHT).max(1)
        };
        if self.paint_visible() {
            let command = DisplayCommand::Image {
                x: self.left_inset,
                y: self.next_y,
                width: width.min(self.available_width()).max(1),
                height: placeholder_height,
                shade: 220,
                alt,
                url: decoded_info.as_ref().map(|image| image.url.clone()),
                decoded_width: decoded_info.as_ref().map(|image| image.width),
                decoded_height: decoded_info.as_ref().map(|image| image.height),
                decoded_hash: decoded_info.map(|image| image.pixel_hash),
            };
            if let Some((command, source_bounds)) = self.clipped_image_command(command) {
                let target = self
                    .node_hit_target(target_node)
                    .with_source_bounds(source_bounds);
                self.push_display_command(command, target);
            }
        }
        self.next_y = self.next_y.saturating_add(placeholder_height);
    }

    fn push_floating_image_placeholder(
        &mut self,
        side: FloatSide,
        width: usize,
        height: usize,
        alt: Option<String>,
        source: &str,
        url: Option<&str>,
        target_node: Option<usize>,
    ) {
        self.break_line();
        let (left_float, right_float) = self.active_float_offsets();
        let available_width = self
            .width
            .saturating_sub(self.left_inset)
            .saturating_sub(self.right_inset)
            .saturating_sub(left_float)
            .saturating_sub(right_float)
            .max(1);
        let image_width = width.min(available_width).max(1);
        let image_height = height.max(1);
        let x = match side {
            FloatSide::Left => self.left_inset.saturating_add(left_float),
            FloatSide::Right => self
                .left_inset
                .saturating_add(left_float)
                .saturating_add(available_width.saturating_sub(image_width)),
        };
        let y = self.next_y;
        let decoded_info = url.and_then(|url| self.cached_decoded_image_info(source, url));
        if self.paint_visible() {
            let command = DisplayCommand::Image {
                x,
                y,
                width: image_width,
                height: image_height,
                shade: 220,
                alt,
                url: decoded_info.as_ref().map(|image| image.url.clone()),
                decoded_width: decoded_info.as_ref().map(|image| image.width),
                decoded_height: decoded_info.as_ref().map(|image| image.height),
                decoded_hash: decoded_info.map(|image| image.pixel_hash),
            };
            if let Some((command, source_bounds)) = self.clipped_image_command(command) {
                let target = self
                    .node_hit_target(target_node)
                    .with_source_bounds(source_bounds);
                self.push_display_command(command, target);
            }
        }
        self.active_floats.push(ActiveFloat {
            side,
            width: image_width,
            bottom_y: y.saturating_add(image_height),
        });
    }

    fn cached_decoded_image_info(&mut self, source: &str, url: &str) -> Option<DecodedImageInfo> {
        let resolved = resolve_browser_href(source, url);
        for key in [url, resolved.as_str()] {
            if let Some(index) = self.decoded_image_cache.get(key) {
                return index
                    .and_then(|index| self.decoded_images.get(index).map(DecodedImageEntry::info));
            }
        }

        let Some(decoded) = decoded_image_entry(source, url) else {
            self.decoded_image_cache.insert(url.to_owned(), None);
            self.decoded_image_cache.insert(resolved, None);
            return None;
        };
        let info = decoded.info();
        let index = self.decoded_images.len();
        self.decoded_images.push(decoded);
        self.decoded_image_cache.insert(url.to_owned(), Some(index));
        self.decoded_image_cache.insert(resolved, Some(index));
        Some(info)
    }

    fn current_row(&self) -> usize {
        self.next_y
    }

    fn current_inline_width(&self) -> usize {
        self.current_width
    }

    fn cap_inline_replaced_height(&mut self, height: usize) {
        if self.inline_replaced_height > 0 {
            self.inline_replaced_height = self.inline_replaced_height.min(height.max(1));
        }
    }

    fn box_x(&self) -> usize {
        let (left_float, _) = self.active_float_offsets();
        self.left_inset.saturating_add(left_float)
    }

    fn available_width(&self) -> usize {
        let (left_float, right_float) = self.active_float_offsets();
        self.width
            .saturating_sub(self.left_inset)
            .saturating_sub(self.right_inset)
            .saturating_sub(left_float)
            .saturating_sub(right_float)
            .max(1)
    }

    fn clear_narrow_float_column_for_text(&mut self) {
        if self.current_width > 0 || self.inline_replaced_height > 0 {
            return;
        }
        let Some(clear_y) = self.next_active_float_bottom_y() else {
            return;
        };
        if self.available_width() >= self.minimum_readable_float_text_width() {
            return;
        }
        self.next_y = clear_y.max(self.next_y);
    }

    fn clear_floats(&mut self, clear: ClearSide) {
        if let Some(clear_y) = self.next_active_float_bottom_y_for_clear(clear) {
            self.next_y = clear_y.max(self.next_y);
        }
    }

    fn minimum_readable_float_text_width(&self) -> usize {
        self.width.saturating_div(4).clamp(8, 16)
    }

    fn viewport_width_css_px(&self) -> usize {
        self.width.saturating_mul(8)
    }

    fn active_float_offsets(&self) -> (usize, usize) {
        let mut left = 0usize;
        let mut right = 0usize;
        for active_float in &self.active_floats {
            if self.next_y >= active_float.bottom_y {
                continue;
            }
            match active_float.side {
                FloatSide::Left => left = left.saturating_add(active_float.width),
                FloatSide::Right => right = right.saturating_add(active_float.width),
            }
        }
        (left, right)
    }

    fn next_active_float_bottom_y(&self) -> Option<usize> {
        self.active_floats
            .iter()
            .filter(|active_float| self.next_y < active_float.bottom_y)
            .map(|active_float| active_float.bottom_y)
            .min()
    }

    fn next_active_float_bottom_y_for_clear(&self, clear: ClearSide) -> Option<usize> {
        self.active_floats
            .iter()
            .filter(|active_float| self.next_y < active_float.bottom_y)
            .filter(|active_float| match clear {
                ClearSide::Left => active_float.side == FloatSide::Left,
                ClearSide::Right => active_float.side == FloatSide::Right,
                ClearSide::Both => true,
            })
            .map(|active_float| active_float.bottom_y)
            .max()
    }

    fn enter_inset(&mut self, border_width: usize) {
        self.enter_insets(border_width, border_width);
    }

    fn exit_inset(&mut self, border_width: usize) {
        self.exit_insets(border_width, border_width);
    }

    fn enter_insets(&mut self, left: usize, right: usize) {
        self.left_inset = self.left_inset.saturating_add(left);
        self.right_inset = self.right_inset.saturating_add(right);
    }

    fn exit_insets(&mut self, left: usize, right: usize) {
        self.left_inset = self.left_inset.saturating_sub(left);
        self.right_inset = self.right_inset.saturating_sub(right);
    }

    fn push_vertical_space(&mut self, rows: usize) {
        self.break_line();
        self.next_y = self.next_y.saturating_add(rows);
    }

    fn ensure_current_row_at_least(&mut self, row: usize) {
        self.break_line();
        self.next_y = self.next_y.max(row);
    }

    fn set_current_row(&mut self, row: usize) {
        self.break_line();
        self.next_y = row;
    }

    fn cap_current_row(&mut self, row: usize) {
        self.break_line();
        self.next_y = self.next_y.min(row);
    }

    fn push_block_border_top(
        &mut self,
        x: usize,
        width: usize,
        border: BorderPaint,
        target_node: Option<usize>,
    ) {
        if self.paint_visible() {
            if let Some(command) =
                self.clipped_rect_command(x, self.next_y, width, border.width, border.shade)
            {
                let target = self.node_hit_target(target_node);
                self.push_border_command(command, target);
            }
        }
        self.next_y = self.next_y.saturating_add(border.width);
    }

    fn push_block_border_sides(
        &mut self,
        x: usize,
        width: usize,
        start_y: usize,
        height: usize,
        border: BorderPaint,
        target_node: Option<usize>,
    ) {
        if height == 0 {
            return;
        }
        if !self.paint_visible() {
            return;
        }
        let border_width = border.width.min(width);
        if let Some(command) =
            self.clipped_rect_command(x, start_y, border_width, height, border.shade)
        {
            let target = self.node_hit_target(target_node);
            self.push_border_command(command, target);
        }
        if width > border_width {
            if let Some(command) = self.clipped_rect_command(
                x.saturating_add(width.saturating_sub(border_width)),
                start_y,
                border_width,
                height,
                border.shade,
            ) {
                let target = self.node_hit_target(target_node);
                self.push_border_command(command, target);
            }
        }
    }

    fn push_block_border_bottom(
        &mut self,
        x: usize,
        width: usize,
        border: BorderPaint,
        target_node: Option<usize>,
    ) {
        if self.paint_visible() {
            if let Some(command) =
                self.clipped_rect_command(x, self.next_y, width, border.width, border.shade)
            {
                let target = self.node_hit_target(target_node);
                self.push_border_command(command, target);
            }
        }
        self.next_y = self.next_y.saturating_add(border.width);
    }

    fn insert_block_background(
        &mut self,
        insert: PaintUnderlayInsertion,
        x: usize,
        width: usize,
        start_y: usize,
        shade: u8,
        target_node: Option<usize>,
    ) {
        if !self.paint_visible() {
            return;
        }
        let height = self.next_y.saturating_sub(start_y).max(1);
        if let Some(command) = self.clipped_rect_command(x, start_y, width, height, shade) {
            let target = self.node_hit_target(target_node);
            self.insert_underlay_command(insert, command, target);
        }
    }

    fn insert_block_background_image(
        &mut self,
        insert: PaintUnderlayInsertion,
        x: usize,
        width: usize,
        start_y: usize,
        source: &str,
        url: &str,
        size: BackgroundImageSize,
        position: BackgroundImagePosition,
        repeat: BackgroundImageRepeat,
        target_node: Option<usize>,
    ) {
        if !self.paint_visible() {
            return;
        }
        let height = self.next_y.saturating_sub(start_y).max(1);
        let decoded_info = self.cached_decoded_image_info(source, url);
        let command = DisplayCommand::BackgroundImage {
            x,
            y: start_y,
            width,
            height,
            shade: 220,
            url: decoded_info
                .as_ref()
                .map(|image| image.url.clone())
                .or_else(|| Some(resolve_browser_href(source, url))),
            decoded_width: decoded_info.as_ref().map(|image| image.width),
            decoded_height: decoded_info.as_ref().map(|image| image.height),
            decoded_hash: decoded_info.map(|image| image.pixel_hash),
            size,
            position,
            repeat,
        };
        if let Some((command, source_bounds)) = self.clipped_background_image_command(command) {
            let target = self
                .node_hit_target(target_node)
                .with_source_bounds(source_bounds);
            self.insert_underlay_command(insert, command, target);
        }
    }

    fn finish(mut self) -> FlowOutput {
        self.break_line();
        self.positive_z_layers.sort_by_key(|layer| layer.z_index);
        let mut display_list = Vec::new();
        let mut hit_targets = Vec::new();
        for layer in &mut self.positive_z_layers {
            if layer.z_index < 0 {
                append_paint_layer_commands(&mut display_list, &mut hit_targets, layer);
            }
        }
        display_list.append(&mut self.underlay_list);
        hit_targets.append(&mut self.underlay_targets);
        display_list.append(&mut self.border_list);
        hit_targets.append(&mut self.border_targets);
        display_list.append(&mut self.display_list);
        hit_targets.append(&mut self.display_targets);
        for layer in &mut self.positive_z_layers {
            if layer.z_index >= 0 {
                append_paint_layer_commands(&mut display_list, &mut hit_targets, layer);
            }
        }
        debug_assert_eq!(display_list.len(), hit_targets.len());
        FlowOutput {
            text: self.lines.join("\n"),
            display_list,
            hit_targets,
            decoded_images: self.decoded_images,
        }
    }
}

fn append_paint_layer_commands(
    display_list: &mut Vec<DisplayCommand>,
    hit_targets: &mut Vec<DisplayHitTarget>,
    layer: &mut PaintLayerCommands,
) {
    display_list.append(&mut layer.underlay_list);
    hit_targets.append(&mut layer.underlay_targets);
    display_list.append(&mut layer.border_list);
    hit_targets.append(&mut layer.border_targets);
    display_list.append(&mut layer.display_list);
    hit_targets.append(&mut layer.display_targets);
}

#[derive(Debug)]
struct FlowOutput {
    text: String,
    display_list: Vec<DisplayCommand>,
    hit_targets: Vec<DisplayHitTarget>,
    decoded_images: Vec<DecodedImageEntry>,
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn chrome_version() -> Option<String> {
    command_output("chromium", &["--version"])
        .or_else(|| command_output("google-chrome", &["--version"]))
        .or_else(|| {
            command_output(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                &["--version"],
            )
        })
}

fn chrome_program() -> Option<String> {
    if command_output("chromium", &["--version"]).is_some() {
        return Some("chromium".to_owned());
    }
    if command_output("google-chrome", &["--version"]).is_some() {
        return Some("google-chrome".to_owned());
    }
    let mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    if Path::new(mac).exists() {
        return Some(mac.to_owned());
    }
    None
}

pub fn ensure_static_target(target: &str) -> Result<()> {
    if target.trim().is_empty() {
        bail!("browser target cannot be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
