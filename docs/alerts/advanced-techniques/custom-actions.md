# 8.4 Custom Actions with `exec`

## 8.4.1 Overview

When an alert triggers, Netdata executes a notification script. By default, this is `alarm-notify.sh` which supports many notification methods. You can:

1. **Configure `custom_sender()` in alarm-notify.sh** - the recommended approach
2. **Use a completely custom exec script** - for full control

## 8.4.2 Recommended: Custom Sender Function

Configure a custom sender function in `/etc/netdata/health_alarm_notify.conf`:

```bash
# Enable custom notifications
SEND_CUSTOM="YES"
DEFAULT_RECIPIENT_CUSTOM=""

# Define your custom sender function
custom_sender() {
    # $1 = space-separated list of recipients
    local to="${1}"

    # Available shell variables (not environment variables):
    # ${host}          - hostname
    # ${name}          - alert name
    # ${chart}         - chart name (type.id)
    # ${status}        - current status: CLEAR, WARNING, CRITICAL
    # ${old_status}    - previous status
    # ${value}         - current numeric value
    # ${value_string}  - current value with units
    # ${units}         - value units
    # ${info}          - alert description
    # ${summary}       - alert summary
    # ${when}          - timestamp
    # ${duration}      - seconds in previous state
    # ${classification} - alert class
    # ${context}       - chart context

    if [ "${status}" = "CRITICAL" ]; then
        curl -X POST https://your-api.example.com/alerts \
            -H 'Content-Type: application/json' \
            -d "{\"host\": \"${host}\", \"alert\": \"${name}\", \"status\": \"${status}\"}"
    fi
}
```

### All Available Variables

| Variable | Description |
|----------|-------------|
| `${host}` | Hostname that generated the alert |
| `${name}` | Alert name |
| `${chart}` | Chart name (type.id) |
| `${status}` | Current status: CLEAR, WARNING, CRITICAL |
| `${old_status}` | Previous status |
| `${value}` | Current numeric value |
| `${old_value}` | Previous numeric value |
| `${value_string}` | Current value with units |
| `${old_value_string}` | Previous value with units |
| `${units}` | Value units |
| `${info}` | Alert description |
| `${summary}` | Alert summary |
| `${when}` | Event timestamp |
| `${duration}` | Seconds in previous state |
| `${non_clear_duration}` | Total seconds in non-clear state |
| `${unique_id}` | Unique event ID |
| `${alarm_id}` | Alert ID |
| `${event_id}` | Incremental event ID for this alert |
| `${classification}` | Alert class field |
| `${context}` | Chart context |
| `${component}` | Alert component |
| `${type}` | Alert type |
| `${calc_expression}` | Expression that triggered the alert |
| `${calc_param_values}` | Values of expression parameters |
| `${total_warnings}` | Count of alerts in WARNING state |
| `${total_critical}` | Count of alerts in CRITICAL state |
| `${goto_url}` | URL to the Netdata dashboard |

## 8.4.3 Alternative: Custom Exec Script

For complete control, specify a custom script:

```conf
template: service_down
      on: system.cpu
  lookup: average -1m of user
   every: 1m
    crit: $this > 90
    exec: /usr/local/bin/alert-handler.sh
      to: ops-team
```

Your script receives 33 positional command-line arguments:

```bash
#!/bin/bash
# Arguments passed to exec scripts (positional):
roles="${1}"          # Recipient roles
host="${2}"           # Hostname
unique_id="${3}"      # Unique event ID
alarm_id="${4}"       # Alert ID
event_id="${5}"       # Event ID for this alert
when="${6}"           # Timestamp
name="${7}"           # Alert name
chart="${8}"          # Chart name
status="${9}"         # Current status
old_status="${10}"    # Previous status
value="${11}"         # Current value
old_value="${12}"     # Previous value
src="${13}"           # Source file:line
duration="${14}"      # Seconds in previous state
non_clear_duration="${15}"  # Total non-clear duration
units="${16}"         # Value units
info="${17}"          # Alert info/description
value_string="${18}"  # Value with units
old_value_string="${19}"    # Previous value with units
calc_expression="${20}"     # Evaluated expression
calc_param_values="${21}"   # Expression parameter values
total_warnings="${22}"      # Count of WARNING alerts
total_critical="${23}"      # Count of CRITICAL alerts
warn_alarms="${24}"         # List of warning alarms
crit_alarms="${25}"         # List of critical alarms
classification="${26}"      # Alert class
edit_command="${27}"        # Edit command
machine_guid="${28}"        # Machine GUID
transition_id="${29}"       # Transition ID
summary="${30}"             # Alert summary
context="${31}"             # Chart context
component="${32}"           # Alert component
type="${33}"                # Alert type

# Example: Send to PagerDuty
if [ "${status}" = "CRITICAL" ]; then
    curl -X POST https://events.pagerduty.com/v2/enqueue \
        -H 'Content-Type: application/json' \
        -d "{\"routing_key\": \"YOUR_KEY\", \"event_action\": \"trigger\", \"dedup_key\": \"${name}-${chart}\"}"
fi
```

## 8.4.4 Testing Custom Notifications

```bash
# Become netdata user
sudo su -s /bin/bash netdata

# Enable debug output
export NETDATA_ALARM_NOTIFY_DEBUG=1

# Send test alerts
/usr/libexec/netdata/plugins.d/alarm-notify.sh test
```

## 8.4.5 Related Sections

- **[Custom Notification Method](../../../src/health/notifications/custom/README.md)** - full reference