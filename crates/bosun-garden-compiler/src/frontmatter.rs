//! YAML 1.2 frontmatter for tenant filtering.
//!
//! Domain keys are top-level scalars (`blog: "tenant-slug"`). `yaml-rust2` follows
//! YAML 1.2, so `yes` stays a string and a leading-zero decimal stays decimal.

use std::collections::BTreeMap;

use yaml_rust2::{Yaml, YamlLoader};

/// Top-level scalar stored from a note's frontmatter mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    String(String),
    Int(i64),
    Bool(bool),
}

/// Parsed frontmatter. Nested collections are ignored; tenant keys are scalars.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoteFrontmatter {
    pub scalars: BTreeMap<String, Scalar>,
}

impl NoteFrontmatter {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.scalars.get(key) {
            Some(Scalar::String(value)) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.scalars.get(key) {
            Some(Scalar::Int(value)) => Some(*value),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.scalars.get(key) {
            Some(Scalar::Bool(value)) => Some(*value),
            _ => None,
        }
    }
}

/// Split optional frontmatter and parse it as YAML 1.2.
///
/// `Ok(None)` means the note has no opening frontmatter fence.
pub fn parse_note_frontmatter(source: &str) -> Result<Option<NoteFrontmatter>, String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let Some(yaml) = frontmatter_yaml(source)? else {
        return Ok(None);
    };
    if yaml.trim().is_empty() {
        return Ok(Some(NoteFrontmatter::default()));
    }
    let docs = YamlLoader::load_from_str(yaml).map_err(|err| err.to_string())?;
    let Some(doc) = docs.first() else {
        return Ok(Some(NoteFrontmatter::default()));
    };
    let Some(mapping) = doc.as_hash() else {
        return Err("frontmatter must be a YAML mapping".to_string());
    };
    let mut scalars = BTreeMap::new();
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            continue;
        };
        if let Some(scalar) = scalar_from(value) {
            scalars.insert(key.to_string(), scalar);
        }
    }
    Ok(Some(NoteFrontmatter { scalars }))
}

fn scalar_from(value: &Yaml) -> Option<Scalar> {
    if let Some(text) = value.as_str() {
        return Some(Scalar::String(text.to_string()));
    }
    if let Some(int) = value.as_i64() {
        return Some(Scalar::Int(int));
    }
    if let Some(flag) = value.as_bool() {
        return Some(Scalar::Bool(flag));
    }
    None
}

fn frontmatter_yaml(source: &str) -> Result<Option<&str>, String> {
    if !source.starts_with("---") {
        return Ok(None);
    }
    let after_open = if source.starts_with("---\r\n") {
        5
    } else if source.starts_with("---\n") {
        4
    } else {
        return Err("frontmatter fence must be its own line".to_string());
    };
    let rest = &source[after_open..];
    let mut search_from = 0usize;
    while let Some(rel) = rest[search_from..].find("---") {
        let abs = search_from + rel;
        let at_line_start = abs == 0 || rest.as_bytes()[abs - 1] == b'\n';
        if !at_line_start {
            search_from = abs + 3;
            continue;
        }
        let after_close = abs + 3;
        let closes_line = rest[after_close..].starts_with("\r\n")
            || rest[after_close..].starts_with('\n')
            || after_close == rest.len();
        if !closes_line {
            search_from = abs + 3;
            continue;
        }
        return Ok(Some(&rest[..abs]));
    }
    Err("unclosed frontmatter fence".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn yaml_1_2_keeps_yes_as_a_string_and_leading_zero_as_decimal() {
        let source =
            "---\nblog: \"tenant-slug\"\nflag: yes\ncount: 010\nenabled: true\n---\nBody\n";
        let frontmatter = parse_note_frontmatter(source).unwrap().unwrap();
        assert_eq!(frontmatter.get_str("blog"), Some("tenant-slug"));
        assert_eq!(
            frontmatter.get_str("flag"),
            Some("yes"),
            "YAML 1.1 would coerce yes to true"
        );
        assert_eq!(frontmatter.get_int("count"), Some(10));
        assert_eq!(frontmatter.get_bool("enabled"), Some(true));
    }

    #[test]
    fn missing_and_unclosed_frontmatter() {
        assert!(parse_note_frontmatter("No fence\n").unwrap().is_none());
        assert!(parse_note_frontmatter("---\nblog: x\n").is_err());
    }

    #[test]
    fn domain_scoped_tenant_keys_group_notes() {
        let notes = [
            "---\nblog: \"acme\"\n---\nA\n",
            "---\nblog: \"other\"\n---\nB\n",
            "---\nblog: \"acme\"\n---\nC\n",
            "No fence\n",
        ];
        let mut by_tenant: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, source) in notes.iter().enumerate() {
            let Some(frontmatter) = parse_note_frontmatter(source).unwrap() else {
                continue;
            };
            if let Some(tenant) = frontmatter.get_str("blog") {
                by_tenant.entry(tenant.to_string()).or_default().push(index);
            }
        }
        assert_eq!(
            by_tenant.get("acme").map(Vec::as_slice),
            Some(&[0usize, 2][..])
        );
        assert_eq!(
            by_tenant.get("other").map(Vec::as_slice),
            Some(&[1usize][..])
        );
        assert!(!by_tenant.contains_key(""));
    }
}
