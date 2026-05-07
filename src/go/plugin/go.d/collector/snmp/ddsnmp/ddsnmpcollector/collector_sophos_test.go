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

func TestCollector_Collect_SophosLicensingProfile_PreservesRawStateAndExpiryOnScalarRows(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPGet(mockHandler, []string{
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
	}, []gosnmp.SnmpPDU{
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.1.1.0", 3),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.1.2.0", "2026-12-31"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.2.1.0", 4),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.2.2.0", "2026-11-30"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.3.1.0", 2),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.3.2.0", "N/A"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.4.1.0", 1),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.4.2.0", "2026-10-15"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.5.1.0", 3),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.5.2.0", "2027-01-20"),
	})
	expectSNMPGet(mockHandler, []string{
		"1.3.6.1.4.1.2604.5.1.5.6.1.0",
		"1.3.6.1.4.1.2604.5.1.5.6.2.0",
		"1.3.6.1.4.1.2604.5.1.5.7.1.0",
		"1.3.6.1.4.1.2604.5.1.5.7.2.0",
		"1.3.6.1.4.1.2604.5.1.5.8.1.0",
		"1.3.6.1.4.1.2604.5.1.5.8.2.0",
		"1.3.6.1.4.1.2604.5.1.5.9.1.0",
		"1.3.6.1.4.1.2604.5.1.5.9.2.0",
	}, []gosnmp.SnmpPDU{
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.6.1.0", 5),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.6.2.0", "2026-08-15"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.7.1.0", 3),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.7.2.0", "2026-09-01"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.8.1.0", 0),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.8.2.0", "Never"),
		createIntegerPDU("1.3.6.1.4.1.2604.5.1.5.9.1.0", 3),
		createStringPDU("1.3.6.1.4.1.2604.5.1.5.9.2.0", "2026-07-01"),
	})

	profile := mustLoadLicensingProfile(t, "sophos-xgs-firewall", func(metric ddprofiledefinition.MetricsConfig) bool {
		return metric.MIB == "SFOS-FIREWALL-MIB" &&
			strings.HasPrefix(strings.TrimPrefix(metric.Symbol.OID, "."), "1.3.6.1.4.1.2604.5.1.5.")
	})
	require.Len(t, profile.Definition.Metrics, 18)

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
		stateValue     int64
		expiryRawValue int64
		state          string
		kind           string
		expiryValue    int64
		hasExpiryValue bool
	}{
		"base_firewall":         {stateValue: 0, expiryRawValue: 3, state: "subscribed", kind: "subscription", expiryValue: 1798675200, hasExpiryValue: true},
		"network_protection":    {stateValue: 2, expiryRawValue: 4, state: "expired", kind: "subscription", expiryValue: 1795996800, hasExpiryValue: true},
		"web_protection":        {stateValue: 0, expiryRawValue: 2, state: "not_subscribed", kind: "subscription", hasExpiryValue: false},
		"mail_protection":       {stateValue: 1, expiryRawValue: 1, state: "trial", kind: "subscription", expiryValue: 1792022400, hasExpiryValue: true},
		"web_server_protection": {stateValue: 0, expiryRawValue: 3, state: "subscribed", kind: "subscription", expiryValue: 1800403200, hasExpiryValue: true},
		"sandstorm":             {stateValue: 2, expiryRawValue: 5, state: "deactivated", kind: "subscription", expiryValue: 1786752000, hasExpiryValue: true},
		"enhanced_support":      {stateValue: 0, expiryRawValue: 3, state: "subscribed", kind: "support", expiryValue: 1788220800, hasExpiryValue: true},
		"enhanced_plus_support": {stateValue: 0, expiryRawValue: 0, state: "none", kind: "support", hasExpiryValue: false},
		"central_orchestration": {stateValue: 0, expiryRawValue: 3, state: "subscribed", kind: "subscription", expiryValue: 1782864000, hasExpiryValue: true},
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

		assert.Empty(t, signals.expiry.Tags["_license_expiry_text"], "expiry helper tag should be removed for %s", id)
		assert.Equal(t, want.kind, signals.expiry.StaticTags["_license_type"], "unexpected expiry license type for %s", id)
		if want.hasExpiryValue {
			assert.EqualValues(t, want.expiryValue, signals.expiry.Value, "unexpected expiry timestamp for %s", id)
			assert.Equal(t, "expiry_timestamp", metricTagValue(*signals.expiry, "_license_value_kind"), "unexpected expiry kind for %s", id)
		} else {
			assert.EqualValues(t, want.expiryRawValue, signals.expiry.Value, "sentinel expiry should keep anchor value for %s", id)
			assert.Empty(t, metricTagValue(*signals.expiry, "_license_value_kind"), "sentinel expiry must not stamp a kind for %s", id)
		}
	}
}

type licenseMetricSignals struct {
	state  *ddsnmp.Metric
	expiry *ddsnmp.Metric
}

func mustGroupLicenseMetricsByID(t *testing.T, metrics []ddsnmp.Metric) map[string]licenseMetricSignals {
	t.Helper()

	byID := make(map[string]licenseMetricSignals, len(metrics))
	for i := range metrics {
		id := metrics[i].StaticTags["_license_id"]
		if id == "" {
			id = metrics[i].Tags["_license_id"]
		}
		if id == "" {
			continue
		}

		signals := byID[id]
		switch metrics[i].Name {
		case "_license_row":
			if signals.state != nil {
				t.Fatalf("duplicate license state metric for id %q", id)
			}
			signals.state = &metrics[i]
		case "_license_row_expiry":
			if signals.expiry != nil {
				t.Fatalf("duplicate license expiry metric for id %q", id)
			}
			signals.expiry = &metrics[i]
		}
		byID[id] = signals
	}
	return byID
}
