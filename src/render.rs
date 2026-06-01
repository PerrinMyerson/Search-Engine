use anyhow::{Result, bail};

use crate::index::SearchIndex;

pub fn render_target(index: &SearchIndex, target: &str) -> Result<String> {
    let doc_id = if let Ok(doc_id) = target.parse::<u32>() {
        doc_id
    } else if let Some(doc_id) = index.doc_id_for_url(target) {
        doc_id
    } else {
        bail!("unknown document id or URL: {target}");
    };

    let Some(text) = index.text(doc_id) else {
        bail!("document text not found: {doc_id}");
    };

    Ok(text.to_owned())
}
