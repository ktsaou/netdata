# 6.4 Anomaly-Based Alerts

Netdata's Machine Learning engine calculates anomaly rates for metrics. You can create alerts based on these rates.

## 6.4.1 Node-Level Anomaly Rate Alert

Alert when the overall node anomaly rate exceeds a threshold. This uses the pre-computed `anomaly_detection.anomaly_rate` chart:

```conf
 template: ml_1min_node_ar
       on: anomaly_detection.anomaly_rate
    class: Workload
     type: System
component: ML
   lookup: average -1m of anomaly_rate
     calc: $this
    units: %
    every: 30s
     warn: $this > 1
  summary: ML node anomaly rate
     info: Rolling 1min node level anomaly rate
       to: silent
```

This is the stock alert from `ml.conf`. Change `to: silent` to your recipient (e.g., `sysadmin`) to enable notifications.

## 6.4.2 Per-Chart Anomaly Rate Alert

Alert on anomaly rate for a specific chart using the `anomaly-bit` lookup option. This computes the anomaly rate directly from the anomaly bits stored with each metric:

```conf
template: ml_5min_cpu_chart
      on: system.cpu
   class: Workload
    type: System
component: ML
  lookup: average -5m anomaly-bit of *
    calc: $this
   units: %
   every: 30s
    warn: $this > (($status >= $WARNING)  ? (5) : (20))
    crit: $this > (($status == $CRITICAL) ? (20) : (100))
    info: Rolling 5min anomaly rate for system.cpu chart
      to: silent
```

**Key differences:**

- `of *` aggregates all dimensions into a single chart-level anomaly rate
- The hysteresis pattern (thresholds adjust based on current status) prevents alert flapping

## 6.4.3 Per-Dimension Anomaly Rate Alert

Use `foreach *` instead of `of *` to create separate alerts for each dimension:

```conf
template: ml_5min_cpu_dims
      on: system.cpu
  lookup: average -5m anomaly-bit foreach *
    calc: $this
   units: %
   every: 30s
    warn: $this > (($status >= $WARNING)  ? (5) : (20))
    crit: $this > (($status == $CRITICAL) ? (20) : (100))
    info: Rolling 5min anomaly rate for each system.cpu dimension
      to: silent
```

This creates one alert instance per CPU dimension (user, system, nice, etc.).

## 6.4.4 Prerequisites

- ML must be enabled in `netdata.conf` under the `[ml]` section
- See [ML Configuration](/src/ml/ml-configuration.md) for details

## 6.4.5 Related Sections

- **6.5 Trend and Capacity Alerts** - Predictive alerts
- **8.4 Custom Actions** - Automation with exec