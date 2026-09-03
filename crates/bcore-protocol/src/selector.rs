//! Разбор селекторов целей Minecraft.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorKind {
    All,
    Nearest,
    Random,
    SelfEntity,
    Entities,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectorOptions {
    pub distance: Option<String>,
    pub entity_type: Option<String>,
    pub limit: Option<usize>,
    pub sort: Option<String>,
    pub gamemode: Option<String>,
    pub name: Option<String>,
    pub level: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub kind: SelectorKind,
    pub options: SelectorOptions,
}

pub fn parse(input: &str) -> Option<Selector> {
    if !input.starts_with('@') {
        return None;
    }
    let (kind, rest) = match input.chars().nth(1)? {
        'a' => (SelectorKind::All, &input[2..]),
        'p' => (SelectorKind::Nearest, &input[2..]),
        'r' => (SelectorKind::Random, &input[2..]),
        's' => (SelectorKind::SelfEntity, &input[2..]),
        'e' => (SelectorKind::Entities, &input[2..]),
        _ => return None,
    };
    let mut o = SelectorOptions::default();
    if rest.is_empty() {
        return Some(Selector { kind, options: o });
    }
    let body = rest.strip_prefix('[')?.strip_suffix(']')?;
    let mut values = BTreeMap::new();
    for item in body.split(',') {
        let (k, v) = item.split_once('=')?;
        values.insert(k.trim(), v.trim());
    }
    o.distance = values.get("distance").map(|s| s.to_string());
    o.entity_type = values.get("type").map(|s| s.to_string());
    o.limit = values.get("limit").and_then(|s| s.parse().ok());
    o.sort = values.get("sort").map(|s| s.to_string());
    o.gamemode = values.get("gamemode").map(|s| s.to_string());
    o.name = values.get("name").map(|s| s.to_string());
    o.level = values.get("level").map(|s| s.to_string());
    Some(Selector { kind, options: o })
}

pub fn resolve(input: &str, online: &[String], sender: &str) -> Vec<String> {
    let Some(s) = parse(input) else {
        return online
            .iter()
            .find(|n| n.eq_ignore_ascii_case(input))
            .cloned()
            .into_iter()
            .collect();
    };
    let mut out = match s.kind {
        SelectorKind::SelfEntity => vec![sender.to_string()],
        SelectorKind::All | SelectorKind::Entities => online.to_vec(),
        SelectorKind::Nearest | SelectorKind::Random => {
            online.first().cloned().into_iter().collect()
        }
    };
    if let Some(n) = s.options.name {
        out.retain(|x| x.eq_ignore_ascii_case(&n));
    }
    if let Some(l) = s.options.limit {
        out.truncate(l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_resolves_common_selectors() {
        let p = parse("@a[limit=2,sort=nearest]").unwrap();
        assert_eq!(p.options.limit, Some(2));
        let names = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(resolve("@a[limit=2]", &names, "A"), vec!["A", "B"]);
        assert_eq!(resolve("@s", &names, "B"), vec!["B"]);
    }
}
