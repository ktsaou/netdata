# 11.3 Application Alerts

Application alerts cover common databases, web servers, caches, and message queues. Each application has unique metrics that indicate health and performance.

:::note
This is a selection of key alerts. For the complete list, check the stock alert files in `/usr/lib/netdata/conf.d/health.d/`.
:::

## MySQL and MariaDB

Stock alerts: `/usr/lib/netdata/conf.d/health.d/mysql.conf`

### mysql_10s_slow_queries

Tracks slow queries over a 10-second window. Excessive slow queries often precede performance degradation.

**Context:** `mysql.queries`
**Thresholds:**
- WARNING: > 10 (stays until < 5)
- CRITICAL: > 20 (stays until < 10)

```conf
 template: mysql_10s_slow_queries
       on: mysql.queries
   lookup: sum -10s of slow_queries
    units: slow queries
     warn: $this > (($status >= $WARNING)  ? (5)  : (10))
     crit: $this > (($status == $CRITICAL) ? (10) : (20))
```

### mysql_connections

Monitors connection pool utilization as percentage of maximum allowed connections.

**Context:** `mysql.connections_active`
**Thresholds:**
- WARNING: > 70% (stays until < 60%)
- CRITICAL: > 90% (stays until < 80%)

### mysql_10s_waited_locks_ratio

Tracks the ratio of table lock waits to immediate locks, indicating contention.

**Context:** `mysql.table_locks`
**Thresholds:**
- WARNING: > 25%
- CRITICAL: > 50%

### mysql_replication

Monitors replication status (SQL thread and I/O thread running).

**Context:** `mysql.slave_status`
**Thresholds:** CRITICAL when replication is stopped

### mysql_replication_lag

Measures seconds behind master for replicas.

**Context:** `mysql.slave_behind`
**Thresholds:**
- WARNING: > 10 seconds
- CRITICAL: > 30 seconds

### mysql_galera_cluster_size

For Galera clusters, monitors cluster node count changes.

**Context:** `mysql.galera_cluster_size`
**Thresholds:**
- WARNING: cluster grew
- CRITICAL: cluster shrank

## PostgreSQL

Stock alerts: `/usr/lib/netdata/conf.d/health.d/postgres.conf`

### postgres_total_connection_utilization

Monitors connection pool usage against maximum connections.

**Context:** `postgres.connections_utilization`
**Thresholds:**
- WARNING: > 80%
- CRITICAL: > 90%

### postgres_database_deadlocks_rate

Detects deadlocks indicating concurrent transaction conflicts.

**Context:** `postgres.database_deadlocks_rate`
**Thresholds:** WARNING > 0

### postgres_replication_lag

Monitors streaming replication lag.

**Context:** `postgres.replication_standby_app_wal_lag_size`
**Thresholds:**
- WARNING: > 200 MB
- CRITICAL: > 400 MB

## Redis

Stock alerts: `/usr/lib/netdata/conf.d/health.d/redis.conf`

### redis_connections_rejected

Monitors rejected connections due to maxclients limit.

**Context:** `redis.connections`
**Thresholds:** WARNING > 0

### redis_master_link_down

For replicas, monitors connection to master.

**Context:** `redis.master_link_status`
**Thresholds:** CRITICAL when link is down

### redis_bgsave_broken

Monitors background save failures.

**Context:** `redis.bgsave_health`
**Thresholds:** CRITICAL > 0

## RabbitMQ

Stock alerts: `/usr/lib/netdata/conf.d/health.d/rabbitmq.conf`

### rabbitmq_node_disk_free_alarm_status

Monitors disk space alarms from RabbitMQ.

**Context:** `rabbitmq.node_disk_free_alarm_status`
**Thresholds:** CRITICAL when alarm is active

### rabbitmq_node_mem_alarm_status

Monitors memory alarms from RabbitMQ.

**Context:** `rabbitmq.node_mem_alarm_status`
**Thresholds:** CRITICAL when alarm is active

## Memcached

Stock alerts: `/usr/lib/netdata/conf.d/health.d/memcached.conf`

### memcached_fill_rate

Monitors the rate at which the cache is filling.

**Context:** `memcached.evictions`
**Thresholds:** WARNING when fill rate exceeds eviction rate significantly

## Related Files

Application alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/mysql.conf`
- `/usr/lib/netdata/conf.d/health.d/postgres.conf`
- `/usr/lib/netdata/conf.d/health.d/redis.conf`
- `/usr/lib/netdata/conf.d/health.d/rabbitmq.conf`
- `/usr/lib/netdata/conf.d/health.d/memcached.conf`

To customize, copy to `/etc/netdata/health.d/` and modify.
