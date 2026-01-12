# 6.5 Trend and Capacity Alerts

Trend-based alerts predict future problems by calculating rates of change. Instead of alerting when a threshold is crossed, they alert when the system is heading toward a problem.

## 6.5.1 Disk Fill Rate and Time Remaining

Netdata's stock `disks.conf` demonstrates the canonical pattern for capacity alerts using a two-template approach:

**Template 1: Calculate the fill rate**

```conf
# From src/health/health.d/disks.conf
template: disk_fill_rate
      on: disk.space
  lookup: min -10m at -50m unaligned of avail
    calc: ($this - $avail) / (($now - $after) / 3600)
   units: GB/hour
   every: 1m
    info: average rate the disk fills up (positive), or frees up (negative) space
```

**How it works:**
- `lookup: min -10m at -50m unaligned of avail` retrieves the minimum available space from 50-60 minutes ago (stored in `$this`)
- `$avail` is the current available space
- `($this - $avail)` calculates how much space was consumed
- `(($now - $after) / 3600)` converts the time difference to hours
- Result: GB consumed per hour (positive = filling, negative = freeing)

**Template 2: Calculate hours until full**

```conf
# From src/health/health.d/disks.conf
template: out_of_disk_space_time
      on: disk.space
    calc: ($disk_fill_rate > 0) ? ($avail / $disk_fill_rate) : (inf)
   units: hours
   every: 10s
    warn: $this > 0 and $this < (($status >= $WARNING)  ? (48) : (8))
    crit: $this > 0 and $this < (($status == $CRITICAL) ? (24) : (2))
   delay: down 15m multiplier 1.2 max 1h
 summary: Disk ${label:mount_point} estimation of lack of space
    info: Estimated time the disk ${label:mount_point} will run out of space
      to: silent
```

**How it works:**
- References `$disk_fill_rate` from the previous template
- If fill rate is positive (disk filling), calculates `$avail / $disk_fill_rate` for hours remaining
- If fill rate is zero or negative (disk not filling), returns `inf` (infinity)
- Uses hysteresis: WARNING triggers at 8 hours, clears at 48 hours

:::tip Key Pattern
Split rate calculations and time-remaining alerts into separate templates. This makes each template simpler and allows the rate to be reused by multiple alerts.
:::

## 6.5.2 Packet Storm Detection (Rate Spike)

The stock `net.conf` shows how to detect sudden rate changes by comparing short-term averages to longer baselines:

**Template 1: Baseline rate**

```conf
# From src/health/health.d/net.conf
template: 1m_received_packets_rate
      on: net.packets
   class: Workload
    type: System
component: Network
  lookup: average -1m unaligned of received
   units: packets
   every: 10s
    info: Average number of packets received over the last minute
```

**Template 2: Spike detection**

```conf
# From src/health/health.d/net.conf
template: 10s_received_packets_storm
      on: net.packets
   class: Workload
    type: System
component: Network
  lookup: average -10s unaligned of received
    calc: $this * 100 / (($1m_received_packets_rate < 1000) ? (1000) : ($1m_received_packets_rate))
   units: %
   every: 10s
    warn: $this > (($status >= $WARNING) ? (200) : (5000))
    crit: $this > (($status == $CRITICAL) ? (5000) : (6000))
 options: no-clear-notification
 summary: System network interface ${label:device} inbound packet storm
    info: Ratio of 10-second average to 1-minute average packet rate
      to: silent
```

**How it works:**
- Compares 10-second average to 1-minute baseline
- Division-by-zero protection: uses minimum of 1000 packets/sec as baseline
- Result is a percentage: 100% = same as baseline, 500% = 5x baseline
- Triggers at 50x (5000%) the baseline, with hysteresis dropping to 2x (200%) to clear

## 6.5.3 Inode Fill Rate

Similar to disk space, inode exhaustion can be predicted:

```conf
# From src/health/health.d/disks.conf
template: disk_inode_rate
      on: disk.inodes
  lookup: min -10m at -50m unaligned of avail
    calc: ($this - $avail) / (($now - $after) / 3600)
   units: inodes/hour
   every: 1m
    info: average rate at which disk inodes are allocated (positive), or freed (negative)

template: out_of_disk_inodes_time
      on: disk.inodes
    calc: ($disk_inode_rate > 0) ? ($avail / $disk_inode_rate) : (inf)
   units: hours
   every: 10s
    warn: $this > 0 and $this < (($status >= $WARNING)  ? (48) : (8))
    crit: $this > 0 and $this < (($status == $CRITICAL) ? (24) : (2))
   delay: down 15m multiplier 1.2 max 1h
 summary: Disk ${label:mount_point} estimation of lack of inodes
    info: Estimated time the disk ${label:mount_point} will run out of inodes
      to: silent
```

## 6.5.4 Key Techniques

| Technique | Pattern | Example |
|-----------|---------|---------|
| **Historical lookup** | `lookup: ... at -Xm` | Get value from X minutes ago |
| **Rate calculation** | `($historical - $current) / time` | Change per unit time |
| **Time remaining** | `$available / $rate` | When resource exhausts |
| **Division protection** | `($rate > 0) ? (calc) : (inf)` | Avoid divide-by-zero |
| **Spike detection** | `$short_window / $long_window * 100` | Percentage of baseline |

## 6.5.5 Related Sections

- **3.3 Calculations and Transformations** - Rate of change calculation patterns
- **3.5 Variables and Special Symbols** - `$now`, `$after`, and time-based variables
- **8.1 Hysteresis and Status-Based Conditions** - Preventing flapping on trend alerts