// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"strings"
	"testing"
	"time"

	"github.com/gosnmp/gosnmp"
	"github.com/netdata/netdata/go/plugins/logger"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestCollector_Collect_CiscoTraditionalLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/cisco-traditional.snmpwalk")
	expectSNMPWalkFromFixture(mockHandler, gosnmp.Version2c, fixture, "1.3.6.1.4.1.9.9.543.1.2.3.1")

	profile := mustLoadLicensingProfile(t, "cisco", func(metric ddprofiledefinition.MetricsConfig) bool {
		return strings.TrimPrefix(metric.Table.OID, ".") == "1.3.6.1.4.1.9.9.543.1.2.3.1"
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
	require.Len(t, pm.HiddenMetrics, 6)

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	permanent := byID["ipbasek9"]
	require.NotNil(t, permanent)
	require.EqualValues(t, 4294967295, permanent["capacity"].Value)
	require.EqualValues(t, 4294967295, permanent["available"].Value)
	require.EqualValues(t, 0, permanent["state_severity"].Value)
	assert.Equal(t, "ipbasek9", metricTagValue(*permanent["state_severity"], "_license_name"))
	assert.Equal(t, "permanent", metricTagValue(*permanent["state_severity"], "_license_type"))
	assert.Equal(t, "true", metricTagValue(*permanent["state_severity"], "_license_perpetual"))
	assert.Equal(t, "in_use", metricTagValue(*permanent["state_severity"], "_license_state_raw"))
	assert.NotContains(t, permanent, "expiry_timestamp")

	feature := byID["cme-srst"]
	require.NotNil(t, feature)
	require.EqualValues(t, 5000, feature["capacity"].Value)
	require.EqualValues(t, 5000, feature["available"].Value)
	require.EqualValues(t, 0, feature["state_severity"].Value)
	assert.Equal(t, "cme-srst", metricTagValue(*feature["state_severity"], "_license_name"))
	assert.Equal(t, "eval_right_to_use", metricTagValue(*feature["state_severity"], "_license_type"))
	assert.Equal(t, "not_in_use", metricTagValue(*feature["state_severity"], "_license_state_raw"))
}

func TestCollector_Collect_CheckPointLicensingProfile_CommunitySample(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/checkpoint-community.snmpwalk")
	expectSNMPWalkFromFixture(mockHandler, gosnmp.Version2c, fixture, "1.3.6.1.4.1.2620.1.6.18.1")

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
	require.Len(t, pm.HiddenMetrics, 10)

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	firewall := byID["0"]
	require.NotNil(t, firewall)
	assert.Equal(t, "Firewall", metricTagValue(*firewall["state_severity"], "_license_name"))
	assert.Equal(t, "Not Entitled", metricTagValue(*firewall["state_severity"], "_license_state_raw"))
	assert.EqualValues(t, 4294967295, firewall["expiry_timestamp"].Value)

	appCtrl := byID["4"]
	require.NotNil(t, appCtrl)
	assert.Equal(t, "Application Ctrl", metricTagValue(*appCtrl["state_severity"], "_license_name"))
	assert.Equal(t, "Evaluation", metricTagValue(*appCtrl["state_severity"], "_license_state_raw"))
	assert.EqualValues(t, 1619246913, appCtrl["expiry_timestamp"].Value)

	urlFiltering := byID["5"]
	require.NotNil(t, urlFiltering)
	assert.Equal(t, "URL Filtering", metricTagValue(*urlFiltering["state_severity"], "_license_name"))
	assert.EqualValues(t, 1619246913, urlFiltering["expiry_timestamp"].Value)

	ips := byID["2"]
	require.NotNil(t, ips)
	assert.Equal(t, "IPS", metricTagValue(*ips["state_severity"], "_license_name"))
	assert.EqualValues(t, 1619246941, ips["expiry_timestamp"].Value)

	smartEvent := byID["1003"]
	require.NotNil(t, smartEvent)
	assert.Equal(t, "SmartEvent", metricTagValue(*smartEvent["state_severity"], "_license_name"))
	assert.EqualValues(t, 1620542985, smartEvent["expiry_timestamp"].Value)
}

func TestCollector_Collect_SophosLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/sophos-xgs-firewall.snmprec")
	mustExpectSNMPGetFromFixture(t, mockHandler, fixture, []string{
		"1.3.6.1.4.1.2604.5.1.5.1.1.0",
		"1.3.6.1.4.1.2604.5.1.5.1.2.0",
		"1.3.6.1.4.1.2604.5.1.5.2.1.0",
		"1.3.6.1.4.1.2604.5.1.5.2.2.0",
		"1.3.6.1.4.1.2604.5.1.5.3.1.0",
		"1.3.6.1.4.1.2604.5.1.5.3.2.0",
		"1.3.6.1.4.1.2604.5.1.5.4.1.0",
		"1.3.6.1.4.1.2604.5.1.5.4.2.0",
		"1.3.6.1.4.1.2604.5.1.5.5.1.0",
		"1.3.6.1.4.1.2604.5.1.5.5.2.0",
	})
	mustExpectSNMPGetFromFixture(t, mockHandler, fixture, []string{
		"1.3.6.1.4.1.2604.5.1.5.6.1.0",
		"1.3.6.1.4.1.2604.5.1.5.6.2.0",
		"1.3.6.1.4.1.2604.5.1.5.7.1.0",
		"1.3.6.1.4.1.2604.5.1.5.7.2.0",
		"1.3.6.1.4.1.2604.5.1.5.8.1.0",
		"1.3.6.1.4.1.2604.5.1.5.8.2.0",
		"1.3.6.1.4.1.2604.5.1.5.9.1.0",
		"1.3.6.1.4.1.2604.5.1.5.9.2.0",
	})

	profile := mustLoadLicensingProfile(t, "sophos-xgs-firewall", func(metric ddprofiledefinition.MetricsConfig) bool {
		return metric.MIB == "SFOS-FIREWALL-MIB" &&
			strings.HasPrefix(strings.TrimPrefix(metric.Symbol.OID, "."), "1.3.6.1.4.1.2604.5.1.5.")
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
	require.Len(t, pm.HiddenMetrics, 18)

	byID := mustGroupLicenseMetricsByID(t, pm.HiddenMetrics)
	require.Len(t, byID, 9)

	expectations := map[string]struct {
		stateValue int64
		state      string
		kind       string
	}{
		"base_firewall":         {stateValue: 2, state: "deactivated", kind: "subscription"},
		"network_protection":    {stateValue: 0, state: "not_subscribed", kind: "subscription"},
		"web_protection":        {stateValue: 1, state: "trial", kind: "subscription"},
		"mail_protection":       {stateValue: 0, state: "subscribed", kind: "subscription"},
		"web_server_protection": {stateValue: 0, state: "subscribed", kind: "subscription"},
		"sandstorm":             {stateValue: 0, state: "subscribed", kind: "subscription"},
		"enhanced_support":      {stateValue: 0, state: "not_subscribed", kind: "support"},
		"enhanced_plus_support": {stateValue: 0, state: "not_subscribed", kind: "support"},
		"central_orchestration": {stateValue: 0, state: "none", kind: "subscription"},
	}

	for id, want := range expectations {
		signals, ok := byID[id]
		require.Truef(t, ok, "missing license signals for %s", id)
		require.NotNilf(t, signals.state, "missing state signal for %s", id)
		require.NotNilf(t, signals.expiry, "missing expiry signal for %s", id)

		assert.EqualValues(t, want.stateValue, signals.state.Value, "unexpected severity for %s", id)
		assert.Equal(t, want.state, signals.state.Tags["_license_state_raw"], "unexpected raw state for %s", id)
		assert.Equal(t, want.kind, signals.state.StaticTags["_license_type"], "unexpected license type for %s", id)
		assert.Equal(t, "state_severity", metricTagValue(*signals.state, "_license_value_kind"), "unexpected state kind for %s", id)

		// This public Datadog capture anonymizes expiry strings, so it can prove
		// the real scalar OID layout and state mapping, but not real expiry parsing.
		assert.Empty(t, signals.expiry.Tags["_license_expiry_text"], "expiry helper tag should be removed for %s", id)
		assert.Equal(t, want.kind, signals.expiry.StaticTags["_license_type"], "unexpected expiry license type for %s", id)
		assert.Empty(t, metricTagValue(*signals.expiry, "_license_value_kind"), "fixture expiry should remain unstamped for %s", id)
	}
}

func TestCollector_Collect_MikroTikLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/mikrotik-router.snmprec")
	mustExpectSNMPGetFromFixture(t, mockHandler, fixture, []string{
		"1.3.6.1.4.1.14988.1.1.4.2.0",
	})

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
	require.EqualValues(t, time.Date(2002, time.September, 21, 13, 53, 32, 300_000_000, time.UTC).Unix(), row["expiry_timestamp"].Value)
	assert.Equal(t, "RouterOS upgrade entitlement", metricTagValue(*row["expiry_timestamp"], "_license_name"))
	assert.Equal(t, "upgrade_entitlement", metricTagValue(*row["expiry_timestamp"], "_license_type"))
	assert.Equal(t, "routeros", metricTagValue(*row["expiry_timestamp"], "_license_component"))
	assert.Equal(t, "mtxrLicUpgrUntil", metricTagValue(*row["expiry_timestamp"], "_license_expiry_source"))
}

func TestCollector_Collect_CiscoSmartLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/cisco-smart-iosxe-c9800.snmprec")
	expectSNMPWalkFromFixture(mockHandler, gosnmp.Version2c, fixture, "1.3.6.1.4.1.9.9.831.0.5.1")

	profile := mustLoadLicensingProfile(t, "cisco", func(metric ddprofiledefinition.MetricsConfig) bool {
		return strings.TrimPrefix(metric.Table.OID, ".") == "1.3.6.1.4.1.9.9.831.0.5.1"
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

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	dna := byID["DNA_NWSTACK_E"]
	require.NotNil(t, dna)
	require.EqualValues(t, 55, dna["usage"].Value)
	require.EqualValues(t, 0, dna["state_severity"].Value)
	assert.Equal(t, "air-network-essentials", metricTagValue(*dna["usage"], "_license_name"))
	assert.Equal(t, "in_use", metricTagValue(*dna["state_severity"], "_license_state_raw"))

	airDNA := byID["AIR-DNA-E"]
	require.NotNil(t, airDNA)
	require.EqualValues(t, 55, airDNA["usage"].Value)
	require.EqualValues(t, 0, airDNA["state_severity"].Value)
	assert.Equal(t, "air-dna-essentials", metricTagValue(*airDNA["usage"], "_license_name"))
	assert.Equal(t, "in_use", metricTagValue(*airDNA["state_severity"], "_license_state_raw"))

	assert.NotContains(t, byID, "smart_registration")
	assert.NotContains(t, byID, "smart_authorization_state")
	assert.NotContains(t, byID, "smart_authorization_expiry")
}
