use std::collections::{HashMap, HashSet};

use super::images::{image_width_variant_canonical_url, image_width_variant_width};
use super::resources::BrowserResource;
use super::{DecodedImageEntry, resolve_browser_href};

pub(super) fn decoded_image_cache_keys(source: &str, url: &str) -> Vec<String> {
    let resolved = resolve_browser_href(source, url);
    let mut keys = Vec::new();
    push_unique_string(&mut keys, url.to_owned());
    push_unique_string(&mut keys, resolved.clone());
    if let Some(canonical) = image_width_variant_canonical_url(url) {
        push_unique_string(&mut keys, canonical);
    }
    if let Some(canonical) = image_width_variant_canonical_url(&resolved) {
        push_unique_string(&mut keys, canonical);
    }
    keys
}

pub(super) fn decoded_image_aliases_for_resources(
    source: &str,
    resources: &[BrowserResource],
) -> HashMap<String, Vec<String>> {
    let mut aliases = HashMap::<String, Vec<String>>::new();
    for resource in resources {
        let resolved = resolve_browser_href(source, &resource.resolved);
        let url = resource.url.trim();
        if url.is_empty() {
            continue;
        }
        let mut candidates = vec![url.to_owned(), resolve_browser_href(source, url)];
        candidates.push(resource.resolved.clone());
        let variant_aliases = candidates
            .iter()
            .filter_map(|candidate| image_width_variant_canonical_url(candidate))
            .collect::<Vec<_>>();
        candidates.extend(variant_aliases);
        for candidate in candidates {
            if candidate.is_empty() {
                continue;
            }
            let entry = aliases.entry(resolved.clone()).or_default();
            if !entry.iter().any(|alias| alias == &candidate) {
                entry.push(candidate);
            }
        }
    }
    aliases
}

pub(super) fn add_decoded_image_alias_entries(
    source: &str,
    images: &mut Vec<DecodedImageEntry>,
    aliases: &HashMap<String, Vec<String>>,
) {
    if images.is_empty() || aliases.is_empty() {
        return;
    }

    let mut seen = images
        .iter()
        .map(|image| image.url.clone())
        .collect::<HashSet<_>>();
    let originals = images.clone();
    for image in originals {
        let resolved = resolve_browser_href(source, &image.url);
        let Some(image_aliases) = aliases.get(&image.url).or_else(|| aliases.get(&resolved)) else {
            continue;
        };
        for alias in image_aliases {
            if !seen.insert(alias.clone()) {
                continue;
            }
            let mut aliased = image.clone();
            aliased.url = alias.clone();
            images.push(aliased);
        }
    }
}

pub(super) fn dedupe_first_viewport_image_width_variants(resources: &mut Vec<BrowserResource>) {
    if resources.len() < 2 {
        return;
    }

    let mut entries = Vec::<(String, usize, BrowserResource)>::new();
    for (index, resource) in resources.drain(..).enumerate() {
        let resolved = resolve_browser_href("", &resource.resolved);
        let key = image_width_variant_canonical_url(&resolved).unwrap_or(resolved);
        if let Some((_, _, existing)) = entries
            .iter_mut()
            .find(|(existing_key, _, _)| existing_key == &key)
        {
            let existing_width = image_width_variant_width(&existing.resolved).unwrap_or(0);
            let candidate_width = image_width_variant_width(&resource.resolved).unwrap_or(0);
            if candidate_width > existing_width {
                *existing = resource;
            }
        } else {
            entries.push((key, index, resource));
        }
    }
    entries.sort_by_key(|(_, index, _)| *index);
    resources.extend(entries.into_iter().map(|(_, _, resource)| resource));
}

pub(super) fn image_render_url_matches_resource(
    source: &str,
    render_url: &str,
    resource_url: &str,
    resource_resolved: &str,
    resource_canonical: &str,
) -> bool {
    if render_url == resource_url {
        return true;
    }
    let render_resolved = resolve_browser_href(source, render_url);
    if render_resolved == resource_resolved {
        return true;
    }
    let render_canonical =
        image_width_variant_canonical_url(&render_resolved).unwrap_or(render_resolved);
    render_canonical == resource_canonical
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::super::DecodedImage;
    use super::*;

    #[test]
    fn first_viewport_image_loading_dedupes_cdn_width_variants() {
        let mut resources = vec![
            BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: "https://images.squarespace-cdn.com/content/logo.png?format=300w".to_owned(),
                resolved: "https://images.squarespace-cdn.com/content/logo.png?format=300w"
                    .to_owned(),
                rel: None,
                media: None,
                alt: None,
                type_hint: None,
            },
            BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: "https://images.squarespace-cdn.com/content/logo.png?format=500w".to_owned(),
                resolved: "https://images.squarespace-cdn.com/content/logo.png?format=500w"
                    .to_owned(),
                rel: None,
                media: None,
                alt: None,
                type_hint: None,
            },
            BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: "https://images.squarespace-cdn.com/content/other.png?format=300w".to_owned(),
                resolved: "https://images.squarespace-cdn.com/content/other.png?format=300w"
                    .to_owned(),
                rel: None,
                media: None,
                alt: None,
                type_hint: None,
            },
        ];

        dedupe_first_viewport_image_width_variants(&mut resources);

        assert_eq!(resources.len(), 2);
        assert_eq!(
            resources[0].resolved,
            "https://images.squarespace-cdn.com/content/logo.png?format=500w"
        );
        assert_eq!(
            resources[1].resolved,
            "https://images.squarespace-cdn.com/content/other.png?format=300w"
        );
        assert!(
            decoded_image_cache_keys(
                "https://www.example.test/",
                "https://images.squarespace-cdn.com/content/logo.png?format=500w"
            )
            .iter()
            .any(|key| key == "https://images.squarespace-cdn.com/content/logo.png")
        );
    }

    #[test]
    fn decoded_image_alias_entries_keep_canonical_width_variant_lookup() {
        let source = "https://www.example.test/page";
        let fetched = "https://images.squarespace-cdn.com/content/logo.png?format=500w".to_owned();
        let render = "https://images.squarespace-cdn.com/content/logo.png?format=300w".to_owned();
        let mut images = vec![DecodedImageEntry {
            url: fetched.clone(),
            width: 2,
            height: 1,
            pixel_hash: "pixel-hash".to_owned(),
            image: DecodedImage {
                width: 2,
                height: 1,
                pixels: vec![0, 0, 0, 255, 255, 255, 255, 255],
                rgb_pixels: None,
            },
        }];
        let aliases = decoded_image_aliases_for_resources(
            source,
            &[BrowserResource {
                kind: "image".to_owned(),
                initiator: "img".to_owned(),
                url: fetched.clone(),
                resolved: fetched,
                rel: None,
                media: None,
                alt: Some("Logo".to_owned()),
                type_hint: None,
            }],
        );

        add_decoded_image_alias_entries(source, &mut images, &aliases);

        let render_keys = decoded_image_cache_keys(source, &render);
        assert!(
            images
                .iter()
                .any(|image| render_keys.iter().any(|key| key == &image.url)),
            "render sibling width variant should resolve through a canonical decoded alias"
        );
    }
}
