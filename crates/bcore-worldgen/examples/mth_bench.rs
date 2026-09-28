//! Native-indexed lookup versus the previous carver scalar implementation.
use bcore_worldgen::mth;
use serde_json::{json, Value};
use std::{env, error::Error, hint::black_box, time::Instant};

const INPUT_COUNT: usize = 16_384;
const WARMUP_PASSES: usize = 16;
const INPUT_SEED: u64 = 0x4d54_4832_3631;
const SIN_SCALE: f64 = 10430.378350470453;

struct Args {
    iterations: usize,
    rounds: usize,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut result = Self {
            iterations: 256,
            rounds: 9,
        };
        let mut args = env::args().skip(1);
        while let Some(flag) = args.next() {
            let field = match flag.as_str() {
                "--iterations" => &mut result.iterations,
                "--rounds" => &mut result.rounds,
                _ => {
                    return Err(format!(
                        "unknown argument {flag}; use --iterations N --rounds N"
                    ))
                }
            };
            *field = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?
                .parse()
                .map_err(|e| format!("invalid {flag}: {e}"))?;
        }
        if result.iterations == 0 || result.rounds < 3 {
            return Err("iterations must be positive and rounds must be at least three".into());
        }
        Ok(result)
    }
}

// Keep the old scalar formula and its native long-cast indexing as the baseline.
#[inline]
fn scalar_sin(angle: f64) -> f32 {
    let index = ((angle * SIN_SCALE) as i64 & 0xffff) as usize;
    (index as f64 / SIN_SCALE).sin() as f32
}

#[inline]
fn scalar_cos(angle: f64) -> f32 {
    let index = ((angle * SIN_SCALE + 16384.0) as i64 & 0xffff) as usize;
    (index as f64 / SIN_SCALE).sin() as f32
}

fn run_kernel<T: Copy, F: Fn(T) -> f32>(inputs: &[T], kernel: &F, iterations: usize) {
    for _ in 0..iterations {
        for &input in inputs {
            black_box(kernel(black_box(input)));
        }
    }
}

fn time_kernel<T: Copy, F: Fn(T) -> f32>(inputs: &[T], kernel: &F, iterations: usize) -> f64 {
    let start = Instant::now();
    run_kernel(inputs, kernel, iterations);
    start.elapsed().as_secs_f64() * 1e9 / iterations as f64 / inputs.len() as f64
}

fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

fn bench<T: Copy, S: Fn(T) -> f32, L: Fn(T) -> f32>(
    name: &str,
    inputs: &[T],
    scalar: S,
    lookup: L,
    args: &Args,
) -> Value {
    for (index, &input) in inputs.iter().enumerate() {
        assert_eq!(
            scalar(input).to_bits(),
            lookup(input).to_bits(),
            "{name} input {index}"
        );
    }
    run_kernel(inputs, &scalar, WARMUP_PASSES);
    run_kernel(inputs, &lookup, WARMUP_PASSES);
    let mut scalar_ns = Vec::with_capacity(args.rounds);
    let mut lookup_ns = Vec::with_capacity(args.rounds);
    for round in 0..args.rounds {
        // Alternate the order so one implementation does not always run first.
        if round % 2 == 0 {
            scalar_ns.push(time_kernel(inputs, &scalar, args.iterations));
            lookup_ns.push(time_kernel(inputs, &lookup, args.iterations));
        } else {
            lookup_ns.push(time_kernel(inputs, &lookup, args.iterations));
            scalar_ns.push(time_kernel(inputs, &scalar, args.iterations));
        }
    }
    let scalar_median = median(&scalar_ns);
    let lookup_median = median(&lookup_ns);
    json!({
        "kernel": name,
        "checked_inputs": inputs.len(),
        "scalar_median_ns_per_call": scalar_median,
        "lookup_median_ns_per_call": lookup_median,
        "median_speedup": scalar_median / lookup_median,
        "scalar_rounds_ns_per_call": scalar_ns,
        "lookup_rounds_ns_per_call": lookup_ns,
    })
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let start = Instant::now();
    black_box(mth::sin_table());
    let table_init_ns = start.elapsed().as_nanos();

    let mut random = INPUT_SEED;
    let doubles: Vec<f64> = (0..INPUT_COUNT)
        .map(|_| {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let unit = (random >> 11) as f64 / ((1u64 << 53) as f64);
            (unit * 2.0 - 1.0) * std::f64::consts::PI * 8.0
        })
        .collect();
    let floats: Vec<f32> = doubles.iter().map(|&value| value as f32).collect();
    let measurements = [
        bench("sin_f64", &doubles, scalar_sin, mth::sin, &args),
        bench("cos_f64", &doubles, scalar_cos, mth::cos, &args),
        bench(
            "sin_f32_promoted",
            &floats,
            |x| scalar_sin(f64::from(x)),
            mth::sin_f32,
            &args,
        ),
        bench(
            "cos_f32_promoted",
            &floats,
            |x| scalar_cos(f64::from(x)),
            mth::cos_f32,
            &args,
        ),
    ];
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "scope": "trigonometric-call microbenchmark; initialization measured separately",
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "input_seed": INPUT_SEED,
            "input_range": "[-8*pi, 8*pi)",
            "inputs": INPUT_COUNT,
            "iterations": args.iterations,
            "rounds": args.rounds,
            "warmup_passes": WARMUP_PASSES,
            "table_bytes": mth::SIN_TABLE_LEN * std::mem::size_of::<f32>(),
            "table_initialization_ns": table_init_ns,
            "measurements": measurements,
        }))?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("mth_bench: {error}");
        std::process::exit(2);
    }
}
