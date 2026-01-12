# 7.3 Alert Flapping

The alert rapidly switches between statuses.

## 7.3.1 Symptoms

- Many notifications in short time
- Status changes every evaluation cycle
- Alert shows WARNING/CRITICAL briefly, then CLEAR

## 7.3.2 Solution: Add Notification Delays

```conf
template: cpu_usage
    on: system.cpu
lookup: average -5m of user,system
    every: 1m
     warn: $this > 80
     crit: $this > 95
   delay: up 5m down 2m
```

The `delay` line controls **notification timing only**, not when the alert status changes. In this example:
- Status changes **immediately** when thresholds are crossed
- The notification is delayed by 5 minutes for status increases (CLEAR→WARNING, WARNING→CRITICAL)
- If status returns to CLEAR within that 5 minutes, no notification is sent

## 7.3.3 Solution: Repeat Intervals

```conf
repeat: warning 6h critical 6h
```

This limits ongoing notifications to every 6 hours while the alert remains active.

## 7.3.4 Solution: Hysteresis Thresholds

For true threshold hysteresis (different entry and exit thresholds), use status-based conditions:

```conf
warn: (($this > 80) && ($status != WARNING)) || (($this > 70) && ($status == WARNING))
```

This enters WARNING at 80% but only clears when below 70%.

## 7.3.5 Related Sections

- **4.4 Reducing Flapping and Noise** for delay and repeat techniques
- **8.1 Hysteresis** for status-based thresholds