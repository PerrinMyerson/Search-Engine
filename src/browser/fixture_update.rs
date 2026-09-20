//! Golden-expectation refresh for `esoteric-browser verify --update`.
//!
//! Re-renders every fixture in a manifest and rewrites the expectation fields
//! that are already present (`expected_text`, `expected_display_list`,
//! `expected_raster_hash`, ...) with the engine's current output. Keys the
//! manifest does not declare are never added, so a fixture that only pins a
//! raster hash keeps pinning only a raster hash. The manifest is written back
//! in the repository's hand-formatted style: one fixture field per line, with
//! display-list, hit-test, and layer entries compact on a single line each.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{
    BrowserFixture, BrowserFixtureManifest, BrowserRasterOptions, BrowserRgbaRaster,
    browser_fixture_raster_options, hit_test_render, layer_tree_render, raster_report,
    rasterize_render, render_browser_fixture,
};

#[derive(Debug, Clone, Serialize)]
pub struct BrowserFixtureUpdateReport {
    pub manifest: String,
    pub fixture_count: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub updated_names: Vec<String>,
    pub written: bool,
}

/// Rewrite `manifest_path` so every declared expectation matches the current
/// engine output. Returns which fixtures changed; nothing is written when the
/// manifest already matches.
pub fn update_browser_fixture_expectations(
    manifest_path: &Path,
) -> Result<BrowserFixtureUpdateReport> {
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("read browser fixture manifest {}", manifest_path.display()))?;
    let typed: BrowserFixtureManifest = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let mut raw: Value = serde_json::from_str(&manifest_text)
        .with_context(|| format!("parse browser fixture manifest {}", manifest_path.display()))?;
    let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));

    let raw_fixtures = raw
        .get_mut("fixtures")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow!("manifest has no `fixtures` array"))?;
    if raw_fixtures.len() != typed.fixtures.len() {
        return Err(anyhow!("manifest fixture count mismatch while updating"));
    }

    let mut updated_names = Vec::new();
    for (raw_fixture, fixture) in raw_fixtures.iter_mut().zip(&typed.fixtures) {
        let path = if fixture.path.is_absolute() {
            fixture.path.clone()
        } else {
            base_dir.join(&fixture.path)
        };
        let name = fixture
            .name
            .clone()
            .unwrap_or_else(|| fixture.path.display().to_string());
        let object = raw_fixture
            .as_object_mut()
            .ok_or_else(|| anyhow!("fixture {name} is not a JSON object"))?;
        let before = object.clone();
        refresh_fixture_expectations(&name, &path, fixture, object)?;
        if *object != before {
            updated_names.push(name);
        }
    }

    let fixture_count = typed.fixtures.len();
    let updated = updated_names.len();
    let formatted = format_fixture_manifest_like(&raw, &manifest_text)?;
    let written = formatted != manifest_text;
    if written {
        fs::write(manifest_path, formatted).with_context(|| {
            format!("write browser fixture manifest {}", manifest_path.display())
        })?;
    }
    Ok(BrowserFixtureUpdateReport {
        manifest: manifest_path.display().to_string(),
        fixture_count,
        updated,
        unchanged: fixture_count - updated,
        updated_names,
        written,
    })
}

fn refresh_fixture_expectations(
    name: &str,
    path: &Path,
    fixture: &BrowserFixture,
    object: &mut Map<String, Value>,
) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read browser fixture {name}"))?;
    let render = render_browser_fixture(path, &bytes, fixture)
        .with_context(|| format!("render browser fixture {name}"))?;

    if object.contains_key("expected_title") {
        object.insert("expected_title".into(), Value::String(render.title.clone()));
    }
    if object.contains_key("expected_text") {
        object.insert("expected_text".into(), Value::String(render.text.clone()));
    }
    if object.contains_key("expected_display_list") {
        object.insert(
            "expected_display_list".into(),
            serde_json::to_value(&render.display_list)?,
        );
    }
    if let Some(Value::Array(hit_tests)) = object.get_mut("expected_hit_tests") {
        for hit_test in hit_tests.iter_mut() {
            let Some(entry) = hit_test.as_object_mut() else {
                continue;
            };
            let x = entry.get("x").and_then(Value::as_u64).unwrap_or(0) as usize;
            let y = entry.get("y").and_then(Value::as_u64).unwrap_or(0) as usize;
            let actual = hit_test_render(&render, x, y).hit;
            let refreshed = match (actual, entry.get("expected")) {
                (Some(actual), Some(Value::Object(expected))) => {
                    // Keep the subset of fields this expectation pins, but
                    // take their values from the current hit.
                    let actual_value = serde_json::to_value(&actual)?;
                    let mut next = Map::new();
                    for key in expected.keys() {
                        if let Some(value) = actual_value.get(key) {
                            next.insert(key.clone(), value.clone());
                        }
                    }
                    Value::Object(next)
                }
                (Some(actual), _) => serde_json::json!({ "kind": actual.kind }),
                (None, _) => Value::Null,
            };
            entry.insert("expected".into(), refreshed);
        }
    }
    if object.contains_key("expected_layers") {
        object.insert(
            "expected_layers".into(),
            serde_json::to_value(&layer_tree_render(&render).layers)?,
        );
    }

    let needs_raster = [
        "expected_raster_hash",
        "expected_screenshot_hash",
        "expected_visible_command_count",
        "expected_culled_command_count",
    ]
    .iter()
    .any(|key| object.contains_key(*key));
    if needs_raster {
        let raster_options =
            browser_fixture_raster_options(fixture, BrowserRasterOptions::default());
        let raster = rasterize_render(&render, raster_options)?;
        let report = raster_report(&render, &raster, raster_options);
        if object.contains_key("expected_raster_hash") {
            object.insert(
                "expected_raster_hash".into(),
                Value::String(raster.pixel_hash()),
            );
        }
        if object.contains_key("expected_screenshot_hash") {
            let screenshot = BrowserRgbaRaster::from_grayscale(&raster);
            object.insert(
                "expected_screenshot_hash".into(),
                Value::String(screenshot.pixel_hash()),
            );
        }
        if object.contains_key("expected_visible_command_count") {
            object.insert(
                "expected_visible_command_count".into(),
                Value::from(report.visible_command_count),
            );
        }
        if object.contains_key("expected_culled_command_count") {
            object.insert(
                "expected_culled_command_count".into(),
                Value::from(report.culled_command_count),
            );
        }
    }
    Ok(())
}

/// Serialize `manifest` in whichever style `original` was written in: the
/// compact-entry layout of `bench/browser-fixtures` (see
/// [`format_fixture_manifest`]) or plain pretty-printed JSON with the
/// original's indent width (the compat corpus manifest uses one space).
pub fn format_fixture_manifest_like(manifest: &Value, original: &str) -> Result<String> {
    let uses_compact_entries = original
        .lines()
        .any(|line| line.starts_with("        {\"") || line.starts_with("        [\""));
    if uses_compact_entries {
        return Ok(format_fixture_manifest(manifest));
    }
    let indent = original
        .lines()
        .nth(1)
        .map(|line| line.len() - line.trim_start().len())
        .filter(|width| *width > 0)
        .unwrap_or(2);
    let indent = " ".repeat(indent);
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    manifest.serialize(&mut serializer)?;
    let mut text = String::from_utf8(out)?;
    text.push('\n');
    Ok(text)
}

/// Serialize a fixture manifest in the checked-in layout: the `fixtures`
/// array pretty-printed one field per line, and any array inside a fixture
/// (display list, hit tests, layers) emitted as compact one-line entries.
pub fn format_fixture_manifest(manifest: &Value) -> String {
    let mut out = String::from("{\n");
    let Some(root) = manifest.as_object() else {
        return compact(manifest) + "\n";
    };
    let root_len = root.len();
    for (index, (key, value)) in root.iter().enumerate() {
        out.push_str("  ");
        out.push_str(&compact(&Value::String(key.clone())));
        out.push_str(": ");
        match (key.as_str(), value) {
            ("fixtures", Value::Array(fixtures)) => write_fixture_array(&mut out, fixtures),
            _ => out.push_str(&compact(value)),
        }
        if index + 1 < root_len {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn write_fixture_array(out: &mut String, fixtures: &[Value]) {
    if fixtures.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (index, fixture) in fixtures.iter().enumerate() {
        out.push_str("    ");
        match fixture.as_object() {
            Some(object) => write_fixture_object(out, object),
            None => out.push_str(&compact(fixture)),
        }
        if index + 1 < fixtures.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]");
}

fn write_fixture_object(out: &mut String, object: &Map<String, Value>) {
    out.push_str("{\n");
    let len = object.len();
    for (index, (key, value)) in object.iter().enumerate() {
        out.push_str("      ");
        out.push_str(&compact(&Value::String(key.clone())));
        out.push_str(": ");
        match value {
            Value::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (item_index, item) in items.iter().enumerate() {
                    out.push_str("        ");
                    out.push_str(&compact(item));
                    if item_index + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str("      ]");
            }
            _ => out.push_str(&compact(value)),
        }
        if index + 1 < len {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("    }");
}

/// Single-line JSON with `": "` and `", "` separators, matching the style the
/// manifests are hand-written in.
fn compact(value: &Value) -> String {
    let mut out = String::new();
    write_compact(&mut out, value);
    out
}

fn write_compact(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => {
            out.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_owned()))
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_compact(out, item);
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            for (index, (key, item)) in object.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_compact(out, &Value::String(key.clone()));
                out.push_str(": ");
                write_compact(out, item);
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_fixture_manifest, format_fixture_manifest_like};
    use serde_json::json;

    #[test]
    fn formats_manifest_in_checked_in_style() {
        let manifest = json!({
            "fixtures": [
                {
                    "name": "one",
                    "width": 40,
                    "expected_display_list": [
                        {"command": "text", "x": 0, "y": 0, "text": "Hi"}
                    ],
                    "expected_hit_tests": [
                        {"x": 1, "y": 0, "expected": null}
                    ],
                    "expected_layers": [
                        {"id": 0, "parent": null, "command_indices": [0, 2]}
                    ],
                    "expected_raster_hash": "abc"
                }
            ]
        });
        let expected = "{\n  \"fixtures\": [\n    {\n      \"name\": \"one\",\n      \"width\": 40,\n      \"expected_display_list\": [\n        {\"command\": \"text\", \"x\": 0, \"y\": 0, \"text\": \"Hi\"}\n      ],\n      \"expected_hit_tests\": [\n        {\"x\": 1, \"y\": 0, \"expected\": null}\n      ],\n      \"expected_layers\": [\n        {\"id\": 0, \"parent\": null, \"command_indices\": [0, 2]}\n      ],\n      \"expected_raster_hash\": \"abc\"\n    }\n  ]\n}\n";
        assert_eq!(format_fixture_manifest(&manifest), expected);
    }

    #[test]
    fn formatting_checked_in_manifests_is_a_fixed_point() {
        for path in [
            "bench/browser-fixtures/manifest.json",
            "bench/document-pages/manifest.json",
            "bench/wpt-subsets/manifest.json",
        ] {
            let text = std::fs::read_to_string(path).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(
                format_fixture_manifest_like(&value, &text).unwrap(),
                text,
                "{path}"
            );
        }
    }
}
