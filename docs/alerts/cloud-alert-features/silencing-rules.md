# 10.2 Silencing Rules Manager

Netdata Cloud provides a **silencing rules** engine to suppress alert notifications based on specific conditions. Rules are managed entirely through the Cloud UI.

:::note

Silencing rules require a [paid plan](/docs/netdata-cloud/view-plan-and-billing.md).

:::

## 10.2.1 Accessing Silencing Rules

1. Click the **Space settings** cog (above your profile icon)
2. Click the **Alert & Notification** tab
3. Click the **Notification Silencing Rules** tab

You can also create silencing rules directly from the **Alerts tab** or **Nodes tab** using the "Actions" column.

## 10.2.2 Rule Types

| Rule Type | Who Can Create | Affects |
|-----------|----------------|---------|
| **Space Rules** | Administrators, Managers | All users in the space |
| **Personal Rules** | Any role except Billing | Only the creating user |

## 10.2.3 Configuration Criteria

**Node Criteria:**
- **Rooms**: Which rooms to target (or all rooms)
- **Nodes**: Specific nodes or `*` for all
- **Host Labels**: Key-value pairs (e.g., `environment:production`)

**Alert Criteria:**
- **Alert Name**: Specific alert names or `*` for all
- **Alert Context**: Chart context (e.g., `system.cpu`)
- **Alert Role**: Target alerts with specific roles

**Timing Options:**
- **Immediate**: Active now until turned off or duration expires
- **Scheduled**: Specify start and end times for maintenance windows

## 10.2.4 Common Silencing Scenarios

| Scenario | Configuration |
|----------|---------------|
| Infrastructure maintenance | All Rooms, All Nodes (`*`) |
| Team-specific silencing | Specific Room only |
| Single node maintenance | Specific node name |
| Environment-based | Host label: `environment:production` |
| Load testing | Alert context: `system.cpu` |

## 10.2.5 Related Sections

- [Silencing in Netdata Cloud](/docs/alerts/controlling-alerts-noise/silencing-cloud.md) - Detailed Cloud silencing guide
- [Silencing vs Disabling](/docs/alerts/controlling-alerts-noise/silencing-vs-disabling.md) - Conceptual differences
- [Manage Silencing Rules Reference](/docs/alerts-and-notifications/notifications/centralized-cloud-notifications/manage-alert-notification-silencing-rules.md) - Complete reference documentation