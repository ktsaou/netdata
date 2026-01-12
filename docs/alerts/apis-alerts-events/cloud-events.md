# 9.5 Cloud Events

Netdata Cloud provides an Events Feed in its UI that displays alert transitions, topology changes, and auditing events across your infrastructure.

:::note

Netdata Cloud does not currently expose a public API for events. The Events Feed is accessed through the Cloud UI, not programmatically.

:::

## 9.5.1 Accessing Cloud Events

1. Log in to [Netdata Cloud](https://app.netdata.cloud)
2. Click the **Events** tab in the left sidebar
3. Use filters to narrow results by:
   - Event domain (Alerts, Topology, Auditing)
   - Node
   - Alert severity
   - Time range

## 9.5.2 Event Types Available

| Event Domain | Description |
|--------------|-------------|
| **Alert Events** | Alert state transitions (CLEAR, WARNING, CRITICAL) |
| **Topology Events** | Node connectivity changes (live, offline, created, deleted) |
| **Auditing Events** | User actions and Space configuration changes |

## 9.5.3 Programmatic Access to Alert History

For programmatic access to alert transitions, use the local Agent API:

```bash
# Get alert transitions from a single agent
curl -s "http://localhost:19999/api/v3/alert_transitions" | jq '.'

# Filter by status
curl -s "http://localhost:19999/api/v3/alert_transitions?f_status=CRITICAL" | jq '.'

# Filter by alert name
curl -s "http://localhost:19999/api/v3/alert_transitions?f_alert=cpu*" | jq '.'
```

See **9.2 Alert History** for the v1 API, or query `/api/v3/alert_transitions` directly for multi-node support.

## 9.5.4 Related Sections

- **10.1 Events Feed** for detailed Cloud Events UI documentation
- **9.2 Alert History** for Agent alert history API
- **5.3 Cloud Notifications** for Cloud notification routing