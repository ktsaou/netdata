# 9.4 Health Management API

The Health Management API allows runtime control of alert evaluation and notifications.

**Base endpoint:** `/api/v1/manage/health`

## 9.4.1 Disable/Silence Alerts

```bash
# Disable a specific alert (stops evaluation entirely)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=DISABLE&alarm=high_cpu"

# Silence a specific alert (evaluates but suppresses notifications)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=SILENCE&alarm=high_cpu"

# Disable ALL alerts on this node
curl -s "http://localhost:19999/api/v1/manage/health?cmd=DISABLE%20ALL"

# Silence ALL alerts on this node
curl -s "http://localhost:19999/api/v1/manage/health?cmd=SILENCE%20ALL"
```

## 9.4.2 Re-enable Alerts

```bash
# Reset a specific alert to normal operation
curl -s "http://localhost:19999/api/v1/manage/health?cmd=RESET&alarm=high_cpu"

# List current health management state
curl -s "http://localhost:19999/api/v1/manage/health?cmd=LIST"
```

## 9.4.3 Reload Configuration

Health configuration reload is done via `netdatacli`, not the API:

```bash
sudo netdatacli reload-health
```

Or by sending SIGUSR2 to the netdata process.

## 9.4.3 Related Sections

- **4.1 Disabling Alerts** for configuration-based disabling
- **8.4 Custom Actions** for exec-based automation