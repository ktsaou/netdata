# 10.4 Room-Based Alerting

Rooms organize nodes and scope alerts to specific groups. When you view the Alerts tab in Netdata Cloud, you see alerts for nodes within the current Room, providing natural alert scoping.

## 10.4.1 Creating Rooms with Automatic Node Assignment

You can automatically assign nodes to Rooms based on their host labels using rule-based assignment.

### Setting Up Room Assignment Rules

1. Navigate to the Room you want to configure
2. Click the **gear icon** (Room settings)
3. Select the **Nodes** tab
4. Click **Add new Rule**
5. Configure the rule:
   - **Action**: Include or Exclude
   - **Clauses**: Conditions that determine which nodes match (all must be satisfied)

Each clause consists of:
- **Label**: The host label to check (e.g., `environment`, `role`)
- **Operator**: `equals`, `starts_with`, `ends_with`, or `contains`
- **Value**: The value to compare against

**Example: Production Database Room**

To create a Room that automatically includes all production database nodes:

| Element | Value |
|---------|-------|
| **Action** | Include |
| **Clause 1** | Label: `environment`, Operator: `equals`, Value: `production` |
| **Clause 2** | Label: `service-type`, Operator: `equals`, Value: `database` |

All clauses use logical AND, so nodes must match all conditions to be included.

:::note

Exclusion rules are evaluated before inclusion rules. If both match, the node is excluded.

:::

For complete details on rule-based room assignment, see [Node Rule-Based Room Assignment](/docs/netdata-cloud/node-rule-based-room-assignment.md).

## 10.4.2 Room-Scoped Alert Viewing

When you open the **Alerts** tab in any Room:

- **Raised Alerts** shows only alerts from nodes in that Room
- **Alert Configurations** shows configurations running on nodes in that Room

This provides natural scoping, allowing you to focus on alerts relevant to specific parts of your infrastructure.

## 10.4.3 Creating Alerts with Room Filters

When creating Cloud-defined alerts via the Health configuration view:

1. Navigate to a node and open its Health configuration
2. Create or edit an alert
3. In the **Scope** section, use **room filters** to apply the alert to all nodes in specific Rooms

This allows a single alert definition to target all nodes within one or more Rooms.

For complete details on creating Cloud-defined alerts, see [Creating and Editing Alerts via Netdata Cloud](/docs/alerts/creating-alerts-pages/creating-and-editing-alerts-via-cloud.md).

## 10.4.4 Related Sections

- **8.3 Host, Chart, and Label-Based Targeting** for label usage in alert configuration files
- **12.4 Patterns for Large and Distributed Environments** for multi-room strategies