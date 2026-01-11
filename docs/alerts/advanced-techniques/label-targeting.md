# 8.3 Host, Chart, and Label-Based Targeting

## 8.3.1 Understanding Labels

Netdata supports:
- **Host labels**: Set in `netdata.conf` under `[host labels]` or auto-detected (e.g., `_os=linux`)
- **Chart labels**: Set by collectors for specific charts (e.g., `mount_point=/data`)

## 8.3.2 Label-Based Alert Scope

Use the `host labels` or `chart labels` line to filter which nodes/charts an alert applies to:

```conf
template: db_cpu_high
      on: system.cpu
  lookup: average -5m of user,system
   every: 1m
    warn: $this > 80
    crit: $this > 95
host labels: role=database env=production
```

This alert only applies to nodes that have **both** `role=database` and `env=production` labels.

```conf
template: disk_space_critical
      on: disk.space
  lookup: average -1m percentage of avail
   every: 1m
    warn: $this < 20
    crit: $this < 10
chart labels: mount_point=!/dev !/dev/* !/run !/run/* *
```

This alert applies to all mount points except `/dev` and `/run` paths.

## 8.3.3 Related Sections

- **3.6 Optional Metadata** for labels documentation
- **10.4 Room-Based Alerting** for Cloud label usage