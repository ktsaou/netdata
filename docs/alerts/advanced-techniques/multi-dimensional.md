# 8.2 Multi-Dimensional and Per-Instance Alerts

## 8.2.1 Dimension Selection

You can select specific dimensions from a chart using the `of` clause in lookup. By default, all dimensions are summed together. The lookup result is available as `$this`.

```conf
template: network_quality
    on: net.net
lookup: average -1m of errors,dropped
    every: 1m
     warn: $this > 10
```

This selects only the `errors` and `dropped` dimensions, calculates their average over the last minute, sums them (default behavior), and alerts if the combined value exceeds 10.

## 8.2.2 Scaling to All Instances

Templates automatically create one alert per matching chart. When you define a template on a context like `net.net`, Netdata creates separate alert instances for each chart in that context.

```conf
template: interface_errors
    on: net.net
lookup: average -5m of errors
    every: 1m
     warn: $this > 10
```

Creates separate alert instances for each network interface chart (e.g., `net.eth0`, `net.eth1`). Each alert evaluates independently with its own `$this` value.

## 8.2.3 Related Sections

- **1.2 Alert Types: alarm vs template** for conceptual understanding