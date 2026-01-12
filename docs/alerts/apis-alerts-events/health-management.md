# 9.4 Health Management API

The Health Management API allows runtime control of alert evaluation and notifications.

**Base endpoint:** `/api/v1/manage/health`

> **Note:** This API requires authentication via bearer token matching the configured `api secret`.

## 9.4.1 Disable/Silence All Alerts

```bash
# Disable ALL alerts on this node (stops evaluation entirely)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=DISABLE%20ALL"

# Silence ALL alerts on this node (evaluates but suppresses notifications)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=SILENCE%20ALL"
```

## 9.4.2 Disable/Silence Alerts by Pattern

Use selector parameters to target specific alerts. Available selectors:

| Parameter | Description |
|-----------|-------------|
| `alarm`   | Alert name pattern (supports wildcards) |
| `chart`   | Chart name pattern |
| `context` | Context pattern |
| `hosts`   | Hostname pattern |

```bash
# Disable alerts matching a pattern (stops evaluation)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=DISABLE&alarm=high_cpu*"

# Silence alerts on a specific context (suppresses notifications only)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=SILENCE&context=system.cpu"

# Combine multiple selectors (all must match)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=DISABLE&alarm=disk*&chart=disk.io"
```

## 9.4.3 Re-enable Alerts

```bash
# Reset ALL silencers and re-enable all alerts
curl -s "http://localhost:19999/api/v1/manage/health?cmd=RESET"

# List current health management state (returns JSON)
curl -s "http://localhost:19999/api/v1/manage/health?cmd=LIST"
```

> **Important:** The RESET command clears all silencers globally. There is no way to reset individual silencer entries via the API.

## 9.4.4 Reload Configuration

Health configuration reload is done via `netdatacli`, not the API:

```bash
sudo netdatacli reload-health
```

Or by sending SIGUSR2 to the netdata process.

## 9.4.5 Related Sections

- **4.1 Disabling Alerts** for configuration-based disabling
- **8.4 Custom Actions** for exec-based automation