// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"testing"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/pkg/snmputils"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestCollectProfileTableMetricsSkipsKnownDownInterfaces(t *testing.T) {
	collr, mx := collectProfileTableMetricsForTest(
		ifTrafficMetric("eth0", 100, 200),
		ifTrafficMetric("eth1", 300, 400),
		ifAdminStatusMetric("eth0", "up"),
		ifOperStatusMetric("eth0", "up"),
		ifAdminStatusMetric("eth1", "up"),
		ifOperStatusMetric("eth1", "down"),
	)

	assert.Equal(t, int64(100), mx["snmp_device_prof_ifTraffic_eth0_in"])
	assert.Equal(t, int64(200), mx["snmp_device_prof_ifTraffic_eth0_out"])
	assert.NotContains(t, mx, "snmp_device_prof_ifTraffic_eth1_in")
	assert.NotContains(t, mx, "snmp_device_prof_ifTraffic_eth1_out")
	assert.NotContains(t, mx, "snmp_device_prof_ifAdminStatus_eth1_up")
	assert.NotContains(t, mx, "snmp_device_prof_ifOperStatus_eth1_down")

	collr.ifaceCache.mu.RLock()
	defer collr.ifaceCache.mu.RUnlock()

	entry := collr.ifaceCache.interfaces["eth1"]
	require.NotNil(t, entry)
	assert.Equal(t, "up", entry.adminStatus)
	assert.Equal(t, "down", entry.operStatus)
}

func TestCollectProfileTableMetricsKeepsUpInterfaces(t *testing.T) {
	_, mx := collectProfileTableMetricsForTest(
		ifTrafficMetric("eth0", 100, 200),
		ifAdminStatusMetric("eth0", "up"),
		ifOperStatusMetric("eth0", "up"),
	)

	assert.Equal(t, int64(100), mx["snmp_device_prof_ifTraffic_eth0_in"])
	assert.Equal(t, int64(200), mx["snmp_device_prof_ifTraffic_eth0_out"])
	assert.Equal(t, int64(1), mx["snmp_device_prof_ifAdminStatus_eth0_up"])
	assert.Equal(t, int64(1), mx["snmp_device_prof_ifOperStatus_eth0_up"])
}

func TestCollectProfileTableMetricsKeepsInterfacesWithNoStatus(t *testing.T) {
	_, mx := collectProfileTableMetricsForTest(ifTrafficMetric("eth0", 100, 200))

	assert.Equal(t, int64(100), mx["snmp_device_prof_ifTraffic_eth0_in"])
	assert.Equal(t, int64(200), mx["snmp_device_prof_ifTraffic_eth0_out"])
}

func TestCollectProfileTableMetricsSkipsInterfacesWithPartialStatus(t *testing.T) {
	_, mx := collectProfileTableMetricsForTest(
		ifTrafficMetric("eth0", 100, 200),
		ifAdminStatusMetric("eth0", "up"),
	)

	assert.NotContains(t, mx, "snmp_device_prof_ifTraffic_eth0_in")
	assert.NotContains(t, mx, "snmp_device_prof_ifTraffic_eth0_out")
}

func TestCollectProfileTableMetricsKeepsNonInterfaceTableMetrics(t *testing.T) {
	_, mx := collectProfileTableMetricsForTest(ddsnmp.Metric{
		Name:    "cpuUsage",
		IsTable: true,
		Tags:    map[string]string{"slot": "1"},
		Value:   42,
	})

	assert.Equal(t, int64(42), mx["snmp_device_prof_cpuUsage_1"])
}

func TestCollectProfileTableMetricsRemovesChartsForInterfacesThatGoDown(t *testing.T) {
	collr := newTestSNMPCollector()

	collr.resetIfaceCache()
	mx := make(map[string]int64)
	collr.collectProfileTableMetrics(mx, profileMetrics(
		ifTrafficMetric("eth0", 100, 200),
		ifAdminStatusMetric("eth0", "up"),
		ifOperStatusMetric("eth0", "up"),
	))
	collr.finalizeIfaceCache()

	const key = "ifTraffic_eth0"
	require.True(t, collr.seenTableMetrics[key])

	chart := collr.Charts().Get("snmp_device_prof_ifTraffic_eth0")
	require.NotNil(t, chart)
	assert.False(t, chart.IsRemoved())

	collr.resetIfaceCache()
	mx = make(map[string]int64)
	collr.collectProfileTableMetrics(mx, profileMetrics(
		ifTrafficMetric("eth0", 300, 400),
		ifAdminStatusMetric("eth0", "up"),
		ifOperStatusMetric("eth0", "down"),
	))
	collr.finalizeIfaceCache()

	assert.NotContains(t, mx, "snmp_device_prof_ifTraffic_eth0_in")
	assert.False(t, collr.seenTableMetrics[key])

	chart = collr.Charts().Get("snmp_device_prof_ifTraffic_eth0")
	require.NotNil(t, chart)
	assert.True(t, chart.IsRemoved())
}

func collectProfileTableMetricsForTest(metrics ...ddsnmp.Metric) (*Collector, map[string]int64) {
	collr := newTestSNMPCollector()
	mx := make(map[string]int64)

	collr.resetIfaceCache()
	collr.collectProfileTableMetrics(mx, profileMetrics(metrics...))
	collr.finalizeIfaceCache()

	return collr, mx
}

func newTestSNMPCollector() *Collector {
	collr := New()
	collr.sysInfo = &snmputils.SysInfo{Name: "mock sysName"}
	return collr
}

func profileMetrics(metrics ...ddsnmp.Metric) []*ddsnmp.ProfileMetrics {
	pm := &ddsnmp.ProfileMetrics{
		Tags:    map[string]string{},
		Metrics: metrics,
	}
	for i := range pm.Metrics {
		if pm.Metrics[i].Profile == nil {
			pm.Metrics[i].Profile = pm
		}
	}
	return []*ddsnmp.ProfileMetrics{pm}
}

func ifTrafficMetric(iface string, in, out int64) ddsnmp.Metric {
	return ddsnmp.Metric{
		Name:    "ifTraffic",
		IsTable: true,
		Tags:    map[string]string{tagInterface: iface},
		MultiValue: map[string]int64{
			"in":  in,
			"out": out,
		},
	}
}

func ifAdminStatusMetric(iface, status string) ddsnmp.Metric {
	return ifaceStatusMetric("ifAdminStatus", iface, status, []string{"up", "down", "testing"})
}

func ifOperStatusMetric(iface, status string) ddsnmp.Metric {
	return ifaceStatusMetric("ifOperStatus", iface, status, []string{"up", "down", "testing", "unknown", "dormant", "notPresent", "lowerLayerDown"})
}

func ifaceStatusMetric(name, iface, status string, values []string) ddsnmp.Metric {
	mv := make(map[string]int64, len(values))
	for _, v := range values {
		if v == status {
			mv[v] = 1
		} else {
			mv[v] = 0
		}
	}

	return ddsnmp.Metric{
		Name:       name,
		IsTable:    true,
		Tags:       map[string]string{tagInterface: iface},
		MultiValue: mv,
	}
}
