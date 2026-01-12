# 4.3 Silencing in Netdata Cloud

Netdata Cloud provides **silencing rules** that let you suppress notifications space-wide without modifying configuration files on individual nodes.

## 4.3.1 Creating Silencing Rules

**Via Cloud UI:**

1. Log in to Netdata Cloud
2. Navigate to **Settings** → **Silencing Rules** (or use the global search)
3. Click **+ Create Silencing Rule**
4. Configure the rule:

| Field | Description | Example |
|-------|-------------|---------|
| **Rule Name** | Descriptive identifier | `Weekend Maintenance` |
| **Match Scope** | What to silence | Alert name, context, node, labels |
| **Schedule** | When rule is active | `Every Saturday 1:00 AM to Monday 6:00 AM` |
| **Duration** | Optional fixed duration | `4 hours` |

## 4.3.2 Silencing Patterns

Silencing rules support Netdata's [simple pattern](/src/libnetdata/simple_pattern/README.md) matching:

| Pattern | Matches | Example |
|---------|---------|---------|
| `*` | Any characters (wildcard) | `*cpu*` matches `high_cpu` and `cpu_usage` |
| `prefix*` | Prefix match | `disk_*` matches `disk_read`, `disk_write` |
| `*suffix` | Suffix match | `*_usage` matches `cpu_usage`, `memory_usage` |
| `!pattern` | Negation (exclude) | `!localhost` excludes localhost |
| `pat1 pat2` | Multiple patterns (OR) | `mysql postgres redis` matches any of these |

## 4.3.3 Personal Silencing Rules

Each user can create **personal silencing rules** that only affect notifications sent to them:

1. Click your **profile icon** → **Notification Settings**
2. Create a personal silencing rule
3. Scope it to alerts you want to quiet

This is useful when:
- You're on vacation or on-call rotation
- You want to reduce noise from certain alert types
- You're debugging and don't want alerts firing to your device

## 4.3.4 Verifying Silencing Is Active

**Check in Cloud UI:**

1. Navigate to **Settings** → **Silencing Rules**
2. Look for active rules (shown with a green indicator)
3. Check the **Next Activation** time for scheduled rules

> **Note:** Cloud silencing rules are managed entirely through the Netdata Cloud UI. For local Agent-level silencing via API, see [Health Management API](/src/web/api/health/README.md#health-management-api).

## 4.3.5 Related Sections

- **4.1 Disabling Alerts** - Permanent alert removal
- **4.2 Silencing vs Disabling** - Conceptual differences
- **4.4 Reducing Flapping and Noise** - Using delays for stability