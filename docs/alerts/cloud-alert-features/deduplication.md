# 10.3 Alert Notification Deduplication

Netdata Cloud deduplicates alert notifications so that you receive them only once, even when the same alert fires on multiple nodes or when multiple Parents report the same alert for the same child node.

## 10.3.1 How It Works

When running multiple Netdata Parents for the same children (for high availability), each Parent independently evaluates health checks and generates alerts. Without Cloud-dispatched notifications, you would receive duplicate notifications from each Parent.

With Cloud-dispatched notifications, Cloud tracks alert state across all connected Agents and Parents. When the same alert triggers on multiple sources, Cloud sends a single notification rather than one per source.

| Scenario | Agent-Dispatched | Cloud-Dispatched |
|----------|------------------|------------------|
| Same alert on 5 nodes | 5 notifications | 1 notification |
| 3 Parents monitoring same child | 3 notifications | 1 notification |

## 10.3.2 Related Sections

- **12.4 Patterns for Large and Distributed Environments** for multi-node setups
- **13.6 Scaling Alerting in Complex Topologies** for complex topologies
- **5.1 Notification Concepts** for understanding dispatch models