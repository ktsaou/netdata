# 11.5 Hardware and Sensor Alerts

Hardware monitoring provides visibility into infrastructure that is often neglected until failure occurs.

:::note
This is a selection of key alerts. For the complete list, check the stock alert files in `/usr/lib/netdata/conf.d/health.d/`.
:::

## RAID Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/mdstat.conf`

### mdstat_mismatch_cnt

Monitors RAID array mismatch counts which indicate potential data corruption.

**Context:** `md.mismatch_cnt`
**Thresholds:** CRITICAL > 0

### mdstat_disks

Monitors RAID disk status for missing or failed disks.

**Context:** `md.disks`
**Thresholds:**
- WARNING: disk count < expected
- CRITICAL: failed disks > 0

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
     warn: $this < 100
     crit: $this < 40
```

### apcupsd_ups_load_capacity

Monitors UPS load percentage against capacity.

**Context:** `apcupsd.ups_load_capacity_utilization`
**Thresholds:** WARNING > 80% (stays until < 70%)

### apcupsd_ups_status_onbatt

Fires when UPS switches to battery power.

**Context:** `apcupsd.ups_status`
**Thresholds:** WARNING when on battery

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

## IPMI Monitoring

Stock alerts: `/usr/lib/netdata/conf.d/health.d/ipmi.conf`

### ipmi_sensor_state

Monitors IPMI sensor states for critical and warning conditions.

**Context:** `ipmi.sensor_state`
**Thresholds:**
- WARNING: sensor in warning state
- CRITICAL: sensor in critical state

## Adaptec RAID

Stock alerts: `/usr/lib/netdata/conf.d/health.d/adaptec_raid.conf`

### adaptec_raid_ld_status

Monitors Adaptec RAID logical drive status.

**Context:** `adaptec_raid.ld_status`
**Thresholds:** CRITICAL when degraded or failed

### adaptec_raid_pd_state

Monitors Adaptec RAID physical drive states.

**Context:** `adaptec_raid.pd_state`
**Thresholds:** WARNING/CRITICAL for non-optimal states

## Related Files

Hardware alerts are defined in:
- `/usr/lib/netdata/conf.d/health.d/apcupsd.conf`
- `/usr/lib/netdata/conf.d/health.d/ipmi.conf`
- `/usr/lib/netdata/conf.d/health.d/mdstat.conf`
- `/usr/lib/netdata/conf.d/health.d/adaptec_raid.conf`
- `/usr/lib/netdata/conf.d/health.d/megacli.conf`

To customize, copy to `/etc/netdata/health.d/` and modify.
