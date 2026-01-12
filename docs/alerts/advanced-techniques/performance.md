# Performance Considerations

## What Affects Performance

| Factor | Impact | Recommendation |
|--------|--------|----------------|
| `every` frequency | Lower (more frequent) = more CPU | Use 1m for most alerts |
| `lookup` window | Longer windows require more data processing | Match window to detection needs |
| Number of alerts | More alerts = more evaluation cycles | Disable unused alerts |

The health engine runs at a minimum interval of 10 seconds (configurable via `run at least every` in `netdata.conf`). Individual alerts can specify longer intervals with the `every` option.

## Efficient Configuration

```yaml
# Efficient: 1-minute evaluation interval
 template: system_health
       on: system.cpu
   lookup: average -5m of user,system
    every: 1m
     warn: $this > 80

# Less efficient: 10-second evaluation interval
 template: system_fast
       on: system.cpu
   lookup: average -1m of user,system
    every: 10s
     warn: $this > 80
```

**Evaluation frequency impact:**
- `every: 1m` results in 60 evaluations per hour
- `every: 10s` results in 360 evaluations per hour

For most monitoring scenarios, 1-minute evaluation provides adequate detection speed while conserving resources.

## Related Sections

- [Evaluation Architecture](../architecture/evaluation-architecture.md) for internal behavior