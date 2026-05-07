<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/sources/ipfix.md"
sidebar_label: "IPFIX"
learn_status: "Published"
learn_rel_path: "Network Flows/Sources"
keywords: ['ipfix', 'rfc 7011', 'flow export', 'biflow', 'router configuration']
endmeta-->

# IPFIX

IPFIX (IP Flow Information eXport) is the IETF-standardised flow-export protocol defined in [RFC 7011](https://www.rfc-editor.org/rfc/rfc7011). It is the successor to NetFlow v9 — sometimes called "NetFlow v10" — and is what most modern enterprise routers, firewalls, and switches use when they speak a flow protocol that isn't sFlow.

If your gear supports both NetFlow v9 and IPFIX, prefer IPFIX. The protocol is standard, the IE registry is broader, biflow is defined explicitly, and template handling is more rigorous. The Netdata netflow plugin handles both, but IPFIX gives you more.

IPFIX runs on UDP. Netdata uses a single socket (default `0.0.0.0:2055`) and identifies IPFIX by the version word in each datagram, so you do not need a separate port from NetFlow.

## What IPFIX gives you

IPFIX records carry a wider set of standardised fields than NetFlow v9. Of the IANA IPFIX Information Element registry's hundreds of entries, Netdata recognises the operationally important ones:

| Field family | Notes |
|---|---|
| Source/destination IPv4 and IPv6 | IE 8/12 (v4), IE 27/28 (v6) |
| Ports, protocol, TCP flags | IE 7/11, 4, 6 |
| Bytes and packets | IE 1, 2, plus the postOctet/postPacket variants |
| Initiator/responder counters (biflow) | IE 231/232 (octets), IE 298/299 (packets) |
| Input/output interface | IE 10/14, plus the *physical* variants 252/253 |
| Source/destination AS | IE 16/17 |
| MAC addresses | IE 56/80, plus post-MAC IE 81/57 |
| VLAN IDs | IE 58/59 (and 243/254 dot1qVlanId variants) |
| Forwarding status | IE 89 |
| Sampling rate | IE 34, 50, plus options-template path with IE 305/306 |
| Flow direction | IE 61 (`flowDirection`), IE 239 (`biflowDirection`) |
| ICMP type/code | IE 32/176-179, IE 139 (combined v6) |
| MPLS labels | IE 70-79 (top label and stack) |
| Post-NAT addresses and ports | IE 225/226/281/282 (addresses), IE 227/228 (ports) |
| Fragment id/offset | IE 54/88 |
| IPv6 flow label | IE 31 |
| TTL | IE 52, 192 |
| Data Link Frame Section (for decapsulation) | IE 315 |

Anything outside this set parses correctly (the field length is honoured) but the value is dropped. Vendor enterprise IEs are recognised only for one specific case — see [Vendor extensions](#vendor-extensions) below.

## Configure IPFIX on your router

The Netdata Agent must be reachable from the router on UDP port 2055 (or whatever you configured). Three settings matter on the router side:

1. **Active timeout** — keep it at 60 seconds. Vendor defaults are often 30 minutes, which delays visibility.
2. **Template refresh** — set it to 60 seconds or less. Frequent refresh means a collector restart recovers within a minute.
3. **Sampling rate** — if you sample, document the rate, and keep it consistent across exporters that share a dashboard.

### Cisco IOS-XE (IPFIX)

```
flow exporter NETDATA
 destination 10.0.0.10
 source Loopback0
 transport udp 2055
 export-protocol ipfix
 template data timeout 60
!
flow record NETDATA-RECORD
 match ipv4 source address
 match ipv4 destination address
 match transport source-port
 match transport destination-port
 match ipv4 protocol
 match interface input
 collect interface output
 collect counter bytes
 collect counter packets
 collect timestamp absolute first
 collect timestamp absolute last
!
flow monitor NETDATA-MONITOR
 record NETDATA-RECORD
 exporter NETDATA
 cache timeout active 60
 cache timeout inactive 15
!
interface GigabitEthernet0/0/1
 ip flow monitor NETDATA-MONITOR input
 ip flow monitor NETDATA-MONITOR output
```

The IPFIX standard port is UDP 4739; this example uses 2055 to match Netdata's listener. Either works as long as the plugin and the exporter agree.

Apply the monitor in both directions. This produces two flow records for every packet that traverses the router (ingress + egress) — see the [Overview](/docs/network-flows/) for why aggregate traffic looks doubled if you do not filter.

This snippet is **IOS-XE only**. IOS-XR uses different syntax (`flow exporter-map`, `flow monitor-map`, named samplers); see the [NetFlow page's IOS-XR example](/docs/network-flows/sources/netflow.md#cisco-ios-xr) and substitute `version v9` with `version ipfix`.

### Cisco ASA (IPFIX, NSEL)

```
flow-export destination inside 10.0.0.10 2055 udp
flow-export template timeout-rate 1
flow-export delay flow-create 60
flow-export active refresh-interval 1

policy-map global_policy
 class class-default
  flow-export event-type all destination 10.0.0.10
service-policy global_policy global
```

`inside` is the ASA's `nameif` for the interface where the collector is reachable — substitute your actual nameif (typically `inside`, `outside`, `dmz`, etc.). The ASA's NSEL events are an IPFIX dialect with stateful firewall semantics (flow-create/flow-teardown rather than periodic snapshots). Netdata treats them as standard IPFIX.

### Juniper JunOS (IPFIX, inline-jflow on MX)

Same supporting structure as the [J-Flow v9 example](/docs/network-flows/sources/netflow.md#juniper-junos-j-flow-v9-inline-jflow-on-mx) — the sampling instance, FPC binding, and firewall filter are all required:

```
set forwarding-options sampling instance NETDATA input rate 1
set forwarding-options sampling instance NETDATA family inet output flow-server 10.0.0.10 port 2055
set forwarding-options sampling instance NETDATA family inet output flow-server 10.0.0.10 version-ipfix template ipv4-ipfix
set forwarding-options sampling instance NETDATA family inet output inline-jflow source-address 10.0.0.1
set services flow-monitoring version-ipfix template ipv4-ipfix flow-active-timeout 60
set services flow-monitoring version-ipfix template ipv4-ipfix flow-inactive-timeout 15
set services flow-monitoring version-ipfix template ipv4-ipfix template-refresh-rate seconds 60
set chassis fpc 0 sampling-instance NETDATA
set firewall filter SAMPLE-NETDATA term MATCH-ALL then sample accept
set interfaces ge-0/0/1 unit 0 family inet filter input SAMPLE-NETDATA
set interfaces ge-0/0/1 unit 0 family inet sampling input
set interfaces ge-0/0/1 unit 0 family inet sampling output
```

Adjust `chassis fpc 0` to match the FPC slot of your interface. IPFIX support varies by platform: QFX10002 needs JunOS 17.2R1+, SRX needs 19.4R1+, NFX150/250/350 needs 20.4R1+. PTX uses a different `flow-aggregation` hierarchy.

### Arista EOS (IPFIX)

EOS uses milliseconds in the timeout commands (the documented form is `record export on interval` and `record export on inactive timeout`):

```
flow tracking hardware
 tracker NETDATA
   record export on interval 60000
   record export on inactive timeout 15000
   exporter NETDATA-EXPORTER
      local interface Loopback0
      collector 10.0.0.10 port 2055
      template interval 60000
      format ipfix version 10
      no shutdown
   no shutdown
!
interface Ethernet1
   flow tracker hardware NETDATA
```

Times above are in **milliseconds** (60000 ms = 60 s, 15000 ms = 15 s). Apply the tracker on every interface you want to monitor; EOS does not separate ingress and egress at the interface level — the tracker observes both directions.

### nProbe (Linux, IPFIX, commercial)

nProbe is commercial software from ntop. The flag set is available in the evaluation binary; production use needs a license.

```bash
sudo nprobe -i eth0 -n 10.0.0.10:2055 -V 10 -t 60 -d 30
# Add -E <license-key> when running in production.
```

`-t 60` is the active timeout, `-d 30` the idle timeout (both in seconds).

### pmacct (Linux, IPFIX, open-source)

The nfprobe plugin runs under the `pmacctd` daemon (packet sniffer mode):

```
# /etc/pmacct/pmacctd.conf
plugins: nfprobe
nfprobe_version: 10
nfprobe_receiver: 10.0.0.10:2055
nfprobe_timeouts: tcp=30:maxlife=60
pcap_interface: eth0
```

Run with `pmacctd -f /etc/pmacct/pmacctd.conf`. Note that pmacct's timeout keys are `tcp` (TCP timeout in seconds) and `maxlife` (maximum flow lifetime in seconds), not `idle`/`active`.

## Biflow (RFC 5103)

IPFIX defines biflow — a single record that carries both directions of a conversation. Reverse-direction fields use the `reverseInformationElement` Private Enterprise Number 29305 (e.g., `reverseOctetDeltaCount`, `reverseTcpControlBits`).

Netdata handles biflow records by emitting **two flow records** — a forward record and a reverse record — so downstream queries see one entry per direction. The reverse record is built by swapping source/destination fields, copying reverse-direction counters into the standard counters, then storing it with the same flow timestamps.

Two consequences:

- **Aggregations by 5-tuple show two entries per biflow record.** This is the same behaviour you get from a non-biflow exporter that emits ingress + egress records.
- **A biflow record with zero reverse counters produces only the forward record.** Netdata drops the reverse half if both bytes and packets are zero. So a partially populated biflow exporter can look like a normal one-direction exporter for some of its flows.

There is no biflow auto-detection. The decoder reacts per-record to the presence of reverse fields. This means you do not need to configure anything to enable biflow handling; if your exporter sends reverse IEs, they are processed.

## Sampling

Same priority chain as NetFlow v9, with two extra IPFIX-only inputs:

1. **Per-record IEs**: `samplingInterval` (IE 34), `samplerRandomInterval` (IE 50).
2. **Per-record packet-interval pair**: `samplingPacketInterval` (IE 305) and `samplingPacketSpace` (IE 306) — Netdata derives the rate as `(interval + space) / interval`.
3. **Sampling Options Templates**, scoped by `samplerId` (IE 48) or `selectorId` (IE 302). Cached per `(exporter IP, observation domain, sampler ID)`.
4. **Default of 1** if none of the above is present.

Bytes and packets are multiplied by the resolved rate at ingestion. Raw, unscaled values remain available as `RAW_BYTES` and `RAW_PACKETS`.

What Netdata does **not** read:

- `selectorAlgorithm` (IE 304) — the plugin treats systematic sampling and random sampling identically.
- The PSAMP "monitored selector" model from RFC 5476 (`selectorIdTotalPktsObserved` IE 318 and friends).

For the broader sampling discussion — when scaling is correct and when it is misleading — see [Validation and Data Quality](/docs/network-flows/validation.md) and the sampling section of the [NetFlow page](/docs/network-flows/sources/netflow.md#sampling).

## Vendor extensions

Most enterprise IEs (Cisco AVC, Cisco NEL/HSL NAT events, Cisco NBAR, vendor-specific application classifiers) are parsed correctly — meaning their length is honoured and decoding continues — but the values are discarded. Netdata recognises only one enterprise IE today:

- **Juniper PEN 2636 `commonPropertiesId`** (IE 137 under PEN 2636), used to surface forwarding-status information when the value's high bits indicate the akvorado-style status mapping.

If you depend on a Cisco AVC application name in flow records or on Cisco NSEL NAT-event metadata, expect those fields to be empty. Open an issue with sample fixtures if you need them mapped.

## Template withdrawal

RFC 7011 § 8 lets an exporter explicitly withdraw a template (graceful retirement). **Netdata does not act on template withdrawals.** A withdrawal arrives as a Template Set with zero fields and is silently skipped. The cached template stays in memory until either:

- An updated persisted-state file replaces the namespace on a future restart, or
- A new template arrives with the same ID and overwrites the cached one.

In practice this is harmless: stale templates are passive cache entries. They consume memory but they are not actively wrong.

## Variable-length fields

IPFIX supports fields with a variable length, encoded with a 1-byte or 3-byte length prefix. Netdata's decoder honours this format. Today it matters for `dataLinkFrameSection` (IE 315), used for decapsulation. Other variable-length fields (URLs, hostnames) parse correctly but are not in Netdata's canonical mapping, so their values are dropped.

## Verifying flow data is arriving

The plugin exposes its own operational metrics on the standard Netdata charts page, under the `netflow.*` family:

- **`netflow.input_packets`** — datagrams seen, parsed, and counted. Look at the `ipfix` dimension. Also watch `template_errors` — non-zero means data records arrived before their templates.
- **`netflow.input_bytes`** — UDP byte rate.
- **`netflow.decoder_scopes`** — `(exporter, observation domain)` template scopes held in memory.

If `ipfix` packets per second is non-zero but `template_errors` is also climbing, your exporter is sending data faster than templates. Lower the template refresh interval (target: 30-60 seconds).

## Common things that go wrong

- **Template refresh too slow.** Vendor defaults are often 30 minutes. After a plugin restart, you'll see template errors for that long unless the exporter refreshes sooner.
- **Sampling rate undocumented.** Without knowing the rate, the dashboard volume numbers cannot be validated. See [Validation](/docs/network-flows/validation.md).
- **NAT in front of multiple exporters.** They share a public IP and overwrite each other's template cache. Place the plugin inside the NAT boundary, or give each exporter a distinct routable address.
- **Enterprise IEs you depend on aren't surfaced.** If your dashboard expects Cisco AVC application names or similar vendor-specific data, those fields will be empty. The plugin only maps the standardised IEs and Juniper PEN 2636.
- **Exporter advertises biflow but only populates reverse fields on some flows.** You'll see two records per biflow record on flows that are populated, and one record per biflow record on flows that aren't. This is expected behaviour but can look inconsistent.

## What's next

- [NetFlow](/docs/network-flows/sources/netflow.md) — The predecessor protocol family. Many of the same concepts apply.
- [sFlow](/docs/network-flows/sources/sflow.md) — A different protocol with different semantics — packet samples, not aggregated flows.
- [Configuration](/docs/network-flows/configuration.md) — Plugin-side configuration: listener, retention, decapsulation, performance tuning.
- [Validation and Data Quality](/docs/network-flows/validation.md) — How to confirm your numbers are right.
- [Anti-patterns](/docs/network-flows/anti-patterns.md) — Mistakes to avoid when reading flow data.
