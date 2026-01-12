# 6.1 Core System Alerts

This section shows examples based on the actual stock alerts shipped with Netdata.
These alerts use hysteresis to prevent flapping (different thresholds for raising
and clearing alerts based on current status).

## 6.1.1 CPU Utilization Alert

Based on stock alert in `src/health/health.d/cpu.conf`:

```yaml
template: 10min_cpu_usage
      on: system.cpu
   class: Utilization
    type: System
component: CPU
host labels: _os=linux
  lookup: average -10m unaligned of user,system,softirq,irq,guest
   units: %
   every: 1m
    warn: $this > (($status >= $WARNING)  ? (75) : (85))
    crit: $this > (($status == $CRITICAL) ? (85) : (95))
   delay: down 15m multiplier 1.5 max 1h
 summary: System CPU utilization
    info: Average CPU utilization over the last 10 minutes (excluding iowait, nice and steal)
      to: sysadmin
```

**Key features:**
- Uses 10-minute average to avoid transient spikes
- Hysteresis: warn triggers at 85%, clears at 75%; crit triggers at 95%, clears at 85%
- Delay prevents rapid notification cycling during recovery
- OS-specific variants exist for Windows and FreeBSD

## 6.1.2 Memory Pressure Alert

Based on stock alert in `src/health/health.d/ram.conf`:

```yaml
alarm: ram_in_use
    on: system.ram
 class: Utilization
  type: System
component: Memory
host labels: _os=linux
  calc: $used * 100 / ($used + $cached + $free + $buffers)
 units: %
 every: 10s
  warn: $this > (($status >= $WARNING)  ? (80) : (90))
  crit: $this > (($status == $CRITICAL) ? (90) : (98))
 delay: down 15m multiplier 1.5 max 1h
summary: System memory utilization
  info: System memory utilization
    to: sysadmin
```

**Key features:**
- Uses `calc` (not `lookup`) to compute percentage from current dimension values
- Hysteresis: warn triggers at 90%, clears at 80%; crit triggers at 98%, clears at 90%
- Checked every 10 seconds for rapid response to memory pressure
- A separate `ram_available` alert monitors available memory percentage (silent by default)

## 6.1.3 Disk Space Alert

Based on stock alert in `src/health/health.d/disks.conf`:

```yaml
template: disk_space_usage
      on: disk.space
   class: Utilization
    type: System
component: Disk
chart labels: mount_point=!/dev !/dev/* !/run !/run/* !HarddiskVolume* *
    calc: $used * 100 / ($avail + $used)
   units: %
   every: 1m
    warn: $this > (($status >= $WARNING ) ? (80) : (90))
    crit: ($this > (($status == $CRITICAL) ? (90) : (98))) && $avail < 5
   delay: up 1m down 15m multiplier 1.5 max 1h
 summary: Disk ${label:mount_point} space usage
    info: Total space utilization of disk ${label:mount_point}
      to: sysadmin
```

**Key features:**
- Uses `calc` to compute usage percentage
- Excludes system mount points (/dev, /run, HarddiskVolume) via chart labels
- Critical requires both high usage AND less than 5GB available
- A companion `out_of_disk_space_time` alert predicts when disk will fill

## 6.1.4 Network Packet Drops Alert

Based on stock alert in `src/health/health.d/net.conf`:

```yaml
template: inbound_packets_dropped_ratio
      on: net.drops
   class: Errors
    type: System
component: Network
chart labels: device=!wl* *
  lookup: sum -10m unaligned absolute of inbound
    calc: (($net_interface_inbound_packets > 10000) ? ($this * 100 / $net_interface_inbound_packets) : (0))
   units: %
   every: 1m
    warn: $this >= 2
   delay: up 1m down 1h multiplier 1.5 max 2h
 summary: System network interface ${label:device} inbound drops
    info: Ratio of inbound dropped packets for the network interface ${label:device} over the last 10 minutes
      to: silent
```

**Key features:**
- Monitors drop ratio (percentage), not absolute count
- Only alerts when traffic is significant (>10000 packets in 10 min)
- Excludes wireless interfaces (`!wl*`) which have separate alerts with higher thresholds
- Additional alerts exist for FIFO errors, traffic overflow, and packet storms

## 6.1.5 Related Sections

- **6.2 Service and Availability** - Service health alerts
- **6.3 Application Alerts** - Database and web server alerts