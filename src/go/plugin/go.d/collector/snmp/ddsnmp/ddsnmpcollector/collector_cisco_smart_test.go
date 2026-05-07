// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"strings"
	"testing"

	"github.com/gosnmp/gosnmp"
	"github.com/netdata/netdata/go/plugins/logger"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestCollector_Collect_CiscoSmartLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPGet(mockHandler,
		[]string{
			"1.3.6.1.4.1.9.9.831.0.6.1",
			"1.3.6.1.4.1.9.9.831.0.7.2",
			"1.3.6.1.4.1.9.9.831.0.7.1",
			"1.3.6.1.4.1.9.9.831.0.6.3",
			"1.3.6.1.4.1.9.9.831.0.7.4.2",
		},
		[]gosnmp.SnmpPDU{
			createIntegerPDU("1.3.6.1.4.1.9.9.831.0.6.1", 2),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.7.2", "Out of Compliance"),
			createGauge32PDU("1.3.6.1.4.1.9.9.831.0.7.1", 1775152800),
			createGauge32PDU("1.3.6.1.4.1.9.9.831.0.6.3", 1777831200),
			createGauge32PDU("1.3.6.1.4.1.9.9.831.0.7.4.2", 1773943200),
		},
	)

	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.9.9.831.0.5.1",
		[]gosnmp.SnmpPDU{
			createGauge32PDU("1.3.6.1.4.1.9.9.831.0.5.1.1.2.1", 42),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.3.1", "dna_advantage"),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.4.1", "17.12"),
			createIntegerPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.5.1", 8),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.6.1", "Cisco DNA Advantage entitlement"),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.7.1", "network-advantage"),
		},
	)

	profile := mustLoadCiscoSmartProfile(t)
	require.True(t, hasMetricTable(profile, "1.3.6.1.4.1.9.9.831.0.5.1"))

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
	require.Len(t, pm.HiddenMetrics, 7)

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	require.EqualValues(t, 0, byID["smart_registration"]["state_severity"].Value)
	assert.Equal(t, "Smart Licensing registration", metricTagValue(*byID["smart_registration"]["state_severity"], "_license_name"))

	require.EqualValues(t, 2, byID["smart_authorization_state"]["state_severity"].Value)
	require.EqualValues(t, 1775152800, byID["smart_authorization_expiry"]["authorization_timestamp"].Value)
	require.EqualValues(t, 1777831200, byID["smart_id_certificate_expiry"]["certificate_timestamp"].Value)
	require.EqualValues(t, 1773943200, byID["smart_evaluation_expiry"]["grace_timestamp"].Value)

	entitlement := byID["dna_advantage"]
	require.NotNil(t, entitlement)
	require.EqualValues(t, 42, entitlement["usage"].Value)
	require.EqualValues(t, 2, entitlement["state_severity"].Value)
	assert.Equal(t, "network-advantage", metricTagValue(*entitlement["usage"], "_license_name"))
	assert.Equal(t, "authorization_expired", metricTagValue(*entitlement["state_severity"], "_license_state_raw"))
}

func TestCollector_Collect_CiscoSmartLicensingProfile_PartialData(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPGet(mockHandler,
		[]string{
			"1.3.6.1.4.1.9.9.831.0.6.1",
			"1.3.6.1.4.1.9.9.831.0.7.2",
			"1.3.6.1.4.1.9.9.831.0.7.1",
			"1.3.6.1.4.1.9.9.831.0.6.3",
			"1.3.6.1.4.1.9.9.831.0.7.4.2",
		},
		[]gosnmp.SnmpPDU{
			createIntegerPDU("1.3.6.1.4.1.9.9.831.0.6.1", 2),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.7.2", "Authorized"),
		},
	)

	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.9.9.831.0.5.1",
		[]gosnmp.SnmpPDU{
			createGauge32PDU("1.3.6.1.4.1.9.9.831.0.5.1.1.2.1", 7),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.3.1", "dna_essentials"),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.4.1", "17.9"),
			createIntegerPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.5.1", 3),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.6.1", "Cisco DNA Essentials entitlement"),
			createStringPDU("1.3.6.1.4.1.9.9.831.0.5.1.1.7.1", "network-essentials"),
		},
	)

	profile := mustLoadCiscoSmartProfile(t)
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

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	require.EqualValues(t, 0, byID["smart_registration"]["state_severity"].Value)
	require.EqualValues(t, 0, byID["smart_authorization_state"]["state_severity"].Value)

	entitlement := byID["dna_essentials"]
	require.NotNil(t, entitlement)
	require.EqualValues(t, 7, entitlement["usage"].Value)
	require.EqualValues(t, 0, entitlement["state_severity"].Value)
	assert.Equal(t, "authorized", metricTagValue(*entitlement["state_severity"], "_license_state_raw"))

	assert.NotContains(t, byID, "smart_authorization_expiry")
	assert.NotContains(t, byID, "smart_id_certificate_expiry")
	assert.NotContains(t, byID, "smart_evaluation_expiry")
}

func mustLoadCiscoSmartProfile(t *testing.T) *ddsnmp.Profile {
	t.Helper()

	return mustLoadLicensingProfile(t, "cisco", func(metric ddprofiledefinition.MetricsConfig) bool {
		const prefix = "1.3.6.1.4.1.9.9.831."

		if metric.MIB == "CISCO-SMART-LIC-MIB" {
			return true
		}
		if oid := strings.TrimPrefix(metric.Symbol.OID, "."); oid != "" && strings.HasPrefix(oid, prefix) {
			return true
		}
		if oid := strings.TrimPrefix(metric.Table.OID, "."); oid != "" && strings.HasPrefix(oid, prefix) {
			return true
		}
		return false
	})
}
