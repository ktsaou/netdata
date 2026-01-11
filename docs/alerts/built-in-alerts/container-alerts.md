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
- WARNING: > 85%
- CRITICAL: > 95%

### cgroup_ram_in_use

Monitors memory usage against container limits.

**Context:** `cgroup.mem_usage`
**Thresholds:**
- WARNING: > 80%
- CRITICAL: > 90%

## Kubernetes Alerts

Stock alerts:
- `/usr/lib/netdata/conf.d/health.d/k8sstate.conf`
- `/usr/lib/netdata/conf.d/health.d/kubelet.conf`

:::note
Kubernetes alerts require the Kubernetes state metrics collector to be enabled.
:::

### k8s_pod_container_waiting

Monitors containers stuck in waiting state (CrashLoopBackOff, ImagePullBackOff, etc.).

**Context:** `k8s_state.pod_container_waiting_state_reason`
**Thresholds:** WARNING when container is waiting

### k8s_pod_container_terminated

Monitors containers that terminated with non-zero exit codes.

**Context:** `k8s_state.pod_container_terminated_state_reason`
**Thresholds:** WARNING when OOMKilled, Error, or similar

### k8s_node_not_ready

Monitors Kubernetes node ready condition.

**Context:** `k8s_state.node_condition`
**Thresholds:** WARNING when node is not ready

### kubelet_token_requests

Monitors kubelet API token request failures.

**Context:** `k8s_kubelet.token_requests`
**Thresholds:** WARNING when failed requests > 0

### kubelet_operations_error

Monitors kubelet runtime operation errors.

**Context:** `k8s_kubelet.kubelet_operations_errors`
**Thresholds:** WARNING when errors > 0 over 10 minutes

## Related Files

Container alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/docker.conf`
- `/usr/lib/netdata/conf.d/health.d/cgroups.conf`
- `/usr/lib/netdata/conf.d/health.d/k8sstate.conf`
- `/usr/lib/netdata/conf.d/health.d/kubelet.conf`

To customize, copy to `/etc/netdata/health.d/` and modify.
