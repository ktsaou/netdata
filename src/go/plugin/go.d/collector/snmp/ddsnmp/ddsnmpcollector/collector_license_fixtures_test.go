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

	profile := mustLoadTypedLicensingProfile(t, "cisco", func(row ddprofiledefinition.LicensingConfig) bool {
		return strings.TrimPrefix(row.Table.OID, ".") == "1.3.6.1.4.1.9.9.543.1.2.3.1"
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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 2)

	byName := licenseRowsByName(pm.LicenseRows)

	permanent := byName["ipbasek9"]
	require.NotEmpty(t, permanent.ID)
	assert.Equal(t, "1.1.1", permanent.ID)
	require.True(t, permanent.Usage.HasCapacity)
	assert.EqualValues(t, 4294967295, permanent.Usage.Capacity)
	require.True(t, permanent.Usage.HasAvailable)
	assert.EqualValues(t, 4294967295, permanent.Usage.Available)
	require.True(t, permanent.State.Has)
	assert.EqualValues(t, 0, permanent.State.Severity)
	assert.Equal(t, "permanent", permanent.Type)
	assert.True(t, permanent.IsPerpetual)
	assert.Equal(t, "3", permanent.State.Raw)
	assert.False(t, permanent.Expiry.Has)

	feature := byName["cme-srst"]
	require.NotEmpty(t, feature.ID)
	assert.Equal(t, "1.2.8", feature.ID)
	require.True(t, feature.Usage.HasCapacity)
	assert.EqualValues(t, 5000, feature.Usage.Capacity)
	require.True(t, feature.Usage.HasAvailable)
	assert.EqualValues(t, 5000, feature.Usage.Available)
	require.True(t, feature.State.Has)
	assert.EqualValues(t, 0, feature.State.Severity)
	assert.Equal(t, "eval_right_to_use", feature.Type)
	assert.Equal(t, "2", feature.State.Raw)
	assert.False(t, feature.Expiry.Has)
}

func TestCollector_Collect_CheckPointLicensingProfile_CommunitySample(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/checkpoint-community.snmpwalk")
	expectSNMPWalkFromFixture(mockHandler, gosnmp.Version2c, fixture, "1.3.6.1.4.1.2620.1.6.18.1")

	profile := mustLoadTypedLicensingProfile(t, "checkpoint", func(row ddprofiledefinition.LicensingConfig) bool {
		return strings.TrimPrefix(row.Table.OID, ".") == "1.3.6.1.4.1.2620.1.6.18.1"
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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 5)

	byID := make(map[string]ddsnmp.LicenseRow, len(pm.LicenseRows))
	for _, row := range pm.LicenseRows {
		byID[row.ID] = row
	}

	firewall := byID["0"]
	assert.Equal(t, "Firewall", firewall.Name)
	assert.Equal(t, "Not Entitled", firewall.State.Raw)
	assert.True(t, firewall.State.Has)
	assert.EqualValues(t, 2, firewall.State.Severity)
	assert.False(t, firewall.Expiry.Has)

	appCtrl := byID["4"]
	assert.Equal(t, "Application Ctrl", appCtrl.Name)
	assert.Equal(t, "Evaluation", appCtrl.State.Raw)
	assert.EqualValues(t, 1, appCtrl.State.Severity)
	assert.EqualValues(t, 1619246913, appCtrl.Expiry.Timestamp)

	urlFiltering := byID["5"]
	assert.Equal(t, "URL Filtering", urlFiltering.Name)
	assert.EqualValues(t, 1619246913, urlFiltering.Expiry.Timestamp)

	ips := byID["2"]
	assert.Equal(t, "IPS", ips.Name)
	assert.EqualValues(t, 1619246941, ips.Expiry.Timestamp)

	smartEvent := byID["1003"]
	assert.Equal(t, "SmartEvent", smartEvent.Name)
	assert.EqualValues(t, 1620542985, smartEvent.Expiry.Timestamp)
}

func TestCollector_Collect_SophosLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/sophos-xgs-firewall.snmprec")
	for _, pair := range [][]string{
		{"1.3.6.1.4.1.2604.5.1.5.1.1.0", "1.3.6.1.4.1.2604.5.1.5.1.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.2.1.0", "1.3.6.1.4.1.2604.5.1.5.2.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.3.1.0", "1.3.6.1.4.1.2604.5.1.5.3.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.4.1.0", "1.3.6.1.4.1.2604.5.1.5.4.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.5.1.0", "1.3.6.1.4.1.2604.5.1.5.5.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.6.1.0", "1.3.6.1.4.1.2604.5.1.5.6.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.7.1.0", "1.3.6.1.4.1.2604.5.1.5.7.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.8.1.0", "1.3.6.1.4.1.2604.5.1.5.8.2.0"},
		{"1.3.6.1.4.1.2604.5.1.5.9.1.0", "1.3.6.1.4.1.2604.5.1.5.9.2.0"},
	} {
		mustExpectSNMPGetFromFixture(t, mockHandler, fixture, pair)
	}

	profile := mustLoadTypedLicensingProfile(t, "sophos-xgs-firewall", func(row ddprofiledefinition.LicensingConfig) bool {
		return row.MIB == "SFOS-FIREWALL-MIB" &&
			strings.HasPrefix(strings.TrimPrefix(ddprofiledefinition.LicenseValueSourceOID(row.State.LicenseValueConfig), "."), "1.3.6.1.4.1.2604.5.1.5.")
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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 9)

	byID := licenseRowsByID(pm.LicenseRows)
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
		row, ok := byID[id]
		require.Truef(t, ok, "missing license row for %s", id)

		require.Truef(t, row.State.Has, "missing state for %s", id)
		assert.EqualValues(t, want.stateValue, row.State.Severity, "unexpected severity for %s", id)
		assert.Equal(t, want.state, row.State.Raw, "unexpected raw state for %s", id)
		assert.Equal(t, want.kind, row.Type, "unexpected license type for %s", id)

		// This public Datadog capture anonymizes expiry strings, so it can prove
		// the real scalar OID layout and state mapping, but not real expiry parsing.
		assert.False(t, row.Expiry.Has, "anonymized fixture expiry should not parse for %s", id)
	}
}

func TestCollector_Collect_MikroTikLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/mikrotik-router.snmprec")
	mustExpectSNMPGetFromFixture(t, mockHandler, fixture, []string{
		"1.3.6.1.4.1.14988.1.1.4.2.0",
	})

	profile := mustLoadTypedLicensingProfile(t, "mikrotik-router", func(row ddprofiledefinition.LicensingConfig) bool {
		return row.ID == "routeros_upgrade"
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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 1)

	row := pm.LicenseRows[0]
	assert.Equal(t, "routeros_upgrade", row.ID)
	assert.Equal(t, "RouterOS upgrade entitlement", row.Name)
	assert.Equal(t, "upgrade_entitlement", row.Type)
	assert.Equal(t, "routeros", row.Component)
	assert.True(t, row.Expiry.Has)
	assert.EqualValues(t, time.Date(2002, time.September, 21, 13, 53, 32, 300_000_000, time.UTC).Unix(), row.Expiry.Timestamp)
	assert.Equal(t, "1.3.6.1.4.1.14988.1.1.4.2.0", row.Expiry.SourceOID)
}

func TestCollector_Collect_CiscoSmartLicensingProfile_Fixture(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	fixture := mustLoadSNMPFixture(t, "testdata/licensing/cisco-smart-iosxe-c9800.snmprec")
	expectSNMPWalkFromFixture(mockHandler, gosnmp.Version2c, fixture, "1.3.6.1.4.1.9.9.831.0.5.1")

	profile := mustLoadTypedLicensingProfile(t, "cisco", func(row ddprofiledefinition.LicensingConfig) bool {
		return strings.TrimPrefix(row.Table.OID, ".") == "1.3.6.1.4.1.9.9.831.0.5.1"
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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 2)

	byID := licenseRowsByID(pm.LicenseRows)

	dna := byID["DNA_NWSTACK_E"]
	require.True(t, dna.Usage.HasUsed)
	assert.EqualValues(t, 55, dna.Usage.Used)
	require.True(t, dna.State.Has)
	assert.EqualValues(t, 0, dna.State.Severity)
	assert.Equal(t, "air-network-essentials", dna.Name)
	assert.Equal(t, "15", dna.State.Raw)

	airDNA := byID["AIR-DNA-E"]
	require.True(t, airDNA.Usage.HasUsed)
	assert.EqualValues(t, 55, airDNA.Usage.Used)
	require.True(t, airDNA.State.Has)
	assert.EqualValues(t, 0, airDNA.State.Severity)
	assert.Equal(t, "air-dna-essentials", airDNA.Name)
	assert.Equal(t, "15", airDNA.State.Raw)

	assert.NotContains(t, byID, "smart_registration")
	assert.NotContains(t, byID, "smart_authorization")
}
