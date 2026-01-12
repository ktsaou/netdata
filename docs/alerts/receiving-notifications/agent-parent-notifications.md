# 5.2 Configuring Agent and Parent Notifications

Notifications from Agents and Parents are configured in `health_alarm_notify.conf`.

## 5.2.1 Finding the Configuration File

```bash
cd /etc/netdata 2>/dev/null || cd /opt/netdata/etc/netdata
sudo ./edit-config health_alarm_notify.conf
```

This file ships with Netdata and contains the notification subsystem configuration.

## 5.2.2 Notification Methods

Netdata supports many notification destinations. The most common are:

| Method | Type | Setup Complexity | Best For |
|--------|------|------------------|----------|
| **Email** | Built-in | Low | Pager schedules, management |
| **Slack** | Webhook | Low | DevOps teams, chat workflows |
| **Discord** | Webhook | Low | Gaming teams, community ops |
| **MS Teams** | Webhook | Low | Microsoft-based teams |
| **PagerDuty** | Integration | Medium | On-call rotations, enterprise |
| **OpsGenie** | Integration | Medium | Enterprise incident management |
| **Telegram** | Bot | Medium | Direct message alerts |
| **Pushover** | Mobile Push | Medium | Mobile notifications |
| **ntfy** | Push | Low | Self-hosted push notifications |
| **Custom Scripts** | Function | Medium | Any custom integration |

Additional methods include: Alerta, AWS SNS, Dynatrace, Fleep, Flock, Gotify, HipChat, iLert, IRC, Kavenegar, Matrix, MessageBird, Prowl, Pushbullet, RocketChat, SIGNL4, SMSEagle, smstools3, Syslog, Twilio.

See the individual notification method documentation in `/src/health/notifications/` for details.

## 5.2.3 Configuring Email Notifications

Email notifications require a local mail transfer agent (sendmail, postfix, etc.) to be configured on the system.

```conf
# Enable email notifications
# Options: YES, NO, AUTO (auto-detect sendmail availability)
# Default is AUTO - Netdata will check if sendmail is available
SEND_EMAIL="AUTO"

# The email address notifications are sent from
# Leave empty to use the system default (netdata user)
EMAIL_SENDER="netdata@yourdomain.com"

# Default recipients (can be overridden per role)
# Default is "root"
DEFAULT_RECIPIENT_EMAIL="admin@yourdomain.com"
```

:::note
Netdata uses the system's `sendmail` command to send emails. Ensure your system has a properly configured MTA (Mail Transfer Agent) such as Postfix, Sendmail, or an SMTP relay.
:::

## 5.2.4 Configuring Slack Notifications

**Step 1: Create a Slack Incoming Webhook**

1. Go to your Slack workspace → **Settings & Admin** → **Manage apps**
2. Create a new incoming webhook
3. Select the channel for alerts
4. Copy the webhook URL

**Step 2: Configure Netdata**

```conf
SEND_SLACK="YES"
SLACK_WEBHOOK_URL="https://hooks.slack.com/services/YOUR/WEBHOOK/URL"
DEFAULT_RECIPIENT_SLACK="#alerts"
```

## 5.2.5 Configuring PagerDuty

```conf
SEND_PD="YES"
# Set your PagerDuty service integration key
# Get this from PagerDuty → Services → Service Directory → [Your Service] → Integrations
DEFAULT_RECIPIENT_PD="your-service-key"
```

## 5.2.6 Using Custom Notification Scripts

Netdata allows defining a custom notification function in `health_alarm_notify.conf`. This function is called for each notification.

```conf
# Enable custom notifications
SEND_CUSTOM="YES"
DEFAULT_RECIPIENT_CUSTOM="your-custom-recipient"

# Define your custom sender function
custom_sender() {
    # $1 contains the recipient(s)
    local to="${1}"

    # Use variables like ${host}, ${status}, ${alarm}, ${value_string}
    local msg="${host} ${status_message}: ${alarm} ${raised_for}"

    # Your custom logic here (HTTP call, file write, etc.)
    for recipient in ${to}; do
        # Example: call an external API
        httpcode=$(docurl -X POST "https://your-endpoint.com/alert" \
            --data-urlencode "to=${recipient}" \
            --data-urlencode "message=${msg}")
    done
}
```

See the custom notification documentation at `src/health/notifications/custom/README.md` for available variables.

:::note
The `exec` option in alert definitions is different - it replaces the entire notification script. Custom sender functions are part of the standard notification flow.
:::

## 5.2.7 Related Sections

- **5.1 Notification Concepts** - Understanding dispatch models
- **5.3 Cloud Notifications** - Cloud-based configuration
- **8.4 Custom Actions with `exec`** - Advanced script integration