//! Probe final_density's two min-arguments separately (after fixes).
use bcore_worldgen::density::{self, EvalContext};

fn main() {
    let root = "target/datapack";
    let settings = std::fs::read_to_string(format!(
        "{root}/data/minecraft/worldgen/noise_settings/overworld.json"
    ))
    .unwrap();
    let final_json = settings
        .split("\"final_density\":")
        .nth(1)
        .unwrap()
        .split("\"vein_toggle\"")
        .next()
        .unwrap();
    let arg1 = sub_obj(final_json, "\"argument1\":");
    let arg2 = sub_obj(final_json, "\"argument2\":");
    let f1 = density::parse_json(&arg1).unwrap();
    let f2 = density::parse_json(&arg2).unwrap();
    let ctx = EvalContext::default();
    for y in [-64.0, 0.0, 63.0, 64.0, 120.0] {
        let a = density::evaluate(&f1, 8.0, y, 8.0, &ctx);
        let b = density::evaluate(&f2, 8.0, y, 8.0, &ctx);
        println!(
            "y={y:>5}: squeeze={a:>8.4}  noodle={b:>8.4}  min={:.4}",
            a.min(b)
        );
    }
}

fn sub_obj(s: &str, key: &str) -> String {
    let p = s.find(key).unwrap() + key.len();
    let rest = &s[p..];
    let start = rest.find('{').or_else(|| rest.find('"')).unwrap();
    let b = rest.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut i = start;
    let mut end = rest.len();
    while i < b.len() {
        let c = b[i] as char;
        if in_str {
            if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    rest[start..end].to_string()
}
