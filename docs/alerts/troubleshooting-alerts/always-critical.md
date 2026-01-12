# 7.2 Alert Always Critical or Warning

The alert never returns to `CLEAR`.

## 7.2.1 Common Causes

| Cause | Example |
|-------|---------|
| Threshold unit mismatch | Alert checks `> 80` but metric is in KB/s |
| Calculation error | `calc` expression always true |
| Variable typo | `$thiss` instead of `$this` (returns `nan`) |

## 7.2.2 Diagnostic Steps

```bash
# Get all raised alerts and their values
curl -s "http://localhost:19999/api/v1/alarms" | jq '.alarms | to_entries[] | {name: .key, value: .value.value, status: .value.status}'

# Get a specific alert (key format: "chart_name.alert_name")
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.alarms["system.cpu.cpu_usage"].value'
```

Check if the value actually crosses the threshold.

## 7.2.3 Fixing Calculation Errors

```conf
# WRONG: Division by zero returns INFINITY, causing unexpected status
calc: $this / ($var - $var2)

# RIGHT: Guard against division by zero
calc: ($var != $var2) ? ($this / ($var - $var2)) : 0
```

Division by zero in Netdata expressions returns `inf` (infinity), not an error. This can cause alerts to remain in unexpected states.

See **3.3 Calculations and Transformations**.

## 7.2.4 Related Sections

- **7.1 Alert Never Triggers** for complementary diagnosis
- **7.3 Alert Flapping** for rapid status changes