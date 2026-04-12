use super::test_support::{
    UdpPayload, extract_udp_payloads, fixture_dir, new_benchmark_ingest_service,
};
use super::*;
use crate::decoder::DecodedFlow;
use crate::plugin_config::DecapsulationMode as ConfigDecapsulationMode;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Instant;

const DEFAULT_PROTOCOL_BENCH_ROUNDS: usize = 5_000;
const DEFAULT_PROTOCOL_WARMUP_ROUNDS: usize = 5;
const DEFAULT_CARDINALITY_BATCH_FLOWS: usize = 50_000;
const MEDIUM_CARDINALITY_BUCKETS: u64 = 256;
const CARDINALITY_BENCH_RECEIVE_TIME_USEC: u64 = 1_700_000_000_000_000;

struct ProtocolScenario {
    name: &'static str,
    template_files: &'static [&'static str],
    data_files: &'static [&'static str],
}

const PROTOCOL_SCENARIOS: &[ProtocolScenario] = &[
    ProtocolScenario {
        name: "netflow-v5",
        template_files: &[],
        data_files: &["nfv5.pcap"],
    },
    ProtocolScenario {
        name: "netflow-v9",
        template_files: &["template.pcap"],
        data_files: &["data.pcap"],
    },
    ProtocolScenario {
        name: "ipfix",
        template_files: &["ipfixprobe-templates.pcap"],
        data_files: &["ipfixprobe-data.pcap"],
    },
    ProtocolScenario {
        name: "sflow",
        template_files: &[],
        data_files: &["data-1140.pcap"],
    },
];

const CARDINALITY_SOURCE_SCENARIO: ProtocolScenario = ProtocolScenario {
    name: "mixed",
    template_files: &[
        "template.pcap",
        "ipfixprobe-templates.pcap",
        "icmp-template.pcap",
    ],
    data_files: &[
        "data.pcap",
        "ipfixprobe-data.pcap",
        "nfv5.pcap",
        "data-1140.pcap",
    ],
};

#[derive(Clone, Copy)]
enum CardinalityMode {
    Low,
    Medium,
    High,
}

impl CardinalityMode {
    fn label(self) -> &'static str {
        match self {
            Self::Low => "low-cardinality",
            Self::Medium => "medium-cardinality",
            Self::High => "high-cardinality",
        }
    }

    fn bucket(self, ordinal: u64) -> u64 {
        match self {
            Self::Low => 0,
            Self::Medium => ordinal % MEDIUM_CARDINALITY_BUCKETS,
            Self::High => ordinal,
        }
    }

    fn unique_buckets(self, total_flows: usize) -> u64 {
        match self {
            Self::Low => 1,
            Self::Medium => total_flows.min(MEDIUM_CARDINALITY_BUCKETS as usize) as u64,
            Self::High => total_flows as u64,
        }
    }
}

#[derive(Debug)]
struct ThroughputReport {
    flows: usize,
    packets: usize,
    bytes: usize,
    elapsed: std::time::Duration,
}

impl ThroughputReport {
    fn flows_per_sec(&self) -> f64 {
        self.flows as f64 / self.elapsed.as_secs_f64()
    }

    fn packets_per_sec(&self) -> f64 {
        self.packets as f64 / self.elapsed.as_secs_f64()
    }

    fn bytes_per_sec(&self) -> f64 {
        self.bytes as f64 / self.elapsed.as_secs_f64()
    }

    fn usec_per_flow(&self) -> f64 {
        self.elapsed.as_micros() as f64 / self.flows as f64
    }
}

#[derive(Debug)]
struct ProtocolBenchmarkReport {
    name: &'static str,
    rounds: usize,
    flows_per_round: usize,
    packets_per_round: usize,
    bytes_per_round: usize,
    full_ingest: ThroughputReport,
    decode_only: ThroughputReport,
    post_decode: ThroughputReport,
}

#[derive(Debug)]
struct CardinalityBenchmarkReport {
    label: &'static str,
    unique_buckets: u64,
    ingest: ThroughputReport,
}

#[test]
#[ignore = "manual ingestion performance benchmark"]
fn bench_ingestion_protocol_matrix() {
    let rounds = protocol_bench_rounds();
    let warmup_rounds = protocol_warmup_rounds();

    eprintln!();
    eprintln!("=== Ingestion Protocol Matrix ===");
    eprintln!("Rounds per scenario: {}", rounds);
    eprintln!("Warmup rounds:       {}", warmup_rounds);

    for scenario in PROTOCOL_SCENARIOS {
        let report = benchmark_protocol_scenario(scenario, rounds, warmup_rounds);
        print_protocol_report(&report);
    }
}

#[test]
#[ignore = "manual ingestion performance benchmark"]
fn bench_ingestion_cardinality_matrix() {
    let total_flows = cardinality_batch_flows();
    let warmup_flows = total_flows.min(1_000);
    let decoded = collect_decoded_flows(&CARDINALITY_SOURCE_SCENARIO);
    assert!(
        !decoded.is_empty(),
        "cardinality benchmark requires at least one decoded flow"
    );

    eprintln!();
    eprintln!("=== Ingestion Cardinality Matrix ===");
    eprintln!("Flow batch size:     {}", total_flows);
    eprintln!("Warmup flows:        {}", warmup_flows);

    for mode in [
        CardinalityMode::Low,
        CardinalityMode::Medium,
        CardinalityMode::High,
    ] {
        let report = benchmark_cardinality_mode(mode, &decoded, total_flows, warmup_flows);
        print_cardinality_report(&report);
    }
}

fn benchmark_protocol_scenario(
    scenario: &'static ProtocolScenario,
    rounds: usize,
    warmup_rounds: usize,
) -> ProtocolBenchmarkReport {
    let data_payloads = load_scenario_payloads(scenario);
    let flows_per_round = count_flows_per_round(scenario, &data_payloads);
    let packets_per_round = data_payloads.len();
    let bytes_per_round = total_payload_bytes(&data_payloads);

    let full_ingest = benchmark_protocol_full_ingest(
        scenario,
        &data_payloads,
        rounds,
        warmup_rounds,
        flows_per_round,
    );
    let decode_only = benchmark_protocol_decode_only(scenario, &data_payloads, rounds);
    let post_decode = benchmark_protocol_post_decode(scenario, &data_payloads, rounds);

    ProtocolBenchmarkReport {
        name: scenario.name,
        rounds,
        flows_per_round,
        packets_per_round,
        bytes_per_round,
        full_ingest,
        decode_only,
        post_decode,
    }
}

fn benchmark_protocol_full_ingest(
    scenario: &ProtocolScenario,
    data_payloads: &[UdpPayload],
    rounds: usize,
    warmup_rounds: usize,
    flows_per_round: usize,
) -> ThroughputReport {
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);
    warm_protocol_templates(&mut service, scenario);

    let mut entries_since_sync = 0;
    for _ in 0..warmup_rounds {
        for payload in data_payloads {
            entries_since_sync = service.handle_received_packet_for_test(
                payload.source,
                &payload.data,
                entries_since_sync,
            );
        }
    }

    let started = Instant::now();
    for _ in 0..rounds {
        for payload in data_payloads {
            entries_since_sync = service.handle_received_packet_for_test(
                payload.source,
                &payload.data,
                entries_since_sync,
            );
        }
    }
    let elapsed = started.elapsed();
    service.finish_shutdown_for_test(entries_since_sync);

    ThroughputReport {
        flows: rounds * flows_per_round,
        packets: rounds * data_payloads.len(),
        bytes: rounds * total_payload_bytes(data_payloads),
        elapsed,
    }
}

fn benchmark_protocol_decode_only(
    scenario: &ProtocolScenario,
    data_payloads: &[UdpPayload],
    rounds: usize,
) -> ThroughputReport {
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);
    warm_protocol_templates(&mut service, scenario);

    let started = Instant::now();
    let mut flows = 0_usize;
    for _ in 0..rounds {
        for payload in data_payloads {
            let receive_time_usec = super::now_usec();
            let batch = service
                .decoders
                .decode_udp_payload_at(payload.source, &payload.data, receive_time_usec);
            flows += batch.flows.len();
        }
    }
    let elapsed = started.elapsed();

    ThroughputReport {
        flows,
        packets: rounds * data_payloads.len(),
        bytes: rounds * total_payload_bytes(data_payloads),
        elapsed,
    }
}

fn benchmark_protocol_post_decode(
    scenario: &ProtocolScenario,
    data_payloads: &[UdpPayload],
    rounds: usize,
) -> ThroughputReport {
    let decoded = collect_decoded_flows_for_scenario(scenario, data_payloads);
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);
    let started = Instant::now();
    let mut entries_since_sync = 0_usize;

    for _ in 0..rounds {
        for flow in &decoded {
            if service.ingest_decoded_record_for_test(
                CARDINALITY_BENCH_RECEIVE_TIME_USEC,
                &flow.record,
            ) {
                entries_since_sync += 1;
            }
        }
    }

    let elapsed = started.elapsed();
    service.finish_shutdown_for_test(entries_since_sync);

    ThroughputReport {
        flows: rounds * decoded.len(),
        packets: 0,
        bytes: 0,
        elapsed,
    }
}

fn benchmark_cardinality_mode(
    mode: CardinalityMode,
    decoded: &[DecodedFlow],
    total_flows: usize,
    warmup_flows: usize,
) -> CardinalityBenchmarkReport {
    let records = build_cardinality_records(decoded, total_flows, mode);
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);

    for record in records.iter().take(warmup_flows) {
        service.ingest_decoded_record_for_test(CARDINALITY_BENCH_RECEIVE_TIME_USEC, record);
    }

    let started = Instant::now();
    let mut entries_since_sync = 0_usize;
    for record in &records {
        if service.ingest_decoded_record_for_test(CARDINALITY_BENCH_RECEIVE_TIME_USEC, record) {
            entries_since_sync += 1;
        }
    }
    let elapsed = started.elapsed();
    service.finish_shutdown_for_test(entries_since_sync);

    CardinalityBenchmarkReport {
        label: mode.label(),
        unique_buckets: mode.unique_buckets(total_flows),
        ingest: ThroughputReport {
            flows: records.len(),
            packets: 0,
            bytes: 0,
            elapsed,
        },
    }
}

fn load_scenario_payloads(scenario: &ProtocolScenario) -> Vec<UdpPayload> {
    let base = fixture_dir();
    let mut payloads = Vec::new();
    for file in scenario.data_files {
        payloads.extend(extract_udp_payloads(&base.join(file)));
    }
    assert!(
        !payloads.is_empty(),
        "scenario {} did not produce any UDP payloads",
        scenario.name
    );
    payloads
}

fn warm_protocol_templates(service: &mut IngestService, scenario: &ProtocolScenario) {
    let base = fixture_dir();
    for file in scenario.template_files {
        for payload in extract_udp_payloads(&base.join(file)) {
            service.decoders.decode_udp_payload(payload.source, &payload.data);
        }
    }
}

fn count_flows_per_round(scenario: &ProtocolScenario, data_payloads: &[UdpPayload]) -> usize {
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);
    warm_protocol_templates(&mut service, scenario);
    count_flows_with_service(&mut service, data_payloads)
}

fn count_flows_with_service(service: &mut IngestService, data_payloads: &[UdpPayload]) -> usize {
    let mut flows = 0_usize;
    for payload in data_payloads {
        let batch = service
            .decoders
            .decode_udp_payload(payload.source, &payload.data);
        flows += batch.flows.len();
    }
    flows
}

fn collect_decoded_flows_for_scenario(
    scenario: &ProtocolScenario,
    data_payloads: &[UdpPayload],
) -> Vec<DecodedFlow> {
    let (_tmp, mut service) = new_benchmark_ingest_service(ConfigDecapsulationMode::None);
    warm_protocol_templates(&mut service, scenario);
    let mut decoded = Vec::new();
    for payload in data_payloads {
        let batch = service
            .decoders
            .decode_udp_payload(payload.source, &payload.data);
        decoded.extend(batch.flows);
    }
    decoded
}

fn collect_decoded_flows(scenario: &ProtocolScenario) -> Vec<DecodedFlow> {
    let data_payloads = load_scenario_payloads(scenario);
    collect_decoded_flows_for_scenario(scenario, &data_payloads)
}

fn build_cardinality_records(
    decoded: &[DecodedFlow],
    total_flows: usize,
    mode: CardinalityMode,
) -> Vec<crate::flow::FlowRecord> {
    let mut records = Vec::with_capacity(total_flows);
    for ordinal in 0..total_flows {
        let mut record = decoded[ordinal % decoded.len()].record.clone();
        mutate_record_for_cardinality(&mut record, mode.bucket(ordinal as u64));
        records.push(record);
    }
    records
}

fn mutate_record_for_cardinality(record: &mut crate::flow::FlowRecord, bucket: u64) {
    record.exporter_ip = Some(mutate_ip(record.exporter_ip, bucket, 11));
    record.src_addr = Some(mutate_ip(record.src_addr, bucket, 31));
    record.dst_addr = Some(mutate_ip(record.dst_addr, bucket, 73));
    record.src_port = mutate_port(bucket, 10_000);
    record.dst_port = mutate_port(bucket.wrapping_mul(7), 20_000);
    record.in_if = (bucket % 4096) as u32;
    record.out_if = ((bucket.wrapping_mul(3)) % 4096) as u32;
    record.src_as = 64_512 + (bucket % 4096) as u32;
    record.dst_as = 65_000 + (bucket % 4096) as u32;
    record.exporter_name = format!("exp-{bucket}");
    record.src_as_name = format!("src-as-{bucket}");
    record.dst_as_name = format!("dst-as-{bucket}");
}

fn mutate_ip(original: Option<IpAddr>, bucket: u64, salt: u8) -> IpAddr {
    match original.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)) {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::new(
            10 + (salt % 10),
            ((bucket >> 16) & 0xff) as u8,
            ((bucket >> 8) & 0xff) as u8,
            (bucket & 0xff) as u8,
        )),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::new(
            0x2001,
            0xdb8,
            salt as u16,
            ((bucket >> 32) & 0xffff) as u16,
            ((bucket >> 16) & 0xffff) as u16,
            (bucket & 0xffff) as u16,
            0,
            salt as u16,
        )),
    }
}

fn mutate_port(bucket: u64, base: u16) -> u16 {
    base.saturating_add((bucket % 30_000) as u16)
}

fn total_payload_bytes(payloads: &[UdpPayload]) -> usize {
    payloads.iter().map(|payload| payload.data.len()).sum()
}

fn protocol_bench_rounds() -> usize {
    std::env::var("NETFLOW_INGEST_BENCH_ROUNDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PROTOCOL_BENCH_ROUNDS)
}

fn protocol_warmup_rounds() -> usize {
    std::env::var("NETFLOW_INGEST_BENCH_WARMUP_ROUNDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PROTOCOL_WARMUP_ROUNDS)
}

fn cardinality_batch_flows() -> usize {
    std::env::var("NETFLOW_INGEST_CARDINALITY_BATCH_FLOWS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CARDINALITY_BATCH_FLOWS)
}

fn print_protocol_report(report: &ProtocolBenchmarkReport) {
    eprintln!();
    eprintln!("Scenario:               {}", report.name);
    eprintln!("  rounds:               {}", report.rounds);
    eprintln!("  flows/round:          {}", report.flows_per_round);
    eprintln!("  packets/round:        {}", report.packets_per_round);
    eprintln!("  bytes/round:          {}", report.bytes_per_round);
    eprintln!(
        "  full ingest:          {:.0} flows/s | {:.0} packets/s | {:.2} µs/flow | {:.0} bytes/s",
        report.full_ingest.flows_per_sec(),
        report.full_ingest.packets_per_sec(),
        report.full_ingest.usec_per_flow(),
        report.full_ingest.bytes_per_sec()
    );
    eprintln!(
        "  decode only:          {:.0} flows/s | {:.0} packets/s | {:.2} µs/flow | {:.0} bytes/s",
        report.decode_only.flows_per_sec(),
        report.decode_only.packets_per_sec(),
        report.decode_only.usec_per_flow(),
        report.decode_only.bytes_per_sec()
    );
    eprintln!(
        "  post-decode ingest:   {:.0} flows/s | {:.2} µs/flow",
        report.post_decode.flows_per_sec(),
        report.post_decode.usec_per_flow()
    );
}

fn print_cardinality_report(report: &CardinalityBenchmarkReport) {
    eprintln!();
    eprintln!("Scenario:               {}", report.label);
    eprintln!("  unique buckets:       {}", report.unique_buckets);
    eprintln!(
        "  post-decode ingest:   {:.0} flows/s | {:.2} µs/flow",
        report.ingest.flows_per_sec(),
        report.ingest.usec_per_flow()
    );
}
