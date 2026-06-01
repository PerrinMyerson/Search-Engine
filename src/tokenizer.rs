pub const MAX_TERM_BYTES: usize = 48;

pub fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for_each_term(query, |term, _| {
        if !terms.iter().any(|seen| seen == term) {
            terms.push(term.to_owned());
        }
    });
    terms
}

pub fn for_each_term(mut text: &str, mut f: impl FnMut(&str, u32)) {
    let mut base = 0usize;

    while !text.is_empty() {
        let bytes = text.as_bytes();
        let mut start = 0usize;
        while start < bytes.len() && !is_term_byte(bytes[start]) {
            start += 1;
        }

        if start == bytes.len() {
            return;
        }

        let mut end = start;
        while end < bytes.len() && is_term_byte(bytes[end]) {
            end += 1;
        }

        if end - start <= MAX_TERM_BYTES {
            let raw = &text[start..end];
            let mut scratch = String::with_capacity(raw.len());
            for byte in raw.bytes() {
                scratch.push((byte as char).to_ascii_lowercase());
            }
            f(&scratch, (base + start) as u32);
        }

        base += end;
        text = &text[end..];
    }
}

#[inline]
fn is_term_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_ascii_terms() {
        let mut terms = Vec::new();
        for_each_term("Fast, FAST search-42!", |term, pos| {
            terms.push((term.to_owned(), pos));
        });
        assert_eq!(
            terms,
            vec![
                ("fast".to_owned(), 0),
                ("fast".to_owned(), 6),
                ("search".to_owned(), 11),
                ("42".to_owned(), 18),
            ]
        );
    }
}
