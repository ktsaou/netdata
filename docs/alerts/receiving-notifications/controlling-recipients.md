# 5.4 Controlling Who Gets Notified

### 5.4.1 Severity Filtering for Recipients

In Agent notifications, you can filter which alert severities reach specific recipients using modifiers appended to recipient addresses:

| Modifier | Effect |
|----------|--------|
| `|critical` | Only receive critical alerts and their subsequent status changes until cleared |
| `|nowarn` | Do not receive warning notifications |
| `|noclear` | Do not receive clear notifications |

**Example Configuration** (`health_alarm_notify.conf`):

```ini
# user1 receives all alerts; user2 only receives critical-related alerts
role_recipients_email[sysadmin]="user1@example.com user2@example.com|critical"

# Receive critical alerts, but no clear notifications
role_recipients_slack[dba]="alerts-channel disasters-channel|critical|noclear"
```

### 5.4.2 Using Cloud Roles

Netdata Cloud uses Role-Based Access Control (RBAC). Available roles are:

| Role | Description |
|------|-------------|
| **Admin** | Full system control including billing and user management |
| **Manager** | Manage users, rooms, and configurations (not billing) |
| **Troubleshooter** | Investigate issues, run diagnostics, view assigned rooms |
| **Observer** | View-only access to assigned rooms |
| **Billing** | Handle invoices and payments only |

Cloud notification delivery is configured through **Settings** → **Notification Integrations** in the Netdata Cloud UI, not through configuration files.

### 5.4.3 Agent Role-Based Routing

In Agent alerts, the `to:` parameter specifies which **role** receives notifications (not email addresses directly):

```conf
# In health configuration
template: critical_service
      on: system.uptime
    calc: $uptime
   every: 1m
    crit: $this < 300
      to: sysadmin
```

Then, in `health_alarm_notify.conf`, assign actual recipients per role:

```ini
# Define recipients for the sysadmin role
role_recipients_email[sysadmin]="ops-team@company.com"
role_recipients_slack[sysadmin]="#critical-alerts"
role_recipients_pagerduty[sysadmin]="PDK3Y5EXAMPLE"
```

### 5.4.4 Related Sections

- **5.2 Agent and Parent Notifications** - Local routing
- **5.5 Testing and Troubleshooting** - Debugging delivery issues
- **12.2 Notification Strategy** - On-call best practices