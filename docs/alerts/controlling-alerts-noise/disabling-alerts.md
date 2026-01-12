# 4.1 Disabling Alerts

Disabling an alert stops it from being **evaluated entirely**. The health engine won't run the check, assign status, or generate events.

<details>
<summary><strong>When to Disable Instead of Silence</strong></summary>

Disable an alert when:

- **The alert is not relevant** to your environment (for example, you don't run MySQL)
- **You've replaced it** with a custom alert (same name)
- **The monitored service is retired** and the alert will never fire correctly
- **You want a permanent solution** rather than temporary silencing

For temporary quiet periods, see **4.3 Silencing in Netdata Cloud** instead.

</details>

## 4.1.1 Disable a Specific Alert

**Method 1: Via Configuration File**

Create or edit a file in `/etc/netdata/health.d/`:

```bash
sudo /etc/netdata/edit-config health.d/disabled.conf
```

Add the alert you want to override with a "never trigger" condition:

```conf
# Override a stock alert to never trigger
# Use the exact same alert name to override
template: mysql_10s_slow_queries
      on: mysql.queries
  lookup: average -10s of slow_queries
   units: queries
   every: 10s
    warn: 0  # Never triggers (condition always false)
    crit: 0
    info: Disabled - not relevant to our environment
```

Alternatively, route to silent recipient:

```conf
template: mysql_10s_slow_queries
      on: mysql.queries
  lookup: average -10s of slow_queries
   units: queries
   every: 10s
    warn: $this > 10
      to: silent
```

Reload configuration:

```bash
sudo netdatacli reload-health
```

**Method 2: Disable All Alerts for a Host**

To disable **all** health checks on a specific host, set in `netdata.conf`:

```ini
[health]
    enabled = no
```

This is useful when:
- A node is being decommissioned but you want to keep it running for diagnostics
- You're doing maintenance and don't want alerts firing
- You want to completely stop health monitoring on a development or test node

## 4.1.2 Disable Evaluation While Keeping Alert Visible

If you want the alert to remain visible in the UI but **never trigger** (stay in CLEAR status), set both conditions to always-false values:

```conf
template: noisy_alert
      on: system.cpu
    warn: 0
    crit: 0
```

Since `0` evaluates to false, the alert will never raise to WARNING or CRITICAL status. The alert remains loaded and visible in the alerts list, but stays in CLEAR state.

## 4.1.3 Disable via Health Management API

You can also disable alerts programmatically at runtime using the health management API.

**Important:** This API requires authentication via the `X-Auth-Token` header. By default, access is only allowed from localhost.

```bash
# Disable alerts matching a context (stops evaluation entirely)
curl "http://localhost:19999/api/v1/manage/health?cmd=DISABLE&context=random" \
     -H "X-Auth-Token: YOUR_API_KEY"

# Disable ALL alerts on this node
curl "http://localhost:19999/api/v1/manage/health?cmd=DISABLE%20ALL" \
     -H "X-Auth-Token: YOUR_API_KEY"

# Re-enable all alerts (clears all silencers and disablers)
curl "http://localhost:19999/api/v1/manage/health?cmd=RESET" \
     -H "X-Auth-Token: YOUR_API_KEY"
```

**Note:** Runtime-disabled alerts are still **loaded** in memory but skip evaluation. To verify the current state, use:

```bash
curl "http://localhost:19999/api/v1/manage/health?cmd=LIST" \
     -H "X-Auth-Token: YOUR_API_KEY"
```

See **9.4 Health Management API** for full documentation on selectors and authentication.

## 4.1.4 Related Sections

- **4.2 Silencing vs Disabling** - Understand the critical difference
- **4.3 Silencing in Netdata Cloud** - Cloud-managed silencing rules
- **4.4 Reducing Flapping and Noise** - Techniques for preventing flapping