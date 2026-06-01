use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};

use super::BrowserRaster;

#[derive(Debug)]
pub(super) struct RasterDiff {
    width: usize,
    height: usize,
    pub(super) diff_pixels: usize,
    pub(super) diff_ratio: f64,
    pixels: Vec<u8>,
}

pub(super) fn compare_raster_with_pgm(
    actual: &BrowserRaster,
    baseline_path: &Path,
) -> Result<RasterDiff> {
    let baseline = decode_pgm(&fs::read(baseline_path)?)?;
    ensure!(
        baseline.width == actual.width && baseline.height == actual.height,
        "visual baseline dimensions differ: expected {}x{}, got {}x{}",
        baseline.width,
        baseline.height,
        actual.width,
        actual.height
    );
    ensure!(
        baseline.pixels.len() == actual.pixels.len(),
        "visual baseline pixel buffer length differs: expected {}, got {}",
        baseline.pixels.len(),
        actual.pixels.len()
    );

    let mut diff_pixels = 0usize;
    let mut pixels = Vec::with_capacity(actual.pixels.len());
    for (&expected, &actual_pixel) in baseline.pixels.iter().zip(actual.pixels.iter()) {
        if expected == actual_pixel {
            pixels.push(255);
        } else {
            diff_pixels += 1;
            pixels.push(0);
        }
    }
    let diff_ratio = if actual.pixels.is_empty() {
        0.0
    } else {
        diff_pixels as f64 / actual.pixels.len() as f64
    };

    Ok(RasterDiff {
        width: actual.width,
        height: actual.height,
        diff_pixels,
        diff_ratio,
        pixels,
    })
}

pub(super) fn diff_within_threshold(
    diff: &RasterDiff,
    max_diff_pixels: Option<usize>,
    max_diff_ratio: Option<f64>,
) -> bool {
    let max_pixels = max_diff_pixels.unwrap_or(0);
    let max_ratio = max_diff_ratio.unwrap_or(0.0);
    diff.diff_pixels <= max_pixels && diff.diff_ratio <= max_ratio
}

pub(super) fn encode_diff_pgm(diff: &RasterDiff) -> Vec<u8> {
    let mut encoded = format!("P5\n{} {}\n255\n", diff.width, diff.height).into_bytes();
    encoded.extend_from_slice(&diff.pixels);
    encoded
}

fn decode_pgm(bytes: &[u8]) -> Result<BrowserRaster> {
    let mut cursor = 0usize;
    let magic = next_pgm_token(bytes, &mut cursor).context("read PGM magic")?;
    ensure!(
        magic == "P5",
        "only binary PGM P5 visual baselines are supported"
    );
    let width = next_pgm_token(bytes, &mut cursor)
        .context("read PGM width")?
        .parse::<usize>()
        .context("parse PGM width")?;
    let height = next_pgm_token(bytes, &mut cursor)
        .context("read PGM height")?
        .parse::<usize>()
        .context("parse PGM height")?;
    let max_value = next_pgm_token(bytes, &mut cursor)
        .context("read PGM max value")?
        .parse::<usize>()
        .context("parse PGM max value")?;
    ensure!(max_value == 255, "only 8-bit PGM baselines are supported");

    if bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }

    let pixel_count = width
        .checked_mul(height)
        .context("PGM dimensions overflow")?;
    let end = cursor
        .checked_add(pixel_count)
        .context("PGM pixel range overflow")?;
    ensure!(
        bytes.len() >= end,
        "PGM pixel data is truncated: expected {} bytes, found {}",
        pixel_count,
        bytes.len().saturating_sub(cursor)
    );

    Ok(BrowserRaster {
        width,
        height,
        background: 255,
        foreground: 0,
        pixels: bytes[cursor..end].to_vec(),
    })
}

fn next_pgm_token(bytes: &[u8], cursor: &mut usize) -> Option<String> {
    loop {
        while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
        if bytes.get(*cursor) != Some(&b'#') {
            break;
        }
        while bytes.get(*cursor).is_some_and(|byte| *byte != b'\n') {
            *cursor += 1;
        }
    }
    let start = *cursor;
    while bytes
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        *cursor += 1;
    }
    (start != *cursor).then(|| String::from_utf8_lossy(&bytes[start..*cursor]).into_owned())
}
