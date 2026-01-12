# 5.3 Configuring Cloud Notifications

Cloud-dispatched notifications are managed through the Netdata Cloud UI rather than configuration files.

## 5.3.1 Accessing Notification Configuration

1. Log in to Netdata Cloud
2. Click on the **Space settings** cog (located above your profile icon)
3. Click on the **Alerts & Notifications** tab
4. Click on the **Notification Methods** tab
5. Click **+ Add configuration**

## 5.3.2 Supported Cloud Integrations

| Integration | Plan Required | Use Case |
|-------------|---------------|----------|
| **Email** | Community | Traditional notification delivery |
| **Discord** | Community | Community and team chat |
| **Netdata Mobile App** | Community | Personal push notifications on mobile |
| **Slack** | Business | Team chat workflows |
| **Microsoft Teams** | Business | Enterprise collaboration |
| **PagerDuty** | Business | On-call rotation management |
| **Opsgenie** | Business | Alert routing to on-call engineers |
| **Webhook** | Business | Custom integrations, SIEM systems |
| **Mattermost** | Business | Self-hosted team communication |
| **RocketChat** | Business | Self-hosted team chat |
| **Amazon SNS** | Business | AWS event routing |
| **Telegram** | Business | Messaging app notifications |
| **Splunk** | Business | SIEM integration via HTTP Event Collector |
| **Splunk VictorOps** | Business | Incident management (Splunk On-Call) |
| **ilert** | Business | Alert management and on-call scheduling |
| **ServiceNow** | Business | IT service management and ticketing |

## 5.3.3 Setting Up Slack in Cloud

**Prerequisites:**
- A Netdata Cloud account with Admin access to the Space
- The Space needs to be on a **paid plan**
- A Slack app on your workspace with incoming webhooks enabled

**Slack Configuration:**
1. Create an app in Slack to receive webhooks
2. Install the app on your workspace
3. Go to **Incoming Webhooks** and activate incoming webhooks
4. Add a new webhook to workspace and select the target channel
5. Copy the Webhook URL

**Netdata Configuration:**
1. Click on the **Space settings** cog (located above your profile icon)
2. Click on the **Alerts & Notifications** tab
3. Click on **+ Add configuration**
4. Select the **Slack** integration
5. Configure the notification settings:
   - **Configuration name** (optional): A name for your configuration
   - **Rooms**: Select which Rooms should trigger notifications
   - **Notifications**: Select notification types (Critical, Warning, Clear, Reachable, Unreachable)
6. Enter the **Webhook URL** from your Slack app
7. Click **Save**

## 5.3.4 Cloud Notification Service Levels

Netdata Cloud organizes notifications into two service levels:

| Service Level | Managed By | Description | Example |
|---------------|------------|-------------|---------|
| **Personal** | Individual users | User-specific destinations | Your email address, your mobile app |
| **System** | Space administrators | Space-wide integrations | Slack channels, PagerDuty services |

### Service Classification

Integrations are classified by plan availability:

| Classification | Plan | Examples |
|----------------|------|----------|
| **Community** | Free tier | Email, Discord, Mobile App |
| **Business** | Paid plans | Slack, PagerDuty, Opsgenie, Webhooks |

## 5.3.5 Related Sections

- **5.1 Notification Concepts** - Dispatch model overview
- **5.4 Controlling Recipients** - Severity routing
- **10.2 Silencing Rules Manager** - Cloud silencing features