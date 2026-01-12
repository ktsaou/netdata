# Application-Level Alerts

This section shows examples of application-level alerts based on actual stock alerts shipped with Netdata.

## MySQL Alerts

Netdata provides stock alerts for MySQL slow queries, connection usage, replication, and Galera cluster health.

**Slow queries alert** - Detects when MySQL is executing too many slow queries:

```yaml
 template: mysql_10s_slow_queries
       on: mysql.queries
    class: Latency
     type: Database
component: MySQL
   lookup: sum -10s of slow_queries
    units: slow queries
    every: 10s
     warn: $this > (($status >= $WARNING)  ? (5)  : (10))
     crit: $this > (($status == $CRITICAL) ? (10) : (20))
    delay: down 5m multiplier 1.5 max 1h
  summary: MySQL slow queries
     info: Number of slow queries in the last 10 seconds
       to: dba
```

**Connection utilization alert** - Monitors active connections against the limit:

```yaml
 template: mysql_connections
       on: mysql.connections_active
    class: Utilization
     type: Database
component: MySQL
     calc: $active * 100 / $limit
    units: %
    every: 10s
     warn: $this > (($status >= $WARNING)  ? (60) : (70))
     crit: $this > (($status == $CRITICAL) ? (80) : (90))
    delay: down 15m multiplier 1.5 max 1h
  summary: MySQL connections utilization
     info: Client connections utilization
       to: dba
```

See `/src/health/health.d/mysql.conf` for the complete set of MySQL alerts, including replication lag and Galera cluster monitoring.

## PostgreSQL Alerts

Netdata provides stock alerts for PostgreSQL connections, deadlocks, cache performance, and table maintenance.

**Deadlocks rate alert** - Detects deadlocks in databases:

```yaml
 template: postgres_db_deadlocks_rate
       on: postgres.db_deadlocks_rate
    class: Errors
     type: Database
component: PostgreSQL
   lookup: sum -1m unaligned of deadlocks
    units: deadlocks
    every: 1m
     warn: $this > (($status >= $WARNING)  ? (0) : (10))
    delay: down 15m multiplier 1.5 max 1h
  summary: PostgreSQL DB ${label:database} deadlocks rate
     info: Number of deadlocks detected in db ${label:database} in the last minute
       to: dba
```

**Connection utilization alert** - Monitors connection pool usage:

```yaml
 template: postgres_total_connection_utilization
       on: postgres.connections_utilization
    class: Utilization
     type: Database
component: PostgreSQL
   lookup: average -1m unaligned of used
    units: %
    every: 1m
     warn: $this > (($status >= $WARNING)  ? (70) : (80))
     crit: $this > (($status == $CRITICAL) ? (80) : (90))
    delay: down 15m multiplier 1.5 max 1h
  summary: PostgreSQL connection utilization
     info: Average total connection utilization over the last minute
       to: dba
```

See `/src/health/health.d/postgres.conf` for the complete set of PostgreSQL alerts, including cache hit ratios, bloat detection, and autovacuum monitoring.

## Redis Alerts

Netdata provides stock alerts for Redis connections, background saves, and replication health.

**Rejected connections alert** - Detects when Redis is rejecting connections due to maxclients limit:

```yaml
 template: redis_connections_rejected
       on: redis.connections
    class: Errors
     type: KV Storage
component: Redis
   lookup: sum -1m unaligned of rejected
    every: 10s
    units: connections
     warn: $this > 0
  summary: Redis rejected connections
     info: Connections rejected because of maxclients limit in the last minute
    delay: down 5m multiplier 1.5 max 1h
       to: dba
```

**Background save health alert** - Monitors RDB save operations:

```yaml
 template: redis_bgsave_broken
       on: redis.bgsave_health
    class: Errors
     type: KV Storage
component: Redis
    every: 10s
     calc: $last_bgsave != nan AND $last_bgsave != 0
     crit: $this
    units: ok/failed
  summary: Redis background save
     info: Status of the last RDB save operation (0: ok, 1: error)
    delay: down 5m multiplier 1.5 max 1h
       to: dba
```

See `/src/health/health.d/redis.conf` for the complete set of Redis alerts, including slow background saves and master link monitoring.

## Web Server Alerts (Nginx, Apache, etc.)

Netdata monitors web servers through access log analysis using the `web_log` collector. These alerts apply to any web server whose logs are parsed.

**Server errors alert** - Detects 5xx error responses:

```yaml
 template: web_log_1m_internal_errors
       on: web_log.type_requests
    class: Errors
     type: Web Server
component: Web log
   lookup: sum -1m unaligned of error
     calc: $this * 100 / $web_log_1m_requests
    units: %
    every: 10s
     warn: ($web_log_1m_requests > 120) ? ($this > (($status >= $WARNING)  ? ( 1 ) : ( 2 )) ) : ( 0 )
     crit: ($web_log_1m_requests > 120) ? ($this > (($status == $CRITICAL) ? ( 2 ) : ( 5 )) ) : ( 0 )
    delay: up 2m down 15m multiplier 1.5 max 1h
  summary: Web log server errors
     info: Ratio of server error HTTP requests over the last minute (5xx)
       to: webmaster
```

**Response time alert** - Detects slow response times:

```yaml
 template: web_log_web_slow
       on: web_log.request_processing_time
    class: Latency
     type: Web Server
component: Web log
   lookup: average -1m unaligned of avg
    units: ms
    every: 10s
    green: 500
      red: 1000
     warn: ($web_log_1m_requests > 120) ? ($this > $green && $this > ($web_log_10m_response_time * 2) ) : ( 0 )
     crit: ($web_log_1m_requests > 120) ? ($this > $red   && $this > ($web_log_10m_response_time * 4) ) : ( 0 )
    delay: down 15m multiplier 1.5 max 1h
  summary: Web log processing time
     info: Average HTTP response time over the last 1 minute
  options: no-clear-notification
       to: webmaster
```

See `/src/health/health.d/web_log.conf` for the complete set of web server alerts, including request ratios and traffic anomaly detection.

## Key Patterns in Application Alerts

These stock alerts demonstrate several important patterns:

1. **Hysteresis thresholds** - Using different warning/critical thresholds based on current alert status to prevent flapping
2. **Delay directives** - Using `delay: down Xm` to prevent false clears during brief recoveries
3. **Classification** - Using `class`, `type`, and `component` for alert organization
4. **Context-aware conditions** - Web log alerts only trigger when there is sufficient traffic (`$web_log_1m_requests > 120`)
5. **Label references** - PostgreSQL alerts use `${label:database}` to include database names in summaries

## Related Sections

- [Hysteresis](../advanced-techniques/hysteresis.md) - Preventing alert flapping
- [Alert Classification](../reference/REFERENCE.md) - Using class, type, and component