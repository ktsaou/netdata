# 9.2 Alert History

The `/api/v1/alarm_log` endpoint returns alert state transitions (status changes) from the local agent's health log database.

## 9.2.1 Basic Query

```bash
curl -s "http://localhost:19999/api/v1/alarm_log" | jq '.'
```

Returns all recent alert transitions up to the configured maximum (default: 1000 entries).

## 9.2.2 Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `after` | integer | Unix timestamp or relative seconds. Only return transitions after this time. Use negative values for relative time (e.g., `-3600` for last hour). |
| `chart` | string | Filter transitions to a specific chart ID. |

### Time-Based Filtering

```bash
# Last hour of transitions
curl -s "http://localhost:19999/api/v1/alarm_log?after=-3600" | jq '.'

# Since a specific timestamp
curl -s "http://localhost:19999/api/v1/alarm_log?after=1704067200" | jq '.'
```

### Filter by Chart

```bash
# All transitions for a specific chart
curl -s "http://localhost:19999/api/v1/alarm_log?chart=system.cpu" | jq '.'

# Combined: last hour for a specific chart
curl -s "http://localhost:19999/api/v1/alarm_log?after=-3600&chart=system.cpu" | jq '.'
```

## 9.2.3 Response Format

Returns a JSON array of transition objects:

```json
[
  {
    "hostname": "server1",
    "utc_offset": 0,
    "timezone": "UTC",
    "unique_id": 12345,
    "alarm_id": 1,
    "alarm_event_id": 5,
    "config_hash_id": "abc123...",
    "transition_id": "def456...",
    "name": "cpu_usage",
    "chart": "system.cpu",
    "context": "system.cpu",
    "class": "Utilization",
    "component": "CPU",
    "type": "System",
    "processed": true,
    "updated": false,
    "exec_run": 1704067200,
    "exec_failed": false,
    "exec": "/usr/libexec/netdata/plugins.d/alarm-notify.sh",
    "recipient": "sysadmin",
    "exec_code": 0,
    "source": "health.d/cpu.conf:10",
    "command": "...",
    "units": "%",
    "when": 1704067200,
    "duration": 60,
    "non_clear_duration": 120,
    "status": "WARNING",
    "old_status": "CLEAR",
    "delay": 0,
    "delay_up_to_timestamp": 0,
    "updated_by_id": 0,
    "updates_id": 0,
    "value_string": "85.5%",
    "old_value_string": "45.2%",
    "last_repeat": 0,
    "silenced": false,
    "summary": "CPU usage is high",
    "info": "CPU utilization exceeded warning threshold",
    "no_clear_notification": false,
    "value": 85.5,
    "old_value": 45.2
  }
]
```

### Key Response Fields

| Field | Description |
|-------|-------------|
| `unique_id` | Unique identifier for this log entry |
| `alarm_id` | The alert's persistent ID |
| `alarm_event_id` | Sequence number of this transition for the alert |
| `transition_id` | UUID identifying this specific state transition |
| `name` | Alert name |
| `chart` | Chart this alert monitors |
| `context` | Chart context |
| `status` | New status after transition (CLEAR, WARNING, CRITICAL, etc.) |
| `old_status` | Previous status before transition |
| `when` | Unix timestamp of the transition |
| `duration` | Seconds since last transition |
| `non_clear_duration` | Seconds in non-CLEAR state |
| `value` | Alert value at transition time |
| `old_value` | Previous alert value |
| `silenced` | Whether notifications were silenced |

## 9.2.4 Related Sections

- **9.1 Query Current Alerts** for current alert status
- **10.1 Events Feed** for Cloud-based alert history with cross-node aggregation