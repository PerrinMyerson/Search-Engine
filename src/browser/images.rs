use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use image::{ColorType, ImageDecoder};
use jpeg_decoder::{Decoder as JpegDecoder, PixelFormat as JpegPixelFormat};
use memchr::memchr;
use url::Url;

use super::resources::BrowserResourceCache;
use super::{
    Dom, ElementData, NodeKind, TagKind, parse_attributes, parse_css_color_shade, parse_tag,
    resolve_browser_href,
};

const MAX_DECODED_IMAGE_SIDE: usize = 4096;
const MAX_JPEG_DECODED_BYTES: usize = MAX_DECODED_IMAGE_SIDE * MAX_DECODED_IMAGE_SIDE * 4;
const MAX_WEBP_DECODED_BYTES: u64 = MAX_JPEG_DECODED_BYTES as u64;

#[derive(Debug, Clone)]
pub(super) struct DecodedImage {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixels: Vec<u8>,
    pub(super) rgb_pixels: Option<Vec<u8>>,
}

impl DecodedImage {
    pub(super) fn pixel_hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"brutal-browser-decoded-image-v1");
        hasher.update(&(self.width as u64).to_le_bytes());
        hasher.update(&(self.height as u64).to_le_bytes());
        hasher.update(&self.pixels);
        hasher.finalize().to_hex().to_string()
    }

    pub(super) fn color_pixel_hash(&self) -> Option<String> {
        let rgb_pixels = self.rgb_pixels.as_ref()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"brutal-browser-decoded-image-rgb-v1");
        hasher.update(&(self.width as u64).to_le_bytes());
        hasher.update(&(self.height as u64).to_le_bytes());
        hasher.update(rgb_pixels);
        Some(hasher.finalize().to_hex().to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodedImageInfo {
    pub(super) url: String,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixel_hash: String,
    pub(super) color_pixel_hash: Option<String>,
    pub(super) color_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageDecodeDiagnostic {
    pub(super) status: &'static str,
    pub(super) error: Option<String>,
    pub(super) width: Option<usize>,
    pub(super) height: Option<usize>,
    pub(super) pixel_hash: Option<String>,
    pub(super) color_pixel_hash: Option<String>,
    pub(super) color_bytes: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct DecodedImageEntry {
    pub(super) url: String,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixel_hash: String,
    pub(super) image: DecodedImage,
}

impl DecodedImageEntry {
    pub(super) fn info(&self) -> DecodedImageInfo {
        DecodedImageInfo {
            url: self.url.clone(),
            width: self.width,
            height: self.height,
            pixel_hash: self.pixel_hash.clone(),
            color_pixel_hash: self.image.color_pixel_hash(),
            color_bytes: self.image.rgb_pixels.as_ref().map(Vec::len),
        }
    }
}

pub(super) fn decoded_image_entry(source: &str, url: &str) -> Option<DecodedImageEntry> {
    let decoded = decode_image_reference(source, url)?;
    Some(DecodedImageEntry {
        url: url.to_owned(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded,
    })
}

pub(super) fn decoded_cached_images(cache: &BrowserResourceCache) -> Vec<DecodedImageEntry> {
    cache
        .cached_resources()
        .filter_map(|(url, content_type, bytes)| {
            decoded_cached_image_entry(url, content_type, bytes)
        })
        .collect()
}

fn decoded_cached_image_entry(
    url: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<DecodedImageEntry> {
    let decoded = decode_cached_resource_image(url, content_type, bytes)?;
    Some(DecodedImageEntry {
        url: url.to_owned(),
        width: decoded.width,
        height: decoded.height,
        pixel_hash: decoded.pixel_hash(),
        image: decoded,
    })
}

fn decode_cached_resource_image(
    url: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<DecodedImage> {
    if let Some(content_type) = content_type
        && let Some(decoded) = decode_image_bytes(content_type, bytes)
    {
        return Some(decoded);
    }
    let image_type = Url::parse(url)
        .ok()
        .and_then(|url| {
            Path::new(url.path())
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_owned)
        })
        .or_else(|| image_type_from_path(Path::new(url)));
    image_type
        .and_then(|image_type| decode_image_bytes(&image_type, bytes))
        .or_else(|| decode_sniffed_image_bytes(bytes))
}

pub(super) fn image_decode_diagnostic(
    url: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> ImageDecodeDiagnostic {
    if let Some(decoded) = decode_cached_resource_image(url, content_type, bytes) {
        return ImageDecodeDiagnostic {
            status: "decoded",
            error: None,
            width: Some(decoded.width),
            height: Some(decoded.height),
            pixel_hash: Some(decoded.pixel_hash()),
            color_pixel_hash: decoded.color_pixel_hash(),
            color_bytes: decoded.rgb_pixels.as_ref().map(Vec::len),
        };
    }

    if let Some(error) = unsupported_image_format_error(url, content_type, bytes) {
        return ImageDecodeDiagnostic {
            status: "unsupported_format",
            error: Some(error),
            width: None,
            height: None,
            pixel_hash: None,
            color_pixel_hash: None,
            color_bytes: None,
        };
    }

    ImageDecodeDiagnostic {
        status: "undecoded",
        error: Some("image bytes did not match a supported decoder".to_owned()),
        width: None,
        height: None,
        pixel_hash: None,
        color_pixel_hash: None,
        color_bytes: None,
    }
}

fn unsupported_image_format_error(
    url: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Option<String> {
    if let Some(content_type) = content_type
        && image_content_type_declares_unsupported_format(content_type)
    {
        return Some(format!(
            "unsupported image content type: {}",
            normalized_image_mime_type(content_type)
        ));
    }
    if let Some(extension) = image_extension_from_url(url)
        && unsupported_image_extension(&extension)
    {
        return Some(format!("unsupported image extension: .{extension}"));
    }
    unsupported_image_signature(bytes)
        .map(|format| format!("unsupported image byte signature: {format}"))
}

fn image_content_type_declares_unsupported_format(content_type: &str) -> bool {
    let mime = normalized_image_mime_type(content_type);
    !mime.is_empty()
        && mime
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
        && !image_mime_type_supported(&mime)
}

fn normalized_image_mime_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase()
}

fn image_extension_from_url(url: &str) -> Option<String> {
    let path = Url::parse(url)
        .ok()
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|| url.split(['?', '#']).next().unwrap_or(url).to_owned());
    Path::new(&path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

fn unsupported_image_extension(extension: &str) -> bool {
    matches!(
        extension,
        "avif" | "avifs" | "heic" | "heif" | "gif" | "bmp" | "ico" | "tif" | "tiff"
    )
}

fn unsupported_image_signature(bytes: &[u8]) -> Option<&'static str> {
    if matches!(bytes, [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..]) {
        Some("image/gif")
    } else if matches!(bytes, [b'B', b'M', ..]) {
        Some("image/bmp")
    } else if matches!(bytes, [0, 0, 1, 0, ..]) {
        Some("image/x-icon")
    } else if matches!(bytes, [b'I', b'I', b'*', 0, ..] | [b'M', b'M', 0, b'*', ..]) {
        Some("image/tiff")
    } else if is_isobmff_image_type(bytes, &["avif", "avis"]) {
        Some("image/avif")
    } else if is_isobmff_image_type(bytes, &["heic", "heix", "hevc", "hevx", "mif1", "msf1"]) {
        Some("image/heif")
    } else {
        None
    }
}

fn is_isobmff_image_type(bytes: &[u8], brands: &[&str]) -> bool {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    bytes[8..]
        .chunks_exact(4)
        .take(8)
        .filter_map(|brand| std::str::from_utf8(brand).ok())
        .any(|brand| brands.iter().any(|expected| brand == *expected))
}

pub(super) fn decode_image_reference(source: &str, url: &str) -> Option<DecodedImage> {
    if let Some((mime_type, bytes)) = decode_data_url(url) {
        return decode_image_bytes(&mime_type, &bytes)
            .or_else(|| decode_sniffed_image_bytes(&bytes));
    }
    let resolved = resolve_browser_href(source, url);
    let path = local_browser_path(&resolved)?;
    decode_image_file(path)
}

fn decode_image_bytes(image_type: &str, bytes: &[u8]) -> Option<DecodedImage> {
    let image_type = image_type
        .split(';')
        .next()
        .unwrap_or(image_type)
        .trim()
        .to_ascii_lowercase();
    match image_type.as_str() {
        "svg" | "image/svg+xml" | "image/svg" => decode_simple_svg(bytes),
        "png" | "image/png" | "image/x-png" => decode_simple_png(bytes),
        "jpg" | "jpeg" | "jpe" | "jfif" | "pjpeg" | "pjp" | "image/jpeg" | "image/jpg"
        | "image/jpe" | "image/pjpeg" | "image/x-jpeg" => decode_jpeg(bytes),
        "webp" | "image/webp" | "image/x-webp" => decode_webp(bytes),
        _ => None,
    }
}

fn decode_sniffed_image_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    if is_jpeg_bytes(bytes) {
        decode_jpeg(bytes)
    } else if is_png_bytes(bytes) {
        decode_simple_png(bytes)
    } else if is_webp_bytes(bytes) {
        decode_webp(bytes)
    } else if is_svg_bytes(bytes) {
        decode_simple_svg(bytes)
    } else {
        None
    }
}

fn is_jpeg_bytes(bytes: &[u8]) -> bool {
    matches!(bytes, [0xff, 0xd8, 0xff, ..])
}

fn is_png_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

fn is_webp_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

fn is_svg_bytes(bytes: &[u8]) -> bool {
    let Some(prefix) = std::str::from_utf8(&bytes[..bytes.len().min(512)]).ok() else {
        return false;
    };
    let prefix = prefix.trim_start_matches('\u{feff}').trim_start();
    prefix.starts_with("<svg") || (prefix.starts_with("<?xml") && prefix.contains("<svg"))
}

fn decode_data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let payload = url.strip_prefix("data:")?;
    let (metadata, data) = payload.split_once(',')?;
    let mut mime_type = "text/plain".to_owned();
    let mut base64 = false;
    for (index, part) in metadata.split(';').enumerate() {
        if index == 0 && !part.is_empty() {
            mime_type = part.to_owned();
        } else if part.eq_ignore_ascii_case("base64") {
            base64 = true;
        }
    }
    let bytes = if base64 {
        decode_base64(data)?
    } else {
        percent_decode_bytes(data)?
    };
    Some((mime_type, bytes))
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut block = [0u8; 4];
    let mut block_len = 0usize;
    let mut padding = 0usize;

    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                0
            }
            _ => return None,
        };
        if padding > 0 && byte != b'=' {
            return None;
        }
        block[block_len] = value;
        block_len += 1;
        if block_len == 4 {
            out.push((block[0] << 2) | (block[1] >> 4));
            if padding < 2 {
                out.push((block[1] << 4) | (block[2] >> 2));
            }
            if padding == 0 {
                out.push((block[2] << 6) | block[3]);
            }
            block_len = 0;
            padding = 0;
        }
    }

    (block_len == 0).then_some(out)
}

fn percent_decode_bytes(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn local_browser_path(resolved: &str) -> Option<PathBuf> {
    if let Ok(url) = Url::parse(resolved) {
        return (url.scheme() == "file")
            .then(|| url.to_file_path().ok())
            .flatten();
    }
    Some(PathBuf::from(resolved))
}

fn image_type_from_path(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

fn decode_image_file(path: PathBuf) -> Option<DecodedImage> {
    let bytes = fs::read(&path).ok()?;
    image_type_from_path(&path)
        .and_then(|extension| decode_image_bytes(&extension, &bytes))
        .or_else(|| decode_sniffed_image_bytes(&bytes))
}

fn decode_simple_svg(bytes: &[u8]) -> Option<DecodedImage> {
    let mut cursor = 0usize;
    let mut width = None;
    let mut height = None;
    let mut rects = Vec::new();
    let mut circles = Vec::new();
    let mut ellipses = Vec::new();
    let mut polygons = Vec::new();
    let mut polylines = Vec::new();
    let mut paths = Vec::new();
    let mut transform_stack = vec![Some(SvgTransform::identity())];

    while cursor < bytes.len() {
        let Some(offset) = memchr(b'<', &bytes[cursor..]) else {
            break;
        };
        let tag_start = cursor + offset;
        let Some(tag_end_offset) = memchr(b'>', &bytes[tag_start + 1..]) else {
            break;
        };
        let tag_end = tag_start + 1 + tag_end_offset;
        let raw_tag = &bytes[tag_start + 1..tag_end];
        if let Some(tag) = parse_tag(raw_tag) {
            match tag.kind {
                TagKind::Opening => {
                    let attrs = parse_attributes(raw_tag);
                    match tag.name.as_str() {
                        "svg" => {
                            width = attrs
                                .get("width")
                                .and_then(|value| parse_svg_pixel_dimension(value))
                                .or(width);
                            height = attrs
                                .get("height")
                                .and_then(|value| parse_svg_pixel_dimension(value))
                                .or(height);
                            if (width.is_none() || height.is_none())
                                && let Some((view_box_width, view_box_height)) = attrs
                                    .get("viewbox")
                                    .and_then(|value| parse_svg_viewbox_dimensions(value))
                            {
                                width = width.or(Some(view_box_width));
                                height = height.or(Some(view_box_height));
                            }
                        }
                        "g" => {
                            if !tag.self_closing {
                                transform_stack.push(svg_child_transform(&transform_stack, &attrs));
                            }
                        }
                        "rect" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                rects.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        "circle" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                circles.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        "ellipse" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                ellipses.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        "polygon" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                polygons.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        "polyline" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                polylines.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        "path" => {
                            if let Some(transform) = svg_child_transform(&transform_stack, &attrs) {
                                paths.push(SvgShapeAttrs { attrs, transform });
                            }
                        }
                        _ => {}
                    }
                }
                TagKind::Closing => {
                    if tag.name == "g" && transform_stack.len() > 1 {
                        transform_stack.pop();
                    }
                }
            }
        }
        cursor = tag_end + 1;
    }

    let width = width?.clamp(1, MAX_DECODED_IMAGE_SIDE);
    let height = height?.clamp(1, MAX_DECODED_IMAGE_SIDE);
    let mut pixels = vec![255u8; width.checked_mul(height)?];
    let mut rgb_pixels = vec![255u8; width.checked_mul(height)?.checked_mul(3)?];
    for rect_shape in rects {
        let rect = &rect_shape.attrs;
        let Some(fill) = svg_shape_fill_paint(&rect) else {
            continue;
        };
        if rect_shape.transform.is_identity() {
            let x = rect
                .get("x")
                .and_then(|value| parse_svg_pixel_dimension(value))
                .unwrap_or(0)
                .min(width);
            let y = rect
                .get("y")
                .and_then(|value| parse_svg_pixel_dimension(value))
                .unwrap_or(0)
                .min(height);
            let rect_width = rect
                .get("width")
                .and_then(|value| parse_svg_pixel_dimension(value))
                .unwrap_or(width.saturating_sub(x))
                .min(width.saturating_sub(x));
            let rect_height = rect
                .get("height")
                .and_then(|value| parse_svg_pixel_dimension(value))
                .unwrap_or(height.saturating_sub(y))
                .min(height.saturating_sub(y));
            fill_decoded_rect(
                &mut pixels,
                &mut rgb_pixels,
                width,
                x,
                y,
                rect_width,
                rect_height,
                fill,
            );
        } else if let Some(points) = svg_rect_points(rect, width, height) {
            let points = transform_svg_points(&points, rect_shape.transform);
            fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
        }
    }
    for circle_shape in circles {
        let circle = &circle_shape.attrs;
        let Some(fill) = svg_shape_fill_paint(&circle) else {
            continue;
        };
        let Some(cx) = circle.get("cx").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let Some(cy) = circle.get("cy").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let Some(radius) = circle.get("r").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let ellipse = SvgEllipse {
            cx,
            cy,
            rx: radius,
            ry: radius,
        };
        if circle_shape.transform.is_identity() {
            fill_decoded_ellipse(&mut pixels, &mut rgb_pixels, width, height, ellipse, fill);
        } else {
            let points =
                transform_svg_points(&svg_ellipse_points(ellipse, 24), circle_shape.transform);
            fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
        }
    }
    for ellipse_shape in ellipses {
        let ellipse = &ellipse_shape.attrs;
        let Some(fill) = svg_shape_fill_paint(&ellipse) else {
            continue;
        };
        let Some(cx) = ellipse.get("cx").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let Some(cy) = ellipse.get("cy").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let Some(rx) = ellipse.get("rx").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let Some(ry) = ellipse.get("ry").and_then(|value| parse_svg_number(value)) else {
            continue;
        };
        let ellipse = SvgEllipse { cx, cy, rx, ry };
        if ellipse_shape.transform.is_identity() {
            fill_decoded_ellipse(&mut pixels, &mut rgb_pixels, width, height, ellipse, fill);
        } else {
            let points =
                transform_svg_points(&svg_ellipse_points(ellipse, 24), ellipse_shape.transform);
            fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
        }
    }
    for polygon_shape in polygons {
        let polygon = &polygon_shape.attrs;
        let Some(fill) = svg_shape_fill_paint(&polygon) else {
            continue;
        };
        let Some(points) = polygon
            .get("points")
            .and_then(|value| parse_svg_points(value))
        else {
            continue;
        };
        let points = transform_svg_points(&points, polygon_shape.transform);
        fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
    }
    for polyline_shape in polylines {
        let polyline = &polyline_shape.attrs;
        let Some(points) = polyline
            .get("points")
            .and_then(|value| parse_svg_points(value))
        else {
            continue;
        };
        let points = transform_svg_points(&points, polyline_shape.transform);
        if let Some(fill) = svg_shape_fill_paint(&polyline) {
            fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
        }
        if let Some(stroke) = svg_shape_stroke_paint(&polyline) {
            let stroke_width = svg_shape_stroke_width(polyline, polyline_shape.transform);
            draw_decoded_polyline(
                &mut pixels,
                &mut rgb_pixels,
                width,
                height,
                &points,
                stroke,
                stroke_width,
            );
        }
    }
    for path_shape in paths {
        let path = &path_shape.attrs;
        let Some(points) = path.get("d").and_then(|value| parse_simple_svg_path(value)) else {
            continue;
        };
        let points = transform_svg_points(&points, path_shape.transform);
        if let Some(fill) = svg_shape_fill_paint(&path) {
            fill_decoded_polygon(&mut pixels, &mut rgb_pixels, width, height, &points, fill);
        }
        if let Some(stroke) = svg_shape_stroke_paint(&path) {
            let stroke_width = svg_shape_stroke_width(path, path_shape.transform);
            draw_decoded_polyline(
                &mut pixels,
                &mut rgb_pixels,
                width,
                height,
                &points,
                stroke,
                stroke_width,
            );
        }
    }

    Some(DecodedImage {
        width,
        height,
        pixels,
        rgb_pixels: Some(rgb_pixels),
    })
}

pub(super) fn decode_simple_png(bytes: &[u8]) -> Option<DecodedImage> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }

    let mut cursor = 8usize;
    let mut width = None;
    let mut height = None;
    let mut bit_depth = None;
    let mut color_type = None;
    let mut compression_method = None;
    let mut filter_method = None;
    let mut interlace_method = None;
    let mut palette = Vec::new();
    let mut transparency = Vec::new();
    let mut idat = Vec::new();

    while cursor.checked_add(12)? <= bytes.len() {
        let chunk_len = read_png_u32(&bytes[cursor..cursor + 4])? as usize;
        cursor += 4;
        let chunk_type = &bytes[cursor..cursor + 4];
        cursor += 4;
        let data_end = cursor.checked_add(chunk_len)?;
        let crc_end = data_end.checked_add(4)?;
        if crc_end > bytes.len() {
            return None;
        }
        let data = &bytes[cursor..data_end];
        cursor = crc_end;

        match chunk_type {
            b"IHDR" => {
                if data.len() != 13 {
                    return None;
                }
                width = read_png_u32(&data[0..4]);
                height = read_png_u32(&data[4..8]);
                bit_depth = Some(data[8]);
                color_type = Some(data[9]);
                compression_method = Some(data[10]);
                filter_method = Some(data[11]);
                interlace_method = Some(data[12]);
            }
            b"PLTE" => {
                if data.len() % 3 != 0 {
                    return None;
                }
                palette.extend_from_slice(data);
            }
            b"tRNS" => transparency.extend_from_slice(data),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
    }

    let width = usize::try_from(width?).ok()?;
    let height = usize::try_from(height?).ok()?;
    if width == 0
        || height == 0
        || width > MAX_DECODED_IMAGE_SIDE
        || height > MAX_DECODED_IMAGE_SIDE
    {
        return None;
    }
    if bit_depth? != 8 || compression_method? != 0 || filter_method? != 0 || interlace_method? != 0
    {
        return None;
    }
    let color_type = color_type?;
    let channels = match color_type {
        0 => 1,
        2 => 3,
        3 => {
            if palette.is_empty() {
                return None;
            }
            1
        }
        4 => 2,
        6 => 4,
        _ => return None,
    };
    let row_bytes = width.checked_mul(channels)?;
    let expected_len = row_bytes.checked_add(1)?.checked_mul(height)?;

    let mut decoder = ZlibDecoder::new(idat.as_slice());
    let mut raw = Vec::with_capacity(expected_len);
    decoder.read_to_end(&mut raw).ok()?;
    if raw.len() != expected_len {
        return None;
    }

    let pixel_count = width.checked_mul(height)?;
    let mut pixels = Vec::with_capacity(pixel_count);
    let mut rgb_pixels = Vec::with_capacity(pixel_count.checked_mul(3)?);
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut offset = 0usize;

    for _ in 0..height {
        let filter = raw[offset];
        offset += 1;
        current.copy_from_slice(&raw[offset..offset + row_bytes]);
        offset += row_bytes;
        reconstruct_png_scanline(filter, channels, &previous, &mut current)?;
        push_png_grayscale_pixels(&current, color_type, &palette, &transparency, &mut pixels)?;
        push_png_rgb_pixels(
            &current,
            color_type,
            &palette,
            &transparency,
            &mut rgb_pixels,
        )?;
        previous.copy_from_slice(&current);
    }

    if pixels.len() != pixel_count || rgb_pixels.len() != pixel_count.checked_mul(3)? {
        return None;
    }

    Some(DecodedImage {
        width,
        height,
        pixels,
        rgb_pixels: Some(rgb_pixels),
    })
}

fn read_png_u32(bytes: &[u8]) -> Option<u32> {
    let bytes: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn reconstruct_png_scanline(
    filter: u8,
    bytes_per_pixel: usize,
    previous: &[u8],
    current: &mut [u8],
) -> Option<()> {
    for index in 0..current.len() {
        let left = if index >= bytes_per_pixel {
            current[index - bytes_per_pixel]
        } else {
            0
        };
        let up = previous[index];
        let upper_left = if index >= bytes_per_pixel {
            previous[index - bytes_per_pixel]
        } else {
            0
        };
        current[index] = match filter {
            0 => current[index],
            1 => current[index].wrapping_add(left),
            2 => current[index].wrapping_add(up),
            3 => current[index].wrapping_add(((left as u16 + up as u16) / 2) as u8),
            4 => current[index].wrapping_add(paeth_predictor(left, up, upper_left)),
            _ => return None,
        };
    }
    Some(())
}

fn paeth_predictor(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = left as i16;
    let up = up as i16;
    let upper_left = upper_left as i16;
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left as u8
    } else if up_distance <= upper_left_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

fn push_png_grayscale_pixels(
    row: &[u8],
    color_type: u8,
    palette: &[u8],
    transparency: &[u8],
    pixels: &mut Vec<u8>,
) -> Option<()> {
    match color_type {
        0 => pixels.extend_from_slice(row),
        2 => {
            for rgb in row.chunks_exact(3) {
                pixels.push(rgb_to_gray(rgb[0], rgb[1], rgb[2]));
            }
        }
        3 => {
            for &index in row {
                let index = usize::from(index);
                let palette_offset = index.checked_mul(3)?;
                let rgb = palette.get(palette_offset..palette_offset.checked_add(3)?)?;
                let gray = rgb_to_gray(rgb[0], rgb[1], rgb[2]);
                let alpha = transparency.get(index).copied().unwrap_or(255);
                pixels.push(blend_gray_over_white(gray, alpha));
            }
        }
        4 => {
            for gray_alpha in row.chunks_exact(2) {
                pixels.push(blend_gray_over_white(gray_alpha[0], gray_alpha[1]));
            }
        }
        6 => {
            for rgba in row.chunks_exact(4) {
                let gray = rgb_to_gray(rgba[0], rgba[1], rgba[2]);
                pixels.push(blend_gray_over_white(gray, rgba[3]));
            }
        }
        _ => {}
    }
    Some(())
}

fn push_png_rgb_pixels(
    row: &[u8],
    color_type: u8,
    palette: &[u8],
    transparency: &[u8],
    pixels: &mut Vec<u8>,
) -> Option<()> {
    match color_type {
        0 => {
            for &gray in row {
                pixels.extend_from_slice(&[gray, gray, gray]);
            }
        }
        2 => pixels.extend_from_slice(row),
        3 => {
            for &index in row {
                let index = usize::from(index);
                let palette_offset = index.checked_mul(3)?;
                let rgb = palette.get(palette_offset..palette_offset.checked_add(3)?)?;
                let alpha = transparency.get(index).copied().unwrap_or(255);
                pixels.push(blend_channel_over_white(rgb[0], alpha));
                pixels.push(blend_channel_over_white(rgb[1], alpha));
                pixels.push(blend_channel_over_white(rgb[2], alpha));
            }
        }
        4 => {
            for gray_alpha in row.chunks_exact(2) {
                let value = blend_channel_over_white(gray_alpha[0], gray_alpha[1]);
                pixels.extend_from_slice(&[value, value, value]);
            }
        }
        6 => {
            for rgba in row.chunks_exact(4) {
                pixels.push(blend_channel_over_white(rgba[0], rgba[3]));
                pixels.push(blend_channel_over_white(rgba[1], rgba[3]));
                pixels.push(blend_channel_over_white(rgba[2], rgba[3]));
            }
        }
        _ => {}
    }
    Some(())
}

fn decode_jpeg(bytes: &[u8]) -> Option<DecodedImage> {
    decode_jpeg_with_max_side(bytes, MAX_DECODED_IMAGE_SIDE)
}

fn decode_webp(bytes: &[u8]) -> Option<DecodedImage> {
    if !is_webp_bytes(bytes) {
        return None;
    }
    let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let (width, height) = decoder.dimensions();
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    if width == 0
        || height == 0
        || width > MAX_DECODED_IMAGE_SIDE
        || height > MAX_DECODED_IMAGE_SIDE
    {
        return None;
    }
    let total_bytes = decoder.total_bytes();
    if total_bytes == 0 || total_bytes > MAX_WEBP_DECODED_BYTES {
        return None;
    }
    let color_type = decoder.color_type();
    let mut decoded = vec![0u8; usize::try_from(total_bytes).ok()?];
    decoder.read_image(&mut decoded).ok()?;
    let pixel_count = width.checked_mul(height)?;
    let pixels = image_pixels_to_grayscale(&decoded, color_type, pixel_count)?;
    let rgb_pixels = image_pixels_to_rgb(&decoded, color_type, pixel_count)?;
    Some(DecodedImage {
        width,
        height,
        pixels,
        rgb_pixels: Some(rgb_pixels),
    })
}

fn image_pixels_to_grayscale(
    decoded: &[u8],
    color_type: ColorType,
    pixel_count: usize,
) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(pixel_count);
    match color_type {
        ColorType::L8 => {
            if decoded.len() != pixel_count {
                return None;
            }
            pixels.extend_from_slice(decoded);
        }
        ColorType::La8 => {
            if decoded.len() != pixel_count.checked_mul(2)? {
                return None;
            }
            for gray_alpha in decoded.chunks_exact(2) {
                pixels.push(blend_gray_over_white(gray_alpha[0], gray_alpha[1]));
            }
        }
        ColorType::Rgb8 => {
            if decoded.len() != pixel_count.checked_mul(3)? {
                return None;
            }
            for rgb in decoded.chunks_exact(3) {
                pixels.push(rgb_to_gray(rgb[0], rgb[1], rgb[2]));
            }
        }
        ColorType::Rgba8 => {
            if decoded.len() != pixel_count.checked_mul(4)? {
                return None;
            }
            for rgba in decoded.chunks_exact(4) {
                let gray = rgb_to_gray(rgba[0], rgba[1], rgba[2]);
                pixels.push(blend_gray_over_white(gray, rgba[3]));
            }
        }
        _ => return None,
    }
    (pixels.len() == pixel_count).then_some(pixels)
}

fn image_pixels_to_rgb(
    decoded: &[u8],
    color_type: ColorType,
    pixel_count: usize,
) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(pixel_count.checked_mul(3)?);
    match color_type {
        ColorType::L8 => {
            if decoded.len() != pixel_count {
                return None;
            }
            for &gray in decoded {
                pixels.extend_from_slice(&[gray, gray, gray]);
            }
        }
        ColorType::La8 => {
            if decoded.len() != pixel_count.checked_mul(2)? {
                return None;
            }
            for gray_alpha in decoded.chunks_exact(2) {
                let value = blend_channel_over_white(gray_alpha[0], gray_alpha[1]);
                pixels.extend_from_slice(&[value, value, value]);
            }
        }
        ColorType::Rgb8 => {
            if decoded.len() != pixel_count.checked_mul(3)? {
                return None;
            }
            pixels.extend_from_slice(decoded);
        }
        ColorType::Rgba8 => {
            if decoded.len() != pixel_count.checked_mul(4)? {
                return None;
            }
            for rgba in decoded.chunks_exact(4) {
                pixels.push(blend_channel_over_white(rgba[0], rgba[3]));
                pixels.push(blend_channel_over_white(rgba[1], rgba[3]));
                pixels.push(blend_channel_over_white(rgba[2], rgba[3]));
            }
        }
        _ => return None,
    }
    (pixels.len() == pixel_count.checked_mul(3)?).then_some(pixels)
}

fn decode_jpeg_with_max_side(bytes: &[u8], max_side: usize) -> Option<DecodedImage> {
    let max_side = max_side.clamp(1, MAX_DECODED_IMAGE_SIDE);
    let mut decoder = JpegDecoder::new(bytes);
    decoder.set_max_decoding_buffer_size(MAX_JPEG_DECODED_BYTES);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    if width == 0 || height == 0 {
        return None;
    }
    if width > max_side || height > max_side {
        let max_side = u16::try_from(max_side).ok()?;
        decoder.scale(max_side, max_side).ok()?;
    }
    let decoded = decoder.decode().ok()?;
    let orientation = decoder
        .exif_data()
        .and_then(exif_orientation_from_tiff)
        .unwrap_or(1);
    let info = decoder.info()?;
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return None;
    }
    let pixel_count = width.checked_mul(height)?;
    let expected_len = pixel_count.checked_mul(info.pixel_format.pixel_bytes())?;
    if decoded.len() != expected_len {
        return None;
    }
    let pixels = jpeg_pixels_to_grayscale(&decoded, info.pixel_format, pixel_count)?;
    let rgb_pixels = jpeg_pixels_to_rgb(&decoded, info.pixel_format, pixel_count)?;
    let mut image = DecodedImage {
        width,
        height,
        pixels,
        rgb_pixels: Some(rgb_pixels),
    };
    apply_exif_orientation(&mut image, orientation)?;
    Some(image)
}

fn jpeg_pixels_to_grayscale(
    decoded: &[u8],
    pixel_format: JpegPixelFormat,
    pixel_count: usize,
) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(pixel_count);
    match pixel_format {
        JpegPixelFormat::L8 => pixels.extend_from_slice(decoded),
        JpegPixelFormat::RGB24 => {
            for rgb in decoded.chunks_exact(3) {
                pixels.push(rgb_to_gray(rgb[0], rgb[1], rgb[2]));
            }
        }
        JpegPixelFormat::CMYK32 => {
            for cmyk in decoded.chunks_exact(4) {
                let key = cmyk[3];
                let red = cmyk_channel_to_rgb(cmyk[0], key);
                let green = cmyk_channel_to_rgb(cmyk[1], key);
                let blue = cmyk_channel_to_rgb(cmyk[2], key);
                pixels.push(rgb_to_gray(red, green, blue));
            }
        }
        JpegPixelFormat::L16 => {
            for gray in decoded.chunks_exact(2) {
                let gray = u16::from_ne_bytes(gray.try_into().ok()?);
                pixels.push(gray_u16_to_u8(gray));
            }
        }
    }
    (pixels.len() == pixel_count).then_some(pixels)
}

fn jpeg_pixels_to_rgb(
    decoded: &[u8],
    pixel_format: JpegPixelFormat,
    pixel_count: usize,
) -> Option<Vec<u8>> {
    let mut pixels = Vec::with_capacity(pixel_count.checked_mul(3)?);
    match pixel_format {
        JpegPixelFormat::L8 => {
            for &gray in decoded {
                pixels.extend_from_slice(&[gray, gray, gray]);
            }
        }
        JpegPixelFormat::RGB24 => pixels.extend_from_slice(decoded),
        JpegPixelFormat::CMYK32 => {
            for cmyk in decoded.chunks_exact(4) {
                let key = cmyk[3];
                pixels.push(cmyk_channel_to_rgb(cmyk[0], key));
                pixels.push(cmyk_channel_to_rgb(cmyk[1], key));
                pixels.push(cmyk_channel_to_rgb(cmyk[2], key));
            }
        }
        JpegPixelFormat::L16 => {
            for gray in decoded.chunks_exact(2) {
                let value = gray_u16_to_u8(u16::from_ne_bytes(gray.try_into().ok()?));
                pixels.extend_from_slice(&[value, value, value]);
            }
        }
    }
    (pixels.len() == pixel_count.checked_mul(3)?).then_some(pixels)
}

fn exif_orientation_from_tiff(tiff: &[u8]) -> Option<u16> {
    let endian = TiffEndian::from_header(tiff)?;
    if read_tiff_u16(tiff, 2, endian)? != 42 {
        return None;
    }
    let ifd_offset = usize::try_from(read_tiff_u32(tiff, 4, endian)?).ok()?;
    let entry_count = usize::from(read_tiff_u16(tiff, ifd_offset, endian)?);
    let entries_offset = ifd_offset.checked_add(2)?;
    for index in 0..entry_count {
        let entry_offset = entries_offset.checked_add(index.checked_mul(12)?)?;
        let tag = read_tiff_u16(tiff, entry_offset, endian)?;
        if tag != 0x0112 {
            continue;
        }
        let field_type = read_tiff_u16(tiff, entry_offset + 2, endian)?;
        let count = read_tiff_u32(tiff, entry_offset + 4, endian)?;
        if field_type != 3 || count == 0 {
            return None;
        }
        let value_offset = entry_offset.checked_add(8)?;
        let orientation = if count == 1 {
            read_tiff_u16(tiff, value_offset, endian)?
        } else {
            let offset = usize::try_from(read_tiff_u32(tiff, value_offset, endian)?).ok()?;
            read_tiff_u16(tiff, offset, endian)?
        };
        return (1..=8).contains(&orientation).then_some(orientation);
    }
    None
}

#[derive(Debug, Clone, Copy)]
enum TiffEndian {
    Little,
    Big,
}

impl TiffEndian {
    fn from_header(bytes: &[u8]) -> Option<Self> {
        match bytes.get(0..2)? {
            b"II" => Some(Self::Little),
            b"MM" => Some(Self::Big),
            _ => None,
        }
    }
}

fn read_tiff_u16(bytes: &[u8], offset: usize, endian: TiffEndian) -> Option<u16> {
    let bytes: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(match endian {
        TiffEndian::Little => u16::from_le_bytes(bytes),
        TiffEndian::Big => u16::from_be_bytes(bytes),
    })
}

fn read_tiff_u32(bytes: &[u8], offset: usize, endian: TiffEndian) -> Option<u32> {
    let bytes: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(match endian {
        TiffEndian::Little => u32::from_le_bytes(bytes),
        TiffEndian::Big => u32::from_be_bytes(bytes),
    })
}

fn apply_exif_orientation(image: &mut DecodedImage, orientation: u16) -> Option<()> {
    if orientation == 1 {
        return Some(());
    }
    let width = image.width;
    let height = image.height;
    let pixel_count = width.checked_mul(height)?;
    if image.pixels.len() != pixel_count {
        return None;
    }
    if let Some(rgb_pixels) = image.rgb_pixels.as_ref()
        && rgb_pixels.len() != pixel_count.checked_mul(3)?
    {
        return None;
    }
    let swaps_axes = matches!(orientation, 5..=8);
    let output_width = if swaps_axes { height } else { width };
    let output_height = if swaps_axes { width } else { height };
    let mut oriented = vec![255u8; pixel_count];
    let mut oriented_rgb = if image.rgb_pixels.is_some() {
        Some(vec![255u8; pixel_count.checked_mul(3)?])
    } else {
        None
    };
    for y in 0..height {
        for x in 0..width {
            let (output_x, output_y) = match orientation {
                2 => (width - 1 - x, y),
                3 => (width - 1 - x, height - 1 - y),
                4 => (x, height - 1 - y),
                5 => (y, x),
                6 => (height - 1 - y, x),
                7 => (height - 1 - y, width - 1 - x),
                8 => (y, width - 1 - x),
                _ => return None,
            };
            let source_index = y.checked_mul(width)?.checked_add(x)?;
            let output_index = output_y.checked_mul(output_width)?.checked_add(output_x)?;
            oriented[output_index] = *image.pixels.get(source_index)?;
            if let (Some(source_rgb), Some(output_rgb)) =
                (image.rgb_pixels.as_ref(), oriented_rgb.as_mut())
            {
                let source_rgb_index = source_index.checked_mul(3)?;
                let output_rgb_index = output_index.checked_mul(3)?;
                output_rgb[output_rgb_index..output_rgb_index.checked_add(3)?]
                    .copy_from_slice(source_rgb.get(source_rgb_index..source_rgb_index + 3)?);
            }
        }
    }
    image.width = output_width;
    image.height = output_height;
    image.pixels = oriented;
    image.rgb_pixels = oriented_rgb;
    Some(())
}

fn rgb_to_gray(red: u8, green: u8, blue: u8) -> u8 {
    (((red as u16 * 77) + (green as u16 * 150) + (blue as u16 * 29) + 128) >> 8) as u8
}

fn cmyk_channel_to_rgb(cmy: u8, key: u8) -> u8 {
    let cmy = ((cmy as u16 * (255 - key as u16) + 127) / 255)
        .saturating_add(key as u16)
        .min(255);
    (255 - cmy) as u8
}

fn gray_u16_to_u8(gray: u16) -> u8 {
    ((gray as u32 * 255 + 32767) / 65535) as u8
}

fn blend_gray_over_white(gray: u8, alpha: u8) -> u8 {
    blend_channel_over_white(gray, alpha)
}

fn blend_channel_over_white(channel: u8, alpha: u8) -> u8 {
    (((channel as u16 * alpha as u16) + (255u16 * (255 - alpha as u16)) + 127) / 255) as u8
}

fn parse_svg_pixel_dimension(value: &str) -> Option<usize> {
    let value = value.trim().trim_end_matches("px").trim();
    if value.is_empty() || value.contains('%') {
        return None;
    }
    let whole = value.split_once('.').map_or(value, |(whole, _)| whole);
    let pixels = whole.parse::<usize>().ok()?;
    (pixels > 0).then_some(pixels)
}

fn parse_svg_number(value: &str) -> Option<f32> {
    let value = value.trim().trim_end_matches("px").trim();
    if value.is_empty() || value.contains('%') {
        return None;
    }
    let number = value.parse::<f32>().ok()?;
    number.is_finite().then_some(number)
}

fn parse_svg_viewbox_dimensions(value: &str) -> Option<(usize, usize)> {
    let mut numbers = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .filter_map(parse_svg_number);
    let _min_x = numbers.next()?;
    let _min_y = numbers.next()?;
    let width = numbers.next()?;
    let height = numbers.next()?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((
        (width.ceil() as usize).clamp(1, MAX_DECODED_IMAGE_SIDE),
        (height.ceil() as usize).clamp(1, MAX_DECODED_IMAGE_SIDE),
    ))
}

#[derive(Debug, Clone)]
struct SvgShapeAttrs {
    attrs: HashMap<String, String>,
    transform: SvgTransform,
}

#[derive(Debug, Clone, Copy)]
struct SvgTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl SvgTransform {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn translate(tx: f32, ty: f32) -> Self {
        Self {
            e: tx,
            f: ty,
            ..Self::identity()
        }
    }

    fn scale(sx: f32, sy: f32) -> Self {
        Self {
            a: sx,
            d: sy,
            ..Self::identity()
        }
    }

    fn matrix(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    fn multiply(self, other: Self) -> Self {
        Self {
            a: self.a.mul_add(other.a, self.c * other.b),
            b: self.b.mul_add(other.a, self.d * other.b),
            c: self.a.mul_add(other.c, self.c * other.d),
            d: self.b.mul_add(other.c, self.d * other.d),
            e: self.a.mul_add(other.e, self.c.mul_add(other.f, self.e)),
            f: self.b.mul_add(other.e, self.d.mul_add(other.f, self.f)),
        }
    }

    fn apply(self, point: SvgPoint) -> SvgPoint {
        SvgPoint {
            x: self.a.mul_add(point.x, self.c.mul_add(point.y, self.e)),
            y: self.b.mul_add(point.x, self.d.mul_add(point.y, self.f)),
        }
    }

    fn is_identity(self) -> bool {
        const EPSILON: f32 = 0.0001;
        (self.a - 1.0).abs() <= EPSILON
            && self.b.abs() <= EPSILON
            && self.c.abs() <= EPSILON
            && (self.d - 1.0).abs() <= EPSILON
            && self.e.abs() <= EPSILON
            && self.f.abs() <= EPSILON
    }

    fn stroke_scale(self) -> f32 {
        let x_scale = self.a.hypot(self.b);
        let y_scale = self.c.hypot(self.d);
        x_scale.max(y_scale).max(1.0)
    }
}

fn svg_child_transform(
    stack: &[Option<SvgTransform>],
    attrs: &HashMap<String, String>,
) -> Option<SvgTransform> {
    let parent = stack.last().copied().flatten()?;
    let local = attrs
        .get("transform")
        .map(|value| parse_svg_transform(value))
        .unwrap_or(Some(SvgTransform::identity()))?;
    Some(parent.multiply(local))
}

fn parse_svg_transform(value: &str) -> Option<SvgTransform> {
    let mut cursor = 0usize;
    let mut transform = SvgTransform::identity();
    while cursor < value.len() {
        cursor = skip_svg_number_delimiters(value, cursor);
        if cursor >= value.len() {
            break;
        }
        let name_start = cursor;
        while value
            .as_bytes()
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'-')
        {
            cursor += 1;
        }
        if cursor == name_start {
            return None;
        }
        let name = &value[name_start..cursor];
        while value
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if value.as_bytes().get(cursor) != Some(&b'(') {
            return None;
        }
        cursor += 1;
        let args_start = cursor;
        let args_end = value[cursor..].find(')')?.saturating_add(cursor);
        let args = parse_svg_number_list(&value[args_start..args_end]).unwrap_or_default();
        let local = match name.to_ascii_lowercase().as_str() {
            "translate" => match args.as_slice() {
                [tx] => SvgTransform::translate(*tx, 0.0),
                [tx, ty] => SvgTransform::translate(*tx, *ty),
                _ => return None,
            },
            "scale" => match args.as_slice() {
                [scale] => SvgTransform::scale(*scale, *scale),
                [sx, sy] => SvgTransform::scale(*sx, *sy),
                _ => return None,
            },
            "matrix" => match args.as_slice() {
                [a, b, c, d, e, f] => SvgTransform::matrix(*a, *b, *c, *d, *e, *f),
                _ => return None,
            },
            _ => return None,
        };
        transform = transform.multiply(local);
        cursor = args_end + 1;
    }
    Some(transform)
}

fn svg_shape_fill_paint(attrs: &HashMap<String, String>) -> Option<SvgPaint> {
    svg_shape_paint(attrs, "fill")
}

fn svg_shape_stroke_paint(attrs: &HashMap<String, String>) -> Option<SvgPaint> {
    svg_shape_paint(attrs, "stroke")
}

fn svg_shape_paint(attrs: &HashMap<String, String>, property: &str) -> Option<SvgPaint> {
    let value = svg_shape_paint_value(attrs, property)?;
    let shade = parse_css_color_shade(value)?;
    let rgb = parse_svg_css_color_rgb(value).unwrap_or([shade, shade, shade]);
    Some(SvgPaint { shade, rgb })
}

fn svg_shape_paint_value<'a>(
    attrs: &'a HashMap<String, String>,
    property: &str,
) -> Option<&'a str> {
    attrs
        .get(property)
        .map(String::as_str)
        .and_then(svg_paint_value)
        .or_else(|| svg_style_paint_value(attrs.get("style")?, property))
}

fn svg_style_paint_value<'a>(style: &'a str, property: &str) -> Option<&'a str> {
    style.split(';').find_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(property)
            .then(|| svg_paint_value(value))
            .flatten()
    })
}

fn svg_paint_value(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.eq_ignore_ascii_case("none")
        && !value.eq_ignore_ascii_case("transparent")
        && !value.is_empty())
    .then_some(value)
}

fn parse_svg_css_color_rgb(value: &str) -> Option<[u8; 3]> {
    if let Some(rgb) = parse_svg_rgb_function(value) {
        return Some(rgb);
    }
    for token in value.split_ascii_whitespace() {
        let token = token.trim_matches(|ch: char| ch == ',' || ch == ';');
        if let Some(rgb) = parse_svg_rgb_function(token) {
            return Some(rgb);
        }
        if let Some(rgb) = parse_svg_hex_color_rgb(token) {
            return Some(rgb);
        }
        match token.to_ascii_lowercase().as_str() {
            "black" => return Some([0, 0, 0]),
            "white" => return Some([255, 255, 255]),
            "gray" | "grey" => return Some([128, 128, 128]),
            "silver" => return Some([192, 192, 192]),
            "red" => return Some([255, 0, 0]),
            "green" => return Some([0, 128, 0]),
            "blue" => return Some([0, 0, 255]),
            "yellow" => return Some([255, 255, 0]),
            _ => {}
        }
    }
    None
}

fn parse_svg_hex_color_rgb(value: &str) -> Option<[u8; 3]> {
    let value = value.strip_prefix('#')?;
    match value.len() {
        3 => {
            let red = u8::from_str_radix(&value[0..1], 16).ok()?;
            let green = u8::from_str_radix(&value[1..2], 16).ok()?;
            let blue = u8::from_str_radix(&value[2..3], 16).ok()?;
            Some([red * 17, green * 17, blue * 17])
        }
        6 => Some([
            u8::from_str_radix(&value[0..2], 16).ok()?,
            u8::from_str_radix(&value[2..4], 16).ok()?,
            u8::from_str_radix(&value[4..6], 16).ok()?,
        ]),
        _ => None,
    }
}

fn parse_svg_rgb_function(value: &str) -> Option<[u8; 3]> {
    let value = value.trim();
    let open = value.find('(')?;
    let name = value[..open].trim();
    if !name.eq_ignore_ascii_case("rgb") && !name.eq_ignore_ascii_case("rgba") {
        return None;
    }
    let close = value[open + 1..].find(')')?.saturating_add(open + 1);
    let args = value[open + 1..close].split('/').next().unwrap_or("");
    let normalized = args.replace(',', " ");
    let mut components = normalized.split_ascii_whitespace();
    Some([
        parse_svg_rgb_component(components.next()?)?,
        parse_svg_rgb_component(components.next()?)?,
        parse_svg_rgb_component(components.next()?)?,
    ])
}

fn parse_svg_rgb_component(value: &str) -> Option<u8> {
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

fn svg_shape_stroke_width(attrs: &HashMap<String, String>, transform: SvgTransform) -> usize {
    let width = attrs
        .get("stroke-width")
        .and_then(|value| parse_svg_number(value))
        .or_else(|| svg_style_number(attrs.get("style")?, "stroke-width"))
        .unwrap_or(1.0);
    (width * transform.stroke_scale()).ceil().max(1.0).min(64.0) as usize
}

fn svg_style_number(style: &str, property: &str) -> Option<f32> {
    style.split(';').find_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(property)
            .then(|| parse_svg_number(value))
            .flatten()
    })
}

#[derive(Debug, Clone, Copy)]
struct SvgPoint {
    x: f32,
    y: f32,
}

fn parse_svg_points(value: &str) -> Option<Vec<SvgPoint>> {
    let numbers = parse_svg_number_list(value)?;
    if numbers.len() < 4 || numbers.len() % 2 != 0 {
        return None;
    }
    Some(
        numbers
            .chunks_exact(2)
            .map(|point| SvgPoint {
                x: point[0],
                y: point[1],
            })
            .collect(),
    )
}

fn parse_svg_number_list(value: &str) -> Option<Vec<f32>> {
    let mut numbers = Vec::new();
    let mut cursor = 0usize;
    while cursor < value.len() {
        cursor = skip_svg_number_delimiters(value, cursor);
        if cursor >= value.len() {
            break;
        }
        let (number, next_cursor) = read_svg_number(value, cursor)?;
        numbers.push(number);
        cursor = next_cursor;
    }
    (!numbers.is_empty()).then_some(numbers)
}

const SVG_PATH_CURVE_SEGMENTS: usize = 12;

fn parse_simple_svg_path(value: &str) -> Option<Vec<SvgPoint>> {
    let mut cursor = 0usize;
    let mut command = None;
    let mut current = SvgPoint { x: 0.0, y: 0.0 };
    let mut subpath_start = None;
    let mut last_cubic_control = None;
    let mut last_quadratic_control = None;
    let mut points = Vec::new();

    while cursor < value.len() {
        cursor = skip_svg_number_delimiters(value, cursor);
        if cursor >= value.len() {
            break;
        }
        if let Some(next_command) = value[cursor..]
            .chars()
            .next()
            .filter(|ch| ch.is_ascii_alphabetic())
        {
            if !matches!(
                next_command,
                'M' | 'm'
                    | 'L'
                    | 'l'
                    | 'H'
                    | 'h'
                    | 'V'
                    | 'v'
                    | 'C'
                    | 'c'
                    | 'S'
                    | 's'
                    | 'Q'
                    | 'q'
                    | 'T'
                    | 't'
                    | 'A'
                    | 'a'
                    | 'Z'
                    | 'z'
            ) {
                return None;
            }
            cursor += next_command.len_utf8();
            command = Some(next_command);
            if matches!(next_command, 'Z' | 'z') {
                if let Some(start) = subpath_start {
                    current = start;
                    points.push(start);
                }
                last_cubic_control = None;
                last_quadratic_control = None;
                continue;
            }
        }

        let command = command?;
        match command {
            'M' | 'm' => {
                let (x, y, next_cursor) = read_svg_point_pair(value, cursor)?;
                let point = if command == 'm' {
                    SvgPoint {
                        x: current.x + x,
                        y: current.y + y,
                    }
                } else {
                    SvgPoint { x, y }
                };
                current = point;
                subpath_start = Some(point);
                points.push(point);
                cursor = next_cursor;

                while let Some((x, y, next_cursor)) = try_read_svg_point_pair(value, cursor) {
                    current = svg_path_point(current, x, y, command == 'm');
                    points.push(current);
                    cursor = next_cursor;
                }
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            'L' | 'l' => {
                let mut read_any = false;
                while let Some((x, y, next_cursor)) = try_read_svg_point_pair(value, cursor) {
                    current = svg_path_point(current, x, y, command == 'l');
                    points.push(current);
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            'H' | 'h' => {
                let mut read_any = false;
                while let Some((x, next_cursor)) = try_read_svg_number(value, cursor) {
                    current.x = if command == 'h' { current.x + x } else { x };
                    points.push(current);
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            'V' | 'v' => {
                let mut read_any = false;
                while let Some((y, next_cursor)) = try_read_svg_number(value, cursor) {
                    current.y = if command == 'v' { current.y + y } else { y };
                    points.push(current);
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
                last_cubic_control = None;
                last_quadratic_control = None;
            }
            'C' | 'c' => {
                let mut read_any = false;
                while let Some((x1, y1, cursor_after_control1)) =
                    try_read_svg_point_pair(value, cursor)
                {
                    let (x2, y2, cursor_after_control2) =
                        read_svg_point_pair(value, cursor_after_control1)?;
                    let (x, y, next_cursor) = read_svg_point_pair(value, cursor_after_control2)?;
                    let control1 = svg_path_point(current, x1, y1, command == 'c');
                    let control2 = svg_path_point(current, x2, y2, command == 'c');
                    let end = svg_path_point(current, x, y, command == 'c');
                    push_cubic_svg_path_points(&mut points, current, control1, control2, end);
                    current = end;
                    last_cubic_control = Some(control2);
                    last_quadratic_control = None;
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
            }
            'S' | 's' => {
                let mut read_any = false;
                while let Some((x2, y2, cursor_after_control2)) =
                    try_read_svg_point_pair(value, cursor)
                {
                    let (x, y, next_cursor) = read_svg_point_pair(value, cursor_after_control2)?;
                    let control1 = reflect_svg_path_control(current, last_cubic_control);
                    let control2 = svg_path_point(current, x2, y2, command == 's');
                    let end = svg_path_point(current, x, y, command == 's');
                    push_cubic_svg_path_points(&mut points, current, control1, control2, end);
                    current = end;
                    last_cubic_control = Some(control2);
                    last_quadratic_control = None;
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
            }
            'Q' | 'q' => {
                let mut read_any = false;
                while let Some((x1, y1, cursor_after_control)) =
                    try_read_svg_point_pair(value, cursor)
                {
                    let (x, y, next_cursor) = read_svg_point_pair(value, cursor_after_control)?;
                    let control = svg_path_point(current, x1, y1, command == 'q');
                    let end = svg_path_point(current, x, y, command == 'q');
                    push_quadratic_svg_path_points(&mut points, current, control, end);
                    current = end;
                    last_cubic_control = None;
                    last_quadratic_control = Some(control);
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
            }
            'T' | 't' => {
                let mut read_any = false;
                while let Some((x, y, next_cursor)) = try_read_svg_point_pair(value, cursor) {
                    let control = reflect_svg_path_control(current, last_quadratic_control);
                    let end = svg_path_point(current, x, y, command == 't');
                    push_quadratic_svg_path_points(&mut points, current, control, end);
                    current = end;
                    last_cubic_control = None;
                    last_quadratic_control = Some(control);
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
            }
            'A' | 'a' => {
                let mut read_any = false;
                while let Some((rx, cursor_after_rx)) = try_read_svg_number(value, cursor) {
                    let (ry, cursor_after_ry) = try_read_svg_number(value, cursor_after_rx)?;
                    let (rotation, cursor_after_rotation) =
                        try_read_svg_number(value, cursor_after_ry)?;
                    let (large_arc, cursor_after_large_arc) =
                        read_svg_arc_flag(value, cursor_after_rotation)?;
                    let (sweep, cursor_after_sweep) =
                        read_svg_arc_flag(value, cursor_after_large_arc)?;
                    let (x, y, next_cursor) = read_svg_point_pair(value, cursor_after_sweep)?;
                    let end = svg_path_point(current, x, y, command == 'a');
                    push_arc_svg_path_points(
                        &mut points,
                        current,
                        rx,
                        ry,
                        rotation,
                        large_arc,
                        sweep,
                        end,
                    );
                    current = end;
                    last_cubic_control = None;
                    last_quadratic_control = None;
                    cursor = next_cursor;
                    read_any = true;
                }
                if !read_any {
                    return None;
                }
            }
            'Z' | 'z' => {}
            _ => return None,
        }
    }

    (points.len() >= 2).then_some(points)
}

fn svg_path_point(current: SvgPoint, x: f32, y: f32, relative: bool) -> SvgPoint {
    if relative {
        SvgPoint {
            x: current.x + x,
            y: current.y + y,
        }
    } else {
        SvgPoint { x, y }
    }
}

fn reflect_svg_path_control(current: SvgPoint, control: Option<SvgPoint>) -> SvgPoint {
    control
        .map(|control| SvgPoint {
            x: current.x.mul_add(2.0, -control.x),
            y: current.y.mul_add(2.0, -control.y),
        })
        .unwrap_or(current)
}

fn push_cubic_svg_path_points(
    points: &mut Vec<SvgPoint>,
    start: SvgPoint,
    control1: SvgPoint,
    control2: SvgPoint,
    end: SvgPoint,
) {
    for step in 1..=SVG_PATH_CURVE_SEGMENTS {
        let t = step as f32 / SVG_PATH_CURVE_SEGMENTS as f32;
        let inverse = 1.0 - t;
        let inverse_squared = inverse * inverse;
        let t_squared = t * t;
        points.push(SvgPoint {
            x: inverse_squared * inverse * start.x
                + 3.0 * inverse_squared * t * control1.x
                + 3.0 * inverse * t_squared * control2.x
                + t_squared * t * end.x,
            y: inverse_squared * inverse * start.y
                + 3.0 * inverse_squared * t * control1.y
                + 3.0 * inverse * t_squared * control2.y
                + t_squared * t * end.y,
        });
    }
}

fn push_quadratic_svg_path_points(
    points: &mut Vec<SvgPoint>,
    start: SvgPoint,
    control: SvgPoint,
    end: SvgPoint,
) {
    for step in 1..=SVG_PATH_CURVE_SEGMENTS {
        let t = step as f32 / SVG_PATH_CURVE_SEGMENTS as f32;
        let inverse = 1.0 - t;
        points.push(SvgPoint {
            x: inverse * inverse * start.x + 2.0 * inverse * t * control.x + t * t * end.x,
            y: inverse * inverse * start.y + 2.0 * inverse * t * control.y + t * t * end.y,
        });
    }
}

fn push_arc_svg_path_points(
    points: &mut Vec<SvgPoint>,
    start: SvgPoint,
    rx: f32,
    ry: f32,
    x_axis_rotation: f32,
    large_arc: bool,
    sweep: bool,
    end: SvgPoint,
) {
    if (start.x - end.x).abs() <= f32::EPSILON && (start.y - end.y).abs() <= f32::EPSILON {
        return;
    }
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    if rx <= f32::EPSILON || ry <= f32::EPSILON {
        points.push(end);
        return;
    }

    let rotation = x_axis_rotation.to_radians();
    let cos_rotation = rotation.cos();
    let sin_rotation = rotation.sin();
    let dx = (start.x - end.x) / 2.0;
    let dy = (start.y - end.y) / 2.0;
    let x1_prime = cos_rotation.mul_add(dx, sin_rotation * dy);
    let y1_prime = (-sin_rotation).mul_add(dx, cos_rotation * dy);
    let radii_scale = (x1_prime * x1_prime / (rx * rx) + y1_prime * y1_prime / (ry * ry)).sqrt();
    if radii_scale > 1.0 {
        rx *= radii_scale;
        ry *= radii_scale;
    }

    let rx_squared = rx * rx;
    let ry_squared = ry * ry;
    let x1_prime_squared = x1_prime * x1_prime;
    let y1_prime_squared = y1_prime * y1_prime;
    let denominator = rx_squared * y1_prime_squared + ry_squared * x1_prime_squared;
    if denominator <= f32::EPSILON {
        points.push(end);
        return;
    }
    let numerator =
        (rx_squared * ry_squared - rx_squared * y1_prime_squared - ry_squared * x1_prime_squared)
            .max(0.0);
    let center_scale =
        if large_arc == sweep { -1.0 } else { 1.0 } * (numerator / denominator).sqrt();
    let center_x_prime = center_scale * rx * y1_prime / ry;
    let center_y_prime = center_scale * -ry * x1_prime / rx;
    let center_x = cos_rotation.mul_add(center_x_prime, -sin_rotation * center_y_prime)
        + (start.x + end.x) / 2.0;
    let center_y = sin_rotation.mul_add(center_x_prime, cos_rotation * center_y_prime)
        + (start.y + end.y) / 2.0;

    let start_vector = SvgPoint {
        x: (x1_prime - center_x_prime) / rx,
        y: (y1_prime - center_y_prime) / ry,
    };
    let end_vector = SvgPoint {
        x: (-x1_prime - center_x_prime) / rx,
        y: (-y1_prime - center_y_prime) / ry,
    };
    let start_angle = svg_vector_angle(SvgPoint { x: 1.0, y: 0.0 }, start_vector);
    let mut delta_angle = svg_vector_angle(start_vector, end_vector);
    if !sweep && delta_angle > 0.0 {
        delta_angle -= std::f32::consts::TAU;
    } else if sweep && delta_angle < 0.0 {
        delta_angle += std::f32::consts::TAU;
    }

    let segments =
        ((delta_angle.abs() / (std::f32::consts::PI / 8.0)).ceil() as usize).clamp(4, 64);
    for step in 1..=segments {
        let angle = start_angle + delta_angle * step as f32 / segments as f32;
        let cos_angle = angle.cos();
        let sin_angle = angle.sin();
        points.push(SvgPoint {
            x: center_x + rx * cos_rotation * cos_angle - ry * sin_rotation * sin_angle,
            y: center_y + rx * sin_rotation * cos_angle + ry * cos_rotation * sin_angle,
        });
    }
}

fn svg_vector_angle(from: SvgPoint, to: SvgPoint) -> f32 {
    (from.x * to.y - from.y * to.x).atan2(from.x * to.x + from.y * to.y)
}

fn read_svg_point_pair(value: &str, cursor: usize) -> Option<(f32, f32, usize)> {
    try_read_svg_point_pair(value, cursor)
}

fn try_read_svg_point_pair(value: &str, cursor: usize) -> Option<(f32, f32, usize)> {
    let (x, cursor) = try_read_svg_number(value, cursor)?;
    let (y, cursor) = try_read_svg_number(value, cursor)?;
    Some((x, y, cursor))
}

fn try_read_svg_number(value: &str, cursor: usize) -> Option<(f32, usize)> {
    let cursor = skip_svg_number_delimiters(value, cursor);
    if cursor >= value.len() {
        return None;
    }
    if value[cursor..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }
    read_svg_number(value, cursor)
}

fn read_svg_arc_flag(value: &str, cursor: usize) -> Option<(bool, usize)> {
    let cursor = skip_svg_number_delimiters(value, cursor);
    let flag = value[cursor..].chars().next()?;
    match flag {
        '0' => Some((false, cursor + flag.len_utf8())),
        '1' => Some((true, cursor + flag.len_utf8())),
        _ => None,
    }
}

fn read_svg_number(value: &str, cursor: usize) -> Option<(f32, usize)> {
    let bytes = value.as_bytes();
    let start = cursor;
    let mut cursor = cursor;
    if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
        cursor += 1;
    }
    let mut has_digits = false;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
        has_digits = true;
    }
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
            has_digits = true;
        }
    }
    if !has_digits {
        return None;
    }
    if matches!(bytes.get(cursor), Some(b'e' | b'E')) {
        let exponent_start = cursor;
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        let exponent_digits_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == exponent_digits_start {
            cursor = exponent_start;
        }
    }
    let number = value[start..cursor].parse::<f32>().ok()?;
    number.is_finite().then_some((number, cursor))
}

fn skip_svg_number_delimiters(value: &str, mut cursor: usize) -> usize {
    while cursor < value.len() {
        let Some(character) = value[cursor..].chars().next() else {
            break;
        };
        if character.is_ascii_whitespace() || character == ',' {
            cursor += character.len_utf8();
        } else {
            break;
        }
    }
    cursor
}

fn fill_decoded_rect(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    image_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    paint: SvgPaint,
) {
    for row in y..y.saturating_add(height) {
        for column in x..x.saturating_add(width) {
            set_decoded_pixel(
                pixels,
                rgb_pixels,
                row.saturating_mul(image_width).saturating_add(column),
                paint,
            );
        }
    }
}

fn svg_rect_points(
    rect: &HashMap<String, String>,
    image_width: usize,
    image_height: usize,
) -> Option<Vec<SvgPoint>> {
    let x = rect
        .get("x")
        .and_then(|value| parse_svg_number(value))
        .unwrap_or(0.0);
    let y = rect
        .get("y")
        .and_then(|value| parse_svg_number(value))
        .unwrap_or(0.0);
    let width = rect
        .get("width")
        .and_then(|value| parse_svg_number(value))
        .unwrap_or((image_width as f32 - x).max(0.0));
    let height = rect
        .get("height")
        .and_then(|value| parse_svg_number(value))
        .unwrap_or((image_height as f32 - y).max(0.0));
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some(vec![
        SvgPoint { x, y },
        SvgPoint { x: x + width, y },
        SvgPoint {
            x: x + width,
            y: y + height,
        },
        SvgPoint { x, y: y + height },
    ])
}

fn svg_ellipse_points(ellipse: SvgEllipse, segments: usize) -> Vec<SvgPoint> {
    let segments = segments.max(8);
    (0..segments)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / segments as f32;
            SvgPoint {
                x: ellipse.cx + ellipse.rx * angle.cos(),
                y: ellipse.cy + ellipse.ry * angle.sin(),
            }
        })
        .collect()
}

fn transform_svg_points(points: &[SvgPoint], transform: SvgTransform) -> Vec<SvgPoint> {
    if transform.is_identity() {
        return points.to_vec();
    }
    points
        .iter()
        .copied()
        .map(|point| transform.apply(point))
        .collect()
}

fn fill_decoded_polygon(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    points: &[SvgPoint],
    paint: SvgPaint,
) {
    if points.len() < 3 {
        return;
    }
    let min_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let max_x = points
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_y = points
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let start_x = min_x.floor().max(0.0) as usize;
    let end_x = max_x.ceil().max(0.0).min(image_width as f32) as usize;
    let start_y = min_y.floor().max(0.0) as usize;
    let end_y = max_y.ceil().max(0.0).min(image_height as f32) as usize;
    for y in start_y..end_y {
        for x in start_x..end_x {
            if !svg_point_inside_polygon(x as f32 + 0.5, y as f32 + 0.5, points) {
                continue;
            }
            set_decoded_pixel(
                pixels,
                rgb_pixels,
                y.saturating_mul(image_width).saturating_add(x),
                paint,
            );
        }
    }
}

fn svg_point_inside_polygon(x: f32, y: f32, points: &[SvgPoint]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let current_point = points[current];
        let previous_point = points[previous];
        if (current_point.y > y) != (previous_point.y > y) {
            let intersection_x = (previous_point.x - current_point.x) * (y - current_point.y)
                / (previous_point.y - current_point.y)
                + current_point.x;
            if x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn draw_decoded_polyline(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    points: &[SvgPoint],
    paint: SvgPaint,
    stroke_width: usize,
) {
    let stroke_width = stroke_width.max(1);
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let steps = (end.x - start.x)
            .abs()
            .max((end.y - start.y).abs())
            .ceil()
            .max(1.0) as usize;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = (start.x + (end.x - start.x) * t).round() as isize;
            let y = (start.y + (end.y - start.y) * t).round() as isize;
            if x < 0 || y < 0 {
                continue;
            }
            let x = x as usize;
            let y = y as usize;
            if x >= image_width || y >= image_height {
                continue;
            }
            set_decoded_stroke_pixel(
                pixels,
                rgb_pixels,
                image_width,
                image_height,
                x,
                y,
                stroke_width,
                paint,
            );
        }
    }
}

fn set_decoded_stroke_pixel(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    x: usize,
    y: usize,
    stroke_width: usize,
    paint: SvgPaint,
) {
    let radius = stroke_width.saturating_sub(1) / 2;
    let start_x = x.saturating_sub(radius);
    let end_x = x.saturating_add(radius).min(image_width.saturating_sub(1));
    let start_y = y.saturating_sub(radius);
    let end_y = y.saturating_add(radius).min(image_height.saturating_sub(1));
    for row in start_y..=end_y {
        for column in start_x..=end_x {
            set_decoded_pixel(
                pixels,
                rgb_pixels,
                row.saturating_mul(image_width).saturating_add(column),
                paint,
            );
        }
    }
}

fn set_decoded_pixel(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    pixel_index: usize,
    paint: SvgPaint,
) {
    let Some(pixel) = pixels.get_mut(pixel_index) else {
        return;
    };
    *pixel = paint.shade;
    let Some(rgb_offset) = pixel_index.checked_mul(3) else {
        return;
    };
    let Some(pixel_rgb) = rgb_pixels.get_mut(rgb_offset..rgb_offset.saturating_add(3)) else {
        return;
    };
    pixel_rgb.copy_from_slice(&paint.rgb);
}

#[derive(Debug, Clone, Copy)]
struct SvgPaint {
    shade: u8,
    rgb: [u8; 3],
}

#[derive(Debug, Clone, Copy)]
struct SvgEllipse {
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
}

fn fill_decoded_ellipse(
    pixels: &mut [u8],
    rgb_pixels: &mut [u8],
    image_width: usize,
    image_height: usize,
    ellipse: SvgEllipse,
    paint: SvgPaint,
) {
    if ellipse.rx <= 0.0 || ellipse.ry <= 0.0 {
        return;
    }
    let min_x = ellipse.cx - ellipse.rx;
    let max_x = ellipse.cx + ellipse.rx;
    let min_y = ellipse.cy - ellipse.ry;
    let max_y = ellipse.cy + ellipse.ry;
    let start_x = min_x.floor().max(0.0) as usize;
    let end_x = max_x.ceil().max(0.0).min(image_width as f32) as usize;
    let start_y = min_y.floor().max(0.0) as usize;
    let end_y = max_y.ceil().max(0.0).min(image_height as f32) as usize;
    for y in start_y..end_y {
        for x in start_x..end_x {
            let dx = (x as f32 + 0.5 - ellipse.cx) / ellipse.rx;
            let dy = (y as f32 + 0.5 - ellipse.cy) / ellipse.ry;
            if dx.mul_add(dx, dy * dy) > 1.0 {
                continue;
            }
            set_decoded_pixel(
                pixels,
                rgb_pixels,
                y.saturating_mul(image_width).saturating_add(x),
                paint,
            );
        }
    }
}

pub(super) fn image_render_source(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    viewport_width_css_px: usize,
) -> Option<String> {
    let srcset_target_width =
        srcset_target_width_from_sizes(image_sizes_attr(element), viewport_width_css_px);
    let selected_source = picture_source_srcset(dom, node_id, viewport_width_css_px)
        .and_then(|source| {
            let source_target_width =
                srcset_target_width_from_sizes(source.sizes, viewport_width_css_px);
            choose_srcset_candidate(source.srcset, source_target_width)
        })
        .or_else(|| {
            element
                .srcset
                .as_deref()
                .and_then(|srcset| choose_srcset_candidate(srcset, srcset_target_width))
        })
        .or_else(|| element.src.clone());
    if selected_source.as_deref().is_none_or(|source| {
        is_lazy_image_placeholder_src(source) || image_source_clearly_unsupported(source)
    }) && let Some(lazy_source) = lazy_image_render_source(
        dom,
        node_id,
        element,
        srcset_target_width,
        viewport_width_css_px,
    ) {
        return Some(lazy_source);
    }
    selected_source
}

pub(super) fn background_image_render_source(element: &ElementData) -> Option<String> {
    first_non_empty_attr(
        element,
        &[
            "data-bgset",
            "data-background-srcset",
            "data-backgroundsrcset",
            "data-lazy-bgset",
            "data-lazybgset",
            "data-lazy-background-srcset",
            "data-lazybackgroundsrcset",
        ],
    )
    .and_then(|srcset| choose_srcset_candidate(srcset, background_srcset_target_width(element)))
    .or_else(|| {
        first_non_empty_attr(
            element,
            &[
                "data-bg",
                "data-bg-src",
                "data-bgsrc",
                "data-background",
                "data-background-image",
                "data-backgroundimage",
                "data-background-src",
                "data-backgroundsrc",
                "data-lazy-bg",
                "data-lazybg",
                "data-lazy-background",
                "data-lazybackground",
                "data-lazy-background-image",
                "data-lazybackgroundimage",
            ],
        )
        .and_then(background_image_attr_source)
    })
}

fn background_srcset_target_width(element: &ElementData) -> Option<usize> {
    image_sizes_attr(element).and_then(|sizes| parse_sizes_attribute(sizes, 0))
}

fn background_image_attr_source(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches(';').trim();
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(url) = background_image_set_attr_source(value) {
        return Some(url);
    }
    let url = value
        .strip_prefix("url(")
        .and_then(|url| url.strip_suffix(')'))
        .map(|url| url.trim().trim_matches(['"', '\'']))
        .unwrap_or(value)
        .trim();
    if url.is_empty() || image_source_clearly_unsupported(url) {
        return None;
    }
    Some(url.to_owned())
}

fn background_image_set_attr_source(value: &str) -> Option<String> {
    let args = css_function_args(value, &["image-set", "-webkit-image-set"])?;
    split_css_top_level_commas_with_quotes(args)
        .into_iter()
        .filter_map(background_image_set_candidate_source)
        .find(|url| !image_source_clearly_unsupported(url))
}

fn background_image_set_candidate_source(candidate: &str) -> Option<String> {
    if background_image_set_candidate_type_unsupported(candidate) {
        return None;
    }
    css_function_args(candidate, &["url"])
        .and_then(css_url_token)
        .or_else(|| css_quoted_url(candidate))
        .map(str::to_owned)
}

fn background_image_set_candidate_type_unsupported(candidate: &str) -> bool {
    css_nested_function_args(candidate, &["type"])
        .and_then(css_url_token)
        .is_some_and(|image_type| !image_mime_type_supported(image_type))
}

fn css_nested_function_args<'a>(value: &'a str, names: &[&str]) -> Option<&'a str> {
    for (open, _) in value.match_indices('(') {
        let name_start = value[..open]
            .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
            .map(|index| index.saturating_add(1))
            .unwrap_or(0);
        let name = value[name_start..open].trim();
        if names
            .iter()
            .any(|expected| name.eq_ignore_ascii_case(expected))
        {
            let close = matching_closing_paren(value, open)?;
            return Some(&value[open + 1..close]);
        }
    }
    None
}

fn css_function_args<'a>(value: &'a str, names: &[&str]) -> Option<&'a str> {
    let value = value.trim();
    let open = value.find('(')?;
    let name = value[..open].trim();
    if !names
        .iter()
        .any(|expected| name.eq_ignore_ascii_case(expected))
    {
        return None;
    }
    let close = matching_closing_paren(value, open)?;
    Some(&value[open + 1..close])
}

fn split_css_top_level_commas_with_quotes(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut start = 0usize;
    for (index, byte) in input.as_bytes().iter().enumerate() {
        if let Some(quote_byte) = quote {
            if *byte == quote_byte {
                quote = None;
            }
            continue;
        }
        match *byte {
            b'\'' | b'"' => quote = Some(*byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(input[start..index].trim());
                start = index.saturating_add(1);
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

fn css_url_token(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let bytes = value.as_bytes();
    let url = if bytes.len() >= 2
        && matches!(bytes[0], b'\'' | b'"')
        && bytes.last() == Some(&bytes[0])
    {
        &value[1..value.len().saturating_sub(1)]
    } else {
        value
    };
    (!url.is_empty()).then_some(url)
}

fn css_quoted_url(value: &str) -> Option<&str> {
    let value = value.trim_start();
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let end = value[1..]
        .find(quote as char)
        .map(|offset| 1usize.saturating_add(offset))?;
    css_url_token(&value[..=end])
}

#[derive(Debug, Clone, Copy)]
struct PictureSourceSet<'a> {
    srcset: &'a str,
    sizes: Option<&'a str>,
}

fn picture_source_srcset(
    dom: &Dom,
    img_node_id: usize,
    viewport_width_css_px: usize,
) -> Option<PictureSourceSet<'_>> {
    picture_source_attr(dom, img_node_id, &["srcset"], viewport_width_css_px)
}

fn picture_source_lazy_srcset(
    dom: &Dom,
    img_node_id: usize,
    viewport_width_css_px: usize,
) -> Option<PictureSourceSet<'_>> {
    picture_source_attr(
        dom,
        img_node_id,
        &[
            "data-srcset",
            "data-lazy-srcset",
            "data-lazysrcset",
            "data-lazyload-srcset",
            "data-original-srcset",
            "data-originalset",
            "data-originalsrcset",
            "data-flickity-lazyload-srcset",
            "data-image-srcset",
            "data-imagesrcset",
            "data-img-srcset",
            "data-imgsrcset",
            "data-current-srcset",
            "data-currentsrcset",
            "current-srcset",
            "currentsrcset",
            "data-src",
            "data-lazy-src",
            "data-lazysrc",
            "data-lazyload",
            "data-lazyload-src",
            "data-original-url",
            "data-original",
            "data-original-src",
            "data-originalsrc",
            "data-flickity-lazyload",
            "data-flickity-lazyload-src",
            "data-image",
            "data-image-src",
            "data-imagesrc",
            "data-img-src",
            "data-imgsrc",
            "data-current-src",
            "data-currentsrc",
            "current-src",
            "currentsrc",
        ],
        viewport_width_css_px,
    )
}

fn picture_source_attr<'a>(
    dom: &'a Dom,
    img_node_id: usize,
    attr_names: &[&str],
    viewport_width_css_px: usize,
) -> Option<PictureSourceSet<'a>> {
    let parent = dom.nodes.get(img_node_id)?.parent?;
    let parent_node = dom.nodes.get(parent)?;
    if !matches!(&parent_node.kind, NodeKind::Element(element) if element.tag == "picture") {
        return None;
    }

    for &child in &parent_node.children {
        if child == img_node_id {
            break;
        }
        if let Some(NodeKind::Element(element)) = dom.nodes.get(child).map(|node| &node.kind)
            && element.tag == "source"
            && picture_source_media_matches(element.media.as_deref(), viewport_width_css_px)
            && picture_source_type_supported(element)
            && let Some(srcset) = first_non_empty_attr(element, attr_names)
        {
            if is_empty_picture_placeholder_source(element, srcset) {
                continue;
            }
            if srcset_all_candidates_clearly_unsupported(srcset) {
                continue;
            }
            return Some(PictureSourceSet {
                srcset,
                sizes: image_sizes_attr(element),
            });
        }
    }
    None
}

fn lazy_image_render_source(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    desired_width: Option<usize>,
    viewport_width_css_px: usize,
) -> Option<String> {
    picture_source_lazy_srcset(dom, node_id, viewport_width_css_px)
        .and_then(|source| {
            let source_target_width =
                srcset_target_width_from_sizes(source.sizes, viewport_width_css_px);
            choose_srcset_candidate(source.srcset, source_target_width)
        })
        .or_else(|| {
            first_non_empty_attr(
                element,
                &[
                    "data-srcset",
                    "data-lazy-srcset",
                    "data-lazysrcset",
                    "data-lazyload-srcset",
                    "data-original-srcset",
                    "data-originalset",
                    "data-originalsrcset",
                    "data-flickity-lazyload-srcset",
                    "data-image-srcset",
                    "data-imagesrcset",
                    "data-img-srcset",
                    "data-imgsrcset",
                    "data-current-srcset",
                    "data-currentsrcset",
                    "current-srcset",
                    "currentsrcset",
                ],
            )
            .and_then(|srcset| choose_srcset_candidate(srcset, desired_width))
        })
        .or_else(|| {
            first_non_empty_attr(
                element,
                &[
                    "data-src",
                    "data-lazy-src",
                    "data-lazysrc",
                    "data-lazyload",
                    "data-lazyload-src",
                    "data-original-url",
                    "data-original",
                    "data-original-src",
                    "data-originalsrc",
                    "data-flickity-lazyload",
                    "data-flickity-lazyload-src",
                    "data-image",
                    "data-image-src",
                    "data-imagesrc",
                    "data-img-src",
                    "data-imgsrc",
                    "data-current-src",
                    "data-currentsrc",
                    "current-src",
                    "currentsrc",
                ],
            )
            .map(str::to_owned)
        })
}

fn first_non_empty_attr<'a>(element: &'a ElementData, attr_names: &[&str]) -> Option<&'a str> {
    attr_names.iter().find_map(|attr_name| {
        if *attr_name == "srcset" {
            element.srcset.as_deref()
        } else {
            element.attrs.get(*attr_name).map(String::as_str)
        }
        .filter(|value| !value.trim().is_empty())
    })
}

pub(super) fn image_sizes_attr(element: &ElementData) -> Option<&str> {
    first_non_empty_attr(
        element,
        &[
            "sizes",
            "data-sizes",
            "data-lazy-sizes",
            "data-lazysizes",
            "data-lazyload-sizes",
            "data-original-sizes",
            "data-originalsizes",
            "data-flickity-lazyload-sizes",
            "data-image-sizes",
            "data-imagesizes",
            "data-img-sizes",
            "data-imgsizes",
            "data-current-sizes",
            "data-currentsizes",
            "current-sizes",
            "currentsizes",
        ],
    )
}

fn is_lazy_image_placeholder_src(src: &str) -> bool {
    let src = src.trim_start().to_ascii_lowercase();
    if src.starts_with("data:image/svg+xml")
        || src.starts_with("data:image/svg")
        || src.starts_with("data:image/png")
        || src.starts_with("data:image/x-png")
        || src.starts_with("data:image/gif")
    {
        return true;
    }
    let path = src
        .split(['?', '#'])
        .next()
        .unwrap_or(src.as_str())
        .trim_end_matches('/');
    let filename = path.rsplit('/').next().unwrap_or(path);
    matches!(
        filename,
        "blank.gif"
            | "blank.png"
            | "blank.svg"
            | "spacer.gif"
            | "spacer.png"
            | "spacer.svg"
            | "transparent.gif"
            | "transparent.png"
            | "transparent.svg"
            | "pixel.gif"
            | "pixel.png"
            | "1x1.gif"
            | "1x1.png"
    ) || filename.contains("placeholder")
}

fn is_empty_picture_placeholder_source(element: &ElementData, srcset: &str) -> bool {
    if !element.attrs.contains_key("data-empty") {
        return false;
    }
    let urls = srcset_candidate_urls(srcset);
    !urls.is_empty() && urls.iter().all(|url| is_lazy_image_placeholder_src(url))
}

fn image_source_clearly_unsupported(url: &str) -> bool {
    srcset_candidate_clearly_unsupported(url)
}

#[cfg(test)]
pub(super) fn tiny_test_jpeg_bytes() -> Vec<u8> {
    decode_base64(TINY_TEST_JPEG_BASE64).unwrap()
}

#[cfg(test)]
pub(super) fn tiny_test_jpeg_data_url() -> String {
    format!("data:image/jpeg;base64,{TINY_TEST_JPEG_BASE64}")
}

#[cfg(test)]
pub(super) fn tiny_test_webp_bytes() -> Vec<u8> {
    decode_base64(TINY_TEST_WEBP_BASE64).unwrap()
}

#[cfg(test)]
pub(super) fn tiny_test_webp_data_url() -> String {
    format!("data:image/webp;base64,{TINY_TEST_WEBP_BASE64}")
}

#[cfg(test)]
fn test_jpeg_data_url_with_mime_type(mime_type: &str) -> String {
    format!("data:{mime_type};base64,{TINY_TEST_JPEG_BASE64}")
}

#[cfg(test)]
pub(super) fn test_webp_data_url_with_mime_type(mime_type: &str) -> String {
    format!("data:{mime_type};base64,{TINY_TEST_WEBP_BASE64}")
}

#[cfg(test)]
fn progressive_test_jpeg_bytes() -> Vec<u8> {
    decode_base64(PROGRESSIVE_TEST_JPEG_BASE64).unwrap()
}

#[cfg(test)]
fn grayscale_test_jpeg_bytes() -> Vec<u8> {
    decode_base64(GRAYSCALE_TEST_JPEG_BASE64).unwrap()
}

#[cfg(test)]
fn jpeg_with_app_segment(bytes: &[u8], marker: u8, payload: &[u8]) -> Vec<u8> {
    assert!(is_jpeg_bytes(bytes));
    assert!((0xe0..=0xef).contains(&marker));

    let segment_len = u16::try_from(payload.len() + 2).unwrap();
    let mut with_segment = Vec::with_capacity(bytes.len() + payload.len() + 4);
    with_segment.extend_from_slice(&bytes[..2]);
    with_segment.extend_from_slice(&[0xff, marker]);
    with_segment.extend_from_slice(&segment_len.to_be_bytes());
    with_segment.extend_from_slice(payload);
    with_segment.extend_from_slice(&bytes[2..]);
    with_segment
}

#[cfg(test)]
fn jpeg_with_exif_tiff(bytes: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(tiff);
    jpeg_with_app_segment(bytes, 0xe1, &payload)
}

#[cfg(test)]
fn jpeg_with_exif_orientation(bytes: &[u8], orientation: u16) -> Vec<u8> {
    assert!((1..=8).contains(&orientation));

    let mut tiff = b"II*\0\x08\0\0\0\x01\0".to_vec();
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&0u16.to_le_bytes());
    tiff.extend_from_slice(&0u32.to_le_bytes());

    jpeg_with_exif_tiff(bytes, &tiff)
}

#[cfg(test)]
fn big_endian_exif_orientation_tiff(orientation: u16) -> Vec<u8> {
    assert!((1..=8).contains(&orientation));

    let mut tiff = b"MM\0*\0\0\0\x08\0\x01".to_vec();
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&0u16.to_be_bytes());
    tiff.extend_from_slice(&0u32.to_be_bytes());
    tiff
}

#[cfg(test)]
const TINY_TEST_JPEG_BASE64: &str = concat!(
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ",
    "EBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/2wBDAQEBAQEBAQEBAQEB",
    "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/",
    "wAARCAACAAIDAREAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8",
    "QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM",
    "2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4",
    "eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19j",
    "Z2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcI",
    "CQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCS",
    "MzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hp",
    "anN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyM",
    "nK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD+dK9/4KC/t7fD",
    "a8u/h18O/wBt39rzwD8PvANzP4L8C+BfBf7Snxm8LeDvBfg7wtK+h+GPCfhPwxofjSw0Tw",
    "54Z8OaJY2Oj6DoOj2NnpekaXZ2un6fa29pbwwp/wBcn0Mfo2fR14p+h79FDififwD8FeI+",
    "JOI/o1eBWfcQ8Q594WcDZvnme55m/hdwtmGbZznObZhkWIx+aZrmmPxGIx2Y5jjsRXxmNx",
    "leticTWq1qs5y836WlSfDf0qfpMcO8OznkPD+Q/SC8ZslyPI8lk8ryfJcnyvxG4kwOWZTl",
    "OWYF0MFl2WZdgqFDB4DAYOhRwuEwtGlh8PSp0qcIL//Z",
);

#[cfg(test)]
const PROGRESSIVE_TEST_JPEG_BASE64: &str = concat!(
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAIBAQEBAQIBAQECAgICAgQDAgICAgUEBAME",
    "BgUGBgYFBgYGBwkIBgcJBwYGCAsICQoKCgoKBggLDAsKDAkKCgr/2wBDAQICAgICAgUD",
    "AwUKBwYHCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoK",
    "CgoKCgr/wgARCAACAAMDAREAAhEBAxEB/8QAFAABAAAAAAAAAAAAAAAAAAAAB//EABUB",
    "AQEAAAAAAAAAAAAAAAAAAAYH/9oADAMBAAIQAxAAAAEhoTL/xAAWEAEBAQAAAAAAAAAA",
    "AAAAAAAEBQb/2gAIAQEAAQUC0dB8pP8A/8QAHREBAQACAgMBAAAAAAAAAAAAAQIDBQQG",
    "ACEiUf/aAAgBAwEBPwHUdc69udZi5ew4eLNlQG7xxdJPzI1QvzISfgAejz//xAAeEQAB",
    "AwQDAAAAAAAAAAAAAAABAgNBAAQFBiEjQ//aAAgBAgEBPwHKavrLmXuyqyZPc75ohxQE",
    "QOK//8QAHBAAAgICAwAAAAAAAAAAAAAAAQIDBAAFEVFx/9oACAEBAAY/AqlLV3Za0I0u",
    "vYRQSFFBanCzHgdkk+nP/8QAGBABAAMBAAAAAAAAAAAAAAAAAQARIUH/2gAIAQEAAT8h",
    "J/a8oChah6i6z//aAAwDAQACAAMAAAAQH//EABYRAQEBAAAAAAAAAAAAAAAAAAERIf/a",
    "AAgBAwEBPxDf4cIiCg6snQB//8QAGBEBAAMBAAAAAAAAAAAAAAAAAREhMQD/2gAIAQIB",
    "AT8QQuAyswMvIAMAAo7/xAAVEAEBAAAAAAAAAAAAAAAAAAABEf/aAAgBAQABPxAfuFoA",
    "In8WzIv/2Q==",
);

#[cfg(test)]
const GRAYSCALE_TEST_JPEG_BASE64: &str = concat!(
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAIBAQEBAQIBAQECAgICAgQDAgICAgUEBAME",
    "BgUGBgYFBgYGBwkIBgcJBwYGCAsICQoKCgoKBggLDAsKDAkKCgr/wAALCAACAAMBAREA",
    "/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQA",
    "AAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJico",
    "KSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKT",
    "lJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo",
    "6erx8vP09fb3+Pn6/9oACAEBAAA/APkv/gsR4i8Qfs//APBQfxf8I/gPrl54J8KaR4f",
    "8Lf2V4Y8I3T6bp1l5vhzTJpfKtrcpFHvlkkkbao3PIzHJYk//2Q==",
);

#[cfg(test)]
const TINY_TEST_WEBP_BASE64: &str = "UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AAAAAA";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_jpeg_bytes_into_grayscale_pixels() {
        let decoded = decode_image_bytes("jpg", &tiny_test_jpeg_bytes()).unwrap();

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.pixels, vec![0, 255, 76, 29]);
    }

    #[test]
    fn downscales_jpeg_before_decode_when_pixel_buffer_would_exceed_limit() {
        let decoded = decode_jpeg_with_max_side(&tiny_test_jpeg_bytes(), 1).unwrap();

        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.pixels.len(), 1);
    }

    #[test]
    fn decodes_jpeg_from_legacy_mime_aliases() {
        let bytes = tiny_test_jpeg_bytes();

        assert!(decode_image_bytes("jpe", &bytes).is_some());
        assert!(decode_image_bytes("jfif", &bytes).is_some());
        assert!(decode_image_bytes("pjpeg", &bytes).is_some());
        assert!(decode_image_bytes("pjp", &bytes).is_some());
        assert!(decode_image_bytes("image/pjpeg", &bytes).is_some());
        assert!(decode_image_bytes("image/x-jpeg", &bytes).is_some());
    }

    #[test]
    fn decodes_progressive_jpeg_into_grayscale_pixels() {
        let decoded = decode_image_bytes("image/jpeg", &progressive_test_jpeg_bytes()).unwrap();

        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.pixels.len(), 6);
        assert!(decoded.pixels[0] <= 8);
        assert!((70..=85).contains(&decoded.pixels[1]));
        assert!((140..=160).contains(&decoded.pixels[2]));
        assert!((20..=40).contains(&decoded.pixels[3]));
        assert!(decoded.pixels[4] >= 245);
        assert!((45..=65).contains(&decoded.pixels[5]));
    }

    #[test]
    fn decodes_grayscale_jpeg_without_rgb_conversion() {
        let decoded = decode_image_bytes("image/jpeg", &grayscale_test_jpeg_bytes()).unwrap();

        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.pixels.len(), 6);
        assert!(decoded.pixels[0] <= 4);
        assert!((58..=68).contains(&decoded.pixels[1]));
        assert!((124..=134).contains(&decoded.pixels[2]));
        assert!((186..=196).contains(&decoded.pixels[3]));
        assert!(decoded.pixels[4] >= 250);
        assert!((24..=34).contains(&decoded.pixels[5]));
    }

    #[test]
    fn decodes_webp_bytes_by_content_type_and_extension() {
        let bytes = tiny_test_webp_bytes();

        let decoded = decode_image_bytes("image/webp; charset=binary", &bytes).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
        assert_eq!(decoded.pixels.len(), 1);

        let decoded = decode_image_bytes("webp", &bytes).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);

        let decoded =
            decode_cached_resource_image("https://example.test/photo.webp", None, &bytes).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
    }

    #[test]
    fn converts_cmyk_jpeg_pixels_using_subtractive_model() {
        let pixels = jpeg_pixels_to_grayscale(
            &[
                0, 0, 0, 0, // white
                0, 0, 0, 255, // black
                255, 0, 0, 0, // cyan
                0, 255, 0, 0, // magenta
                0, 0, 255, 0, // yellow
            ],
            JpegPixelFormat::CMYK32,
            5,
        )
        .unwrap();

        assert_eq!(
            pixels,
            vec![
                255,
                0,
                rgb_to_gray(0, 255, 255),
                rgb_to_gray(255, 0, 255),
                rgb_to_gray(255, 255, 0),
            ]
        );
    }

    #[test]
    fn converts_l16_jpeg_pixels_to_eight_bit_shades() {
        let mut bytes = Vec::new();
        for gray in [0u16, 32768, 65535] {
            bytes.extend_from_slice(&gray.to_ne_bytes());
        }

        let pixels = jpeg_pixels_to_grayscale(&bytes, JpegPixelFormat::L16, 3).unwrap();

        assert_eq!(pixels, vec![0, 128, 255]);
    }

    #[test]
    fn decodes_cached_jpeg_resource_by_content_type() {
        let decoded = decode_cached_resource_image(
            "https://example.test/image.bin",
            Some("image/jpeg; charset=binary"),
            &tiny_test_jpeg_bytes(),
        )
        .unwrap();

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.pixels, vec![0, 255, 76, 29]);
    }

    #[test]
    fn decodes_cached_jfif_resource_by_extension() {
        let decoded = decode_cached_resource_image(
            "https://example.test/photo.jfif",
            None,
            &tiny_test_jpeg_bytes(),
        )
        .unwrap();

        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
    }

    #[test]
    fn decodes_jpeg_by_signature_when_type_and_extension_do_not_match() {
        let bytes = tiny_test_jpeg_bytes();
        let decoded = decode_cached_resource_image(
            "https://example.test/image.bin",
            Some("application/octet-stream"),
            &bytes,
        )
        .unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);

        let data_url = test_jpeg_data_url_with_mime_type("application/octet-stream");
        let decoded = decode_image_reference("mem://page", &data_url).unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
    }

    #[test]
    fn decodes_webp_by_signature_when_type_and_extension_do_not_match() {
        let bytes = tiny_test_webp_bytes();
        let decoded = decode_cached_resource_image(
            "https://example.test/image.bin",
            Some("application/octet-stream"),
            &bytes,
        )
        .unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);

        let data_url = test_webp_data_url_with_mime_type("application/octet-stream");
        let decoded = decode_image_reference("mem://page", &data_url).unwrap();
        assert_eq!(decoded.width, 1);
        assert_eq!(decoded.height, 1);
    }

    #[test]
    fn image_real_page_resources_sniffs_png_and_svg_by_signature() {
        let png = test_png_bytes();
        let decoded = decode_cached_resource_image(
            "https://cdn.example.test/image",
            Some("application/octet-stream"),
            &png,
        )
        .unwrap();
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);

        let svg = br#"<?xml version="1.0"?><svg width="3" height="2" xmlns="http://www.w3.org/2000/svg"><rect width="3" height="2" fill="black"/></svg>"#;
        let decoded =
            decode_cached_resource_image("https://cdn.example.test/vector", None, svg).unwrap();
        assert_eq!(decoded.width, 3);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.pixels, vec![0; 6]);
    }

    #[test]
    fn decodes_jpeg_metadata_app_segments_without_changing_pixels() {
        let bytes = tiny_test_jpeg_bytes();
        let baseline = decode_image_bytes("image/jpeg", &bytes).unwrap();
        let mut icc_payload = b"ICC_PROFILE\0\x01\x01".to_vec();
        icc_payload.extend_from_slice(b"deterministic-test-profile");
        let xmp_payload = b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>search-engine</x:xmpmeta>";
        let bytes = jpeg_with_app_segment(&bytes, 0xe2, &icc_payload);
        let bytes = jpeg_with_app_segment(&bytes, 0xe1, xmp_payload);
        let decoded = decode_image_bytes("image/jpeg", &bytes).unwrap();

        assert_eq!(decoded.width, baseline.width);
        assert_eq!(decoded.height, baseline.height);
        assert_eq!(decoded.pixels, baseline.pixels);
        assert_eq!(decoded.pixel_hash(), baseline.pixel_hash());
    }

    #[test]
    fn applies_all_exif_orientation_transforms_to_decoded_pixels() {
        let source = DecodedImage {
            width: 2,
            height: 3,
            pixels: vec![0, 1, 2, 3, 4, 5],
            rgb_pixels: Some(vec![
                0, 10, 20, 1, 11, 21, 2, 12, 22, 3, 13, 23, 4, 14, 24, 5, 15, 25,
            ]),
        };
        let cases = [
            (1, 2, 3, vec![0, 1, 2, 3, 4, 5]),
            (2, 2, 3, vec![1, 0, 3, 2, 5, 4]),
            (3, 2, 3, vec![5, 4, 3, 2, 1, 0]),
            (4, 2, 3, vec![4, 5, 2, 3, 0, 1]),
            (5, 3, 2, vec![0, 2, 4, 1, 3, 5]),
            (6, 3, 2, vec![4, 2, 0, 5, 3, 1]),
            (7, 3, 2, vec![5, 3, 1, 4, 2, 0]),
            (8, 3, 2, vec![1, 3, 5, 0, 2, 4]),
        ];

        for (orientation, width, height, pixels) in cases {
            let mut image = source.clone();
            apply_exif_orientation(&mut image, orientation).unwrap();
            assert_eq!(
                (image.width, image.height, image.pixels.as_slice()),
                (width, height, pixels.as_slice())
            );
            let rgb_pixels = image.rgb_pixels.as_ref().unwrap();
            assert_eq!(rgb_pixels.len(), width * height * 3);
            for (index, gray) in image.pixels.iter().enumerate() {
                assert_eq!(rgb_pixels[index * 3], *gray);
                assert_eq!(rgb_pixels[index * 3 + 1], (*gray).saturating_add(10));
                assert_eq!(rgb_pixels[index * 3 + 2], (*gray).saturating_add(20));
            }
        }
    }

    #[test]
    fn applies_exif_orientation_to_jpeg_decode_output() {
        let bytes = tiny_test_jpeg_bytes();
        let baseline = decode_image_bytes("image/jpeg", &bytes).unwrap();
        let oriented_bytes = jpeg_with_exif_orientation(&bytes, 3);
        let oriented = decode_image_bytes("image/jpeg", &oriented_bytes).unwrap();
        let mut expected = baseline;
        apply_exif_orientation(&mut expected, 3).unwrap();

        assert_eq!(oriented.width, expected.width);
        assert_eq!(oriented.height, expected.height);
        assert_eq!(oriented.pixels, expected.pixels);
    }

    #[test]
    fn applies_big_endian_exif_orientation_to_jpeg_decode_output() {
        let bytes = tiny_test_jpeg_bytes();
        let baseline = decode_image_bytes("image/jpeg", &bytes).unwrap();
        let tiff = big_endian_exif_orientation_tiff(6);
        let oriented_bytes = jpeg_with_exif_tiff(&bytes, &tiff);
        let oriented = decode_image_bytes("image/jpeg", &oriented_bytes).unwrap();
        let mut expected = baseline;
        apply_exif_orientation(&mut expected, 6).unwrap();

        assert_eq!(oriented.width, expected.width);
        assert_eq!(oriented.height, expected.height);
        assert_eq!(oriented.pixels, expected.pixels);
    }

    #[test]
    fn reads_out_of_line_exif_orientation_value() {
        let mut tiff = b"II*\0\x08\0\0\0\x01\0".to_vec();
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&2u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(&8u16.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());

        assert_eq!(exif_orientation_from_tiff(&tiff), Some(8));
    }

    fn test_png_bytes() -> Vec<u8> {
        use std::io::Write as _;

        let filtered_scanlines = [0, 0, 0, 0, 255, 255, 255, 1, 255, 0, 0, 1, 0, 255];
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&filtered_scanlines).unwrap();
        let idat = encoder.finish().unwrap();

        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&2u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        push_test_png_chunk(&mut png, b"IHDR", &ihdr);
        push_test_png_chunk(&mut png, b"IDAT", &idat);
        push_test_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn push_test_png_chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        png.extend_from_slice(&0u32.to_be_bytes());
    }
}

fn picture_source_media_matches(media: Option<&str>, viewport_width_css_px: usize) -> bool {
    media.is_none_or(|media| {
        media
            .split(',')
            .any(|query| media_query_matches_current_screen(query, viewport_width_css_px))
    })
}

fn srcset_target_width_from_sizes(
    sizes: Option<&str>,
    viewport_width_css_px: usize,
) -> Option<usize> {
    sizes
        .and_then(|sizes| parse_sizes_attribute(sizes, viewport_width_css_px))
        .or_else(|| (viewport_width_css_px > 0).then_some(viewport_width_css_px))
}

fn parse_sizes_attribute(sizes: &str, viewport_width_css_px: usize) -> Option<usize> {
    split_css_top_level_commas(sizes)
        .into_iter()
        .find_map(|candidate| {
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return None;
            }
            parse_sizes_candidate(candidate, viewport_width_css_px)
        })
}

fn parse_sizes_candidate(candidate: &str, viewport_width_css_px: usize) -> Option<usize> {
    if let Some(size) = parse_source_size_dimension(candidate, viewport_width_css_px) {
        return Some(size);
    }
    let media_end = matching_closing_paren(candidate, 0)?;
    let media = candidate[..=media_end].trim();
    let size = candidate[media_end + 1..].trim();
    media_query_matches_current_screen(media, viewport_width_css_px)
        .then(|| parse_source_size_dimension(size, viewport_width_css_px))
        .flatten()
}

fn parse_source_size_dimension(value: &str, viewport_width_css_px: usize) -> Option<usize> {
    let pixels = parse_source_size_value(value, viewport_width_css_px)?;
    if !pixels.is_finite() || pixels <= 0.0 {
        return None;
    }
    Some(pixels.ceil() as usize)
}

fn parse_source_size_value(value: &str, viewport_width_css_px: usize) -> Option<f64> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    if value.len() >= 6 && value[..5].eq_ignore_ascii_case("calc(") && value.ends_with(')') {
        let value = &value[5..value.len() - 1];
        return parse_source_size_calc(value, viewport_width_css_px);
    }
    if value.len() >= 5 && value[..4].eq_ignore_ascii_case("min(") && value.ends_with(')') {
        let value = &value[4..value.len() - 1];
        let values = parse_source_size_function_args(value, viewport_width_css_px)?;
        return values.into_iter().reduce(f64::min);
    }
    if value.len() >= 5 && value[..4].eq_ignore_ascii_case("max(") && value.ends_with(')') {
        let value = &value[4..value.len() - 1];
        let values = parse_source_size_function_args(value, viewport_width_css_px)?;
        return values.into_iter().reduce(f64::max);
    }
    if value.len() >= 7 && value[..6].eq_ignore_ascii_case("clamp(") && value.ends_with(')') {
        let value = &value[6..value.len() - 1];
        let values = parse_source_size_function_args(value, viewport_width_css_px)?;
        if values.len() != 3 {
            return None;
        }
        if values[0] > values[2] {
            return None;
        }
        return Some(values[1].clamp(values[0], values[2]));
    }
    if let Some(vw) = strip_ascii_case_suffix(value, "vw") {
        let vw = vw.trim().parse::<f64>().ok()?;
        if !vw.is_finite() || vw <= 0.0 || viewport_width_css_px == 0 {
            return None;
        }
        return Some((viewport_width_css_px as f64) * vw / 100.0);
    }
    let pixels = strip_ascii_case_suffix(value, "px")?
        .trim()
        .parse::<f64>()
        .ok()?;
    pixels.is_finite().then_some(pixels)
}

fn parse_source_size_function_args(value: &str, viewport_width_css_px: usize) -> Option<Vec<f64>> {
    let values = split_css_top_level_commas(value)
        .into_iter()
        .map(|arg| parse_source_size_value(arg, viewport_width_css_px))
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty()).then_some(values)
}

fn parse_source_size_calc(expression: &str, viewport_width_css_px: usize) -> Option<f64> {
    let mut total = 0.0f64;
    let mut sign = 1.0f64;
    let mut term_start = 0usize;
    let bytes = expression.as_bytes();
    let mut index = 0usize;
    while index <= bytes.len() {
        let at_end = index == bytes.len();
        let is_operator = !at_end && matches!(bytes[index], b'+' | b'-');
        if at_end || is_operator {
            let term = expression[term_start..index].trim();
            if !term.is_empty() {
                total += sign * parse_source_size_calc_term(term, viewport_width_css_px)?;
            }
            if at_end {
                break;
            }
            sign = if bytes[index] == b'-' { -1.0 } else { 1.0 };
            term_start = index + 1;
        }
        index += 1;
    }
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    Some(total)
}

fn parse_source_size_calc_term(term: &str, viewport_width_css_px: usize) -> Option<f64> {
    if let Some(vw) = strip_ascii_case_suffix(term, "vw") {
        let vw = vw.trim().parse::<f64>().ok()?;
        if !vw.is_finite() || viewport_width_css_px == 0 {
            return None;
        }
        return Some((viewport_width_css_px as f64) * vw / 100.0);
    }
    let px = strip_ascii_case_suffix(term, "px")?
        .trim()
        .parse::<f64>()
        .ok()?;
    px.is_finite().then_some(px)
}

fn strip_ascii_case_suffix<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let split = value.len().checked_sub(suffix.len())?;
    let tail = value.get(split..)?;
    tail.eq_ignore_ascii_case(suffix)
        .then(|| value.get(..split))
        .flatten()
}

fn split_css_top_level_commas(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, byte) in input.as_bytes().iter().enumerate() {
        match byte {
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

fn matching_closing_paren(input: &str, open_index: usize) -> Option<usize> {
    if input.as_bytes().get(open_index) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    for (index, byte) in input.as_bytes().iter().enumerate().skip(open_index) {
        match byte {
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn media_query_matches_current_screen(query: &str, viewport_width_css_px: usize) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    if let Some(query) = query.strip_prefix("only ") {
        return positive_media_query_matches_current_screen(query.trim(), viewport_width_css_px);
    }
    if let Some(query) = query.strip_prefix("not ") {
        return !positive_media_query_matches_current_screen(query.trim(), viewport_width_css_px);
    }
    positive_media_query_matches_current_screen(&query, viewport_width_css_px)
}

fn positive_media_query_matches_current_screen(query: &str, viewport_width_css_px: usize) -> bool {
    let mut saw_condition = false;
    for (index, part) in media_query_parts(query).into_iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            match part {
                "all" | "screen" => {
                    saw_condition = true;
                    continue;
                }
                "print" | "speech" => return false,
                _ => {}
            }
        }
        let Some(matches) = media_width_feature_matches(part, viewport_width_css_px) else {
            return false;
        };
        if !matches {
            return false;
        }
        saw_condition = true;
    }
    saw_condition
}

fn media_query_parts(query: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let bytes = query.as_bytes();
    let mut index = 0usize;
    while index + 3 <= bytes.len() {
        if &bytes[index..index + 3] == b"and"
            && index > start
            && bytes[index - 1].is_ascii_whitespace()
            && bytes
                .get(index + 3)
                .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            parts.push(query[start..index].trim());
            start = index + 3;
            index = start;
        } else {
            index += 1;
        }
    }
    parts.push(query[start..].trim());
    parts
}

fn media_width_feature_matches(feature: &str, viewport_width_css_px: usize) -> Option<bool> {
    let feature = feature.strip_prefix('(')?.strip_suffix(')')?.trim();
    let (name, value) = feature.split_once(':')?;
    let width = parse_css_pixel_dimension(value)?;
    let viewport_width = viewport_width_css_px as f64;
    match name.trim() {
        "min-width" => Some(viewport_width >= width),
        "max-width" => Some(viewport_width <= width),
        "width" => Some((viewport_width - width).abs() < f64::EPSILON),
        _ => None,
    }
}

fn picture_source_type_supported(element: &ElementData) -> bool {
    element
        .attrs
        .get("type")
        .is_none_or(|source_type| image_mime_type_supported(source_type))
}

pub(super) fn image_mime_type_supported(source_type: &str) -> bool {
    let source_type = source_type
        .split(';')
        .next()
        .unwrap_or(source_type)
        .trim()
        .to_ascii_lowercase();
    if source_type.is_empty() {
        return true;
    }
    matches!(
        source_type.as_str(),
        "image/svg+xml"
            | "image/svg"
            | "image/png"
            | "image/x-png"
            | "image/jpeg"
            | "image/jpg"
            | "image/jpe"
            | "image/pjpeg"
            | "image/x-jpeg"
            | "image/webp"
            | "image/x-webp"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SrcsetCandidate {
    url: String,
    width: Option<usize>,
    density_milli: Option<usize>,
    order: usize,
}

fn choose_srcset_candidate(srcset: &str, desired_width: Option<usize>) -> Option<String> {
    let candidates = parse_srcset_candidates(srcset);
    if candidates.is_empty() {
        return None;
    }
    let candidates = supported_srcset_candidates(&candidates);

    let width_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.width.is_some())
        .collect::<Vec<_>>();
    if !width_candidates.is_empty() {
        let target = desired_width.unwrap_or(usize::MAX);
        let best = width_candidates
            .iter()
            .filter(|candidate| candidate.width.unwrap_or(0) >= target)
            .min_by_key(|candidate| (candidate.width.unwrap_or(usize::MAX), candidate.order))
            .copied()
            .or_else(|| {
                width_candidates
                    .iter()
                    .max_by_key(|candidate| {
                        (candidate.width.unwrap_or(0), usize::MAX - candidate.order)
                    })
                    .copied()
            })?;
        return Some(best.url.clone());
    }

    let density_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate| candidate.density_milli.is_some())
        .collect::<Vec<_>>();
    if !density_candidates.is_empty() {
        let best = density_candidates
            .iter()
            .filter(|candidate| candidate.density_milli.unwrap_or(0) >= 1_000)
            .min_by_key(|candidate| {
                (
                    candidate.density_milli.unwrap_or(usize::MAX),
                    candidate.order,
                )
            })
            .copied()
            .or_else(|| {
                density_candidates
                    .iter()
                    .max_by_key(|candidate| {
                        (
                            candidate.density_milli.unwrap_or(0),
                            usize::MAX - candidate.order,
                        )
                    })
                    .copied()
            })?;
        return Some(best.url.clone());
    }

    candidates.first().map(|candidate| candidate.url.clone())
}

pub(super) fn selected_srcset_candidate(
    srcset: &str,
    sizes: Option<&str>,
    viewport_width_css_px: usize,
) -> Option<String> {
    choose_srcset_candidate(
        srcset,
        srcset_target_width_from_sizes(sizes, viewport_width_css_px),
    )
}

pub(super) fn selected_supported_srcset_candidate(
    srcset: &str,
    sizes: Option<&str>,
    viewport_width_css_px: usize,
) -> Option<String> {
    let candidate = selected_srcset_candidate(srcset, sizes, viewport_width_css_px)?;
    (!srcset_candidate_clearly_unsupported(&candidate)).then_some(candidate)
}

pub(super) fn supported_srcset_candidate_urls(srcset: &str) -> Vec<String> {
    let candidates = parse_srcset_candidates(srcset);
    supported_srcset_candidates(&candidates)
        .into_iter()
        .map(|candidate| candidate.url.clone())
        .collect()
}

fn supported_srcset_candidates(candidates: &[SrcsetCandidate]) -> Vec<&SrcsetCandidate> {
    let supported = candidates
        .iter()
        .filter(|candidate| !srcset_candidate_clearly_unsupported(&candidate.url))
        .collect::<Vec<_>>();
    if supported.is_empty() {
        candidates.iter().collect()
    } else {
        supported
    }
}

fn srcset_all_candidates_clearly_unsupported(srcset: &str) -> bool {
    let candidates = parse_srcset_candidates(srcset);
    !candidates.is_empty()
        && candidates
            .iter()
            .all(|candidate| srcset_candidate_clearly_unsupported(&candidate.url))
}

fn srcset_candidate_clearly_unsupported(url: &str) -> bool {
    let url = url.trim();
    if let Some(metadata) = url
        .strip_prefix("data:")
        .or_else(|| url.strip_prefix("DATA:"))
        .and_then(|payload| payload.split_once(',').map(|(metadata, _)| metadata))
    {
        let mime = metadata.split(';').next().unwrap_or_default();
        return mime
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
            && !image_mime_type_supported(mime);
    }

    let Some(extension) = image_extension_from_url(url) else {
        return false;
    };
    unsupported_image_extension(&extension)
}

fn parse_srcset_candidates(srcset: &str) -> Vec<SrcsetCandidate> {
    srcset_candidate_strings(srcset)
        .into_iter()
        .enumerate()
        .filter_map(|(order, raw_candidate)| {
            let mut parts = raw_candidate.split_ascii_whitespace();
            let url = parts.next()?.trim();
            if url.is_empty() {
                return None;
            }
            let (width, density_milli) = parse_srcset_candidate_descriptors(parts)?;
            Some(SrcsetCandidate {
                url: url.to_owned(),
                width,
                density_milli,
                order,
            })
        })
        .collect()
}

fn parse_srcset_candidate_descriptors<'a>(
    descriptors: impl Iterator<Item = &'a str>,
) -> Option<(Option<usize>, Option<usize>)> {
    let mut width = None;
    let mut density_milli = None;
    let mut future_compat_h = None;
    let mut has_descriptor = false;
    for descriptor in descriptors {
        has_descriptor = true;
        if let Some(parsed_width) = parse_srcset_width_descriptor(descriptor) {
            if width.is_some() || density_milli.is_some() {
                return None;
            }
            width = Some(parsed_width);
        } else if let Some(parsed_density) = parse_srcset_density_descriptor(descriptor) {
            if width.is_some() || density_milli.is_some() || future_compat_h.is_some() {
                return None;
            }
            density_milli = Some(parsed_density);
        } else if let Some(parsed_height) = parse_srcset_height_descriptor(descriptor) {
            if density_milli.is_some() || future_compat_h.is_some() {
                return None;
            }
            future_compat_h = Some(parsed_height);
        } else {
            return None;
        }
    }
    if !has_descriptor {
        density_milli = Some(1_000);
    }
    if future_compat_h.is_some() && width.is_none() {
        return None;
    }
    Some((width, density_milli))
}

pub(super) fn srcset_candidate_urls(srcset: &str) -> Vec<String> {
    parse_srcset_candidates(srcset)
        .into_iter()
        .map(|candidate| candidate.url)
        .collect()
}

fn srcset_candidate_strings(srcset: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut start = 0usize;
    while start < srcset.len() {
        let candidate_start = skip_ascii_whitespace(srcset, start);
        if candidate_start >= srcset.len() {
            break;
        }
        let is_data_url = srcset[candidate_start..]
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"));
        let mut skipped_data_metadata_comma = !is_data_url;
        let mut end = srcset.len();
        for (offset, byte) in srcset.as_bytes()[candidate_start..].iter().enumerate() {
            if *byte != b',' {
                continue;
            }
            if !skipped_data_metadata_comma {
                skipped_data_metadata_comma = true;
                continue;
            }
            end = candidate_start + offset;
            break;
        }
        candidates.push(&srcset[start..end]);
        if end == srcset.len() {
            break;
        }
        start = end + 1;
    }
    candidates
}

fn skip_ascii_whitespace(input: &str, mut index: usize) -> usize {
    while input
        .as_bytes()
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    index
}

fn parse_srcset_width_descriptor(descriptor: &str) -> Option<usize> {
    let width = descriptor.strip_suffix('w')?.parse::<usize>().ok()?;
    (width > 0).then_some(width)
}

fn parse_srcset_height_descriptor(descriptor: &str) -> Option<usize> {
    let height = descriptor.strip_suffix('h')?.parse::<usize>().ok()?;
    (height > 0).then_some(height)
}

fn parse_srcset_density_descriptor(descriptor: &str) -> Option<usize> {
    let density = descriptor.strip_suffix('x')?.parse::<f64>().ok()?;
    if !density.is_finite() || density <= 0.0 {
        return None;
    }
    Some((density * 1_000.0).round() as usize)
}

fn parse_css_pixel_dimension(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let pixels = if let Some(px) = strip_ascii_case_suffix(value, "px") {
        px.trim().parse::<f64>().ok()?
    } else {
        let zero = value.parse::<f64>().ok()?;
        if zero != 0.0 {
            return None;
        }
        zero
    };
    (pixels.is_finite() && pixels >= 0.0).then_some(pixels)
}
