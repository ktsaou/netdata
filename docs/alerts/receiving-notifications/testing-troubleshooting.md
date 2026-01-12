# 5.5 Testing and Troubleshooting Notification Delivery

## 5.5.1 Testing Agent Notifications

**Send a test notification:**

```bash
# Switch to the Netdata user first
sudo su -s /bin/bash netdata

# Enable debugging (optional, for detailed output)
export NETDATA_ALARM_NOTIFY_DEBUG=1

# Test notifications for the default role (sysadmin)
/usr/libexec/netdata/plugins.d/alarm-notify.sh test

# Test notifications for a specific role
/usr/libexec/netdata/plugins.d/alarm-notify.sh test webmaster
```

This sends WARNING, CRITICAL, and CLEAR test alerts sequentially to verify your notification pipeline.

**Check current alerts via API:**

```bash
# Get currently raised alerts (WARNING or CRITICAL)
curl -s "http://localhost:19999/api/v1/alarms"

# Get all enabled alerts (including CLEAR)
curl -s "http://localhost:19999/api/v1/alarms?all"
```

**Check notification logs:**

```bash
# Check Netdata error log for notification issues
sudo tail -n 100 /var/log/netdata/error.log | grep -i "alarm.notify"

# Or use journalctl if using systemd
sudo journalctl -u netdata --since "1 hour ago" | grep -i "alarm"
```

## 5.5.2 Testing Cloud Notifications

**Create a test alert in Cloud UI:**

1. Navigate to **Alerts** → **Alert Configuration**
2. Create a test alert with low thresholds
3. Monitor the target integration

## 5.5.3 Common Issues

| Issue | Likely Cause | Fix |
|-------|--------------|-----|
| No notifications received | Recipient misconfigured | Verify `to:` field |
| Duplicate notifications | Both Agent and Cloud sending | Disable one dispatch model |
| Notifications delayed | Network latency or batching | Check connection status |
| Wrong channel | Routing misconfigured | Verify channel name in config |
| Alerts not triggering | Thresholds too high | Lower threshold or check metrics |

## 5.5.4 Debugging Checklist

1. Is the alert firing? Check API: `curl http://localhost:19999/api/v1/alarms`
2. Is the recipient correct? Check the `to:` line in your alert configuration
3. Is the notification method enabled? Check `health_alarm_notify.conf` for `SEND_SLACK="YES"` (or the relevant method)
4. Is the recipient configured for the role? Check `role_recipients_slack[sysadmin]` in `health_alarm_notify.conf`
5. Are required credentials set? Check webhook URLs, API tokens, or service keys in `health_alarm_notify.conf`
6. Are logs showing errors? Check `/var/log/netdata/error.log` or use `journalctl -u netdata`
7. Is Cloud connected? Verify Agent-Cloud link status in the dashboard

## 5.5.5 Related Sections

- **7.5 Notifications Not Being Sent** - Debugging guide
- **9.1 Query Current Alerts** - API-based verification
- **12.2 Notification Strategy** - Best practices