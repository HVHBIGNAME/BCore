//! Vanilla-style density-function evaluation primitives.
//! The JSON reader intentionally has no dependency on serde, keeping worldgen usable standalone.
use crate::noise::value_noise_3d;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum DensityFunction {
    Constant(f64),
    Noise {
        name: String,
        xz: f64,
        y: f64,
    },
    Add(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    Min(Box<Self>, Box<Self>),
    Max(Box<Self>, Box<Self>),
    Clamp(Box<Self>, f64, f64),
    Abs(Box<Self>),
    Square(Box<Self>),
    Cube(Box<Self>),
    Lerp {
        alpha: Box<Self>,
        min: Box<Self>,
        max: Box<Self>,
    },
    Range {
        input: Box<Self>,
        min: f64,
        max: f64,
        when_in: Box<Self>,
        when_out: Box<Self>,
    },
    YGradient {
        from: i32,
        to: i32,
        from_value: f64,
        to_value: f64,
    },
    Spline(Vec<(f64, f64, f64)>),
    Interpolated(Box<Self>),
    ShiftA(String),
    ShiftB(String),
    Cache2d(Box<Self>),
    CacheAllInCell(Box<Self>),
    FlatCache(Box<Self>),
    BlendOffset(Box<Self>),
    BlendAlpha(Box<Self>),
    WeirdScaledSampler {
        input: Box<Self>,
        noise: String,
        rarity: f64,
    },
    EndIslands,
    OldBlendedNoise,
    NoOp(Box<Self>),
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct EvalContext {
    pub seed: i64,
    pub cell_width: i32,
    pub cell_height: i32,
}
impl Default for EvalContext {
    fn default() -> Self {
        Self {
            seed: 0,
            cell_width: 4,
            cell_height: 8,
        }
    }
}
impl DensityFunction {
    pub fn constant(v: f64) -> Self {
        Self::Constant(v)
    }
    pub fn evaluate(&self, x: f64, y: f64, z: f64, ctx: &EvalContext) -> f64 {
        match self {
            Self::Constant(v) => *v,
            Self::Noise { name, xz, y: ys } => {
                value_noise_3d(ctx.seed ^ hash_name(name), x * xz, y * ys, z * xz)
            }
            Self::Add(a, b) => a.evaluate(x, y, z, ctx) + b.evaluate(x, y, z, ctx),
            Self::Mul(a, b) => a.evaluate(x, y, z, ctx) * b.evaluate(x, y, z, ctx),
            Self::Min(a, b) => a.evaluate(x, y, z, ctx).min(b.evaluate(x, y, z, ctx)),
            Self::Max(a, b) => a.evaluate(x, y, z, ctx).max(b.evaluate(x, y, z, ctx)),
            Self::Clamp(a, l, h) => a.evaluate(x, y, z, ctx).clamp(*l, *h),
            Self::Abs(a) => a.evaluate(x, y, z, ctx).abs(),
            Self::Square(a) => {
                let v = a.evaluate(x, y, z, ctx);
                v * v
            }
            Self::Cube(a) => {
                let v = a.evaluate(x, y, z, ctx);
                v * v * v
            }
            Self::Lerp { alpha, min, max } => {
                let a = alpha.evaluate(x, y, z, ctx);
                let lo = min.evaluate(x, y, z, ctx);
                lo + (max.evaluate(x, y, z, ctx) - lo) * a
            }
            Self::Range {
                input,
                min,
                max,
                when_in,
                when_out,
            } => {
                let v = input.evaluate(x, y, z, ctx);
                if v >= *min && v <= *max {
                    when_in.evaluate(x, y, z, ctx)
                } else {
                    when_out.evaluate(x, y, z, ctx)
                }
            }
            Self::YGradient {
                from,
                to,
                from_value,
                to_value,
            } => {
                let t = ((y - *from as f64) / (*to as f64 - *from as f64)).clamp(0., 1.);
                from_value + (to_value - from_value) * t
            }
            Self::Spline(p) => spline(p, y),
            Self::Interpolated(a)
            | Self::Cache2d(a)
            | Self::CacheAllInCell(a)
            | Self::FlatCache(a)
            | Self::BlendOffset(a)
            | Self::BlendAlpha(a)
            | Self::NoOp(a) => a.evaluate(x, y, z, ctx),
            Self::ShiftA(_) | Self::ShiftB(_) => 0.,
            Self::WeirdScaledSampler { input, rarity, .. } => {
                input.evaluate(x, y, z, ctx) * *rarity
            }
            Self::EndIslands | Self::OldBlendedNoise => 0.,
            Self::Unknown => 0.,
        }
    }
}
fn hash_name(s: &str) -> i64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3)
    }
    h as i64
}
fn spline(p: &[(f64, f64, f64)], x: f64) -> f64 {
    if p.is_empty() {
        return 0.;
    }
    if x <= p[0].0 {
        return p[0].1 + (x - p[0].0) * p[0].2;
    }
    for w in p.windows(2) {
        if x <= w[1].0 {
            let (x0, y0, d0) = w[0];
            let (x1, y1, d1) = w[1];
            let t = (x - x0) / (x1 - x0);
            return (2. * t * t * t - 3. * t * t + 1.) * y0
                + (t * t * t - 2. * t * t + t) * d0 * (x1 - x0)
                + (-2. * t * t * t + 3. * t * t) * y1
                + (t * t * t - t * t) * d1 * (x1 - x0);
        }
    }
    let q = p[p.len() - 1];
    q.1 + (x - q.0) * q.2
}

#[derive(Debug, Clone)]
enum J {
    N(f64),
    S(String),
    A(Vec<J>),
    O(HashMap<String, J>),
    B(bool),
    Null,
}
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}
impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1
        }
    }
    fn val(&mut self) -> Result<J, String> {
        self.ws();
        if self.i >= self.s.len() {
            return Err("unexpected eof".into());
        }
        match self.s[self.i] {
            b'{' => self.obj(),
            b'[' => self.arr(),
            b'"' => self.string().map(J::S),
            b't' => {
                self.i += 4;
                Ok(J::B(true))
            }
            b'f' => {
                self.i += 5;
                Ok(J::B(false))
            }
            b'n' => {
                self.i += 4;
                Ok(J::Null)
            }
            _ => self.number().map(J::N),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let st = self.i;
        let mut out = String::new();
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'"' => {
                    out.push_str(std::str::from_utf8(&self.s[st..self.i]).map_err(|_| "utf8")?);
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    out.push_str(std::str::from_utf8(&self.s[st..self.i]).map_err(|_| "utf8")?);
                    self.i += 1;
                    if self.i >= self.s.len() {
                        break;
                    }
                    out.push(self.s[self.i] as char);
                    self.i += 1
                }
                _ => self.i += 1,
            }
        }
        Err("unterminated string".into())
    }
    fn number(&mut self) -> Result<f64, String> {
        let st = self.i;
        while self.i < self.s.len() && b"-+.0123456789eE".contains(&self.s[self.i]) {
            self.i += 1
        }
        std::str::from_utf8(&self.s[st..self.i])
            .map_err(|_| "number")?
            .parse()
            .map_err(|_| "number".into())
    }
    fn arr(&mut self) -> Result<J, String> {
        self.i += 1;
        let mut a = Vec::new();
        loop {
            self.ws();
            if self.s.get(self.i) == Some(&b']') {
                self.i += 1;
                break;
            }
            a.push(self.val()?);
            self.ws();
            if self.s.get(self.i) == Some(&b',') {
                self.i += 1
            } else if self.s.get(self.i) != Some(&b']') {
                return Err("array separator".into());
            }
        }
        Ok(J::A(a))
    }
    fn obj(&mut self) -> Result<J, String> {
        self.i += 1;
        let mut o = HashMap::new();
        loop {
            self.ws();
            if self.s.get(self.i) == Some(&b'}') {
                self.i += 1;
                break;
            }
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return Err("object colon".into());
            }
            self.i += 1;
            o.insert(k, self.val()?);
            self.ws();
            if self.s.get(self.i) == Some(&b',') {
                self.i += 1
            } else if self.s.get(self.i) != Some(&b'}') {
                return Err("object separator".into());
            }
        }
        Ok(J::O(o))
    }
}
fn num(o: &HashMap<String, J>, k: &str, d: f64) -> f64 {
    match o.get(k) {
        Some(J::N(v)) => *v,
        _ => d,
    }
}
fn boxed(v: Option<&J>) -> DensityFunction {
    v.map(parse_value).unwrap_or(DensityFunction::Constant(0.))
}
fn parse_value(v: &J) -> DensityFunction {
    match v {
        J::N(n) => DensityFunction::Constant(*n),
        J::S(_s) => DensityFunction::Unknown,
        J::O(o) => {
            let typ = match o.get("type") {
                Some(J::S(s)) => s.as_str(),
                _ => "",
            };
            match typ {
                "minecraft:constant" => DensityFunction::Constant(num(o, "value", 0.)),
                "minecraft:add" => DensityFunction::Add(
                    Box::new(boxed(o.get("argument_a").or_else(|| o.get("a")))),
                    Box::new(boxed(o.get("argument_b").or_else(|| o.get("b")))),
                ),
                "minecraft:mul" => DensityFunction::Mul(
                    Box::new(boxed(o.get("argument1").or_else(|| o.get("a")))),
                    Box::new(boxed(o.get("argument2").or_else(|| o.get("b")))),
                ),
                "minecraft:min" => DensityFunction::Min(
                    Box::new(boxed(o.get("argument1"))),
                    Box::new(boxed(o.get("argument2"))),
                ),
                "minecraft:max" => DensityFunction::Max(
                    Box::new(boxed(o.get("argument1"))),
                    Box::new(boxed(o.get("argument2"))),
                ),
                "minecraft:abs" => DensityFunction::Abs(Box::new(boxed(o.get("argument")))),
                "minecraft:square" => DensityFunction::Square(Box::new(boxed(o.get("argument")))),
                "minecraft:cube" => DensityFunction::Cube(Box::new(boxed(o.get("argument")))),
                "minecraft:clamp" => DensityFunction::Clamp(
                    Box::new(boxed(o.get("input"))),
                    num(o, "min", 0.),
                    num(o, "max", 0.),
                ),
                "minecraft:y_clamped_gradient" => DensityFunction::YGradient {
                    from: num(o, "from_y", 0.) as i32,
                    to: num(o, "to_y", 0.) as i32,
                    from_value: num(o, "from_value", 0.),
                    to_value: num(o, "to_value", 0.),
                },
                "minecraft:spline" => {
                    let mut p = Vec::new();
                    if let Some(J::A(a)) = o.get("points") {
                        for q in a {
                            if let J::A(v) = q {
                                if v.len() >= 3 {
                                    p.push((asnum(&v[0]), asnum(&v[1]), asnum(&v[2])))
                                }
                            }
                        }
                    }
                    DensityFunction::Spline(p)
                }
                "minecraft:noise" => DensityFunction::Noise {
                    name: match o.get("noise") {
                        Some(J::S(s)) => s.clone(),
                        _ => String::new(),
                    },
                    xz: num(o, "xz_scale", 1.),
                    y: num(o, "y_scale", 1.),
                },
                "minecraft:interpolated" => {
                    DensityFunction::Interpolated(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:shift_a" => DensityFunction::ShiftA(match o.get("argument") {
                    Some(J::S(s)) => s.clone(),
                    _ => String::new(),
                }),
                "minecraft:shift_b" => DensityFunction::ShiftB(match o.get("argument") {
                    Some(J::S(s)) => s.clone(),
                    _ => String::new(),
                }),
                "minecraft:cache_2d" => {
                    DensityFunction::Cache2d(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:cache_all_in_cell" => {
                    DensityFunction::CacheAllInCell(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:flat_cache" => {
                    DensityFunction::FlatCache(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:blend_offset" => {
                    DensityFunction::BlendOffset(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:blend_alpha" => {
                    DensityFunction::BlendAlpha(Box::new(boxed(o.get("argument"))))
                }
                "minecraft:weird_scaled_sampler" => DensityFunction::WeirdScaledSampler {
                    input: Box::new(boxed(o.get("input"))),
                    noise: match o.get("noise") {
                        Some(J::S(s)) => s.clone(),
                        _ => String::new(),
                    },
                    rarity: num(o, "rarity_value", 1.),
                },
                "minecraft:end_islands" => DensityFunction::EndIslands,
                "minecraft:old_blended_noise" => DensityFunction::OldBlendedNoise,
                "minecraft:no_op" => DensityFunction::NoOp(Box::new(boxed(o.get("argument")))),
                _ => DensityFunction::Unknown,
            }
        }
        _ => DensityFunction::Unknown,
    }
}
fn asnum(v: &J) -> f64 {
    match v {
        J::N(n) => *n,
        _ => 0.,
    }
}
/// Parse one density-function JSON object (or a numeric constant).
pub fn parse_json(input: &str) -> Result<DensityFunction, String> {
    let mut p = Parser {
        s: input.as_bytes(),
        i: 0,
    };
    let v = p.val()?;
    Ok(parse_value(&v))
}
/// Evaluate a parsed function at block coordinates.
pub fn evaluate(f: &DensityFunction, x: f64, y: f64, z: f64, ctx: &EvalContext) -> f64 {
    f.evaluate(x, y, z, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constants_and_arithmetic() {
        let f=parse_json(r#"{"type":"minecraft:add","argument_a":{"type":"minecraft:constant","value":2},"argument_b":{"type":"minecraft:square","argument":{"type":"minecraft:constant","value":3}}}"#).unwrap();
        assert_eq!(f.evaluate(0., 0., 0., &Default::default()), 11.);
    }
    #[test]
    fn gradient() {
        let f = DensityFunction::YGradient {
            from: 0,
            to: 10,
            from_value: -1.,
            to_value: 1.,
        };
        assert_eq!(f.evaluate(0., 5., 0., &Default::default()), 0.);
    }
}
