use super::*;
use crate::plugin_config::DecapsulationMode as ConfigDecapsulationMode;
use etherparse::{NetSlice, SlicedPacket, TransportSlice};
use pcap_file::pcap::PcapReader;
use std::fs::File;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub(super) fn new_test_ingest_service(
    decapsulation_mode: ConfigDecapsulationMode,
) -> (TempDir, IngestService) {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let service = new_test_ingest_service_in_dir(tmp.path(), decapsulation_mode);
    (tmp, service)
}

pub(super) fn new_benchmark_ingest_service(
    decapsulation_mode: ConfigDecapsulationMode,
) -> (TempDir, IngestService) {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let mut cfg = PluginConfig::default();
    cfg.journal.journal_dir = tmp.path().join("flows").to_string_lossy().to_string();
    cfg.protocols.decapsulation_mode = decapsulation_mode;
    cfg.listener.sync_every_entries = usize::MAX;
    cfg.listener.sync_interval = Duration::from_secs(60 * 60);

    for dir in cfg.journal.all_tier_dirs() {
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("create tier directory {}: {e}", dir.display()));
    }

    let service = IngestService::new(
        cfg,
        Arc::new(IngestMetrics::default()),
        Arc::new(RwLock::new(OpenTierState::default())),
        Arc::new(RwLock::new(TierFlowIndexStore::default())),
    )
    .expect("create ingest benchmark service");

    (tmp, service)
}

pub(super) fn new_disk_benchmark_ingest_service(
    decapsulation_mode: ConfigDecapsulationMode,
) -> (TempDir, IngestService) {
    let base = std::env::current_dir()
        .expect("resolve current dir")
        .join("src/crates/target/netflow-resource-bench");
    std::fs::create_dir_all(&base)
        .unwrap_or_else(|e| panic!("create disk benchmark root {}: {e}", base.display()));

    let tmp = tempfile::Builder::new()
        .prefix("resource-bench-")
        .tempdir_in(&base)
        .unwrap_or_else(|e| panic!("create disk benchmark temp dir {}: {e}", base.display()));
    let mut cfg = PluginConfig::default();
    cfg.journal.journal_dir = tmp.path().join("flows").to_string_lossy().to_string();
    cfg.protocols.decapsulation_mode = decapsulation_mode;
    cfg.listener.sync_every_entries = usize::MAX;
    cfg.listener.sync_interval = Duration::from_secs(60 * 60);

    for dir in cfg.journal.all_tier_dirs() {
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("create tier directory {}: {e}", dir.display()));
    }

    let service = IngestService::new(
        cfg,
        Arc::new(IngestMetrics::default()),
        Arc::new(RwLock::new(OpenTierState::default())),
        Arc::new(RwLock::new(TierFlowIndexStore::default())),
    )
    .expect("create disk-backed ingest benchmark service");

    (tmp, service)
}

pub(super) fn new_test_ingest_service_in_dir(
    base_dir: &Path,
    decapsulation_mode: ConfigDecapsulationMode,
) -> IngestService {
    let mut cfg = PluginConfig::default();
    cfg.journal.journal_dir = base_dir.join("flows").to_string_lossy().to_string();
    cfg.protocols.decapsulation_mode = decapsulation_mode;

    for dir in cfg.journal.all_tier_dirs() {
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("create tier directory {}: {e}", dir.display()));
    }

    IngestService::new(
        cfg,
        Arc::new(IngestMetrics::default()),
        Arc::new(RwLock::new(OpenTierState::default())),
        Arc::new(RwLock::new(TierFlowIndexStore::default())),
    )
    .expect("create ingest service")
}

pub(super) fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/flows")
}

pub(super) fn decode_fixture_sequence(
    service: &mut IngestService,
    fixtures: &[&str],
) -> Vec<crate::decoder::FlowFields> {
    let base = fixture_dir();
    let mut out = Vec::new();
    for fixture in fixtures {
        out.extend(decode_pcap_flows(&base.join(fixture), service));
    }
    out
}

fn decode_pcap_flows(path: &Path, service: &mut IngestService) -> Vec<crate::decoder::FlowFields> {
    let file = File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut reader =
        PcapReader::new(file).unwrap_or_else(|e| panic!("pcap reader {}: {e}", path.display()));

    let mut flows = Vec::new();
    while let Some(packet) = reader.next_packet() {
        let packet = packet.unwrap_or_else(|e| panic!("read packet {}: {e}", path.display()));
        if let Some((source, payload)) = extract_udp_payload(packet.data.as_ref()) {
            service.prepare_decoder_state_namespace(source, payload);
            let decoded = service.decoders.decode_udp_payload(source, payload);
            flows.extend(decoded.flows.into_iter().map(|flow| flow.record.to_fields()));
        }
    }
    flows
}

pub(super) fn extract_udp_payload(packet: &[u8]) -> Option<(SocketAddr, &[u8])> {
    let sliced = SlicedPacket::from_ethernet(packet).ok()?;
    let src_ip = match sliced.net {
        Some(NetSlice::Ipv4(v4)) => IpAddr::V4(v4.header().source_addr()),
        Some(NetSlice::Ipv6(v6)) => IpAddr::V6(v6.header().source_addr()),
        _ => return None,
    };

    let (src_port, payload) = match sliced.transport {
        Some(TransportSlice::Udp(udp)) => (udp.source_port(), udp.payload()),
        _ => return None,
    };
    Some((SocketAddr::new(src_ip, src_port), payload))
}

pub(super) fn find_flow<'a>(
    flows: &'a [crate::decoder::FlowFields],
    predicates: &[(&str, &str)],
) -> &'a crate::decoder::FlowFields {
    flows
        .iter()
        .find(|fields| {
            predicates
                .iter()
                .all(|(k, v)| fields.get(*k).map(String::as_str) == Some(*v))
        })
        .unwrap_or_else(|| {
            panic!(
                "flow not found for predicates {:?}; decoded flow count={}",
                predicates,
                flows.len()
            )
        })
}

pub(super) struct UdpPayload {
    pub(super) source: SocketAddr,
    pub(super) data: Vec<u8>,
}

pub(super) fn extract_udp_payloads(path: &Path) -> Vec<UdpPayload> {
    let file = File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut reader =
        PcapReader::new(file).unwrap_or_else(|e| panic!("pcap reader {}: {e}", path.display()));
    let mut payloads = Vec::new();
    while let Some(packet) = reader.next_packet() {
        let packet = packet.unwrap_or_else(|e| panic!("read packet {}: {e}", path.display()));
        if let Some((source, payload)) = extract_udp_payload(packet.data.as_ref()) {
            payloads.push(UdpPayload {
                source,
                data: payload.to_vec(),
            });
        }
    }
    payloads
}
