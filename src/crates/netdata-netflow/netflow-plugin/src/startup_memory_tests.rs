use super::{facet_runtime, ingest, plugin_config, query, tiering};
use bytesize::ByteSize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_PROFILE_BASE_DIR: &str = "/var/cache/netdata/flows";
const PROFILE_DIR_ENV: &str = "NETFLOW_PROFILE_BASE_DIR";
const PROFILE_WORKER_THREADS_ENV: &str = "NETFLOW_PROFILE_WORKER_THREADS";
const PROFILE_BLOCKING_THREADS_ENV: &str = "NETFLOW_PROFILE_MAX_BLOCKING_THREADS";
const PROFILE_SETTLE_SECS_ENV: &str = "NETFLOW_PROFILE_SETTLE_SECS";
const PROFILE_DISABLE_THP_ENV: &str = "NETFLOW_PROFILE_DISABLE_THP";
const DEFAULT_PROFILE_WORKER_THREADS: usize = 4;
const DEFAULT_PROFILE_BLOCKING_THREADS: usize = 8;
const DEFAULT_PROFILE_SETTLE_SECS: u64 = 12;

#[derive(Debug, Clone, Copy, Default)]
struct ProcessMemorySnapshot {
    rss_bytes: u64,
    hwm_bytes: u64,
    thread_count: usize,
    allocator: crate::memory_allocator::AllocatorMemorySample,
}

#[derive(Debug)]
struct StartupMemoryPhase {
    name: &'static str,
    snapshot: ProcessMemorySnapshot,
}

#[derive(Debug)]
struct StartupMemoryReport {
    phases: Vec<StartupMemoryPhase>,
    facet_breakdown: crate::facet_runtime::FacetMemoryBreakdown,
    journal_files: usize,
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "manual production-data startup profiling harness"]
async fn stress_profile_live_startup_memory() {
    let report = profile_live_startup_memory()
        .await
        .expect("profile live startup memory");
    print_report("current_thread", None, None, &report);
    assert_report_grew_rss(&report);
}

#[test]
#[ignore = "manual production-data startup profiling harness with multi-thread tokio runtime"]
fn stress_profile_live_startup_memory_multithreaded() {
    let worker_threads = profile_worker_threads();
    let max_blocking_threads = profile_blocking_threads();
    let disable_thp = profile_disable_thp();
    if disable_thp {
        let _ = crate::memory_allocator::disable_transparent_huge_pages_for_process();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .max_blocking_threads(max_blocking_threads)
        .build()
        .expect("build profiling runtime");
    let report = runtime
        .block_on(profile_live_startup_memory())
        .expect("profile live startup memory");
    print_report(
        "multi_thread",
        Some(worker_threads),
        Some(max_blocking_threads),
        &report,
    );
    assert_report_grew_rss(&report);
}

fn print_report(
    runtime_flavor: &str,
    worker_threads: Option<usize>,
    max_blocking_threads: Option<usize>,
    report: &StartupMemoryReport,
) {
    eprintln!(
        "startup memory profile: runtime={} workers={} max_blocking={} base_dir={} journal_files={}",
        runtime_flavor,
        worker_threads
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        max_blocking_threads
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        profile_base_dir().display(),
        report.journal_files,
    );

    let baseline = report
        .phases
        .first()
        .map(|phase| phase.snapshot)
        .unwrap_or_default();
    for phase in &report.phases {
        eprintln!(
            "phase={} rss={} hwm={} threads={} heap_in_use={} heap_free={} heap_arena={} mmap_in_use={} delta_rss={}",
            phase.name,
            phase.snapshot.rss_bytes,
            phase.snapshot.hwm_bytes,
            phase.snapshot.thread_count,
            phase.snapshot.allocator.heap_in_use_bytes,
            phase.snapshot.allocator.heap_free_bytes,
            phase.snapshot.allocator.heap_arena_bytes,
            phase.snapshot.allocator.mmap_in_use_bytes,
            phase.snapshot.rss_bytes.saturating_sub(baseline.rss_bytes),
        );
    }

    eprintln!(
        "facet_breakdown={{archived:{} active:{} active_contrib:{} published:{} archived_paths:{}}}",
        report.facet_breakdown.archived_bytes,
        report.facet_breakdown.active_bytes,
        report.facet_breakdown.active_contributions_bytes,
        report.facet_breakdown.published_bytes,
        report.facet_breakdown.archived_path_bytes,
    );
}

fn assert_report_grew_rss(report: &StartupMemoryReport) {
    let baseline = report
        .phases
        .first()
        .map(|phase| phase.snapshot)
        .unwrap_or_default();
    assert!(
        report
            .phases
            .last()
            .map(|phase| phase.snapshot.rss_bytes)
            .unwrap_or_default()
            > baseline.rss_bytes,
        "expected startup profiling to grow RSS",
    );
}

async fn profile_live_startup_memory() -> anyhow::Result<StartupMemoryReport> {
    let base_dir = profile_base_dir();
    anyhow::ensure!(
        base_dir.join("raw").is_dir(),
        "profile base dir {} is missing raw tier",
        base_dir.display()
    );

    let cfg = profile_config(&base_dir);
    let journal_files = journal_file_count(&base_dir);
    let mut phases = Vec::new();
    phases.push(StartupMemoryPhase {
        name: "baseline",
        snapshot: current_process_memory()?,
    });

    let facet_runtime = Arc::new(facet_runtime::FacetRuntime::new(&base_dir));
    phases.push(StartupMemoryPhase {
        name: "facet_runtime_new",
        snapshot: current_process_memory()?,
    });

    let (query_service, _notify_rx) =
        query::FlowQueryService::new_with_facet_runtime(&cfg, Arc::clone(&facet_runtime)).await?;
    phases.push(StartupMemoryPhase {
        name: "query_service_new",
        snapshot: current_process_memory()?,
    });

    query_service.initialize_facets().await?;
    phases.push(StartupMemoryPhase {
        name: "initialize_facets",
        snapshot: current_process_memory()?,
    });

    let metrics = Arc::new(ingest::IngestMetrics::default());
    let open_tiers = Arc::new(std::sync::RwLock::new(tiering::OpenTierState::default()));
    let tier_flow_indexes = Arc::new(std::sync::RwLock::new(tiering::TierFlowIndexStore::default()));
    let mut ingest_service = ingest::IngestService::new_with_facet_runtime(
        cfg.clone(),
        metrics,
        open_tiers,
        tier_flow_indexes,
        Arc::clone(&facet_runtime),
    )?;
    phases.push(StartupMemoryPhase {
        name: "ingest_service_new",
        snapshot: current_process_memory()?,
    });

    ingest_service.rebuild_materialized_from_raw_for_test().await?;
    phases.push(StartupMemoryPhase {
        name: "rebuild_materialized_from_raw",
        snapshot: current_process_memory()?,
    });

    let settle_secs = profile_settle_secs();
    if settle_secs > 0 {
        tokio::time::sleep(std::time::Duration::from_secs(settle_secs)).await;
        phases.push(StartupMemoryPhase {
            name: "rebuild_settle",
            snapshot: current_process_memory()?,
        });

        let _ = crate::memory_allocator::trim_allocator_if_worthwhile();
        phases.push(StartupMemoryPhase {
            name: "rebuild_settle_trim",
            snapshot: current_process_memory()?,
        });
    }

    let facet_breakdown = facet_runtime.estimated_memory_breakdown();
    Ok(StartupMemoryReport {
        phases,
        facet_breakdown,
        journal_files,
    })
}

fn profile_config(base_dir: &Path) -> plugin_config::PluginConfig {
    let mut cfg = plugin_config::PluginConfig::default();
    cfg.journal.journal_dir = base_dir.to_string_lossy().to_string();
    cfg.listener.listen = "127.0.0.1:0".to_string();
    cfg.listener.sync_every_entries = 1024;
    cfg.listener.sync_interval = std::time::Duration::from_secs(1);
    cfg.journal.size_of_journal_file = ByteSize::mb(256);
    cfg.journal.number_of_journal_files = 64;
    cfg.journal.size_of_journal_files = ByteSize::gb(10);
    cfg.journal.duration_of_journal_files = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    cfg
}

fn profile_base_dir() -> PathBuf {
    env::var_os(PROFILE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PROFILE_BASE_DIR))
}

fn current_process_memory() -> anyhow::Result<ProcessMemorySnapshot> {
    let status = fs::read_to_string("/proc/self/status")?;
    let mut snapshot = ProcessMemorySnapshot::default();

    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            snapshot.rss_bytes = parse_status_kib(value)?;
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            snapshot.hwm_bytes = parse_status_kib(value)?;
        }
    }

    snapshot.thread_count = current_thread_count()?;
    snapshot.allocator = crate::memory_allocator::current_allocator_memory();
    Ok(snapshot)
}

fn parse_status_kib(raw: &str) -> anyhow::Result<u64> {
    let numeric = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing numeric status value"))?;
    let kib = numeric.parse::<u64>()?;
    Ok(kib.saturating_mul(1024))
}

fn current_thread_count() -> anyhow::Result<usize> {
    Ok(fs::read_dir("/proc/self/task")?.count())
}

fn profile_worker_threads() -> usize {
    env::var(PROFILE_WORKER_THREADS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PROFILE_WORKER_THREADS)
}

fn profile_blocking_threads() -> usize {
    env::var(PROFILE_BLOCKING_THREADS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PROFILE_BLOCKING_THREADS)
}

fn profile_settle_secs() -> u64 {
    env::var(PROFILE_SETTLE_SECS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PROFILE_SETTLE_SECS)
}

fn profile_disable_thp() -> bool {
    env::var(PROFILE_DISABLE_THP_ENV)
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn journal_file_count(path: &Path) -> usize {
    fn count(path: &Path) -> usize {
        fs::read_dir(path)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .map(|entry_path| {
                if entry_path.is_dir() {
                    return count(&entry_path);
                }

                usize::from(entry_path.extension().and_then(|ext| ext.to_str()) == Some("journal"))
            })
            .sum()
    }

    count(path)
}
