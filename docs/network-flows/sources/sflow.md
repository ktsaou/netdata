<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/sources/sflow.md"
sidebar_label: "sFlow"
learn_status: "Published"
learn_rel_path: "Network Flows/Sources"
keywords: ['sflow', 'sflow v5', 'packet sampling', 'switch monitoring']
endmeta-->

# sFlow

sFlow is a packet-sampling protocol commonly found on switches. Aruba/HP, Arista, Brocade, Mellanox, Juniper, and many merchant-silicon platforms speak sFlow natively in hardware. Netdata supports **sFlow version 5**.

sFlow is fundamentally different from NetFlow and IPFIX. Read this section before you read anything else about sFlow on Netdata.

## sFlow is not "flows"

sFlow exporters do not aggregate packets into flow records. They sample. Two independent streams come out of an sFlow agent:

- **Flow samples** — a randomly sampled packet's header (typically the first 128 bytes), tagged with the sampling rate at the time of capture. Roughly: "every Nth packet on this port, here is what it looked like."
- **Counter samples** — a periodic snapshot of interface counters: bytes in/out, packets in/out, errors, queue depths. "Every M seconds on this port, here are the cumulative counters."

The two streams travel in the same UDP datagram opportunistically but they measure different things. They cannot be reconciled byte-for-byte. This is RFC-level behaviour (see [RFC 3176](https://www.rfc-editor.org/rfc/rfc3176)), not a Netdata limitation. If you cross-check sFlow byte counts against SNMP, expect a mismatch — both are correct, but they're different measurements.

## What Netdata does with sFlow datagrams

Netdata processes flow samples and produces a flow record for each. **Counter samples are dropped.** Counter information from sFlow is not surfaced anywhere in the netflow dashboard — for interface-level counters, use Netdata's normal SNMP collector or the network-viewer plugin.

Flow samples become flow records like this:

| sFlow source | Netdata flow-record fields |
|---|---|
| `SampledHeader` (raw frame, default 128 bytes) | After parsing Ethernet / IPv4 / IPv6 / TCP / UDP / ICMP / MPLS: src/dst MAC, src/dst IP, ports, protocol, ToS, TTL, TCP flags, MPLS labels, fragment id/offset, IPv6 flow label |
| `SampledIPv4` / `SampledIPv6` (IP-only summary) | src/dst IP, ports, protocol, IP DSCP |
| `SampledEthernet` (L2 only, no IP) | src/dst MAC |
| `ExtendedSwitch` | src/dst VLAN |
| `ExtendedRouter` | src/dst prefix length, next-hop IP |
| `ExtendedGateway` | src/dst AS, AS path, BGP communities, next-hop |

Each flow sample produces one flow record with `packets = 1` and `bytes = wire length of the sampled packet`. Both are then multiplied by the sampling rate at ingestion, so the dashboard shows estimates of actual traffic.

What is **not** decoded, even when the sFlow agent sends it:

- Counter samples (interface counters)
- `ExtendedUser` (user identity), `ExtendedUrl` (HTTP URL), `ExtendedMpls*` (full MPLS context)
- `ExtendedNat*` (NAT events)
- `Extended80211*` (Wi-Fi-specific)
- VxLAN VNI (`Extended80211*`, tunnel egress/ingress records)
- HTTP/Memcache/application records (sFlow's higher-layer extensions)
- Packet-discard records (`DiscardedPacket`)
- `DiscardedReason` semantics

If you depend on any of these, the data is not available through the netflow plugin today. Open an issue if you need a specific record type added.

## Sampling rate

sFlow always carries a sampling rate, and it travels inside each individual flow sample — not in the datagram header, and not negotiated separately. **Different ports on the same agent can use different sampling rates legitimately** (a 10 Gbps uplink at 1-in-10 000 next to a 1 Gbps access port at 1-in-1 000 is normal sFlow design).

Netdata multiplies bytes and packets by the per-sample rate at ingestion. Raw, unscaled values are available as `RAW_BYTES` and `RAW_PACKETS`.

This is different from NetFlow/IPFIX, where sampling is optional and often global. With sFlow you should expect heterogeneous rates by design.

## Sample header parsing

When the sFlow agent sends a `SampledHeader` (a truncated copy of the original packet), Netdata parses the raw bytes to extract IP and L4 fields. Two important limits:

- **Only Ethernet, IPv4, and IPv6 inner headers are parsed.** sFlow's `HeaderProtocol` enum allows Token Ring, FDDI, X.25, Frame Relay, MPLS-only, IEEE 802.11, and others. None of those are decoded — flow samples wrapping non-Ethernet headers produce a record with most fields empty.
- **IPv6 extension headers are not walked.** A packet with a Hop-by-Hop, Routing, Fragment, AH, ESP, or Destination Options header presents its `next_header` value as the protocol number to the L4 parser, which doesn't recognise those values. The result is `protocol = 0/43/44/50/51/60` and no port fields. If your traffic uses IPv6 extension headers heavily, prefer IPFIX over sFlow for that path.

VLAN tags inside `SampledHeader` are stripped during parsing but **not** copied into `src_vlan`/`dst_vlan` — VLAN IDs come exclusively from `ExtendedSwitch` records. If your switch sends `SampledHeader` with 802.1Q tags but does not also send `ExtendedSwitch`, the VLAN information is lost.

## Configure sFlow on your switch

The Netdata Agent must be reachable from the switch on UDP port 2055 (or whatever you configured). On the switch side, three settings matter:

1. **Sampling rate** — choose carefully. A 10 Gbps interface saturates with too-low sampling and underrepresents itself with too-high sampling. Industry default for 10 Gbps is 1-in-2 000 to 1-in-4 000; for 1 Gbps, 1-in-512 to 1-in-1 000.
2. **Counter polling interval** — even though Netdata drops counter samples, switches still need a healthy polling interval (30 seconds is typical) to avoid stale counters used internally.
3. **Agent address** — the IP the switch claims as its identity in the sFlow datagram. This becomes Netdata's `exporter_ip` if it is set; otherwise the UDP source IP is used. Set it to a stable management IP, not an interface that may flap.

### Arista EOS

```
sflow run
sflow source-interface Loopback0
sflow destination 10.0.0.10 2055
sflow polling-interval 30
sflow sample dangerous 2000
!
interface Ethernet1
   sflow enable
```

`sflow sample dangerous 2000` means 1-in-2 000 packet sampling on every interface. EOS treats values below 16384 as "aggressive" — the `dangerous` keyword is required to opt in. For higher-rate interfaces, omit `dangerous` and use a value at or above 16384 (e.g., `sflow sample 32768`). To configure per-interface rates, place a `sflow sample` line under the interface.

### Juniper JunOS

```
set protocols sflow agent-id 10.0.0.1
set protocols sflow collector 10.0.0.10 udp-port 2055
set protocols sflow polling-interval 30
set protocols sflow sample-rate ingress 2000
set protocols sflow sample-rate egress 2000
set protocols sflow interfaces ge-0/0/0
set protocols sflow interfaces ge-0/0/1
```

### Aruba CX (HP / Aruba data-centre switches)

This is AOS-CX syntax (6200, 6300, 6400, 8100, 8320, 8360, 9300, 10000-series). The legacy AOS-Switch / ProVision platform uses different commands.

```
sflow agent-ip 10.0.0.1
sflow collector 10.0.0.10 port 2055
sflow polling 30
sflow sampling 2000
interface 1/1/1
   sflow
```

In the interface context, the bare `sflow` enables sampling — there is no `sflow enable` form on AOS-CX.

### Ruckus / RUCKUS ICX FastIron (formerly Brocade)

FastIron uses a two-step model: enable globally, then enable forwarding per-interface. The agent IP is auto-derived from the management or routing identity — there is no `sflow source` command.

```
sflow enable
sflow destination 10.0.0.10 2055
sflow polling-interval 30
sflow sample 2000
!
interface ethernet 1/1/1
   sflow forwarding
```

Without `sflow forwarding` on the interface, the global config is inert — no packets get sampled.

### Linux host (`hsflowd`)

For Linux servers and software switches:

```
# /etc/hsflowd.conf
sflow {
  agent.cidr = 10.0.0.0/24
  # sFlow's standard port is 6343; using 2055 to match Netdata's listener.
  collector { ip = 10.0.0.10 udpport = 2055 }
  sampling = 2000
  polling = 30
  pcap { dev = eth0 }
}
```

Then `sudo systemctl restart hsflowd`. The `pcap` section requires `hsflowd` built with libpcap support (the standard distro packages typically include it).

## Agent identity and sub-agents

The sFlow datagram has two identity fields:

- **`agent_address`** — the IP the device claims as its sFlow agent identity. Netdata uses this as `exporter_ip` when it is present.
- **`sub_agent_id`** — a logical identifier for separate sampling engines on the same physical device. **Netdata does not currently use this field.** Multiple sub-agents on one device appear as one exporter in the dashboard.

If you have a chassis with multiple supervisor cards or a switch stack with independent sampling engines, you cannot distinguish them in the dashboard today. Open an issue if you need this.

## Verifying sFlow is arriving

The plugin exposes operational metrics under the `netflow.*` chart family:

- **`netflow.input_packets`** — look at the `sflow` dimension. If it's increasing but you see no flow records on the dashboard, your switch is probably sending only counter samples (Netdata drops those). Check the switch's sFlow configuration and confirm flow sampling is enabled per interface.
- **`netflow.input_bytes`** — UDP byte rate.

Quick sanity check from the agent host:

```bash
sudo tcpdump -i any -n udp port 2055
```

If you see datagrams from your switches but no records appear in the dashboard, the next checks are:

1. Is the switch sending **flow** samples, or only **counter** samples? Verify with the switch's `show sflow` (or vendor equivalent).
2. Is the sampled traffic actually IPv4/IPv6 inside Ethernet? Non-IP traffic produces records with empty L3 fields.
3. Is sampling actually enabled on the interfaces carrying traffic? Some platforms require an explicit per-interface enable.

## Common things that go wrong

- **Counter samples mistaken for flow samples.** "I see sFlow datagrams but no flows" usually means the switch is configured to send only counter polls. The fix is on the switch — enable packet sampling on the interfaces.
- **Sampling rate set too low** ("oversampled"). The switch CPU can't keep up; sFlow itself drops samples on the device. The switch usually logs this; the plugin sees fewer flow samples than expected.
- **Sampling rate set too high** ("undersampled"). Statistical accuracy collapses for short flows. A 1-in-10 000 rate misses 99.99% of single-packet flows entirely. For security monitoring, prefer 1-in-512 to 1-in-2 048 if your CPU allows.
- **Counter samples needed for capacity planning.** They are not surfaced through the netflow plugin. Use SNMP for interface byte counters; flow data answers "who and what", not "how full is the link".
- **Agent IP not configured.** When `agent_address` is `Unknown` (some devices), `exporter_ip` defaults to the UDP source IP, which can change if the device routes via different interfaces. Set the agent IP explicitly.
- **VLAN IDs missing in dashboard.** VLAN data comes from `ExtendedSwitch`, not from 802.1Q tags inside `SampledHeader`. If your switch doesn't send `ExtendedSwitch` records, configure it to do so.
- **Disabling sFlow without disabling the listener.** If you set `protocols.sflow: false` in `netflow.yaml` but a switch is still sending sFlow datagrams, the plugin sees them as malformed NetFlow and increments `parse_errors`. Quiet the source, not just the listener.

## What's next

- [NetFlow](/docs/network-flows/sources/netflow.md) — Aggregated flows, very different from sampling.
- [IPFIX](/docs/network-flows/sources/ipfix.md) — The IETF-standardised flow protocol.
- [Configuration](/docs/network-flows/configuration.md) — Plugin-side configuration: listener, retention, decapsulation, performance tuning.
- [Validation and Data Quality](/docs/network-flows/validation.md) — How to confirm your numbers are right.
- [Anti-patterns](/docs/network-flows/anti-patterns.md) — Mistakes to avoid when reading flow data.
