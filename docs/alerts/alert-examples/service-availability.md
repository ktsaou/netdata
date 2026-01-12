# Service and Availability Alerts

This section shows real examples from Netdata's stock alerts for monitoring service availability and health.

## Systemd Service Unit Failed

Detects when a systemd service unit enters a failed state.

```conf
# From: src/health/health.d/systemdunits.conf

template: systemd_service_unit_failed_state
      on: systemd.service_unit_state
   class: Errors
    type: Linux
   component: Systemd units
chart labels: unit_name=!*
    calc: $failed
   units: state
   every: 10s
    warn: $this != nan AND $this == 1
   delay: down 5m multiplier 1.5 max 1h
 summary: systemd unit ${label:unit_name} state
    info: systemd service unit in the failed state
      to: sysadmin
```

**Key points:**
- Uses `calc: $failed` to check the failed dimension directly (no lookup needed)
- The `chart labels: unit_name=!*` ensures the template applies to all units
- Delay prevents flapping with `down 5m multiplier 1.5 max 1h`

## HTTP Endpoint Health Check

Monitors HTTP endpoints for failures.

```conf
# From: src/health/health.d/httpcheck.conf

template: httpcheck_web_service_no_connection
      on: httpcheck.status
   class: Errors
    type: Other
   component: HTTP endpoint
  lookup: average -5m unaligned percentage of no_connection
   every: 10s
   units: %
    warn: $this >= 10 AND $this < 40
    crit: $this >= 40
   delay: down 5m multiplier 1.5 max 1h
 summary: HTTP check for ${label:url} failed requests
    info: Percentage of failed HTTP requests to ${label:url} in the last 5 minutes
      to: webmaster
```

**Key points:**
- Uses percentage-based threshold (not binary up/down)
- Warning at 10%, Critical at 40% failure rate
- 5-minute lookback window smooths out transient issues

## TCP Port Availability

Checks if a TCP port is reachable.

```conf
# From: src/health/health.d/portcheck.conf

template: portcheck_connection_fails
      on: portcheck.status
   class: Errors
    type: Other
   component: TCP endpoint
  lookup: average -5m unaligned percentage of no_connection,failed
   every: 10s
   units: %
    warn: $this >= 10 AND $this < 40
    crit: $this >= 40
   delay: down 5m multiplier 1.5 max 1h
 summary: Portcheck fails for ${label:host}:${label:port}
    info: Percentage of failed TCP connections to host ${label:host} port ${label:port} in the last 5 minutes
      to: sysadmin
```

**Key points:**
- Combines `no_connection` and `failed` dimensions
- Uses chart labels (${label:host}, ${label:port}) for dynamic info
- Same threshold pattern as HTTP checks for consistency

## Plugin Availability Status

Monitors Netdata plugin availability.

```conf
# From: src/health/health.d/plugin.conf

template: plugin_availability_status
      on: netdata.plugin_availability_status
   class: Errors
    type: Netdata
    calc: $now - $last_collected_t
   units: seconds ago
   every: 10s
    warn: $this > (($status >= $WARNING)  ? ($update_every) : (20 * $update_every))
   delay: down 5m multiplier 1.5 max 1h
 summary: Plugin ${label:_collect_plugin} availability status
    info: the amount of time that ${label:_collect_plugin} did not report its availability status
      to: sysadmin
```

**Key points:**
- Uses `$now - $last_collected_t` to calculate staleness
- Hysteresis: tighter threshold when already in warning state
- Monitors internal Netdata plugin health

## Streaming Node Disconnection

Alerts when permanent streaming child nodes disconnect.

```conf
# From: src/health/health.d/streaming.conf

template: streaming_disconnected
      on: netdata.streaming_inbound
   class: Availability
    type: Streaming
   component: Streaming
chart labels: type=permanent
    calc: ${stale disconnected}
   units: nodes
   every: 10s
    warn: $netdata.uptime.uptime > 30 * 60 AND $this > 0
   delay: up 5m down 5m multiplier 1.5 max 30m
 summary: Permanent streaming nodes disconnected
    info: Permanent child nodes disconnected from this parent.
      to: silent
```

**Key points:**
- Only applies to permanent nodes (`chart labels: type=permanent`)
- Waits 30 minutes after parent startup before alerting
- Bi-directional delay (`up 5m down 5m`) for stability

## Related Sections

- [Application Alerts](application-alerts.md) - Database and web server examples
- [Anomaly-Based Alerts](../advanced-techniques/anomaly-detection.md) - ML-driven detection
