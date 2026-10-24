//! Strict decoding helpers. Unknown behavior is an error, not a no-op node.
use super::{LootError, Result};
use serde_json::{Map, Value};

pub(super) fn invalid(path: &str, message: impl Into<String>) -> LootError {
    LootError::InvalidData {
        path: path.into(),
        message: message.into(),
    }
}

pub(super) fn unsupported(path: &str, kind: &str) -> LootError {
    LootError::Unsupported {
        path: path.into(),
        kind: kind.into(),
    }
}

pub(super) fn parse(text: &str, path: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|e| invalid(path, e.to_string()))
}

pub(super) fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| invalid(path, "expected object"))
}

pub(super) fn array<'a>(value: &'a Value, path: &str) -> Result<&'a [Value]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| invalid(path, "expected array"))
}

pub(super) fn optional_array<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a [Value]> {
    match value.get(key) {
        Some(value) => array(value, &format!("{path}.{key}")),
        None => Ok(&[]),
    }
}

pub(super) fn string<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| invalid(path, "expected string"))
}

pub(super) fn identifier(value: &Value, path: &str) -> Result<String> {
    resource(string(value, path)?, path)
}

pub(super) fn resource(value: &str, path: &str) -> Result<String> {
    let (namespace, name) = value.split_once(':').unwrap_or(("minecraft", value));
    let valid = |s: &str, slash: bool| {
        !s.is_empty()
            && s.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b"_.-".contains(&b)
                    || slash && b == b'/'
            })
    };
    if !valid(namespace, false) || !valid(name, true) {
        return Err(invalid(
            path,
            format!("invalid resource identifier {value:?}"),
        ));
    }
    Ok(format!("{namespace}:{name}"))
}

pub(super) fn int(value: &Value, path: &str) -> Result<i32> {
    value
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| invalid(path, "expected signed 32-bit integer"))
}

pub(super) fn float(value: &Value, path: &str) -> Result<f32> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid(path, "expected finite float32"))
}

pub(super) fn boolean(value: &Value, key: &str, default: bool, path: &str) -> Result<bool> {
    match value.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| invalid(path, format!("{key}: expected boolean"))),
    }
}

pub(super) fn fields(value: &Value, allowed: &[&str], path: &str) -> Result<()> {
    for key in object(value, path)?.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(unsupported(path, &format!("field {key}")));
        }
    }
    Ok(())
}
