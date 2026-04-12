use super::bench_support::{
    CARDINALITY_SOURCE_SCENARIO, CardinalityMode, build_cardinality_records, collect_decoded_flows,
};
use super::resource_bench_support::{
    ResourceEnvelopeReport, cpu_percent_of_one_core, parse_child_report, print_resource_report,
    take_proc_snapshot,
};
use super::test_support::new_disk_benchmark_ingest_service;
use super::*;
use crate::plugin_config::DecapsulationMode as ConfigDecapsulationMode;
use std::process::Command;
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "NETFLOW_RESOURCE_BENCH_CHILD";
const PROFILE_ENV: &str = "NETFLOW_RESOURCE_BENCH_PROFILE";
const RATE_ENV: &str = "NETFLOW_RESOURCE_BENCH_FLOWS_PER_SEC";
const WARMUP_ENV: &str = "NETFLOW_RESOURCE_BENCH_WARMUP_SECS";
const MEASURE_ENV: &str = "NETFLOW_RESOURCE_BENCH_MEASURE_SECS";
const HIGH_POOL_ENV: &str = "NETFLOW_RESOURCE_BENCH_HIGH_POOL_FLOWS";
const LOW_POOL_ENV: &str = "NETFLOW_RESOURCE_BENCH_LOW_POOL_FLOWS";

const DEFAULT_WARMUP_SECS: u64 = 5;
const DEFAULT_MEASURE_SECS: u64 = 15;
const DEFAULT_LOW_POOL_FLOWS: usize = 256;
const DEFAULT_HIGH_POOL_FLOWS: usize = 4_096;
const RESOURCE_BENCH_RECEIVE_TIME_USEC: u64 = 1_700_000_000_000_000;
const TICKS_PER_SECOND: u64 = 10;
const BENCHMARK_RATES: &[u64] = &[5_000, 10_000, 20_000, 30_000];

#[derive(Clone, Copy)]
enum ResourceProfile {
    Low,
    High,
}

impl ResourceProfile {
    fn label(self) -> &'static str {
        match self {
            Self::Low => "low-cardinality-mixed",
            Self::High => "high-cardinality-mixed",
        }
    }

    fn cardinality_mode(self) -> CardinalityMode {
        match self {
            Self::Low => CardinalityMode::Low,
            Self::High => CardinalityMode::High,
        }
    }

    fn record_pool_size(self) -> usize {
        match self {
            Self::Low => env_usize(LOW_POOL_ENV, DEFAULT_LOW_POOL_FLOWS),
            Self::High => env_usize(HIGH_POOL_ENV, DEFAULT_HIGH_POOL_FLOWS),
        }
    }

    fn from_env() -> Self {
        match std::env::var(PROFILE_ENV).as_deref() {
            Ok("low") => Self::Low,
            Ok("high") => Self::High,
            Ok(other) => panic!("unsupported resource benchmark profile: {other}"),
            Err(err) => panic!("missing {PROFILE_ENV}: {err}"),
        }
    }
}

struct PacedLoopResult {
    ingested_flows: usize,
    entries_since_sync: usize,
    peak_rss_bytes: u64,
    peak_rss_anon_bytes: u64,
    peak_rss_file_bytes: u64,
}

#[test]
#[ignore = "manual paced resource-envelope benchmark"]
fn bench_resource_envelope_matrix() {
    for profile in [ResourceProfile::Low, ResourceProfile::High] {
        eprintln!();
        eprintln!("=== Resource Envelope: {} ===", profile.label());
        for &rate in BENCHMARK_RATES {
            let report = run_resource_envelope_case(profile, rate);
            print_resource_report(&report);
        }
    }
}

#[test]
#[ignore = "manual paced resource-envelope benchmark child helper"]
fn bench_resource_envelope_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let report = run_resource_envelope_child();
    println!(
        "RESOURCE_BENCH_RESULT:{}",
        serde_json::to_string(&report).expect("serialize resource benchmark result")
    );
}

fn run_resource_envelope_case(
    profile: ResourceProfile,
    flows_per_sec: u64,
) -> ResourceEnvelopeReport {
    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = Command::new(current_exe)
        .arg("--ignored")
        .arg("--exact")
        .arg("ingest::resource_bench_tests::bench_resource_envelope_child")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_ENV, "1")
        .env(
            PROFILE_ENV,
            match profile {
                ResourceProfile::Low => "low",
                ResourceProfile::High => "high",
            },
        )
        .env(RATE_ENV, flows_per_sec.to_string())
        .env(WARMUP_ENV, env_u64(WARMUP_ENV, DEFAULT_WARMUP_SECS).to_string())
        .env(
            MEASURE_ENV,
            env_u64(MEASURE_ENV, DEFAULT_MEASURE_SECS).to_string(),
        )
        .env(
            LOW_POOL_ENV,
            env_usize(LOW_POOL_ENV, DEFAULT_LOW_POOL_FLOWS).to_string(),
        )
        .env(
            HIGH_POOL_ENV,
            env_usize(HIGH_POOL_ENV, DEFAULT_HIGH_POOL_FLOWS).to_string(),
        )
        .output()
        .expect("run resource bench child");

    if !output.status.success() {
        panic!(
            "resource bench child failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    parse_child_report(&output)
}

fn run_resource_envelope_child() -> ResourceEnvelopeReport {
    let profile = ResourceProfile::from_env();
    let flows_per_sec = env_u64(RATE_ENV, BENCHMARK_RATES[0]);
    let warmup_secs = env_u64(WARMUP_ENV, DEFAULT_WARMUP_SECS);
    let measurement_secs = env_u64(MEASURE_ENV, DEFAULT_MEASURE_SECS);
    let decoded = collect_decoded_flows(&CARDINALITY_SOURCE_SCENARIO);
    let records = build_cardinality_records(
        &decoded,
        profile.record_pool_size(),
        profile.cardinality_mode(),
    );
    let (_tmp, mut service) = new_disk_benchmark_ingest_service(ConfigDecapsulationMode::None);

    let warmup_result = run_paced_ingest_loop(
        &mut service,
        &records,
        flows_per_sec,
        Duration::from_secs(warmup_secs),
        0,
    );
    let metrics_before = service.metrics.snapshot();
    let proc_before = take_proc_snapshot();
    let started = Instant::now();
    let measurement_result = run_paced_ingest_loop(
        &mut service,
        &records,
        flows_per_sec,
        Duration::from_secs(measurement_secs),
        warmup_result.entries_since_sync,
    );
    let elapsed = started.elapsed();
    let proc_after = take_proc_snapshot();
    let metrics_after = service.metrics.snapshot();
    service.finish_shutdown_for_test(measurement_result.entries_since_sync);

    let logical_bytes = counter_delta(&metrics_before, &metrics_after, "raw_journal_logical_bytes");
    let entries_written =
        counter_delta(&metrics_before, &metrics_after, "journal_entries_written");

    ResourceEnvelopeReport {
        methodology: "post-decode paced mixed-flow ingest benchmark with disk-backed journals"
            .to_string(),
        profile: profile.label().to_string(),
        requested_flows_per_sec: flows_per_sec,
        achieved_flows_per_sec: measurement_result.ingested_flows as f64 / elapsed.as_secs_f64(),
        cpu_percent_of_one_core: cpu_percent_of_one_core(proc_before, proc_after, elapsed),
        logical_write_bytes_per_sec: logical_bytes as f64 / elapsed.as_secs_f64(),
        logical_entries_per_sec: entries_written as f64 / elapsed.as_secs_f64(),
        read_bytes_per_sec: proc_after
            .read_bytes
            .saturating_sub(proc_before.read_bytes) as f64
            / elapsed.as_secs_f64(),
        write_bytes_per_sec: proc_after
            .write_bytes
            .saturating_sub(proc_before.write_bytes) as f64
            / elapsed.as_secs_f64(),
        final_rss_bytes: proc_after.rss_bytes,
        peak_rss_bytes: measurement_result.peak_rss_bytes.max(proc_after.rss_bytes),
        peak_rss_anon_bytes: measurement_result
            .peak_rss_anon_bytes
            .max(proc_after.rss_anon_bytes),
        peak_rss_file_bytes: measurement_result
            .peak_rss_file_bytes
            .max(proc_after.rss_file_bytes),
        warmup_secs,
        measurement_secs,
        record_pool_size: records.len(),
    }
}

fn run_paced_ingest_loop(
    service: &mut IngestService,
    records: &[crate::flow::FlowRecord],
    flows_per_sec: u64,
    duration: Duration,
    initial_entries_since_sync: usize,
) -> PacedLoopResult {
    let total_ticks = duration.as_secs().saturating_mul(TICKS_PER_SECOND);
    let tick_interval = Duration::from_millis(1000 / TICKS_PER_SECOND);
    let mut flow_budget = 0_u64;
    let mut entries_since_sync = initial_entries_since_sync;
    let mut ingested_flows = 0_usize;
    let mut record_index = 0_usize;
    let mut peak = take_proc_snapshot();
    let mut next_sync = Instant::now() + service.cfg.listener.sync_interval;
    let mut next_deadline = Instant::now() + tick_interval;

    for _ in 0..total_ticks {
        flow_budget = flow_budget.saturating_add(flows_per_sec);
        let flows_this_tick = (flow_budget / TICKS_PER_SECOND) as usize;
        flow_budget %= TICKS_PER_SECOND;

        for _ in 0..flows_this_tick {
            let record = &records[record_index % records.len()];
            entries_since_sync = service.handle_decoded_record_for_test(
                RESOURCE_BENCH_RECEIVE_TIME_USEC,
                record,
                entries_since_sync,
            );
            ingested_flows += 1;
            record_index += 1;
        }

        let now = Instant::now();
        while now >= next_sync {
            entries_since_sync = service.handle_sync_tick_for_test(entries_since_sync);
            next_sync += service.cfg.listener.sync_interval;
        }

        let sample = take_proc_snapshot();
        peak.rss_bytes = peak.rss_bytes.max(sample.rss_bytes);
        peak.rss_anon_bytes = peak.rss_anon_bytes.max(sample.rss_anon_bytes);
        peak.rss_file_bytes = peak.rss_file_bytes.max(sample.rss_file_bytes);

        let now = Instant::now();
        if now < next_deadline {
            std::thread::sleep(next_deadline - now);
        }
        next_deadline += tick_interval;
    }

    PacedLoopResult {
        ingested_flows,
        entries_since_sync,
        peak_rss_bytes: peak.rss_bytes,
        peak_rss_anon_bytes: peak.rss_anon_bytes,
        peak_rss_file_bytes: peak.rss_file_bytes,
    }
}

fn counter_delta(
    before: &std::collections::HashMap<String, u64>,
    after: &std::collections::HashMap<String, u64>,
    key: &str,
) -> u64 {
    after
        .get(key)
        .copied()
        .unwrap_or(0)
        .saturating_sub(before.get(key).copied().unwrap_or(0))
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
