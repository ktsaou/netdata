# 2.3 Creating and Editing Alerts via Netdata Cloud

This section shows you how to **create, edit, and manage alert definitions** using the **Netdata Cloud UI** (Health configuration), and how changes are applied to your connected nodes.

:::tip

Use this workflow when you want **centralized management**, **instant rollout** across your fleet, or prefer a **visual, UI-driven** approach to alert configuration.

:::

## Required Permissions

:::note

You need appropriate permissions in Netdata Cloud to manage alerts. Alert management is handled through the **Dynamic Configuration Manager**, which requires a Netdata Cloud paid subscription:

| Role | Can View Alerts | Can Create Alerts | Can Edit Alerts | Can Disable Alerts |
|------|-----------------|-------------------|-----------------|-------------------|
| **Admin** | ✓ | ✓ | ✓ | ✓ |
| **Manager** | ✓ | ✓ | ✓ | ✓ |
| **Troubleshooter** | ✓ | - | - | - |
| **Observer** | ✓ | - | - | - |

For the complete RBAC permissions reference, see [Role-Based Access Control](/docs/netdata-cloud/authentication-and-authorization/role-based-access-model.md).

:::

## 2.3.1 Accessing Alert Configuration in Netdata Cloud

You create and edit Cloud-defined alerts from the **Health** configuration view of a specific node (server or network device).

### How to Access Health Configuration for a Node

1. Log in to Netdata Cloud
2. Go to your **Space**
3. In the **top horizontal navigation**, click **Nodes**
4. From the nodes list, **choose the server or network device** you want to create an alert for
5. On that node's card/header, click the **arrow-up icon** to expand actions
6. Click the **Configuration (gear) icon**
7. In the configuration menu, select **Health**

This opens the **Health configuration** view for that node, where you can see existing alerts and add new ones.

:::note

Cloud-defined alerts you create here are stored in Netdata Cloud and pushed to the selected node (and, depending on your setup, other nodes in scope) at runtime. They are **persisted locally** in `/var/lib/netdata/config/` for reliability, but do **not** create user-editable `.conf` files in `/etc/netdata/health.d/`.

:::

:::tip

The **Alerts** tab in the top navigation shows all alerts across your room. You can also view node-specific alerts by navigating to a node and clicking its **Alerts** sub-tab.

:::

## 2.3.2 Creating a New Alert

Once you are in the **Health** configuration view for a node:

### Step 1: Open the New Alert Form

Click the **+** (plus) icon in the Health view to create a new alert.

### Step 2: Fill in the Alert Form

You'll see a form with fields like:

| Field | Description | Example |
|-------|-------------|---------|
| **Alert Name** | Unique identifier for this alert | `high_cpu_usage` |
| **Description** | Human-readable explanation of what this alert monitors | `CPU usage exceeds 80% for 5 minutes` |
| **Metric / Chart** | The metric/chart to monitor on this node | Select `system.cpu` |
| **Dimension** | Specific dimension within the chart (optional) | `user` (user CPU time) |
| **Data Source** | What data to use for the alert (see 2.3.3) | `Samples` |
| **Time Aggregation** | How data is aggregated over the time window | `average` |
| **Time Window** | How far back to look when evaluating | `5 minutes` |
| **Warning Threshold** | Condition that triggers WARNING status | `> 80` |
| **Critical Threshold** | Condition that triggers CRITICAL status | `> 95` |
| **Notification Recipients / Routing** | Who gets notified when this alert fires | Select integration or role |

### Step 3: Choose Data Source

Select the **Data Source** (Samples, Percentages, or Anomalies). See **2.3.3 Data Sources** for details on each option.

### Step 4: Set Scope (Optional)

By default, the alert applies **only to the node you selected** in the Nodes view.

To apply this alert to additional nodes:
- Use **node filters** (select specific nodes or node labels, such as `environment:production` or `role:database`)
- Use **room filters** (apply to all nodes in specific rooms)

:::tip

Use node labels to target alerts dynamically. Any new node with matching labels will automatically inherit the alert.

:::

### Step 5: Save the Alert

Click **Save**. Netdata Cloud will:

1. Store the alert definition in Cloud
2. Push it to the selected node (and any additional nodes in scope)
3. The node will load it into memory and begin evaluating it immediately

## 2.3.3 Data Sources

The **Data Source** field determines what type of data the alert uses for evaluation.

<details>
<summary><strong>Samples (Default)</strong></summary>

**What it does:**
Uses the actual time-series values for each dimension. This is the standard approach for threshold-based alerting.

**When to use:**
- You know the exact acceptable range for a metric (for example, "CPU should stay below 80%")
- You want predictable, consistent alerting behavior based on actual values

**Example:**

| Setting | Value |
|---------|-------|
| Metric | `system.cpu` |
| Data Source | `Samples` |
| Time Aggregation | `average` |
| Time Window | `5 minutes` |
| Warning | `> 80` |
| Critical | `> 95` |

This fires WARNING if average CPU over the last 5 minutes exceeds 80%.

</details>

<details>
<summary><strong>Percentages</strong></summary>

**What it does:**
Uses the percentage of each dimension relative to the sum of all dimensions, rather than absolute values.

**When to use:**
- You want to alert on relative proportions rather than absolute values
- Useful when the total varies but the ratio matters (for example, one process consuming disproportionate resources)

**Example:**

| Setting | Value |
|---------|-------|
| Metric | `system.cpu` |
| Data Source | `Percentages` |
| Dimension | `user` |
| Warning | `> 90` |

This fires if user CPU time represents more than 90% of total CPU time.

</details>

<details>
<summary><strong>Anomalies (ML-Based)</strong></summary>

**What it does:**
Uses Netdata's machine learning anomaly rate for each dimension. Instead of comparing values against thresholds, it alerts when the metric behaves anomalously compared to learned patterns.

**When to use:**
- Metrics have variable "normal" ranges (for example, traffic patterns that differ by time of day)
- You want to detect unusual behavior without manually tuning thresholds

**How it works:**
Netdata's ML engine continuously trains on historical data and assigns an anomaly rate (0-100%) to each metric. Higher values indicate more unusual behavior.

**Example:**

| Setting | Value |
|---------|-------|
| Metric | `net.net` (network traffic) |
| Data Source | `Anomalies` |
| Time Aggregation | `average` |
| Time Window | `5 minutes` |
| Warning | `> 50` |

This fires when the average anomaly rate exceeds 50% over the last 5 minutes, indicating unusual network traffic patterns.

:::note

Anomaly-based alerts require Netdata's ML features to be enabled and trained. See the [ML Anomaly Detection documentation](/docs/ml-ai/ml-anomaly-detection/ml-anomaly-detection.md) for details.

:::

</details>

<details>
<summary><strong>Detecting Rate of Change</strong></summary>

To alert on rapid changes (spikes or drops), use the **Calculation** field with an expression that computes rate of change.

**Example approach:**

1. Use `Data Source: Samples`
2. Set a short **Time Window** (for example, 1 minute)
3. Use the **Calculation** field to compute the difference or derivative

Alternatively, use `incremental_sum` as the Time Aggregation method, which computes the sum of changes over the time window.

:::tip

For disk fill rate alerts, the stock alerts in `health.d/disks.conf` demonstrate this pattern using calculations like `$this / $last * 100`.

:::

</details>

## 2.3.4 Managing Cloud-Defined Alerts

### Editing Alert Configuration

To modify a Cloud-defined alert, while on the Health configuration view for the node:

1. Click the **pen icon** next to the alert you want to edit
2. Modify any fields (thresholds, time windows, scope, etc.)
3. Click **Save**

Alternatively, you can access alerts from the top navigation:

1. Click the **Alerts** tab in the top horizontal navigation
2. Find the alert in the alerts table
3. Click on the alert row to open the **Alert Details Modal**
4. Click the **Configuration** tab
5. Click **"Edit alert configuration"** to modify settings
6. Click **Save**

### What Happens When You Edit

- Netdata Cloud immediately pushes the updated definition to all connected nodes in scope
- Nodes reload the alert configuration in memory (no restart required)
- The alert continues evaluating with the new settings within seconds

### Editing vs File-Based Alerts

- The Cloud UI shows **all alerts** (both file-based and Cloud-defined)
- When you **edit a file-based alert** via the Cloud UI, Netdata creates a Cloud-defined version that overrides the original
- You **cannot remove** file-based alerts via the Cloud UI (only Cloud-defined alerts can be removed)
- To permanently modify or remove file-based alerts, use the workflow in **2.2 Creating and Editing Alerts via Configuration Files**

## 2.3.5 Disabling Cloud-Defined Alerts

### Disabling an Alert

To stop an alert from evaluating, while on the Health configuration view for the node:

1. Click the **three dots** (more options) next to the alert
2. Select **disable**

### Temporarily Silencing Alerts

If you want to temporarily stop notifications without disabling the alert:
- Use **silencing rules** (see **Chapter 4: Controlling Alerts and Noise**)

## 2.3.6 Cloud Alert Lifecycle and Propagation

<details>
<summary><strong>How Quickly Do Changes Apply?</strong></summary>

When you create, edit, or disable a Cloud-defined alert:
- Changes propagate to connected nodes within seconds (typically 5 to 15 seconds)
- Nodes receive the update via the Cloud-agent connection
- The alert begins evaluating (or stops, if disabled) immediately after the node receives the update

</details>

<details>
<summary><strong>What Happens When Nodes Are Offline?</strong></summary>

- If a node is **offline** when you create/edit an alert, it will receive the update **when it reconnects**
- Cloud queues the configuration change and delivers it on next connection
- Once a Cloud-defined alert has been delivered to a node, it continues to evaluate locally even if Cloud connectivity is lost
- Only **new or changed** Cloud alerts created while a node is offline will wait until reconnection to be delivered

</details>

<details>
<summary><strong>Interaction with File-Based Alerts</strong></summary>

Cloud-defined alerts and file-based alerts work together in the same health engine:

- When you **edit a file-based alert** via the Cloud UI, a Cloud-defined version is created that **replaces** the file-based version in memory
- Alerts are identified by **name**: a Cloud-defined alert with the same name as a file-based alert will override it
- File-based alerts with the same name are **appended** (multiple rules can exist), but Cloud-defined alerts **replace** any existing alert with that name
- The original file-based configuration remains on disk and will be restored if the Cloud-defined version is removed

:::warning

Creating a Cloud-defined alert with the same name as an existing file-based alert will override the file-based behavior. Choose unique names to avoid conflicts, or intentionally use the same name when you want to override stock alerts.

:::

</details>

## 2.3.7 Verifying Cloud-Defined Alerts Are Active

After creating or editing a Cloud alert, verify it's working:

<details>
<summary><strong>Method 1: Check the Events Tab</strong></summary>

1. Go to **Events** (left sidebar)
2. Look for an event: `Alert <name> created` or `Alert <name> updated`
3. This confirms Cloud successfully stored and distributed the alert

</details>

<details>
<summary><strong>Method 2: Check a Node's Alert List</strong></summary>

1. Navigate to **Nodes** → select a node
2. Click the **bell icon** (top-right)
3. Search for your alert name
4. Verify its status (`CLEAR`, `WARNING`, `CRITICAL`, etc.)

</details>

<details>
<summary><strong>Method 3: Use the Alarms API (on the node)</strong></summary>

SSH to a node and query:

```bash
curl -s "http://localhost:19999/api/v1/alarms" | jq '.alarms | keys[]' | grep -i "your_alert_name"
```

If the alert appears, it's loaded and evaluating.

</details>

## 2.3.8 Bulk Operations and Advanced Patterns

### Applying the Same Alert to Multiple Spaces

If you manage multiple spaces and want consistent alerting:
- Create the alert in one space
- Manually recreate it in other spaces (Cloud does not currently support cross-space alert templates)

:::tip

Use **file-based alerts** deployed via configuration management (Ansible, Terraform, etc.) for true multi-space consistency. See **2.2 Creating and Editing Alerts via Configuration Files**.

:::

### Using Node Labels for Targeted Alerts

Instead of manually selecting nodes, use **node labels** to dynamically scope alerts:

**Example:**
- Label your production nodes with `environment:production`
- Create a Cloud alert with node filter: `environment:production`
- Any new node with that label automatically inherits the alert

This is especially powerful in dynamic environments (Kubernetes, auto-scaling groups).

## 2.3.9 When to Use Cloud UI vs Configuration Files

Both workflows produce alerts that **run on Agents/Parents**. Choose based on your operational model:

| Use Cloud UI When | Use Configuration Files When |
|-------------------|------------------------------|
| You want **centralized control** across many nodes | You need **version control** (Git, CI/CD) |
| You prefer **visual workflows** | You want alerts to work **without Cloud connectivity** |
| You need **instant rollout** without SSH | You need **advanced syntax** not yet in the UI |
| Your team is **less comfortable with CLI** | You treat alerts as **infrastructure-as-code** |

:::tip

You can use both: Manage fleet-wide standards via Cloud UI, and node-specific or advanced cases via files.

:::

## Key takeaway

Cloud-defined alerts provide **centralized management and instant rollout**. They are stored in Cloud, pushed to nodes, and evaluated locally on Agents/Parents, giving you the best of both worlds.

## What's Next

- **2.4 Managing Stock versus Custom Alerts** Patterns for combining built-in, custom, and Cloud-defined alerts
- **2.5 Reloading and Validating Alert Configuration** Deeper troubleshooting and validation techniques
- **Chapter 4: Controlling Alerts and Noise** Silencing rules, delays, and hysteresis
- **Chapter 5: Receiving Notifications** How to route alert events to Slack, PagerDuty, email, etc.