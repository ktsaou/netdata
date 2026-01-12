# 11.4 Network and Connectivity Alerts

Network alerts focus on endpoints and services rather than interface statistics.

:::note
This is a selection of key alerts. For the complete list, check the stock alert files in `/usr/lib/netdata/conf.d/health.d/`.
:::

## DNS Query Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/dns_query.conf`

### dns_query_query_status

Monitors DNS query success for configured servers.

**Context:** `dns_query.query_status`
**Thresholds:** WARNING when queries fail (`$this != nan && $this != 1`)

```conf
 template: dns_query_query_status
       on: dns_query.query_status
    class: Errors
     type: DNS
component: DNS
     calc: $success
    units: status
    every: 10s
     warn: $this != nan && $this != 1
    delay: up 30s down 5m multiplier 1.5 max 1h
  summary: DNS query unsuccessful requests to ${label:server}
     info: DNS request type ${label:record_type} to server ${label:server} is unsuccessful
       to: sysadmin
```

## HTTP Endpoint Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/httpcheck.conf`

### httpcheck_web_service_up

Fast-reacting liveness check for HTTP endpoints. Silent notification - ideal for dashboards or badges.

**Context:** `httpcheck.status`
**Thresholds:** Returns 0 (down) when success rate < 75% over 1 minute

### httpcheck_web_service_bad_content

Monitors responses with unexpected content.

**Context:** `httpcheck.status`
**Thresholds:**
- WARNING: >= 10% bad content
- CRITICAL: >= 40% bad content

### httpcheck_web_service_bad_status

Monitors responses with unexpected HTTP status codes.

**Context:** `httpcheck.status`
**Thresholds:**
- WARNING: >= 10% bad status
- CRITICAL: >= 40% bad status

### httpcheck_web_service_bad_header

Monitors responses with unexpected headers.

**Context:** `httpcheck.status`
**Thresholds:**
- WARNING: >= 10% bad headers
- CRITICAL: >= 40% bad headers

### httpcheck_web_service_timeouts

Monitors connection timeouts.

**Context:** `httpcheck.status`
**Thresholds:**
- WARNING: >= 10% timeouts
- CRITICAL: >= 40% timeouts

```conf
 template: httpcheck_web_service_timeouts
       on: httpcheck.status
    class: Latency
     type: Web Server
component: HTTP endpoint
   lookup: average -5m unaligned percentage of timeout
    every: 10s
    units: %
     warn: $this >= 10 AND $this < 40
     crit: $this >= 40
    delay: down 5m multiplier 1.5 max 1h
  summary: HTTP check for ${label:url} timeouts
     info: Percentage of timed-out HTTP requests to ${label:url} in the last 5 minutes
       to: webmaster
```

### httpcheck_web_service_no_connection

Monitors failed connection attempts.

**Context:** `httpcheck.status`
**Thresholds:**
- WARNING: >= 10% connection failures
- CRITICAL: >= 40% connection failures

## SSL/TLS Certificate Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/x509check.conf`

### x509check_days_until_expiration

Monitors SSL/TLS certificate expiration dates.

**Context:** `x509check.time_until_expiration`
**Thresholds:**
- WARNING: < 14 days
- CRITICAL: < 7 days

### x509check_revocation_status

Monitors certificate revocation status.

**Context:** `x509check.revocation_status`
**Thresholds:** CRITICAL when `$revoked == 1`

## TCP Port Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/portcheck.conf`

### portcheck_service_reachable

Fast-reacting liveness check for TCP ports. Silent notification - ideal for dashboards or badges.

**Context:** `portcheck.status`
**Thresholds:** Returns 0 (down) when success rate < 75% over 1 minute

### portcheck_connection_timeouts

Monitors TCP connection timeouts.

**Context:** `portcheck.status`
**Thresholds:**
- WARNING: >= 10% timeouts
- CRITICAL: >= 40% timeouts

### portcheck_connection_fails

Monitors failed TCP connections (no_connection or failed states).

**Context:** `portcheck.status`
**Thresholds:**
- WARNING: >= 10% failures
- CRITICAL: >= 40% failures

## Related Files

Network and connectivity alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/dns_query.conf`
- `/usr/lib/netdata/conf.d/health.d/httpcheck.conf`
- `/usr/lib/netdata/conf.d/health.d/x509check.conf`
- `/usr/lib/netdata/conf.d/health.d/portcheck.conf`
- `/usr/lib/netdata/conf.d/health.d/ping.conf` - Host reachability and latency
- `/usr/lib/netdata/conf.d/health.d/net.conf` - Network interface statistics

To customize, copy to `/etc/netdata/health.d/` and modify.
