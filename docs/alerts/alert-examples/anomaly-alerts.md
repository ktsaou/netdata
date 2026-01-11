# 6.4 Anomaly-Based Alerts

Netdata's Machine Learning engine calculates anomaly rates for metrics. You can create alerts based on these rates.

## 6.4.1 Node-Level Anomaly Rate Alert

Alert when the overall node anomaly rate exceeds a threshold:

```conf
template: node_anomaly_rate
      on: anomaly_detection.anomaly_rate
  lookup: average -1m of anomaly_rate
   units: %
   every: 30s
    warn: $this > 1
    info: Rolling 1min node level anomaly rate
      to: sysadmin
```

## 6.4.2 Per-Chart Anomaly Rate Alert

Alert on anomaly rate for a specific chart using the `anomaly-bit` lookup option:

```conf
template: cpu_chart_anomaly
      on: system.cpu
  lookup: average -5m anomaly-bit of *
   units: %
   every: 30s
    warn: $this > 20
    info: Rolling 5min anomaly rate for system.cpu chart
      to: sysadmin
```

Requires ML to be enabled (`[ml]` section in `netdata.conf`).

## 6.4.3 Related Sections

- **6.5 Trend and Capacity Alerts** - Predictive alerts
- **8.4 Custom Actions** - Automation with exec