# 11.1 System Resource Alerts

System resource alerts cover the fundamental building blocks of any server: CPU, memory, disk, and network. These alerts apply to every Netdata node and provide baseline monitoring.

## CPU Alerts

### 10min_cpu_usage

The primary CPU alert tracks aggregate CPU usage across all cores. Uses hysteresis to prevent flapping.

**Context:** `system.cpu`
**Thresholds:**
- WARNING: > 85% (stays in WARNING until drops below 75%)
- CRITICAL: > 95% (stays in CRITICAL until drops below 85%)

```conf
   template: 10min_cpu_usage
         on: system.cpu
     lookup: average -10m unaligned of user,system,softirq,irq,guest
      units: %
       warn: $this > (($status >= $WARNING)  ? (75) : (85))
       crit: $this > (($status == $CRITICAL) ? (85) : (95))
```

### 10min_cpu_iowait

Monitors time spent waiting for I/O. High iowait indicates disk bottlenecks affecting CPU scheduling.

**Context:** `system.cpu`
**Thresholds:** WARNING > 40% (with 20% recovery)

### 20min_steal_cpu

For virtual machines, monitors CPU cycles stolen by the hypervisor. Indicates overcommitted host resources.

**Context:** `system.cpu`
**Thresholds:** WARNING > 10% (with 5% recovery)

## Memory Alerts

### ram_in_use

Tracks system memory utilization. Accounts for buffers and cached memory that's reclaimable.

**Context:** `system.ram`
**Thresholds:**
- WARNING: > 90% (stays in WARNING until drops below 80%)
- CRITICAL: > 98% (stays in CRITICAL until drops below 90%)

```conf
      alarm: ram_in_use
         on: system.ram
       calc: $used * 100 / ($used + $cached + $free + $buffers)
      units: %
       warn: $this > (($status >= $WARNING)  ? (80) : (90))
       crit: $this > (($status == $CRITICAL) ? (90) : (98))
```

### ram_available

Monitors the percentage of RAM available for userspace processes without causing swapping.

**Context:** `mem.available`
**Thresholds:** WARNING < 10% (with 15% recovery)

### oom_kill

Counts Out-of-Memory killer events. Any OOM kill indicates severe memory pressure.

**Context:** `mem.oom_kill`
**Thresholds:** WARNING > 0 kills in 30 minutes

## Disk Space Alerts

### disk_space_usage

Monitors disk space utilization for all mounted filesystems (excluding /dev, /run, and similar).

**Context:** `disk.space`
**Thresholds:**
- WARNING: > 90% (stays in WARNING until drops below 80%)
- CRITICAL: > 98% (stays in CRITICAL until drops below 90%) AND available < 5GB

```conf
    template: disk_space_usage
          on: disk.space
chart labels: mount_point=!/dev !/dev/* !/run !/run/* !HarddiskVolume* *
        calc: $used * 100 / ($avail + $used)
       units: %
        warn: $this > (($status >= $WARNING ) ? (80) : (90))
        crit: ($this > (($status == $CRITICAL) ? (90) : (98))) && $avail < 5
```

### disk_inode_usage

For filesystems with many small files, tracks inode exhaustion which can occur before space exhaustion.

**Context:** `disk.inodes`
**Thresholds:**
- WARNING: > 90% (stays in WARNING until drops below 80%)
- CRITICAL: > 98% (stays in CRITICAL until drops below 90%)

### out_of_disk_space_time

Predicts when disk will run out of space based on current fill rate. Provides early warning for capacity planning.

**Context:** `disk.space`
**Thresholds:**
- WARNING: < 8 hours (stays until > 48 hours)
- CRITICAL: < 2 hours (stays until > 24 hours)

## Disk I/O Alerts

### 10min_disk_utilization

Measures the percentage of time the disk spends processing I/O requests. High utilization indicates the disk is a bottleneck.

**Context:** `disk.util`
**Thresholds:** WARNING > 98% (stays in WARNING until drops below 68.6%)

### 10min_disk_backlog

Monitors disk I/O backlog (queued operations). High backlog indicates the disk cannot keep up with demand.

**Context:** `disk.backlog`
**Thresholds:** WARNING > 5000ms (stays in WARNING until drops below 3500ms)

## Network Interface Alerts

### 1m_received_traffic_overflow / 1m_sent_traffic_overflow

Monitors bandwidth utilization as a percentage of interface speed. **Linux and Windows only.**

**Context:** `net.net`
**Thresholds:** WARNING > 90% (with 85% recovery)

### inbound_packets_dropped_ratio / outbound_packets_dropped_ratio

Tracks the ratio of dropped packets to total packets. Excludes wireless interfaces which have separate thresholds.

**Context:** `net.drops`
**Thresholds:** WARNING >= 2% drop rate

### interface_inbound_errors / interface_outbound_errors

Monitors interface errors which may indicate cable problems or hardware degradation. **FreeBSD only.**

**Context:** `net.errors`
**Thresholds:** WARNING >= 5 errors in 10 minutes

### 10min_fifo_errors

Monitors FIFO buffer errors indicating kernel buffer overflow on the interface. **Linux only.**

**Context:** `net.fifo`
**Thresholds:** WARNING > 0 errors

### 10s_received_packets_storm

Detects sudden packet storms by comparing short-term to long-term packet rates.

**Context:** `net.packets`
**Thresholds:**
- WARNING: 10-second rate > 5000% of 1-minute average (stays in WARNING until drops below 200%)
- CRITICAL: 10-second rate > 6000% of 1-minute average (stays in CRITICAL until drops below 5000%)

## Related Files

Stock alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/cpu.conf`
- `/usr/lib/netdata/conf.d/health.d/ram.conf`
- `/usr/lib/netdata/conf.d/health.d/disks.conf`
- `/usr/lib/netdata/conf.d/health.d/net.conf`

To customize thresholds, copy to `/etc/netdata/health.d/` and modify.
