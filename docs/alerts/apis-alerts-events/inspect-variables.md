# 9.3 Inspect Alert Variables

```bash
curl -s "http://localhost:19999/api/v1/alarm_variables?chart=system.cpu" | jq '.'
```

Shows all variables available for alert expressions on a specific chart.

## 9.3.1 Response Format

```json
{
  "chart": "system.cpu",
  "chart_name": "system.cpu",
  "chart_context": "system.cpu",
  "family": "cpu",
  "host": "hostname",
  "current_alert_values": {
    "this": null,
    "after": 1736654400,
    "before": 1736654401,
    "now": 1736654401,
    "status": -2,
    "REMOVED": -2,
    "UNDEFINED": -1,
    "UNINITIALIZED": 0,
    "CLEAR": 1,
    "WARNING": 2,
    "CRITICAL": 3,
    "green": null,
    "red": null
  },
  "dimensions_last_stored_values": {
    "user": 5.2,
    "system": 3.1,
    "nice": 0.0
  },
  "dimensions_last_collected_values": {
    "user_raw": 12345,
    "system_raw": 7890,
    "nice_raw": 0
  },
  "dimensions_last_collected_time": {
    "user_last_collected_t": 1736654400,
    "system_last_collected_t": 1736654400,
    "nice_last_collected_t": 1736654400
  },
  "chart_variables": {
    "update_every": 1,
    "last_collected_t": 1736654400
  },
  "host_variables": {},
  "alerts": {}
}
```

### Key Sections

| Section | Description |
|---------|-------------|
| `current_alert_values` | Special variables for alert expressions (`this`, `status`, timestamps, thresholds) |
| `dimensions_last_stored_values` | Dimension values (use these as `$user`, `$system`, etc.) |
| `dimensions_last_collected_values` | Raw collected values (`$user_raw`, `$system_raw`) |
| `dimensions_last_collected_time` | Last collection timestamps per dimension |
| `chart_variables` | Chart-level variables (`$update_every`, `$last_collected_t`) |
| `host_variables` | Host-level custom variables |
| `alerts` | Related alerts with their current values |

### Status Constants

The numeric status constants can be used in expressions:

- `-2` = REMOVED
- `-1` = UNDEFINED
- `0` = UNINITIALIZED
- `1` = CLEAR
- `2` = WARNING
- `3` = CRITICAL

## 9.3.2 Related Sections

- **7.4 Variables Not Found** for troubleshooting
- **3.5 Variables and Special Symbols** for reference