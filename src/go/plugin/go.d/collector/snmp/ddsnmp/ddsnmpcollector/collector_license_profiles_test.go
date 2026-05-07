// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"strings"
	"testing"
	"time"

	"github.com/gosnmp/gosnmp"
	snmpmock "github.com/gosnmp/gosnmp/mocks"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/logger"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
)

func TestCollector_Collect_CheckPointLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.2620.1.6.18.1",
		[]gosnmp.SnmpPDU{
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.1.17", 17),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.2.17", 17),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.3.17", "Application Control"),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.4.17", "about-to-expire"),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.5.17", 1775152800),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.6.17", "Threat prevention coverage"),
			createIntegerPDU("1.3.6.1.4.1.2620.1.6.18.1.1.7.17", 1),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.8.17", 100),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.9.17", 85),
		},
	)

	profile := mustLoadLicensingProfile(t, "checkpoint", func(metric ddprofiledefinition.MetricsConfig) bool {
		return strings.TrimPrefix(metric.Table.OID, ".") == "1.3.6.1.4.1.2620.1.6.18.1"
	})

	collector := New(Config{
		SnmpClient:  mockHandler,
		Profiles:    []*ddsnmp.Profile{profile},
		Log:         logger.New(),
		SysObjectID: "",
	})

	results, err := collector.Collect()
	require.NoError(t, err)
	require.Len(t, results, 1)

	pm := results[0]
	assert.Empty(t, pm.Metrics)
	require.Len(t, pm.HiddenMetrics, 4)

	row := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)["17"]
	require.NotNil(t, row)
	require.EqualValues(t, 100, row["capacity"].Value)
	require.EqualValues(t, 85, row["usage"].Value)
	require.EqualValues(t, 1, row["state_severity"].Value)
	require.EqualValues(t, 1775152800, row["expiry_timestamp"].Value)
	assert.Equal(t, "Application Control", metricTagValue(*row["state_severity"], "_license_name"))
	assert.Equal(t, "about-to-expire", metricTagValue(*row["state_severity"], "_license_state_raw"))
	assert.Equal(t, "Threat prevention coverage", metricTagValue(*row["state_severity"], "_license_impact"))
}

func TestCollector_Collect_FortiGateLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectFortiGateLicensingWalks(mockHandler)

	profile := mustLoadLicensingProfile(t, "fortinet-fortigate", func(metric ddprofiledefinition.MetricsConfig) bool {
		return strings.HasPrefix(strings.TrimPrefix(metric.Table.OID, "."), "1.3.6.1.4.1.12356.101.4.6.3.")
	})

	collector := New(Config{
		SnmpClient:  mockHandler,
		Profiles:    []*ddsnmp.Profile{profile},
		Log:         logger.New(),
		SysObjectID: "",
	})

	results, err := collector.Collect()
	require.NoError(t, err)
	require.Len(t, results, 1)

	pm := results[0]
	assert.Empty(t, pm.Metrics)
	require.Len(t, pm.HiddenMetrics, 3)

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	contract := byID["FortiCare Support"]
	require.NotNil(t, contract)
	require.EqualValues(t, 1920585600, contract["expiry_timestamp"].Value)
	assert.Equal(t, "FortiCare Support", metricTagValue(*contract["expiry_timestamp"], "_license_name"))
	assert.Equal(t, "contract", metricTagValue(*contract["expiry_timestamp"], "_license_type"))
	assert.Equal(t, "device", metricTagValue(*contract["expiry_timestamp"], "_license_component"))

	service := byID["FortiGuard Antivirus"]
	require.NotNil(t, service)
	require.EqualValues(t, 1753491600, service["expiry_timestamp"].Value)
	assert.Equal(t, "1.00000", metricTagValue(*service["expiry_timestamp"], "_license_feature"))
	assert.Equal(t, "service", metricTagValue(*service["expiry_timestamp"], "_license_type"))
	assert.Equal(t, "fortiguard", metricTagValue(*service["expiry_timestamp"], "_license_component"))

	accountContract := byID["FortiCare Premium"]
	require.NotNil(t, accountContract)
	require.EqualValues(t, 1920585600, accountContract["expiry_timestamp"].Value)
	assert.Equal(t, "account_contract", metricTagValue(*accountContract["expiry_timestamp"], "_license_type"))
	assert.Equal(t, "account", metricTagValue(*accountContract["expiry_timestamp"], "_license_component"))
}

func TestCollector_Collect_MikroTikLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPGet(mockHandler,
		[]string{
			"1.3.6.1.4.1.14988.1.1.4.2.0",
		},
		[]gosnmp.SnmpPDU{
			createDateAndTimePDU("1.3.6.1.4.1.14988.1.1.4.2.0", time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC)),
		},
	)

	profile := mustLoadLicensingProfile(t, "mikrotik-router", func(metric ddprofiledefinition.MetricsConfig) bool {
		return metric.MIB == "MIKROTIK-MIB" && strings.TrimPrefix(metric.Symbol.OID, ".") == "1.3.6.1.4.1.14988.1.1.4.2.0"
	})

	collector := New(Config{
		SnmpClient:  mockHandler,
		Profiles:    []*ddsnmp.Profile{profile},
		Log:         logger.New(),
		SysObjectID: "",
	})

	results, err := collector.Collect()
	require.NoError(t, err)
	require.Len(t, results, 1)

	pm := results[0]
	assert.Empty(t, pm.Metrics)
	require.Len(t, pm.HiddenMetrics, 1)

	row := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)["routeros_upgrade"]
	require.NotNil(t, row)
	require.EqualValues(t, time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(), row["expiry_timestamp"].Value)
	assert.Equal(t, "RouterOS upgrade entitlement", metricTagValue(*row["expiry_timestamp"], "_license_name"))
	assert.Equal(t, "upgrade_entitlement", metricTagValue(*row["expiry_timestamp"], "_license_type"))
	assert.Equal(t, "routeros", metricTagValue(*row["expiry_timestamp"], "_license_component"))
	assert.Equal(t, "mtxrLicUpgrUntil", metricTagValue(*row["expiry_timestamp"], "_license_expiry_source"))
}

func expectFortiGateLicensingWalks(mockHandler *snmpmock.MockHandler) {
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.1.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.1.2.1.1.1", "FortiCare Support"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.1.2.1.2.1", "Mon 11 November 2030"),
		},
	)
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.2.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.1.2", "FortiGuard Antivirus"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.2.2", "Sat Jul 26 01:00:00 2025"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.3.2", "1.00000"),
		},
	)
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.3.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.3.2.1.1.7", "FortiCare Premium"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.3.2.1.2.7", "Mon 11 November 2030"),
		},
	)
}
