# 7.5 Notifications Not Being Sent

The alert fires but no one receives notifications.

## 7.5.1 Is It Evaluation or Notification?

```bash
curl -s "http://localhost:19999/api/v1/alarms" | jq '.alarms.your_alert_name'
```

- Status changes? → Evaluation works, notification problem
- Status never changes? → Evaluation problem

## 7.5.2 Notification Checklist

1. Is recipient defined? `to: sysadmin` not `to: silent` or `to: disabled`
2. Is channel enabled? `SEND_SLACK="YES"` in `health_alarm_notify.conf`
3. Check logs:
   ```bash
   # On systemd systems (preferred - notification script logs here)
   journalctl -u netdata --since "1 hour ago" | grep alarm-notify

   # Alternative - Netdata error log
   tail /var/log/netdata/error.log | grep -i alarm
   ```

## 7.5.3 Common Issues

| Issue | Fix |
|-------|-----|
| Silent recipient | Change `to: silent` to `to: sysadmin` (or another valid role) |
| Channel disabled | Set `SEND_SLACK="YES"` (or relevant channel) in `health_alarm_notify.conf` |
| Wrong severity | Configure severity filtering for recipient (e.g., `user@example.com\|critical`) |

## 7.5.4 Related Sections

- **5.5 Testing and Troubleshooting** for delivery verification
- **12.2 Notification Strategy** for routing best practices