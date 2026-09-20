//! Opt-in vector-glyph text rasterization for screenshot lanes.
//!
//! Layout stays on the deterministic cell grid; when enabled, glyphs are
//! rasterized from a system TrueType font with proportional advances inside
//! the text run's cell span. Bitmap glyphs remain the default so raster hash
//! tests stay machine-independent.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

static VECTOR_FONT: OnceLock<Option<FontVec>> = OnceLock::new();
static VECTOR_FONT_BOLD: OnceLock<Option<FontVec>> = OnceLock::new();
static VECTOR_FONT_ITALIC: OnceLock<Option<FontVec>> = OnceLock::new();
static VECTOR_FONT_BOLD_ITALIC: OnceLock<Option<FontVec>> = OnceLock::new();

/// Fonts loaded from @font-face declarations, keyed by page source so pages
/// cannot leak faces into each other. Bounded: oldest entries evicted.
static PAGE_FONTS: OnceLock<RwLock<HashMap<String, Arc<FontVec>>>> = OnceLock::new();
const PAGE_FONT_MAX_ENTRIES: usize = 16;

fn page_fonts() -> &'static RwLock<HashMap<String, Arc<FontVec>>> {
    PAGE_FONTS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(super) fn register_page_font(source: &str, bytes: Vec<u8>) -> bool {
    let Ok(font) = FontVec::try_from_vec(bytes) else {
        return false;
    };
    let Ok(mut fonts) = page_fonts().write() else {
        return false;
    };
    if fonts.len() >= PAGE_FONT_MAX_ENTRIES && !fonts.contains_key(source) {
        fonts.clear();
    }
    fonts.insert(source.to_owned(), Arc::new(font));
    true
}

pub(super) fn page_font(source: &str) -> Option<Arc<FontVec>> {
    page_fonts().read().ok()?.get(source).cloned()
}

fn load_first_font(candidates: impl IntoIterator<Item = String>) -> Option<FontVec> {
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(font) = FontVec::try_from_vec(bytes)
        {
            return Some(font);
        }
    }
    None
}

pub(super) fn vector_font() -> Option<&'static FontVec> {
    VECTOR_FONT
        .get_or_init(|| {
            let override_path = std::env::var("BRUTAL_VECTOR_FONT").ok();
            load_first_font(
                override_path.into_iter().chain(
                    [
                        "/System/Library/Fonts/Supplemental/Arial.ttf",
                        "/System/Library/Fonts/Supplemental/Helvetica.ttf",
                        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                    ]
                    .map(str::to_owned),
                ),
            )
        })
        .as_ref()
}

fn vector_font_bold() -> Option<&'static FontVec> {
    VECTOR_FONT_BOLD
        .get_or_init(|| {
            let override_path = std::env::var("BRUTAL_VECTOR_FONT_BOLD").ok();
            load_first_font(
                override_path.into_iter().chain(
                    [
                        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                        "/System/Library/Fonts/Supplemental/Helvetica Bold.ttf",
                        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                        "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                    ]
                    .map(str::to_owned),
                ),
            )
        })
        .as_ref()
}

fn vector_font_italic() -> Option<&'static FontVec> {
    VECTOR_FONT_ITALIC
        .get_or_init(|| {
            let override_path = std::env::var("BRUTAL_VECTOR_FONT_ITALIC").ok();
            load_first_font(
                override_path.into_iter().chain(
                    [
                        "/System/Library/Fonts/Supplemental/Arial Italic.ttf",
                        "/System/Library/Fonts/Supplemental/Helvetica Oblique.ttf",
                        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf",
                        "/usr/share/fonts/truetype/liberation/LiberationSans-Italic.ttf",
                    ]
                    .map(str::to_owned),
                ),
            )
        })
        .as_ref()
}

fn vector_font_bold_italic() -> Option<&'static FontVec> {
    VECTOR_FONT_BOLD_ITALIC
        .get_or_init(|| {
            let override_path = std::env::var("BRUTAL_VECTOR_FONT_BOLD_ITALIC").ok();
            load_first_font(
                override_path.into_iter().chain(
                    [
                        "/System/Library/Fonts/Supplemental/Arial Bold Italic.ttf",
                        "/usr/share/fonts/truetype/dejavu/DejaVuSans-BoldOblique.ttf",
                        "/usr/share/fonts/truetype/liberation/LiberationSans-BoldItalic.ttf",
                    ]
                    .map(str::to_owned),
                ),
            )
        })
        .as_ref()
}

/// Draw one glyph at `pen_x` (pixel space) with its baseline derived from the
/// cell row top. Returns the horizontal advance in pixels.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_rgba_vector_glyph(
    pixels: &mut [u8],
    raster_width: usize,
    pen_x: f32,
    cell_top_y: usize,
    cell_height: usize,
    ch: char,
    color: (u8, u8, u8),
    bold: bool,
    italic: bool,
    page_face: Option<&FontVec>,
) -> Option<f32> {
    let font = if bold && italic {
        vector_font_bold_italic()
            .or_else(vector_font_bold)
            .or_else(vector_font)?
    } else if bold {
        vector_font_bold().or_else(vector_font)?
    } else if italic {
        vector_font_italic().or_else(vector_font)?
    } else if let Some(face) = page_face {
        face
    } else {
        vector_font()?
    };
    let real_bold_face = bold && vector_font_bold().is_some();
    let px = cell_height as f32;
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let glyph_id = font.glyph_id(ch);
    let advance = scaled.h_advance(glyph_id);
    let baseline = cell_top_y as f32 + scaled.ascent();
    let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, baseline));
    if let Some(outlined) = font.outline_glyph(glyph) {
        let bounds = outlined.px_bounds();
        let raster_height = pixels.len() / 4 / raster_width.max(1);
        outlined.draw(|gx, gy, coverage| {
            if coverage <= 0.02 {
                return;
            }
            let x = bounds.min.x as i64 + gx as i64;
            let y = bounds.min.y as i64 + gy as i64;
            if x < 0 || y < 0 {
                return;
            }
            let (x, y) = (x as usize, y as usize);
            if x >= raster_width || y >= raster_height {
                return;
            }
            let coverage = if bold && !real_bold_face {
                (coverage * 1.35).min(1.0)
            } else {
                coverage
            };
            blend_rgba_pixel(pixels, raster_width, x, y, color, coverage);
        });
    }
    Some(advance)
}

/// Natural advance of the space glyph at the given cell height, for
/// proportional word spacing inside vector text runs.
pub(super) fn space_advance(cell_height: usize) -> Option<f32> {
    let font = vector_font()?;
    let scaled = font.as_scaled(PxScale::from(cell_height as f32));
    Some(scaled.h_advance(font.glyph_id(' ')))
}

fn blend_rgba_pixel(
    pixels: &mut [u8],
    raster_width: usize,
    x: usize,
    y: usize,
    color: (u8, u8, u8),
    coverage: f32,
) {
    let index = y
        .saturating_mul(raster_width)
        .saturating_add(x)
        .saturating_mul(4);
    let Some(pixel) = pixels.get_mut(index..index + 4) else {
        return;
    };
    let blend = |dst: u8, src: u8| -> u8 {
        (dst as f32 * (1.0 - coverage) + src as f32 * coverage).round() as u8
    };
    pixel[0] = blend(pixel[0], color.0);
    pixel[1] = blend(pixel[1], color.1);
    pixel[2] = blend(pixel[2], color.2);
    pixel[3] = 255;
}
