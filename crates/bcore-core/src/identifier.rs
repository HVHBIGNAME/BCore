//! Namespaced resource identifiers (`minecraft:stone`).

use std::fmt;
use std::str::FromStr;

/// A namespaced resource identifier, e.g. `minecraft:stone`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub namespace: String,
    pub path: String,
}

impl Identifier {
    pub fn new(namespace: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            path: path.into(),
        }
    }

    /// Shorthand for identifiers in the default `minecraft` namespace.
    pub fn minecraft(path: impl Into<String>) -> Self {
        Self::new("minecraft", path)
    }

    /// Parse `namespace:path`, or a bare `path` (defaults to `minecraft`).
    ///
    /// Returns `None` if the input contains characters outside the allowed
    /// resource-location character set.
    pub fn parse(input: &str) -> Option<Self> {
        match input.split_once(':') {
            Some((ns, path)) => {
                if Self::valid_namespace(ns) && Self::valid_path(path) {
                    Some(Self::new(ns, path))
                } else {
                    None
                }
            }
            None => {
                if Self::valid_path(input) {
                    Some(Self::minecraft(input))
                } else {
                    None
                }
            }
        }
    }

    fn valid_namespace(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-')
            })
    }

    fn valid_path(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '/' | '_' | '.' | '-')
            })
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

impl FromStr for Identifier {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Identifier::parse(s).ok_or("invalid resource identifier")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_namespaced() {
        let id = Identifier::parse("minecraft:stone").unwrap();
        assert_eq!(id.namespace, "minecraft");
        assert_eq!(id.path, "stone");
        assert_eq!(id.to_string(), "minecraft:stone");
    }

    #[test]
    fn parse_bare_defaults_to_minecraft() {
        let id = Identifier::parse("stone").unwrap();
        assert_eq!(id.namespace, "minecraft");
        assert_eq!(id.path, "stone");
    }

    #[test]
    fn rejects_invalid() {
        assert!(Identifier::parse("Bad:Name").is_none());
        assert!(Identifier::parse("").is_none());
        assert!(Identifier::parse("minecraft:").is_none());
    }
}
