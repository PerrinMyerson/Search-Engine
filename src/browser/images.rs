use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
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

#[derive(Debug, Clone)]
pub(super) struct DecodedImage {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixels: Vec<u8>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodedImageInfo {
    pub(super) url: String,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixel_hash: String,
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
        "svg" | "image/svg+xml" => decode_simple_svg(bytes),
        "png" | "image/png" => decode_simple_png(bytes),
        "jpg" | "jpeg" | "jpe" | "jfif" | "pjpeg" | "pjp" | "image/jpeg" | "image/jpg"
        | "image/jpe" | "image/pjpeg" | "image/x-jpeg" => decode_jpeg(bytes),
        _ => None,
    }
}

fn decode_sniffed_image_bytes(bytes: &[u8]) -> Option<DecodedImage> {
    is_jpeg_bytes(bytes).then(|| decode_jpeg(bytes)).flatten()
}

fn is_jpeg_bytes(bytes: &[u8]) -> bool {
    matches!(bytes, [0xff, 0xd8, 0xff, ..])
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
        if let Some(tag) = parse_tag(raw_tag)
            && tag.kind == TagKind::Opening
        {
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
                }
                "rect" => {
                    rects.push(attrs);
                }
                _ => {}
            }
        }
        cursor = tag_end + 1;
    }

    let width = width?.clamp(1, MAX_DECODED_IMAGE_SIDE);
    let height = height?.clamp(1, MAX_DECODED_IMAGE_SIDE);
    let mut pixels = vec![255u8; width.checked_mul(height)?];
    for rect in rects {
        let Some(fill) = rect
            .get("fill")
            .and_then(|value| parse_css_color_shade(value))
        else {
            continue;
        };
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
        fill_decoded_rect(&mut pixels, width, x, y, rect_width, rect_height, fill);
    }

    Some(DecodedImage {
        width,
        height,
        pixels,
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

    let mut pixels = Vec::with_capacity(width.checked_mul(height)?);
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut offset = 0usize;

    for _ in 0..height {
        let filter = raw[offset];
        offset += 1;
        current.copy_from_slice(&raw[offset..offset + row_bytes]);
        offset += row_bytes;
        reconstruct_png_scanline(filter, channels, &previous, &mut current)?;
        push_png_grayscale_pixels(&current, color_type, &mut pixels);
        previous.copy_from_slice(&current);
    }

    Some(DecodedImage {
        width,
        height,
        pixels,
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

fn push_png_grayscale_pixels(row: &[u8], color_type: u8, pixels: &mut Vec<u8>) {
    match color_type {
        0 => pixels.extend_from_slice(row),
        2 => {
            for rgb in row.chunks_exact(3) {
                pixels.push(rgb_to_gray(rgb[0], rgb[1], rgb[2]));
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
}

fn decode_jpeg(bytes: &[u8]) -> Option<DecodedImage> {
    let mut decoder = JpegDecoder::new(bytes);
    decoder.set_max_decoding_buffer_size(MAX_JPEG_DECODED_BYTES);
    let decoded = decoder.decode().ok()?;
    let orientation = decoder
        .exif_data()
        .and_then(exif_orientation_from_tiff)
        .unwrap_or(1);
    let info = decoder.info()?;
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    if width == 0
        || height == 0
        || width > MAX_DECODED_IMAGE_SIDE
        || height > MAX_DECODED_IMAGE_SIDE
    {
        return None;
    }
    let pixel_count = width.checked_mul(height)?;
    let expected_len = pixel_count.checked_mul(info.pixel_format.pixel_bytes())?;
    if decoded.len() != expected_len {
        return None;
    }
    let pixels = jpeg_pixels_to_grayscale(&decoded, info.pixel_format, pixel_count)?;
    let mut image = DecodedImage {
        width,
        height,
        pixels,
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
                let red = multiply_u8(cmyk[0], key);
                let green = multiply_u8(cmyk[1], key);
                let blue = multiply_u8(cmyk[2], key);
                pixels.push(rgb_to_gray(red, green, blue));
            }
        }
        JpegPixelFormat::L16 => return None,
    }
    (pixels.len() == pixel_count).then_some(pixels)
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
    let swaps_axes = matches!(orientation, 5..=8);
    let output_width = if swaps_axes { height } else { width };
    let output_height = if swaps_axes { width } else { height };
    let mut oriented = vec![255u8; pixel_count];
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
        }
    }
    image.width = output_width;
    image.height = output_height;
    image.pixels = oriented;
    Some(())
}

fn rgb_to_gray(red: u8, green: u8, blue: u8) -> u8 {
    (((red as u16 * 77) + (green as u16 * 150) + (blue as u16 * 29) + 128) >> 8) as u8
}

fn multiply_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

fn blend_gray_over_white(gray: u8, alpha: u8) -> u8 {
    (((gray as u16 * alpha as u16) + (255u16 * (255 - alpha as u16)) + 127) / 255) as u8
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

fn fill_decoded_rect(
    pixels: &mut [u8],
    image_width: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    value: u8,
) {
    for row in y..y.saturating_add(height) {
        for column in x..x.saturating_add(width) {
            let Some(pixel) =
                pixels.get_mut(row.saturating_mul(image_width).saturating_add(column))
            else {
                continue;
            };
            *pixel = value;
        }
    }
}

pub(super) fn image_render_source(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
) -> Option<String> {
    let desired_width = element
        .attrs
        .get("width")
        .and_then(|value| parse_css_pixel_dimension(value));
    let selected_source = picture_source_srcset(dom, node_id)
        .and_then(|srcset| choose_srcset_candidate(srcset, desired_width))
        .or_else(|| {
            element
                .srcset
                .as_deref()
                .and_then(|srcset| choose_srcset_candidate(srcset, desired_width))
        })
        .or_else(|| element.src.clone());
    if selected_source
        .as_deref()
        .is_none_or(is_lazy_svg_placeholder_src)
        && let Some(lazy_source) = lazy_image_render_source(dom, node_id, element, desired_width)
    {
        return Some(lazy_source);
    }
    selected_source
}

fn picture_source_srcset(dom: &Dom, img_node_id: usize) -> Option<&str> {
    picture_source_attr(dom, img_node_id, &["srcset"])
}

fn picture_source_lazy_srcset(dom: &Dom, img_node_id: usize) -> Option<&str> {
    picture_source_attr(dom, img_node_id, &["data-srcset", "data-lazy-srcset"])
}

fn picture_source_attr<'a>(
    dom: &'a Dom,
    img_node_id: usize,
    attr_names: &[&str],
) -> Option<&'a str> {
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
            && picture_source_media_matches(element.media.as_deref())
            && let Some(srcset) = first_non_empty_attr(element, attr_names)
        {
            return Some(srcset);
        }
    }
    None
}

fn lazy_image_render_source(
    dom: &Dom,
    node_id: usize,
    element: &ElementData,
    desired_width: Option<usize>,
) -> Option<String> {
    picture_source_lazy_srcset(dom, node_id)
        .and_then(|srcset| choose_srcset_candidate(srcset, desired_width))
        .or_else(|| {
            first_non_empty_attr(element, &["data-srcset", "data-lazy-srcset"])
                .and_then(|srcset| choose_srcset_candidate(srcset, desired_width))
        })
        .or_else(|| {
            first_non_empty_attr(element, &["data-src", "data-lazy-src", "data-original"])
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

fn is_lazy_svg_placeholder_src(src: &str) -> bool {
    src.trim_start()
        .to_ascii_lowercase()
        .starts_with("data:image/svg+xml")
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
fn test_jpeg_data_url_with_mime_type(mime_type: &str) -> String {
    format!("data:{mime_type};base64,{TINY_TEST_JPEG_BASE64}")
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
fn jpeg_with_exif_orientation(bytes: &[u8], orientation: u16) -> Vec<u8> {
    assert!(is_jpeg_bytes(bytes));
    assert!((1..=8).contains(&orientation));

    let mut payload = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
    payload.extend_from_slice(&0x0112u16.to_le_bytes());
    payload.extend_from_slice(&3u16.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&orientation.to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    let segment_len = u16::try_from(payload.len() + 2).unwrap();
    let mut with_exif = Vec::with_capacity(bytes.len() + payload.len() + 4);
    with_exif.extend_from_slice(&bytes[..2]);
    with_exif.extend_from_slice(&[0xff, 0xe1]);
    with_exif.extend_from_slice(&segment_len.to_be_bytes());
    with_exif.extend_from_slice(&payload);
    with_exif.extend_from_slice(&bytes[2..]);
    with_exif
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
    fn applies_all_exif_orientation_transforms_to_decoded_pixels() {
        let source = DecodedImage {
            width: 2,
            height: 3,
            pixels: vec![0, 1, 2, 3, 4, 5],
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
                (image.width, image.height, image.pixels),
                (width, height, pixels)
            );
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
}

fn picture_source_media_matches(media: Option<&str>) -> bool {
    media.is_none_or(|media| media.trim().is_empty() || media.trim() == "all")
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

    let width_candidates = candidates
        .iter()
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

fn parse_srcset_candidates(srcset: &str) -> Vec<SrcsetCandidate> {
    srcset
        .split(',')
        .enumerate()
        .filter_map(|(order, raw_candidate)| {
            let mut parts = raw_candidate.split_ascii_whitespace();
            let url = parts.next()?.trim();
            if url.is_empty() {
                return None;
            }
            let mut width = None;
            let mut density_milli = None;
            for descriptor in parts {
                if let Some(parsed_width) = parse_srcset_width_descriptor(descriptor) {
                    width = Some(parsed_width);
                } else if let Some(parsed_density) = parse_srcset_density_descriptor(descriptor) {
                    density_milli = Some(parsed_density);
                }
            }
            Some(SrcsetCandidate {
                url: url.to_owned(),
                width,
                density_milli,
                order,
            })
        })
        .collect()
}

fn parse_srcset_width_descriptor(descriptor: &str) -> Option<usize> {
    let width = descriptor.strip_suffix('w')?.parse::<usize>().ok()?;
    (width > 0).then_some(width)
}

fn parse_srcset_density_descriptor(descriptor: &str) -> Option<usize> {
    let density = descriptor.strip_suffix('x')?.parse::<f64>().ok()?;
    if !density.is_finite() || density <= 0.0 {
        return None;
    }
    Some((density * 1_000.0).round() as usize)
}

fn parse_css_pixel_dimension(value: &str) -> Option<usize> {
    let trimmed = value.trim().trim_end_matches("px").trim();
    if trimmed.contains('%') || trimmed.is_empty() {
        return None;
    }
    let whole = trimmed.split_once('.').map_or(trimmed, |(whole, _)| whole);
    let pixels = whole.parse::<usize>().ok()?;
    (pixels > 0).then_some(pixels)
}
