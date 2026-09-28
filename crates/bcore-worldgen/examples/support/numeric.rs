use bcore_worldgen::{
    density::{self, DensityFunction, EvalContext, EvaluationMode},
    noise_perlin::{self, BlendedNoise, ImprovedNoise, NormalNoise, PerlinNoise, Xoroshiro},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const JAR_SHA256: &str = "a7fed6f7d88379349e35ae0c6e9881d4484605132f6f620376a3868eea6cce52";

#[derive(Deserialize, Serialize)]
pub struct Fixture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_sha256: Option<String>,
    pub points: BTreeMap<String, Vec<[f64; 3]>>,
    pub cases: Vec<Case>,
    #[serde(default)]
    pub samples: Vec<Sample>,
}

#[derive(Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub points: String,
    #[serde(flatten)]
    pub spec: Spec,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    F32,
    F64,
}

impl Precision {
    pub fn bits(self, value: f64) -> u64 {
        match self {
            Self::F32 => u64::from((value as f32).to_bits()),
            Self::F64 => value.to_bits(),
        }
    }

    pub fn value(self, bits: u64) -> f64 {
        match self {
            Self::F32 => f32::from_bits(bits as u32) as f64,
            Self::F64 => f64::from_bits(bits),
        }
    }

    pub fn hex(self, bits: u64) -> String {
        match self {
            Self::F32 => format!("{bits:08x}"),
            Self::F64 => format!("{bits:016x}"),
        }
    }

    pub fn parse(self, hex: &str) -> Result<u64, String> {
        let len = if self == Self::F32 { 8 } else { 16 };
        if hex.len() != len {
            return Err(format!("expected {len} hex digits, got {hex:?}"));
        }
        u64::from_str_radix(hex, 16).map_err(|e| e.to_string())
    }

    pub fn ulps(self, a: u64, b: u64) -> u64 {
        let (sign, mask) = match self {
            Self::F32 => (1u64 << 31, u32::MAX as u64),
            Self::F64 => (1u64 << 63, u64::MAX),
        };
        let ordered = |bits: u64| {
            if bits & sign == 0 {
                bits | sign
            } else {
                !bits & mask
            }
        };
        ordered(a).abs_diff(ordered(b))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Spec {
    Smoothstep,
    Wrap,
    ClampedLerp,
    Lerp {
        precision: Precision,
    },
    Improved {
        seed: i64,
        y_scale: f64,
        y_fudge: f64,
    },
    Perlin {
        seed: i64,
        first_octave: i32,
        amplitudes: Vec<f64>,
    },
    Normal {
        seed: i64,
        noise: String,
    },
    Blended {
        seed: i64,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear: f64,
    },
    Density {
        seed: i64,
        function: serde_json::Value,
    },
    Router {
        seed: i64,
        field: String,
    },
    NoiseChunk {
        seed: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        function: Option<serde_json::Value>,
    },
}

pub enum Kernel {
    Scalar(fn([f64; 3]) -> f64),
    Improved(Box<ImprovedNoise>, f64, f64),
    Perlin(PerlinNoise),
    Normal(NormalNoise),
    Blended(Box<BlendedNoise>),
    Density(Box<DensityFunction>, EvalContext),
    NoiseChunk(Box<DensityFunction>, i64),
}

impl Kernel {
    pub fn sample(&self, [x, y, z]: [f64; 3]) -> f64 {
        match self {
            Self::Scalar(f) => f([x, y, z]),
            Self::Improved(n, scale, fudge) => n.noise(x, y, z, *scale, *fudge),
            Self::Perlin(n) => n.value(x, y, z),
            Self::Normal(n) => n.get_value(x, y, z),
            Self::Blended(n) => n.compute(x, y, z),
            Self::Density(n, ctx) => n.evaluate(x, y, z, ctx),
            Self::NoiseChunk(n, seed) => {
                let ctx = EvalContext {
                    seed: *seed,
                    ..Default::default()
                }
                .with_noise_bounds(
                    (x as i32).div_euclid(4) * 4,
                    (z as i32).div_euclid(4) * 4,
                    1,
                );
                n.evaluate(x, y, z, &ctx)
            }
        }
    }
}

impl Case {
    pub fn precision(&self) -> Precision {
        match self.spec {
            Spec::Lerp { precision } => precision,
            _ => Precision::F64,
        }
    }

    pub fn prepare(&self, points: &[[f64; 3]]) -> Result<Kernel, String> {
        density::clear_density_caches();
        if points.is_empty() || points.iter().flatten().any(|v| !v.is_finite()) {
            return Err(format!("{}: expected nonempty finite inputs", self.id));
        }
        if matches!(
            self.spec,
            Spec::Blended { .. }
                | Spec::Density { .. }
                | Spec::Router { .. }
                | Spec::NoiseChunk { .. }
        ) && points.iter().flatten().any(|v| *v != (*v as i32) as f64)
        {
            return Err(format!("{}: density coordinates must be int32", self.id));
        }
        if matches!(self.spec, Spec::NoiseChunk { .. })
            && points.iter().any(|p| p[1] < -64.0 || p[1] >= 320.0)
        {
            return Err(format!("{}: noise_chunk Y must be -64..319", self.id));
        }
        let raw = |seed, function| {
            Kernel::Density(
                Box::new(function),
                EvalContext {
                    seed,
                    mode: EvaluationMode::Raw,
                    ..Default::default()
                },
            )
        };
        Ok(match &self.spec {
            Spec::Smoothstep => Kernel::Scalar(|p| noise_perlin::smoothstep(p[0])),
            Spec::Wrap => Kernel::Scalar(|p| noise_perlin::wrap(p[0])),
            Spec::ClampedLerp => Kernel::Scalar(|p| noise_perlin::clamped_lerp(p[0], p[1], p[2])),
            Spec::Lerp { precision } => Kernel::Scalar(match precision {
                Precision::F64 => |p| noise_perlin::lerp(p[0], p[1], p[2]),
                Precision::F32 => |p| {
                    let [t, a, b] = p.map(|v| v as f32);
                    noise_perlin::lerp_f32(t, a, b) as f64
                },
            }),
            Spec::Improved {
                seed,
                y_scale,
                y_fudge,
            } => Kernel::Improved(
                Box::new(ImprovedNoise::from_random(&mut Xoroshiro::new(*seed))),
                *y_scale,
                *y_fudge,
            ),
            Spec::Perlin {
                seed,
                first_octave,
                amplitudes,
            } => {
                if amplitudes.is_empty() || amplitudes.iter().any(|a| !a.is_finite()) {
                    return Err("Perlin amplitudes must be nonempty and finite".into());
                }
                Kernel::Perlin(PerlinNoise::create(
                    &mut Xoroshiro::new(*seed),
                    *first_octave,
                    amplitudes,
                ))
            }
            Spec::Normal { seed, noise } => {
                let name = noise.strip_prefix("minecraft:").unwrap_or(noise);
                let def = density::noise_registry()
                    .defs
                    .get(name)
                    .ok_or_else(|| format!("unknown noise {noise}"))?;
                Kernel::Normal(NormalNoise::for_world(
                    *seed,
                    &format!("minecraft:{name}"),
                    def.first_octave,
                    &def.amplitudes,
                ))
            }
            Spec::Blended {
                seed,
                xz_scale,
                y_scale,
                xz_factor,
                y_factor,
                smear,
            } => Kernel::Blended(Box::new(BlendedNoise::for_world(
                *seed, *xz_scale, *y_scale, *xz_factor, *y_factor, *smear,
            ))),
            Spec::Density { seed, function } => {
                raw(*seed, density::parse_json(&function.to_string())?)
            }
            Spec::Router { seed, field } => raw(*seed, density::parse_router("overworld", field)?),
            Spec::NoiseChunk { seed, function } => {
                let function = if let Some(json) = function {
                    density::parse_json(&json.to_string())?
                } else {
                    density::parse_router("overworld", "final_density")?
                };
                Kernel::NoiseChunk(
                    Box::new(DensityFunction::CacheAllInCell(Box::new(
                        DensityFunction::Add(
                            Box::new(function),
                            Box::new(DensityFunction::Constant(0.0)), // Empty native beardifier.
                        ),
                    ))),
                    *seed,
                )
            }
        })
    }
}

#[derive(Deserialize, Serialize)]
pub struct Sample {
    pub id: String,
    pub bits: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Difference {
    pub index: usize,
    pub input: [f64; 3],
    pub expected: String,
    pub actual: String,
    pub expected_bits: String,
    pub actual_bits: String,
    pub ulps: u64,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub id: String,
    pub precision: Precision,
    pub values: usize,
    pub mismatches: usize,
    pub max_ulps: u64,
    pub max_absolute_error: f64,
    /// Rounding the native f64 result to f32 and back; not an all-f32 noise port.
    pub f32_output_changes: usize,
    pub first: Vec<Difference>,
}

pub fn compare(
    case: &Case,
    points: &[[f64; 3]],
    expected: &Sample,
    mut sample: impl FnMut([f64; 3]) -> f64,
) -> Result<Report, String> {
    if expected.id != case.id || expected.bits.len() != points.len() {
        return Err(format!("{}: reference id or sample count differs", case.id));
    }
    let precision = case.precision();
    let mut report = Report {
        id: case.id.clone(),
        precision,
        values: points.len(),
        mismatches: 0,
        max_ulps: 0,
        max_absolute_error: 0.0,
        f32_output_changes: 0,
        first: Vec::new(),
    };
    for (index, (&input, hex)) in points.iter().zip(&expected.bits).enumerate() {
        let expected_bits = precision.parse(hex)?;
        let expected = precision.value(expected_bits);
        let actual_bits = precision.bits(sample(input));
        let actual = precision.value(actual_bits);
        if precision == Precision::F64 && (expected as f32 as f64).to_bits() != expected_bits {
            report.f32_output_changes += 1;
        }
        if actual_bits != expected_bits {
            report.mismatches += 1;
            let ulps = precision.ulps(expected_bits, actual_bits);
            report.max_ulps = report.max_ulps.max(ulps);
            report.max_absolute_error = report.max_absolute_error.max((actual - expected).abs());
            if report.first.len() < 3 {
                report.first.push(Difference {
                    index,
                    input,
                    expected: format!("{expected:.17e}"),
                    actual: format!("{actual:.17e}"),
                    expected_bits: precision.hex(expected_bits),
                    actual_bits: precision.hex(actual_bits),
                    ulps,
                });
            }
        }
    }
    Ok(report)
}

impl Fixture {
    pub fn validate(&self, require_reference: bool) -> Result<(), String> {
        if self.cases.is_empty() {
            return Err("empty numeric case set".into());
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if !ids.insert(&case.id) || !self.points.contains_key(&case.points) {
                return Err(format!("duplicate case or missing point set: {}", case.id));
            }
        }
        if require_reference {
            if self.minecraft.as_deref() != Some("26.1")
                || self.jar_sha256.as_deref() != Some(JAR_SHA256)
                || self.probe_sha256.as_ref().is_none_or(|s| s.len() != 64)
            {
                return Err(
                    "expected a native 26.1 capture with the pinned JAR/source hashes".into(),
                );
            }
            let mut sample_ids = BTreeSet::new();
            for sample in &self.samples {
                if !sample_ids.insert(&sample.id) {
                    return Err(format!("duplicate reference {}", sample.id));
                }
            }
            if sample_ids != ids {
                return Err("case and reference IDs differ".into());
            }
        }
        Ok(())
    }

    pub fn reference(&self, case: &Case) -> Result<&Sample, String> {
        self.samples
            .iter()
            .find(|s| s.id == case.id)
            .ok_or_else(|| format!("missing reference {}", case.id))
    }

    pub fn check(&self, case: &Case) -> Result<Report, String> {
        let points = &self.points[&case.points];
        let kernel = case.prepare(points)?;
        let report = compare(case, points, self.reference(case)?, |p| kernel.sample(p));
        density::clear_density_caches();
        report
    }
}
