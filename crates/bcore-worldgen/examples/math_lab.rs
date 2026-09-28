//! Bit-exact native comparison and opt-in microbenchmarks; see docs/numeric-parity.md.
#[path = "support/formulas.rs"]
mod formulas;
#[path = "support/numeric.rs"]
mod numeric;

use numeric::{Fixture, Kernel, Sample};
use serde_json::json;
use std::{env, error::Error, fs, hint::black_box, time::Instant};

struct Args {
    command: String,
    path: String,
    filter: Option<String>,
    iterations: usize,
    rounds: usize,
    json: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = env::args().skip(1);
        let usage = "usage: math_lab <check|sample|bench|experiment> <json-file> [--filter text] [--iterations 1000] [--rounds 7] [--json]";
        let mut result = Self {
            command: args.next().ok_or(usage)?,
            path: args.next().ok_or(usage)?,
            filter: None,
            iterations: 1000,
            rounds: 7,
            json: false,
        };
        if !matches!(
            result.command.as_str(),
            "check" | "sample" | "bench" | "experiment"
        ) {
            return Err(usage.into());
        }
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--json" => result.json = true,
                "--filter" => result.filter = Some(args.next().ok_or("missing filter")?),
                "--iterations" => {
                    result.iterations = args
                        .next()
                        .ok_or("missing iterations")?
                        .parse::<usize>()
                        .map_err(|e| e.to_string())?;
                }
                "--rounds" => {
                    result.rounds = args
                        .next()
                        .ok_or("missing rounds")?
                        .parse::<usize>()
                        .map_err(|e| e.to_string())?;
                }
                _ => return Err(format!("unknown option {flag}")),
            }
        }
        if result.iterations == 0 || result.rounds == 0 {
            return Err("iterations and rounds must be positive".into());
        }
        if result.command == "bench" && result.filter.is_none() {
            return Err("bench needs --filter to select the kernels to measure".into());
        }
        Ok(result)
    }
}

fn timing(kernel: &Kernel, points: &[[f64; 3]], iterations: usize, rounds: usize) -> Vec<f64> {
    // Construction, JSON and reference checks are outside the timed section.
    for _ in 0..100 {
        for p in points {
            black_box(kernel.sample(black_box(*p)));
        }
    }
    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let start = Instant::now();
        for _ in 0..iterations {
            for p in points {
                black_box(black_box(kernel).sample(black_box(*p)));
            }
        }
        times.push(start.elapsed().as_secs_f64() * 1e9 / iterations as f64 / points.len() as f64);
    }
    times.sort_by(f64::total_cmp);
    times
}

fn run() -> Result<bool, Box<dyn Error>> {
    let args = Args::parse()?;
    if env::var_os("BCORE_DATAPACK").is_some() {
        return Err("unset BCORE_DATAPACK for the pinned numeric comparison".into());
    }
    let mut fixture: Fixture = serde_json::from_str(&fs::read_to_string(&args.path)?)?;
    fixture.validate(args.command != "sample")?;
    let cases: Vec<_> = fixture
        .cases
        .iter()
        .filter(|case| {
            args.filter
                .as_ref()
                .is_none_or(|filter| case.id.contains(filter))
        })
        .collect();
    if cases.is_empty() {
        return Err("no cases matched the filter".into());
    }
    if args.command == "sample" {
        let mut samples = Vec::new();
        for case in &cases {
            let points = &fixture.points[&case.points];
            let kernel = case.prepare(points)?;
            samples.push(Sample {
                id: case.id.clone(),
                bits: points
                    .iter()
                    .map(|p| {
                        case.precision()
                            .hex(case.precision().bits(kernel.sample(*p)))
                    })
                    .collect(),
            });
        }
        // Rust results are not native captures and must not retain native provenance.
        fixture.minecraft = None;
        fixture.jar_sha256 = None;
        fixture.probe_sha256 = None;
        fixture.samples = samples;
        if let Some(filter) = &args.filter {
            fixture.cases.retain(|c| c.id.contains(filter));
        }
        println!("{}", serde_json::to_string(&fixture)?);
        return Ok(true);
    }

    let start = Instant::now();
    let reports: Vec<_> = cases
        .iter()
        .map(|c| fixture.check(c))
        .collect::<Result<_, _>>()?;
    let values: usize = reports.iter().map(|r| r.values).sum();
    let mismatches: usize = reports.iter().map(|r| r.mismatches).sum();
    let elapsed = start.elapsed().as_secs_f64();
    let mut measurements = Vec::new();
    if mismatches == 0 && matches!(args.command.as_str(), "bench" | "experiment") {
        for case in &cases {
            let points = &fixture.points[&case.points];
            let candidates = formulas::candidates(&case.spec);
            if args.command == "experiment" && candidates.is_empty() {
                continue;
            }
            let kernel = case.prepare(points)?;
            let times = timing(&kernel, points, args.iterations, args.rounds);
            let baseline = times[times.len() / 2];
            measurements.push(json!({"id": case.id, "candidate": "production", "mismatches": 0,
                "median_ns": baseline, "min_ns": times[0], "max_ns": times[times.len()-1], "samples_ns": times}));
            if args.command == "experiment" {
                for candidate in candidates {
                    let report =
                        numeric::compare(case, points, fixture.reference(case)?, candidate.sample)?;
                    let times = timing(
                        &Kernel::Scalar(candidate.sample),
                        points,
                        args.iterations,
                        args.rounds,
                    );
                    let median = times[times.len() / 2];
                    measurements.push(json!({"id": case.id, "candidate": candidate.name,
                        "bit_exact": report.mismatches == 0, "mismatches": report.mismatches,
                        "max_ulps": report.max_ulps, "first": report.first,
                        "median_ns": median, "min_ns": times[0], "max_ns": times[times.len()-1],
                        "speedup": baseline/median, "samples_ns": times}));
                }
            }
        }
        if measurements.is_empty() {
            return Err(
                "no formula experiments matched; select scalar/smoothstep or scalar/lerp".into(),
            );
        }
    }
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "cases": cases.len(), "values": values, "mismatches": mismatches,
                "check_seconds": elapsed, "reports": reports, "measurements": measurements,
                "iterations": args.iterations, "rounds": args.rounds,
            }))?
        );
    } else {
        for report in reports.iter().filter(|r| r.mismatches > 0) {
            println!(
                "{}: {}/{} different, max {} ULP, max abs {:.9e}",
                report.id,
                report.mismatches,
                report.values,
                report.max_ulps,
                report.max_absolute_error
            );
            for first in &report.first {
                println!(
                    "  [{}] {:?}: Java {} ({}) Rust {} ({}) [{} ULP]",
                    first.index,
                    first.input,
                    first.expected,
                    first.expected_bits,
                    first.actual,
                    first.actual_bits,
                    first.ulps
                );
            }
        }
        println!("{} cases, {values} values, {mismatches} bit mismatches; {elapsed:.3}s (including kernel setup)", cases.len());
        for m in &measurements {
            println!(
                "{} {}: median {:.2} ns/value, {} bit mismatches{}",
                m["id"].as_str().unwrap(),
                m["candidate"].as_str().unwrap(),
                m["median_ns"].as_f64().unwrap(),
                m["mismatches"],
                m.get("speedup")
                    .map(|s| format!(", {:.3}x", s.as_f64().unwrap()))
                    .unwrap_or_default()
            );
        }
    }
    Ok(mismatches == 0)
}

fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("math_lab: {e}");
            std::process::exit(2);
        }
    }
}
