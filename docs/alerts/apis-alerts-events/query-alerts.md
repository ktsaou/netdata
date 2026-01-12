# 9.1 Query Current Alerts

## 9.1.1 API Endpoints

### /api/v1/alarms

```bash
# Get all alerts (all statuses)
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.'

# Get only triggered alerts (WARNING or CRITICAL status)
curl -s "http://localhost:19999/api/v1/alarms?active" | jq '.'
```

**Parameters:**
- `all` or `all=true` - Returns all alerts regardless of status
- `active` or `active=true` - Returns only alerts with WARNING or CRITICAL status (default)

**Response Structure:**
- `hostname` - The host name
- `latest_alarm_log_unique_id` - ID of the most recent alarm log entry
- `status` - Whether health monitoring is enabled (true/false)
- `now` - Current Unix timestamp
- `alarms` - Object containing all matching alerts, keyed by `chart.alert_name`

**Alert Status Values** (in order of severity):
- `REMOVED` - Alert configuration was removed
- `UNDEFINED` - Alert cannot be evaluated
- `UNINITIALIZED` - Alert not yet evaluated
- `CLEAR` - Alert condition is not met
- `RAISED` - Alert was raised (generic raised state)
- `WARNING` - Alert is in warning state
- `CRITICAL` - Alert is in critical state

## 9.1.2 Filter Results with jq

The API returns all matching alerts; use `jq` to filter further:

```bash
# Get only WARNING and CRITICAL alerts
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.alarms | to_entries[] | select(.value.status == "WARNING" or .value.status == "CRITICAL")'

# Get all raised alerts (RAISED, WARNING, or CRITICAL)
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.alarms | to_entries[] | select(.value.status == "RAISED" or .value.status == "WARNING" or .value.status == "CRITICAL")'

# Get alerts for a specific chart
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.alarms | to_entries[] | select(.value.chart == "system.cpu")'
```

## 9.1.3 Related Sections

- **9.3 Inspect Variables** for variable debugging
- **7.1 Alert Never Triggers** for diagnostic approach