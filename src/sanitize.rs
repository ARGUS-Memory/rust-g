use crate::argus_json;
use crate::error::Result;
use std::collections::HashSet;

byond_fn!(fn sanitize_html(text, attribute_whitelist_json, tag_whitelist_json) {
    match seriously_sanitize_html(text, attribute_whitelist_json, tag_whitelist_json) {
        Ok(r) => return Some(r),
        Err(e) => return Some(e.to_string())
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_basic_html() {
        let result = seriously_sanitize_html(
            "<p>Hello <b>world</b></p>",
            "[]",
            "[\"p\",\"b\"]"
        ).unwrap();
        assert!(result.contains("Hello"));
        assert!(result.contains("<b>world</b>"));
    }

    #[test]
    fn test_sanitize_strips_script() {
        let result = seriously_sanitize_html(
            "<p>Safe</p><script>alert('xss')</script>",
            "[]",
            "[\"p\"]"
        ).unwrap();
        assert!(result.contains("Safe"));
        assert!(!result.contains("script"));
        assert!(!result.contains("alert"));
    }

    #[test]
    fn test_sanitize_strips_style() {
        let result = seriously_sanitize_html(
            "<style>body{color:red}</style><p>Text</p>",
            "[]",
            "[\"p\"]"
        ).unwrap();
        assert!(!result.contains("style"));
        assert!(!result.contains("color:red"));
    }

    #[test]
    fn test_sanitize_empty_whitelists() {
        let result = seriously_sanitize_html(
            "<p>Hello <b>world</b></p>",
            "[]",
            "[]"
        ).unwrap();
        // With empty tag whitelist, all tags stripped, text preserved
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
        assert!(!result.contains("<p>"));
        assert!(!result.contains("<b>"));
    }

    #[test]
    fn test_sanitize_attributes() {
        let result = seriously_sanitize_html(
            "<p class=\"test\" id=\"p1\">Hello</p>",
            "[\"class\"]",
            "[\"p\"]"
        ).unwrap();
        assert!(result.contains("class=\"test\""));
        // id not in whitelist, should be stripped
        assert!(!result.contains("id="));
    }

    #[test]
    fn test_sanitize_plain_text() {
        let result = seriously_sanitize_html(
            "Just plain text, no HTML",
            "[]",
            "[]"
        ).unwrap();
        assert_eq!(result, "Just plain text, no HTML");
    }

    #[test]
    fn test_sanitize_invalid_attribute_json() {
        let result = seriously_sanitize_html("test", "not json", "[]");
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_invalid_tag_json() {
        let result = seriously_sanitize_html("test", "[]", "not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_byond_links() {
        let result = seriously_sanitize_html(
            "<a href=\"byond://action\">Click</a>",
            "[\"href\"]",
            "[\"a\"]"
        ).unwrap();
        assert!(result.contains("byond://action"));
    }
}

fn seriously_sanitize_html(
    text: &str,
    attribute_whitelist_json: &str,
    tag_whitelist_json: &str,
) -> Result<String> {
    let attribute_list = argus_json::parse_string_array(attribute_whitelist_json.as_bytes())
        .map_err(|_| crate::error::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid attribute whitelist JSON")))?;
    let tag_list = argus_json::parse_string_array(tag_whitelist_json.as_bytes())
        .map_err(|_| crate::error::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid tag whitelist JSON")))?;

    let attribute_whitelist: HashSet<&str> = attribute_list.iter().map(|s| s.as_str()).collect();
    let tag_whitelist: HashSet<&str> = tag_list.iter().map(|s| s.as_str()).collect();

    let mut prune_url_schemes = ammonia::Builder::default().clone_url_schemes();
    prune_url_schemes.insert("byond");

    let sanitized = ammonia::Builder::empty()
        .clean_content_tags(HashSet::from_iter(["script", "style"])) // Completely forbid script and style attributes.
        .link_rel(Some("noopener")) // https://mathiasbynens.github.io/rel-noopener/
        .url_schemes(prune_url_schemes)
        .generic_attributes(attribute_whitelist)
        .tags(tag_whitelist)
        .clean(text)
        .to_string();

    Ok(sanitized)
}
