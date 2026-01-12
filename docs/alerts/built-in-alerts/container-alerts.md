# 11.2 Container and Orchestration Alerts

Container and orchestration alerts address the unique monitoring requirements of dynamic infrastructure.

## Docker Container Alerts

Stock alerts: `/usr/lib/netdata/conf.d/health.d/docker.conf`

### docker_container_unhealthy

Monitors Docker's built-in health check status. Fires when containers report unhealthy status.

**Context:** `docker.container_health_status`
**Thresholds:** WARNING when unhealthy

```conf
template: docker_container_unhealthy
       on: docker.container_health_status
   lookup: average -10s of unhealthy
     warn: $this > 0
  summary: Docker container ${label:container_name} health
     info: ${label:container_name} docker container health status is unhealthy
```

### docker_container_down

Monitors container running state. Fires when containers exit unexpectedly.

**Context:** `docker.container_state`
**Thresholds:** WARNING when container is exited

:::note
This alert is disabled by default (`chart labels: container_name=!*`). To enable for specific containers, modify the chart labels filter.
:::

```conf
    template: docker_container_down
          on: docker.container_state
chart labels: container_name=!*
      lookup: average -10s of exited
        warn: $this > 0
```

## cgroups Container Alerts

Stock alerts: `/usr/lib/netdata/conf.d/health.d/cgroups.conf`

These alerts apply to containers monitored via Linux cgroups (Docker, systemd-nspawn, LXC, etc.).

### cgroup_10min_cpu_usage

Monitors CPU utilization for containers over 10-minute windows.

**Context:** `cgroup.cpu_limit`
**Thresholds:**
- WARNING: > 95% (with hysteresis to 85%)

### cgroup_ram_in_use

Monitors memory usage against container limits.

**Context:** `cgroup.mem_usage`
**Thresholds:**
- WARNING: > 90% (with hysteresis to 80%)
- CRITICAL: > 98% (with hysteresis to 90%)

### k8s_cgroup_10min_cpu_usage

Monitors CPU utilization for Kubernetes containers over 10-minute windows.

**Context:** `k8s.cgroup.cpu_limit`
**Thresholds:**
- WARNING: > 85% (with hysteresis to 75%)

### k8s_cgroup_ram_in_use

Monitors memory usage for Kubernetes containers against limits.

**Context:** `k8s.cgroup.mem_usage`
**Thresholds:**
- WARNING: > 90% (with hysteresis to 80%)
- CRITICAL: > 98% (with hysteresis to 90%)

## Kubernetes Alerts

Stock alerts:
- `/usr/lib/netdata/conf.d/health.d/k8sstate.conf`
- `/usr/lib/netdata/conf.d/health.d/kubelet.conf`

:::note
Kubernetes alerts require the Kubernetes state metrics collector to be enabled.
:::

### k8s_state_deployment_condition_available

Monitors Kubernetes deployment availability status.

**Context:** `k8s_state.deployment_conditions`
**Thresholds:** WARNING when deployment does not have minimum required replicas

### k8s_state_cronjob_last_execution_failed

Monitors Kubernetes CronJob execution failures.

**Context:** `k8s_state.cronjob_last_execution_status`
**Thresholds:** WARNING when last CronJob execution failed

### kubelet_node_config_error

Monitors kubelet node configuration errors.

**Context:** `k8s_kubelet.kubelet_node_config_error`
**Thresholds:** WARNING when the node is experiencing a configuration-related error

### kubelet_token_requests

Monitors kubelet API token request failures.

**Context:** `k8s_kubelet.kubelet_token_requests`
**Thresholds:** WARNING when failed requests > 0 over the last 10 seconds

### kubelet_operations_error

Monitors kubelet runtime operation errors.

**Context:** `k8s_kubelet.kubelet_operations_errors`
**Thresholds:** WARNING when errors > 20 over 1 minute (with hysteresis to 0)

### kubelet_10s_pleg_relist_latency_quantile_05/09/099

Monitors Pod Lifecycle Event Generator (PLEG) relisting latency. These alerts compare the 10-second average latency to the 1-minute baseline for quantiles 0.5, 0.9, and 0.99.

**Context:** `k8s_kubelet.kubelet_pleg_relist_latency_microseconds`
**Thresholds:** Vary by quantile, WARNING/CRITICAL when 10s latency exceeds baseline by 2x-12x

## Related Files

Container alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/docker.conf`
- `/usr/lib/netdata/conf.d/health.d/cgroups.conf`
- `/usr/lib/netdata/conf.d/health.d/k8sstate.conf`
- `/usr/lib/netdata/conf.d/health.d/kubelet.conf`

To customize, copy to `/etc/netdata/health.d/` and modify.
