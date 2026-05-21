//! Parsing for `[[doc-id]]` wikilinks in markdown bodies. Frontmatter `links:`
//! arrays are merged with the regex-discovered links by the caller; this
//! module only handles the body-side syntax.

use regex::Regex;
use std::collections::BTreeSet;
use std::sync::OnceLock;

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\[\[([a-z0-9][a-z0-9\-]{0,63})\]\]").expect("wikilink regex")
    })
}

/// Extract all `[[id]]` wikilinks from a markdown body, deduplicated and sorted.
/// The regex enforces the same slug grammar used for doc IDs:
/// `[a-z0-9][a-z0-9-]{0,63}`.
pub fn extract_wikilinks(body: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for cap in wikilink_re().captures_iter(body) {
        if let Some(m) = cap.get(1) {
            set.insert(m.as_str().to_string());
        }
    }
    set.into_iter().collect()
}

/// Merge frontmatter-declared links with body-discovered wikilinks. Both lists
/// are normalised (lowercased, slug-validated) and deduplicated.
pub fn merge_links(frontmatter: &[String], body: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for s in frontmatter {
        let lower = s.to_lowercase();
        if is_valid_slug(&lower) {
            set.insert(lower);
        }
    }
    for s in extract_wikilinks(body) {
        set.insert(s);
    }
    set.into_iter().collect()
}

/// Validate that a string is a usable doc slug.
pub fn is_valid_slug(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let bytes = s.as_bytes();
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_wikilinks() {
        let body = "See [[user-profile]] and also [[project-nebula]].";
        let links = extract_wikilinks(body);
        assert_eq!(links, vec!["project-nebula", "user-profile"]);
    }

    #[test]
    fn deduplicates_repeated_links() {
        let body = "[[a]] and [[a]] and [[a]]";
        assert_eq!(extract_wikilinks(body), vec!["a"]);
    }

    #[test]
    fn rejects_invalid_link_syntax() {
        let body = "[[Bad Name]] [[ok-1]] [[UPPER]] [[-leading-dash]] [[trailing-]]";
        let links = extract_wikilinks(body);
        assert!(links.contains(&"ok-1".to_string()));
        assert!(links.contains(&"trailing-".to_string()));
        assert!(!links.iter().any(|s| s == "Bad Name"));
        assert!(!links.iter().any(|s| s == "UPPER"));
        assert!(!links.iter().any(|s| s == "-leading-dash"));
    }

    #[test]
    fn merges_frontmatter_and_body() {
        let fm = vec!["frontmatter-only".into(), "shared".into()];
        let body = "see [[body-only]] and [[shared]]";
        let merged = merge_links(&fm, body);
        assert_eq!(merged, vec!["body-only", "frontmatter-only", "shared"]);
    }

    #[test]
    fn slug_validator_matches_regex() {
        assert!(is_valid_slug("a"));
        assert!(is_valid_slug("user-profile"));
        assert!(is_valid_slug("3rd-party"));
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("Bad"));
        assert!(!is_valid_slug("-leading"));
        assert!(!is_valid_slug("has spaces"));
        assert!(!is_valid_slug(&"a".repeat(65)));
    }
}
