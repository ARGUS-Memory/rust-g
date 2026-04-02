use crate::argus_json;
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind, StartKind};
use std::{cell::RefCell, collections::hash_map::HashMap};

struct Replacements {
    pub automaton: AhoCorasick,
    pub replacements: Vec<String>,
}

struct AhoCorasickOptions {
    pub anchored: bool,
    pub ascii_case_insensitive: bool,
    pub match_kind: MatchKind,
}

impl AhoCorasickOptions {
    fn from_json(src: &str) -> std::result::Result<Self, ()> {
        let val = argus_json::parse_value(src.as_bytes())?;
        let anchored = val
            .get("anchored")
            .and_then(|v| v.as_i64())
            .map(|n| n != 0)
            .unwrap_or(false);
        let ascii_case_insensitive = val
            .get("ascii_case_insensitive")
            .and_then(|v| v.as_i64())
            .map(|n| n != 0)
            .unwrap_or(false);
        let match_kind = match val
            .get("match_kind")
            .and_then(|v| v.as_str())
        {
            Some("LeftmostFirst") => MatchKind::LeftmostFirst,
            Some("LeftmostLongest") => MatchKind::LeftmostLongest,
            _ => MatchKind::Standard,
        };
        Ok(Self {
            anchored,
            ascii_case_insensitive,
            match_kind,
        })
    }

    fn auto_configure_and_build(&self, patterns: &[String]) -> Option<AhoCorasick> {
        AhoCorasickBuilder::new()
            .start_kind(if self.anchored {
                StartKind::Anchored
            } else {
                StartKind::Unanchored
            })
            .ascii_case_insensitive(self.ascii_case_insensitive)
            .match_kind(self.match_kind)
            .build(patterns)
            .ok()
            .or_else(|| AhoCorasickBuilder::new().build(patterns).ok())
    }
}

thread_local! {
    static CREPLACE_MAP: RefCell<HashMap<String, Replacements>> = RefCell::new(HashMap::new());
}

byond_fn!(fn acreplace_remove(key) {
    CREPLACE_MAP.with(|cell| cell.borrow_mut().remove(key));
    Some("")
});

byond_fn!(fn acreplace_clear() {
    CREPLACE_MAP.with(|cell| cell.borrow_mut().clear());
    Some("")
});

byond_fn!(fn setup_acreplace(key, patterns_json, replacements_json) {
    let patterns: Vec<String> = argus_json::parse_string_array(patterns_json.as_bytes()).ok()?;
    let replacements: Vec<String> = argus_json::parse_string_array(replacements_json.as_bytes()).ok()?;
    let ac = AhoCorasickBuilder::new().build(patterns).ok()?;
    CREPLACE_MAP.with(|cell| {
        let mut map = cell.borrow_mut();
        map.insert(key.to_owned(), Replacements { automaton: ac, replacements });
    });
    Some("")
});

byond_fn!(fn setup_acreplace_with_options(key, options_json, patterns_json, replacements_json) {
    let options = AhoCorasickOptions::from_json(options_json).ok()?;
    let patterns: Vec<String> = argus_json::parse_string_array(patterns_json.as_bytes()).ok()?;
    let replacements: Vec<String> = argus_json::parse_string_array(replacements_json.as_bytes()).ok()?;
    let ac = options.auto_configure_and_build(&patterns)?;
    CREPLACE_MAP.with(|cell| {
        let mut map = cell.borrow_mut();
        map.insert(key.to_owned(), Replacements { automaton: ac, replacements });
    });
    Some("")
});

byond_fn!(fn acreplace(key, text) {
    CREPLACE_MAP.with(|cell| -> Option<String> {
        let map = cell.borrow_mut();
        let replacements = map.get(key)?;
        Some(replacements.automaton.replace_all(text, &replacements.replacements))
    })
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_options_from_json_defaults() {
        let opts = AhoCorasickOptions::from_json("{}").unwrap();
        assert!(!opts.anchored);
        assert!(!opts.ascii_case_insensitive);
        assert!(matches!(opts.match_kind, MatchKind::Standard));
    }

    #[test]
    fn test_options_from_json_byond_bools_true() {
        // BYOND uses 1 for true
        let opts = AhoCorasickOptions::from_json("{\"anchored\":1,\"ascii_case_insensitive\":1}").unwrap();
        assert!(opts.anchored);
        assert!(opts.ascii_case_insensitive);
    }

    #[test]
    fn test_options_from_json_byond_bools_false() {
        // BYOND uses 0 for false
        let opts = AhoCorasickOptions::from_json("{\"anchored\":0,\"ascii_case_insensitive\":0}").unwrap();
        assert!(!opts.anchored);
        assert!(!opts.ascii_case_insensitive);
    }

    #[test]
    fn test_options_from_json_match_kinds() {
        let opts = AhoCorasickOptions::from_json("{\"match_kind\":\"LeftmostFirst\"}").unwrap();
        assert!(matches!(opts.match_kind, MatchKind::LeftmostFirst));

        let opts = AhoCorasickOptions::from_json("{\"match_kind\":\"LeftmostLongest\"}").unwrap();
        assert!(matches!(opts.match_kind, MatchKind::LeftmostLongest));

        let opts = AhoCorasickOptions::from_json("{\"match_kind\":\"unknown\"}").unwrap();
        assert!(matches!(opts.match_kind, MatchKind::Standard));
    }

    #[test]
    fn test_options_from_json_invalid() {
        assert!(AhoCorasickOptions::from_json("not json").is_err());
    }

    #[test]
    fn test_options_from_json_missing_fields() {
        // All fields optional, should use defaults
        let opts = AhoCorasickOptions::from_json("{\"extra_field\":42}").unwrap();
        assert!(!opts.anchored);
        assert!(!opts.ascii_case_insensitive);
    }

    #[test]
    fn test_auto_configure_and_build() {
        let opts = AhoCorasickOptions {
            anchored: false,
            ascii_case_insensitive: false,
            match_kind: MatchKind::Standard,
        };
        let patterns = vec!["hello".to_owned(), "world".to_owned()];
        let ac = opts.auto_configure_and_build(&patterns).unwrap();
        // Verify it actually works
        let result = ac.replace_all("hello world", &["HI", "EARTH"]);
        assert_eq!(result, "HI EARTH");
    }

    #[test]
    fn test_auto_configure_case_insensitive() {
        let opts = AhoCorasickOptions {
            anchored: false,
            ascii_case_insensitive: true,
            match_kind: MatchKind::LeftmostFirst,
        };
        let patterns = vec!["hello".to_owned()];
        let ac = opts.auto_configure_and_build(&patterns).unwrap();
        let result = ac.replace_all("HELLO Hello hElLo", &["hi"]);
        assert_eq!(result, "hi hi hi");
    }

    #[test]
    fn test_options_nonzero_is_true() {
        // Any non-zero value should be treated as true
        let opts = AhoCorasickOptions::from_json("{\"anchored\":5}").unwrap();
        assert!(opts.anchored);

        let opts = AhoCorasickOptions::from_json("{\"anchored\":-1}").unwrap();
        assert!(opts.anchored);
    }
}

byond_fn!(fn acreplace_with_replacements(key, text, replacements_json) {
    let call_replacements: Vec<String> = argus_json::parse_string_array(replacements_json.as_bytes()).ok()?;
    CREPLACE_MAP.with(|cell| -> Option<String> {
        let map = cell.borrow_mut();
        let replacements = map.get(key)?;
        Some(replacements.automaton.replace_all(text, &call_replacements))
    })
});
