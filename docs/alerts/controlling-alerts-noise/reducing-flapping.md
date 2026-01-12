# 4.4 Reducing Flapping and Noise

Alert flapping—rapidly switching between CLEAR, WARNING, and CRITICAL states—creates noise and erodes trust in alerts. Users start to ignore flapping alerts, which defeats the purpose of alerting.

## 4.4.1 Understanding Flapping

**What causes flapping:**

| Cause | Example |
|-------|---------|
| **Threshold too close to normal values** | Alert fires at >80%, system hovers at 79-81% |
| **Short evaluation window** | 10-second window catches momentary spikes |
| **Noisy metrics** | Metrics with high variance (e.g., network packets) |
| **Race conditions** | Alert fires just before scheduled maintenance |

**The cost of flapping:**
- Notification fatigue → responders ignore alerts
- False negatives → real alerts get lost in noise
- Wasted time → investigating non-issues

## 4.4.2 The `delay` Line

The `delay` line controls how long **notifications** are delayed after a status change. This prevents notification floods when alerts flap rapidly between states. Note that status changes happen immediately; only the notification timing is affected.

**Syntax:**

```conf
delay: [[[up U] [down D] multiplier M] max X]
```

**Parameters:**

| Param | Description | Default |
|-------|-------------|---------|
| `up U` | Delay for status increases (CLEAR→WARNING, WARNING→CRITICAL) | 0 |
| `down D` | Delay for status decreases (CRITICAL→WARNING, WARNING→CLEAR) | 0 |
| `multiplier M` | Multiplies delays when alert changes state during delay period | 1.0 |
| `max X` | Maximum absolute notification delay | max(U×M, D×M) |

**Example: Basic Delay**

```conf
template: high_cpu
   on: system.cpu
   lookup: average -5m of user,system
   every: 1m
   warn: $this > 80
   crit: $this > 95
   delay: up 5m down 1m
```

This means:
- **Up delay (5 minutes):** After entering WARNING/CRITICAL, wait **5 minutes** before sending the notification
- **Down delay (1 minute):** After returning to CLEAR, wait **1 minute** before sending the notification

Note: The status **changes immediately** when the threshold is crossed. The delay affects only when you receive the notification, not when the status changes.

## 4.4.3 The `repeat` Line

The `repeat` line controls **how often notifications are sent** while the alert remains in a non-CLEAR state.

**Syntax:**

```conf
repeat: [off] [warning DURATION] [critical DURATION]
```

**Parameters:**

| Param | Description | Default |
|-------|-------------|---------|
| `off` | Turns off repeating for this alert | - |
| `warning DURATION` | Interval between WARNING notifications (use `0s` to disable) | from netdata.conf |
| `critical DURATION` | Interval between CRITICAL notifications (use `0s` to disable) | from netdata.conf |

**Example: Daily Repeat for Sustained Issues**

```conf
template: disk_space_low
   on: disk.space
   lookup: average -1m percentage of avail
   every: 1m
   warn: $this < 20
   crit: $this < 10
   repeat: warning 24h critical 24h
   info: Disk space is running low
```

This sends notifications once per day while the alert is active—not once per minute.

## 4.4.4 Combining Delay and Repeat

```conf
template: service_health
   on: health.service
   lookup: average -3m of status
   every: 1m
   crit: $this == 0
   delay: up 10m down 2m
   repeat: warning 6h critical 6h
```

**Behavior:**
1. When service goes down, status changes to CRITICAL **immediately**
2. First CRITICAL notification is delayed by **10 minutes** (up delay)
3. If service stays down, repeat notifications every **6 hours**
4. When service recovers, status changes to CLEAR **immediately**
5. CLEAR notification is delayed by **2 minutes** (down delay)

## 4.4.5 Status-Dependent Conditions

For advanced hysteresis, use `if` conditions that depend on the **current status**:

```conf
template: cpu_trend
   on: system.cpu
   lookup: average -5m of user,system
   every: 1m
   warn: $this > (($status >= $WARNING)  ? (75) : (85))
   crit: $this > (($status == $CRITICAL) ? (85) : (95))
```

This means:
- **Entering WARNING:** needs >85% CPU
- **Staying in WARNING:** threshold drops to 75% (harder to clear)
- **Entering CRITICAL:** needs >95%
- **Staying in CRITICAL:** threshold drops to 85% (harder to clear)

See **8.1 Hysteresis and Status-Based Conditions** for more examples.

## 4.4.6 Related Sections

- **8.1 Hysteresis and Status-Based Conditions** - Advanced status-aware logic
- **7.3 Alert Flapping** - Debugging and fixing flapping alerts
- **5.5 Testing and Troubleshooting** - Verifying notification delivery