# 10.1 Events Feed

The Events Feed provides a historical view of key activities across your infrastructure and Space, helping you correlate changes with anomalies or node behavior.

## 10.1.1 Accessing the Events Feed

1. Log in to Netdata Cloud
2. Click the **Events** tab in the left sidebar
3. Define the timeframe using the Date and Time selector
4. Apply filters from the right-hand bar

## 10.1.2 Event Domains

| Event Domain | Description | Retention (Community) | Retention (Homelab/Business) |
|--------------|-------------|----------------------|------------------------------|
| **Alert Events** | Alert state transitions (CLEAR, WARNING, CRITICAL, Removed, Error, Unknown) | 4 hours | 90 days |
| **Topology Events** | Node connectivity changes (live, offline, created, deleted) | 4 hours | 14 days |
| **Auditing Events** | User actions and Space configuration changes | 4 hours | 90 days |

## 10.1.3 Filtering Options

The right-hand filter bar provides options for:

- **Event domain** - Filter by Alerts, Topology, or Auditing events
- **Node** - Filter by specific nodes
- **Alert severity** - Filter by alert severity level
- **Time range** - Define the timeframe for event history

## 10.1.4 Access Restrictions

| User Role | Accessible Event Domains |
|-----------|--------------------------|
| Administrators | All domains (Auditing, Topology, Alerts) |
| Non-administrators | Topology and Alerts only |

## 10.1.5 Related Sections

- **9.5 Cloud Events** for Cloud Events overview
- **5.3 Cloud Notifications** for notification routing
- See also: [Events Tab](/docs/dashboards-and-charts/events-feed.md) for complete reference