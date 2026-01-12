# 8.1 Hysteresis and Status-Based Conditions

## 8.1.1 The Problem with Simple Thresholds

```conf
warn: $this > 80
```

This causes flapping when CPU hovers around 80%.

## 8.1.2 Hysteresis Solution

The conditional operator (`? :`) creates "sticky" thresholds that use different values depending on current alert state:

```conf
template: cpu_hysteresis
      on: system.cpu
  lookup: average -5m of user,system
   every: 1m
    warn: $this > (($status >= $WARNING)  ? (70) : (80))
    crit: $this > (($status == $CRITICAL) ? (85) : (95))
```

**How It Works:**

| Alert State | Triggers At | Clears At | Explanation                                              |
|-------------|-------------|-----------|----------------------------------------------------------|
| Warning     | 80% CPU     | 70% CPU   | Creates 10% buffer - must drop below 70% to clear        |
| Critical    | 95% CPU     | 85% CPU   | Creates 10% buffer - must drop below 85% to return to warning |

**Pattern Explanation:**
- When `$status >= $WARNING` is false (alert is CLEAR), the higher threshold (80) is used
- When `$status >= $WARNING` is true (already in WARNING), the lower threshold (70) keeps it raised
- Result: Quick to alert, slower to clear, preventing notification spam

## 8.1.3 Related Sections

- **4.4 Reducing Flapping** for delay techniques
- **7.3 Alert Flapping** for debugging