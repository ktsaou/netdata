# 9.1 Query Current Alerts

```bash
# Get all alerts (active and inactive)
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.'

# Get only active alerts (non-CLEAR status)
curl -s "http://localhost:19999/api/v1/alarms?active" | jq '.'
```

Response includes alert name, status, value, thresholds in the `.alarms` object.

## 9.1.1 Filter Results with jq

The API returns all alerts; use `jq` to filter by status:

```bash
# Get only WARNING and CRITICAL alerts
curl -s "http://localhost:19999/api/v1/alarms?all" | jq '.alarms | to_entries[] | select(.value.status == "WARNING" or .value.status == "CRITICAL")'
```

## 9.1.2 Related Sections

- **9.3 Inspect Variables** for variable debugging
- **7.1 Alert Never Triggers** for diagnostic approach