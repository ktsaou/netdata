<!--startmeta
custom_edit_url: "https://github.com/netdata/netdata/edit/master/docs/network-flows/sources/netflow.md"
sidebar_label: "NetFlow"
learn_status: "Published"
learn_rel_path: "Network Flows/Sources"
keywords: ['netflow', 'netflow v5', 'netflow v9', 'flow export', 'router configuration']
endmeta-->

# NetFlow

NetFlow is Cisco's flow-export protocol. Most routers and switches can export NetFlow, even outside the Cisco ecosystem. Netdata decodes three versions:

- **NetFlow v5** — fixed format, IPv4 only, simple. Found on legacy gear and in some software exporters (`softflowd`, `nProbe`).
- **NetFlow v7** — Cisco Catalyst 5000 family. Same shape as v5 minus the sampling field.
- **NetFlow v9** — template-based, IPv6-capable, extensible. The version most modern routers use when they say "NetFlow".

All three run over UDP. Netdata listens on a single socket for everything (default `0.0.0.0:2055`) and identifies each datagram by its version word, so you don't need separate ports for v5 vs v9.

If your gear supports both NetFlow v9 and IPFIX, prefer IPFIX. IPFIX is the IETF-standardised successor (essentially "NetFlow v10") and supports more field types. See the [IPFIX page](/docs/network-flows/sources/ipfix.md). Only stay on v9 when your platform doesn't speak IPFIX.

## What you get from NetFlow

The fields populated in Netdata's flow records depend on the version and on what your router actually exports.

| Field family | v5 | v7 | v9 |
|---|---|---|---|
| Source/destination IPv4 | yes | yes | yes |
| Source/destination IPv6 | no | no | yes (IE 27/28) |
| Ports, protocol, TCP flags | yes | yes | yes |
| Bytes, packets | yes | yes | yes |
| Input/output interface index | yes | yes | yes |
| Source/destination AS | yes | yes | yes |
| MAC addresses | no | no | yes (IE 56/57/80/81) |
| VLAN IDs | no | no | yes (IE 58/59) |
| MPLS labels, NAT, ICMP type/code | no | no | yes |
| Sampling rate | yes (header) | no | yes (data field 34/50, or option template) |
| First/last switched timestamps | yes | yes | yes |

NetFlow v9's exact field set is up to your exporter — it sends a template describing the format, and the plugin maps the supported Information Elements to its 91-field flow record. Vendor extensions outside Netdata's mapping table are silently dropped.

## Configure NetFlow on your router

The Netdata Agent must be reachable from the router on UDP port 2055 (or whatever you configured). On the router side, three things matter:

1. **Active timeout** — how long a long-lived flow can stay in the router's cache before it is exported. Cisco's default is 30 minutes; that hides traffic for 30 minutes. **Set it to 60 seconds.** This is the industry best practice for enterprise networks and keeps the dashboard close to real-time without overwhelming the exporter.
2. **Template refresh interval (v9 only)** — how often the router re-sends templates to the collector. Set this to 60 seconds or less. Frequent refresh means a collector restart recovers within a minute.
3. **Sampling** — if the link is faster than your router can fully observe, configure sampling explicitly. Document the rate. Do not change it without telling whoever uses the data.

### Cisco IOS / IOS-XE (Flexible NetFlow, v9)

```
flow exporter NETDATA
 destination 10.0.0.10
 source GigabitEthernet0/0/0
 transport udp 2055
 export-protocol netflow-v9
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
 collect timestamp sys-uptime first
 collect timestamp sys-uptime last
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

Apply the monitor in both directions on each interface you care about. That is the standard configuration. It produces two flow records for every packet that traverses the router (one ingress, one egress) — this is normal, and it is the reason aggregate traffic appears doubled when you do not filter by interface and direction. See the [Overview](/docs/network-flows/) for why.

### Cisco IOS-XR

IOS-XR requires a named sampler-map at the interface, even when you don't actually want to sample. Define a `1 out-of 1` sampler that captures every packet:

```
sampler-map NETDATA-SAMPLER
 random 1 out-of 1
!
flow exporter-map NETDATA
 destination 10.0.0.10
 source Loopback0
 transport udp 2055
 version v9
  template data timeout 60
!
flow monitor-map NETDATA
 record ipv4
 exporter NETDATA
 cache timeout active 60
 cache timeout inactive 15
!
interface GigabitEthernet0/0/0/1
 flow ipv4 monitor NETDATA sampler NETDATA-SAMPLER ingress
 flow ipv4 monitor NETDATA sampler NETDATA-SAMPLER egress
```

The `sampler` argument is mandatory at the interface level — IOS-XR has no equivalent of "off". Use `random 1 out-of 1` to get every packet observed; raise the divisor (`100`, `1000`, etc.) to actually sample.

### Juniper JunOS (J-Flow v9, inline-jflow on MX)

Inline-jflow is supported on MX (except MX80/MX104), QFX10002+, and recent EVO platforms. The configuration needs the sampling instance, the FPC binding, a firewall filter on the interface, and the interface sampling statements:

```
set forwarding-options sampling instance NETDATA input rate 1
set forwarding-options sampling instance NETDATA family inet output flow-server 10.0.0.10 port 2055
set forwarding-options sampling instance NETDATA family inet output flow-server 10.0.0.10 version9 template ipv4-template
set forwarding-options sampling instance NETDATA family inet output inline-jflow source-address 10.0.0.1
set services flow-monitoring version9 template ipv4-template flow-active-timeout 60
set services flow-monitoring version9 template ipv4-template flow-inactive-timeout 15
set services flow-monitoring version9 template ipv4-template template-refresh-rate seconds 60
set chassis fpc 0 sampling-instance NETDATA
set firewall filter SAMPLE-NETDATA term MATCH-ALL then sample accept
set interfaces ge-0/0/1 unit 0 family inet filter input SAMPLE-NETDATA
set interfaces ge-0/0/1 unit 0 family inet sampling input
set interfaces ge-0/0/1 unit 0 family inet sampling output
```

Adjust `chassis fpc 0 sampling-instance NETDATA` to match the FPC slot of your interface. The firewall filter is what actually triggers sampling on the data path; the interface `sampling input/output` statements alone are not enough on most FPC types.

### FortiGate (NetFlow v9)

The configuration shape changed in FortiOS 7.2.8 / 7.4.2 — the collector keys moved from the top level into a nested `config collectors` block, and timeout units shifted from minutes to seconds. The example below is for current FortiOS:

```
config system netflow
    set active-flow-timeout 1800
    set inactive-flow-timeout 15
    set template-tx-timeout 1800
    config collectors
        edit 1
            set collector-ip 10.0.0.10
            set collector-port 2055
            set source-ip 10.0.0.1
            set interface-select-method auto
        next
    end
end
config system interface
    edit "port1"
        set netflow-sampler both
    next
end
```

For older FortiOS (7.2.7 and earlier, 7.4.0-7.4.1) the collector fields go at the top level and timeouts are in minutes:

```
config system netflow
    set collector-ip 10.0.0.10
    set collector-port 2055
    set source-ip 10.0.0.1
    set active-flow-timeout 30           # minutes on older FortiOS
    set inactive-flow-timeout 15         # seconds on older FortiOS
    set template-tx-timeout 30           # minutes on older FortiOS
end
```

FortiOS does not expose a NetFlow version selector; it always speaks NetFlow v9.

### Arista EOS

Arista's `flow tracking hardware` always exports IPFIX (version 10), not classic NetFlow v9. If you specifically need NetFlow v9 from an Arista switch, the netflow plugin will accept whatever the device sends — but on Arista, the recommended path is to use the [IPFIX configuration](/docs/network-flows/sources/ipfix.md#arista-eos-ipfix). Netdata decodes both equivalently.

### Linux host as an exporter (`softflowd`, NetFlow v9)

For Linux servers, hypervisors, or anything that doesn't run NetFlow natively:

```bash
sudo softflowd -i eth0 -n 10.0.0.10:2055 -v 9 -t maxlife=60 -t expint=15
```

`softflowd` watches the kernel interface and synthesises flow records from the packet stream.

## What happens with each version

**NetFlow v5** is the simplest case. The router sends fixed-format records. Netdata reads the sampling rate from the header (low 14 bits — the high 2 bits encode the sampling mode, which Netdata discards), multiplies bytes and packets by that rate, and stores the flow.

**NetFlow v7** is rare in modern networks. The header has no sampling field at all. If you enable sampling on a v7 exporter, Netdata cannot learn the rate and the bytes/packets numbers will be the raw sampled values — undercounted by the sampling factor. **Avoid v7 if you sample.**

**NetFlow v9** is template-driven. Your router first sends a template (a small descriptor saying "here is what fields I will send and in what order"), then sends data records that match that template. Netdata caches the templates and decodes the data. If a data record arrives before the template it references (collector started after the router, or the template UDP datagram was dropped), Netdata cannot decode it; those datagrams are counted as "template errors" and dropped until the next template refresh.

## Sampling

Netdata multiplies bytes and packets by the sampling rate at ingestion. The numbers you see in the dashboard are scaled estimates of the actual traffic.

How Netdata learns the rate, in priority order:

1. **Per-record fields**, if the v9 template includes them: `samplingInterval` (IE 34) or `flowSamplerRandomInterval` (IE 50).
2. **Sampling Options**, sent by the router as separate option records: keyed by `flowSamplerId` (IE 48) and the observation domain. Netdata caches these, and applies them to flows that reference the same sampler.
3. **Header rate** for NetFlow v5.
4. **Default of 1** if none of the above is present (no scaling).

This works correctly when:

- Every exporter uses the same sampling rate, or
- Your exporters all send sampling metadata correctly, or
- All your exporters are unsampled.

This is **misleading** when sampling rates differ across exporters and the dashboard aggregates them. The result is a blend of estimates with no single rate. The clean path: pick a uniform rate across your network, or run unsampled where flow rates allow.

The raw, unscaled values are still available in the flow records as `RAW_BYTES` and `RAW_PACKETS` if you need to compute your own scaling.

### Caveats specific to each version

- **v5 sampling field of 0** is treated as rate 1. Some Cisco IOS-XE versions send sampled v5 with the rate field zeroed out — those flows are stored without scaling. Cross-check against SNMP after enabling.
- **v5 sampling mode** (deterministic vs. random, encoded in the high 2 bits of the field) is discarded. Netdata does not surface the mode.
- **v7 has no sampling field**. Sampling on v7 silently undercounts.
- **v9 sampling options arriving after data records**: data records that arrived before the option are stored unscaled. Once the option arrives, future records pick up the rate; past records are not retroactively corrected.

## Templates and what survives a restart

When you restart `netflow-plugin`, you do not lose templates. The plugin persists decoder state to the cache directory (typically `/var/cache/netdata/flows`) every 30 seconds and at shutdown, and reloads on startup. Restart recovery is roughly:

1. Plugin starts, loads persisted templates into memory.
2. Resumes UDP listening immediately.
3. The first datagram from each known exporter re-applies the persisted templates.
4. Decoding continues with no template-error spike.

You **do** lose templates if:

- The cache directory is wiped between runs.
- The exporter changed its template ID assignments while the plugin was down (rare, but happens after firmware upgrades).
- An exporter sits behind NAT and shares a public IP with another exporter — Netdata keys templates on `(exporter IP, observation domain)`, so two exporters NATted behind the same address will overwrite each other's templates.

Templates have **no time-based expiry** in the cache. An exporter that frequently rotates template IDs will grow the in-memory cache without bound. If you run many exporters with unstable template behaviour, monitor the `decoder_scopes` chart described below.

## Verifying flow data is arriving

The plugin exposes its own operational metrics on the standard Netdata charts page, under the `netflow.*` family:

- **`netflow.input_packets`** — datagrams seen, parsed, and counted per-version (`netflow_v5`, `netflow_v7`, `netflow_v9`, `ipfix`, `sflow`). The most useful chart for "is anything arriving at all".
- **`netflow.input_bytes`** — UDP byte rate.
- **`netflow.decoder_scopes`** — number of distinct `(exporter, observation domain)` template scopes held in memory. Effectively the v9/IPFIX template cache size. Watch this for unbounded growth.

If `netflow_v9` packets per second is non-zero but `template_errors` is also climbing, your router is sending data faster than templates. Lower the template refresh interval on the exporter (target: 30-60 seconds).

If `netflow_v9` is zero but `udp_received` is positive, templates may not be arriving at all — verify the router's exporter configuration and that no firewall is dropping the template-bearing datagrams (they are usually small and infrequent).

## Common things that go wrong

- **Active timeout too long.** Default Cisco is 30 minutes. The dashboard will lag by up to 30 minutes. Fix: `cache timeout active 60`.
- **Template refresh too slow.** Default may be 30 minutes. After a plugin restart, you'll see template errors for that long if the cache is gone. Fix: `template data timeout 60`.
- **Both `input` and `output` monitors enabled** (the standard configuration). This is correct and gives you full visibility, but produces two flow records per packet — aggregate volume looks doubled. Read the [Anti-patterns page](/docs/network-flows/anti-patterns.md) for the framing.
- **Sampling rate undocumented.** If nobody knows the rate, the numbers in the dashboard cannot be interpreted with confidence. See [Validation and Data Quality](/docs/network-flows/validation.md).
- **Datagrams larger than `max_packet_size`** (default 9216 bytes). The kernel truncates silently and the plugin's parser fails on the truncated datagram. Increase the plugin's `max_packet_size` or reduce the exporter's payload size.
- **NAT in front of multiple exporters.** They share an IP and overwrite each other's templates. Place the plugin inside the NAT boundary, or give each exporter a distinct routable address.

## What's next

- [IPFIX](/docs/network-flows/sources/ipfix.md) — The standardised successor to NetFlow v9. Prefer it when both are available.
- [sFlow](/docs/network-flows/sources/sflow.md) — A different protocol with different semantics — packet samples, not aggregated flows.
- [Configuration](/docs/network-flows/configuration.md) — Plugin-side configuration: listener, retention, decapsulation, performance tuning.
- [Validation and Data Quality](/docs/network-flows/validation.md) — How to confirm your numbers are right.
- [Anti-patterns](/docs/network-flows/anti-patterns.md) — Mistakes to avoid when reading flow data.
