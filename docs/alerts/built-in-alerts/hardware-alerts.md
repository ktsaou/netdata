# 11.5 Hardware and Sensor Alerts

Hardware monitoring provides visibility into infrastructure that is often neglected until failure occurs.

:::note
This is a selection of key alerts. For the complete list, check the stock alert files in `/usr/lib/netdata/conf.d/health.d/`.
:::

## RAID Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/mdstat.conf`

### mdstat_disks

Monitors RAID disk status for failed/down disks.

**Context:** `md.disks`
**Thresholds:** WARNING when down > 0 (any failed device)

### mdstat_mismatch_cnt

Monitors RAID array mismatch counts which indicate potential data corruption.

**Context:** `md.mismatch_cnt`
**Thresholds:** WARNING > 1024 unsynchronized blocks
**Note:** Excludes raid1 and raid10 arrays (mismatch count is not reliable for mirrored arrays).

### mdstat_nonredundant_last_collected

Monitors data collection freshness for non-redundant MD arrays.

**Context:** `md.nonredundant`
**Thresholds:**
- WARNING: stale data (> 5 update intervals)
- CRITICAL: very stale data (> 60 update intervals)

## UPS Monitoring (APC UPS)

Stock alerts: `/usr/lib/netdata/conf.d/health.d/apcupsd.conf`

### apcupsd_ups_battery_charge

Monitors remaining battery capacity.

**Context:** `apcupsd.ups_battery_charge`
**Thresholds:**
- WARNING: < 100%
- CRITICAL: < 40%

```conf
 template: apcupsd_ups_battery_charge
       on: apcupsd.ups_battery_charge
   lookup: average -60s unaligned of charge
    units: %
    every: 60s
     warn: $this < 100
     crit: $this < 40
    delay: down 10m multiplier 1.5 max 1h
```

### apcupsd_ups_load_capacity

Monitors UPS load percentage against capacity.

**Context:** `apcupsd.ups_load_capacity_utilization`
**Thresholds:** WARNING > 80% (with hysteresis: stays in warning until < 70%)
**Lookup:** 10-minute average

### apcupsd_ups_selftest_warning

Fires when UPS self-test fails.

**Context:** `apcupsd.ups_selftest`
**Thresholds:** WARNING when self-test result is BT (battery capacity) or NG (failure)

### apcupsd_ups_status_onbatt

Fires when UPS switches to battery power.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when on battery (with 1 minute delay)

### apcupsd_ups_status_overload

Fires when UPS is overloaded.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when overloaded

### apcupsd_ups_status_lowbatt

Fires when battery charge is critically low.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when low battery

### apcupsd_ups_status_replacebatt

Fires when battery needs replacement.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when battery replacement needed

### apcupsd_ups_status_nobatt

Fires when no battery is detected.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when no battery

### apcupsd_ups_status_commlost

Fires when communication with UPS is lost.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when communication lost

### apcupsd_last_collected_secs

Monitors data collection freshness.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when data collection is stale

## IPMI Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/ipmi.conf`

### ipmi_sensor_state

Monitors IPMI sensor states for critical and warning conditions.

**Context:** `ipmi.sensor_state`
**Thresholds:**
- WARNING: sensor in warning state
- CRITICAL: sensor in critical state

### ipmi_events

Monitors IPMI System Event Log entries.

**Context:** `ipmi.events`
**Thresholds:** WARNING when events > 0 (sent to silent by default)

## Adaptec RAID

Stock alerts: `/usr/lib/netdata/conf.d/health.d/adaptec_raid.conf`

### adaptec_raid_ld_health_status

Monitors Adaptec RAID logical drive health status.

**Context:** `adaptecraid.logical_device_status`
**Thresholds:** CRITICAL when health < 100% (any non-ok state)

### adaptec_raid_pd_health_state

Monitors Adaptec RAID physical drive health states.

**Context:** `adaptecraid.physical_device_state`
**Thresholds:** CRITICAL when health < 100% (any non-ok state)

## MegaCLI RAID

Stock alerts: `/usr/lib/netdata/conf.d/health.d/megacli.conf`

### megacli_adapter_health_state

Monitors MegaCLI RAID adapter (controller) health.

**Context:** `megacli.adapter_health_state`
**Thresholds:** CRITICAL when health < 100% (degraded state)

### megacli_phys_drive_media_errors

Monitors physical drive media errors.

**Context:** `megacli.phys_drive_media_errors`
**Thresholds:** WARNING when media errors > 0

### megacli_phys_drive_predictive_failures

Monitors physical drive predictive failures (SMART).

**Context:** `megacli.phys_drive_predictive_failures`
**Thresholds:** WARNING when predictive failures > 0

### megacli_bbu_charge

Monitors Backup Battery Unit charge level.

**Context:** `megacli.bbu_charge`
**Thresholds:**
- WARNING: <= 80% (with hysteresis at 85%)
- CRITICAL: <= 40% (with hysteresis at 50%)

### megacli_bbu_recharge_cycles

Monitors BBU recharge cycle count (battery wear indicator).

**Context:** `megacli.bbu_recharge_cycles`
**Thresholds:**
- WARNING: >= 100 cycles
- CRITICAL: >= 500 cycles

## StorCLI RAID

Stock alerts: `/usr/lib/netdata/conf.d/health.d/storcli.conf`

### storcli_controller_health_status

Monitors RAID controller health status.

**Context:** `storcli.controller_health_status`
**Thresholds:** CRITICAL when health < 100%

### storcli_controller_bbu_status

Monitors RAID controller BBU (Backup Battery Unit) health.

**Context:** `storcli.controller_bbu_status`
**Thresholds:** CRITICAL when BBU health < 100%

### storcli_phys_drive_errors

Monitors physical drive errors.

**Context:** `storcli.phys_drive_errors`
**Thresholds:** WARNING when errors > 0

### storcli_phys_drive_predictive_failures

Monitors physical drive predictive failures.

**Context:** `storcli.phys_drive_predictive_failures`
**Thresholds:** WARNING when predictive failures > 0

## Related Files

Hardware alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/apcupsd.conf`
- `/usr/lib/netdata/conf.d/health.d/ipmi.conf`
- `/usr/lib/netdata/conf.d/health.d/mdstat.conf`
- `/usr/lib/netdata/conf.d/health.d/adaptec_raid.conf`
- `/usr/lib/netdata/conf.d/health.d/megacli.conf`
- `/usr/lib/netdata/conf.d/health.d/storcli.conf`

To customize, copy to `/etc/netdata/health.d/` and modify.
