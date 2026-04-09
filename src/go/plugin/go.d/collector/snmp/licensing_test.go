// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"testing"
	"time"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

// signal builds one hidden _license_row metric tagged with the given id, name,
// kind, and value. The merge logic in licensing.go folds repeated calls with
// the same id into a single licenseRow.
func signal(id, name, kind string, value int64, extras ...map[string]string) ddsnmp.Metric {
	tags := map[string]string{
		tagLicenseID:        id,
		tagLicenseName:      name,
		tagLicenseValueKind: kind,
	}
	for _, extra := range extras {
		for k, v := range extra {
			tags[k] = v
		}
	}
	return ddsnmp.Metric{Name: licenseSourceMetricName, Value: value, Tags: tags}
}

func profileWith(metrics ...ddsnmp.Metric) []*ddsnmp.ProfileMetrics {
	pm := &ddsnmp.ProfileMetrics{Source: "test.yaml", HiddenMetrics: metrics}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}
	return []*ddsnmp.ProfileMetrics{pm}
}

func TestExtractLicenseRows_MergesSignalsByID(t *testing.T) {
	now := time.Now().UTC()
	expiry := now.Add(48 * time.Hour).Unix()

	rows := extractLicenseRows(profileWith(
		signal("base", "Base Firewall", licenseValueKindStateSeverity, 0),
		signal("base", "Base Firewall", licenseValueKindExpiryTimestamp, expiry),
		signal("base", "Base Firewall", licenseValueKindUsage, 75),
		signal("base", "Base Firewall", licenseValueKindCapacity, 100),
	), now)

	require.Len(t, rows, 1)
	row := rows[0]
	assert.Equal(t, "base", row.ID)
	assert.Equal(t, "Base Firewall", row.Name)
	assert.True(t, row.HasState)
	assert.EqualValues(t, 0, row.StateSeverity)
	assert.True(t, row.HasExpiry)
	assert.EqualValues(t, expiry, row.ExpiryTS)
	assert.True(t, row.HasUsage)
	assert.EqualValues(t, 75, row.Usage)
	assert.True(t, row.HasCapacity)
	assert.EqualValues(t, 100, row.Capacity)
	assert.True(t, row.HasUsagePct)
	assert.InDelta(t, 75.0, row.UsagePercent, 0.001)
	assert.Equal(t, licenseStateBucketHealthy, row.StateBucket)
}

func TestExtractLicenseRows_DerivesUsageFromCapacityMinusAvailable(t *testing.T) {
	now := time.Now().UTC()
	rows := extractLicenseRows(profileWith(
		signal("pool", "Connection pool", licenseValueKindCapacity, 100),
		signal("pool", "Connection pool", licenseValueKindAvailable, 25),
	), now)

	require.Len(t, rows, 1)
	assert.True(t, rows[0].HasUsage)
	assert.EqualValues(t, 75, rows[0].Usage)
	assert.True(t, rows[0].HasUsagePct)
	assert.InDelta(t, 75.0, rows[0].UsagePercent, 0.001)
}

func TestExtractLicenseRows_RowsWithDifferentIDsStaySeparate(t *testing.T) {
	now := time.Now().UTC()
	rows := extractLicenseRows(profileWith(
		signal("a", "License A", licenseValueKindStateSeverity, 0),
		signal("b", "License B", licenseValueKindStateSeverity, 2),
	), now)

	require.Len(t, rows, 2)
}

func TestExtractLicenseRows_RemainingKindsAreRebasedOnNow(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		signal("a", "Auth", licenseValueKindAuthorizationRemaining, 3600),
		signal("b", "Cert", licenseValueKindCertificateRemaining, 7200),
		signal("c", "Grace", licenseValueKindGraceRemaining, 1800),
		signal("d", "Sub", licenseValueKindExpiryRemaining, 600),
	), now)

	require.Len(t, rows, 4)
	for _, row := range rows {
		switch row.ID {
		case "a":
			assert.True(t, row.HasAuthorizationTime)
			assert.EqualValues(t, now.Unix()+3600, row.AuthorizationExpiry)
		case "b":
			assert.True(t, row.HasCertificateTime)
			assert.EqualValues(t, now.Unix()+7200, row.CertificateExpiry)
		case "c":
			assert.True(t, row.HasGraceTime)
			assert.EqualValues(t, now.Unix()+1800, row.GraceExpiry)
		case "d":
			assert.True(t, row.HasExpiry)
			assert.EqualValues(t, now.Unix()+600, row.ExpiryTS)
		}
	}
}

func TestAggregateLicenseRows_SelectsMinExpiryAndMaxUsage(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	earliest := now.Add(2 * time.Hour).Unix()
	latest := now.Add(48 * time.Hour).Unix()

	rows := extractLicenseRows(profileWith(
		signal("a", "First", licenseValueKindExpiryTimestamp, latest),
		signal("a", "First", licenseValueKindUsage, 30),
		signal("a", "First", licenseValueKindCapacity, 100),
		signal("b", "Second", licenseValueKindExpiryTimestamp, earliest),
		signal("b", "Second", licenseValueKindUsage, 95),
		signal("b", "Second", licenseValueKindCapacity, 100),
	), now)

	agg := aggregateLicenseRows(rows, now)
	assert.True(t, agg.hasRemainingTime)
	// min remaining wins
	assert.Equal(t, earliest-now.Unix(), agg.remainingTime)
	// max usage wins
	assert.True(t, agg.hasUsagePercent)
	assert.EqualValues(t, 95, agg.usagePercent)
}

func TestAggregateLicenseRows_PerpetualLicensesSkipExpiryAggregation(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	perpExpiry := now.Add(1 * time.Hour).Unix()  // would dominate if counted
	realExpiry := now.Add(24 * time.Hour).Unix() // real winner

	rows := extractLicenseRows(profileWith(
		signal("perp", "Perpetual", licenseValueKindExpiryTimestamp, perpExpiry,
			map[string]string{tagLicensePerpetual: "true"}),
		signal("real", "Subscription", licenseValueKindExpiryTimestamp, realExpiry),
	), now)

	agg := aggregateLicenseRows(rows, now)
	assert.True(t, agg.hasRemainingTime)
	assert.Equal(t, realExpiry-now.Unix(), agg.remainingTime)
}

func TestAggregateLicenseRows_UnlimitedRowsSkipUsageAggregation(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		signal("limited", "Limited", licenseValueKindUsagePercent, 60),
		signal("infinite", "Infinite", licenseValueKindUsagePercent, 100,
			map[string]string{tagLicenseUnlimited: "true"}),
	), now)

	agg := aggregateLicenseRows(rows, now)
	assert.True(t, agg.hasUsagePercent)
	assert.EqualValues(t, 60, agg.usagePercent)
}

func TestAggregateLicenseRows_StateBucketCounts(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		signal("a", "Healthy A", licenseValueKindStateSeverity, 0),
		signal("b", "Healthy B", licenseValueKindStateSeverity, 0),
		signal("c", "Degraded", licenseValueKindStateSeverity, 1),
		signal("d", "Broken", licenseValueKindStateSeverity, 2),
	), now)

	agg := aggregateLicenseRows(rows, now)
	assert.True(t, agg.hasStateCounts)
	assert.EqualValues(t, 2, agg.stateHealthy)
	assert.EqualValues(t, 1, agg.stateDegraded)
	assert.EqualValues(t, 1, agg.stateBroken)
	assert.EqualValues(t, 0, agg.stateIgnored)
}

func TestNormalizeBucket_FreshSeverityWinsOverCachedRawState(t *testing.T) {
	// The raw state string is collected as a same-table metric_tag and the
	// SNMP table cache reuses it on cache hits. Severity comes from a fresh
	// symbol PDU on every poll. When both are present the fresh severity
	// must take precedence — otherwise a renewed license would still report
	// the previous broken/degraded text for up to the cache TTL.
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		// Fresh severity says healthy. Stale raw state still says
		// "about-to-expire". The fresh signal wins.
		signal("a", "About", licenseValueKindStateSeverity, 0,
			map[string]string{tagLicenseStateRaw: "about-to-expire"}),
		// Fresh severity says healthy. Stale raw state says "invalid".
		// The fresh signal wins again.
		signal("b", "Inv", licenseValueKindStateSeverity, 0,
			map[string]string{tagLicenseStateRaw: "invalid"}),
	), now)

	require.Len(t, rows, 2)
	assert.Equal(t, licenseStateBucketHealthy, rows[0].StateBucket)
	assert.Equal(t, licenseStateBucketHealthy, rows[1].StateBucket)
}

func TestNormalizeBucket_RawStateUsedWhenNoFreshSeverity(t *testing.T) {
	// With no _license_value_kind=state_severity, the raw state string is
	// the only classification signal available — fall back to it.
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		// State raw classifies as broken; no other signals exist.
		ddsnmp.Metric{
			Name:  licenseSourceMetricName,
			Value: 0,
			Tags: map[string]string{
				tagLicenseID:        "stale",
				tagLicenseName:      "Stale",
				tagLicenseStateRaw:  "expired",
				tagLicenseValueKind: licenseValueKindUsage, // any kind to keep the row
			},
		},
	), now)

	require.Len(t, rows, 1)
	// Without a fresh severity the cached raw text is still better than
	// nothing, so the row classifies as broken.
	assert.Equal(t, licenseStateBucketBroken, rows[0].StateBucket)
}

func TestNormalizeBucket_ExpiredTimerForcesBroken(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	pastExpiry := now.Add(-1 * time.Hour).Unix()

	rows := extractLicenseRows(profileWith(
		signal("expired", "Expired", licenseValueKindExpiryTimestamp, pastExpiry,
			map[string]string{tagLicenseStateRaw: "valid"}),
	), now)

	require.Len(t, rows, 1)
	assert.Equal(t, licenseStateBucketBroken, rows[0].StateBucket)
}

// TestExtractLicenseRows_CheckPointPublicFixtureValues replays the exact
// vendor strings and Gauge32 expiry values captured from the public Check
// Point community thread (testdata/licensing/checkpoint-community.snmpwalk):
//
//	Firewall          state="Not Entitled" expiry=4294967295 (sentinel)
//	Application Ctrl  state="Evaluation"  expiry=1619246913 (2021-04-24)
//	IPS               state="Evaluation"  expiry=1619246941 (2021-04-24)
//
// It guards three regressions caught during reviewer iteration:
//  1. The Check Point state mapping must cover the real on-wire TitleCase
//     strings (Not Entitled, Evaluation), not only the hyphenated tokens
//     from the MIB DESCRIPTION block.
//  2. The 0xFFFFFFFF "never expires" sentinel must be filtered out, not
//     treated as a real epoch (which would inflate remaining_time by ~80
//     years and corrupt the device-wide minimum).
//  3. A past expiry must force broken regardless of vendor severity, so
//     the device-level alerts trip even when the vendor still reports the
//     blade as "Evaluation".
func TestExtractLicenseRows_CheckPointPublicFixtureValues(t *testing.T) {
	// "now" is well after the fixture's 2021 expiries on purpose: in
	// production the expiries from this fixture are firmly in the past,
	// which is exactly the "broken timer overrides severity" path we
	// want to exercise.
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		// "Firewall" → Not Entitled → severity 2; expiry is the sentinel.
		signal("0", "Firewall", licenseValueKindStateSeverity, 2,
			map[string]string{tagLicenseStateRaw: "Not Entitled"}),
		signal("0", "Firewall", licenseValueKindExpiryTimestamp, 4294967295, nil),
		// "Application Ctrl" → Evaluation → severity 1; expiry in the past.
		signal("4", "Application Ctrl", licenseValueKindStateSeverity, 1,
			map[string]string{tagLicenseStateRaw: "Evaluation"}),
		signal("4", "Application Ctrl", licenseValueKindExpiryTimestamp, 1619246913, nil),
		// "IPS" → Evaluation → severity 1; expiry in the past.
		signal("2", "IPS", licenseValueKindStateSeverity, 1,
			map[string]string{tagLicenseStateRaw: "Evaluation"}),
		signal("2", "IPS", licenseValueKindExpiryTimestamp, 1619246941, nil),
	), now)

	require.Len(t, rows, 3)

	for _, row := range rows {
		switch row.ID {
		case "0":
			// Severity 2 wins; the sentinel expiry is dropped.
			assert.Equal(t, licenseStateBucketBroken, row.StateBucket, "Not Entitled → broken")
			assert.False(t, row.HasExpiry, "0xFFFFFFFF sentinel must be dropped")
		case "4":
			// Past expiry overrides the vendor's optimistic Evaluation
			// severity — alerting must trip.
			assert.Equal(t, licenseStateBucketBroken, row.StateBucket, "expired Evaluation → broken")
			assert.True(t, row.HasExpiry)
			assert.EqualValues(t, 1619246913, row.ExpiryTS)
		case "2":
			assert.Equal(t, licenseStateBucketBroken, row.StateBucket, "expired Evaluation → broken")
			assert.True(t, row.HasExpiry)
			assert.EqualValues(t, 1619246941, row.ExpiryTS)
		default:
			t.Fatalf("unexpected row id %q", row.ID)
		}
	}
}

func TestNormalizeBucket_FreshSeverityRecoversFromCachedBrokenState(t *testing.T) {
	// Concrete cache-staleness regression scenario: a Check Point license
	// was previously "expired", got renewed on the device, but the next
	// poll is still inside the table cache TTL. The row therefore carries
	// the cached "expired" raw state alongside a freshly-collected severity
	// of 0. The bucket must reflect the renewal, not the cache.
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		signal("renewed", "Renewed License", licenseValueKindStateSeverity, 0,
			map[string]string{tagLicenseStateRaw: "expired"}),
	), now)

	require.Len(t, rows, 1)
	assert.Equal(t, licenseStateBucketHealthy, rows[0].StateBucket)
}

func TestExtractLicenseRows_DropsRowsThatProducedNoSignal(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	// Identity tags but no _license_value_kind → no signal merges in.
	// The row would otherwise show up as "ignored" and inflate device counts.
	rows := extractLicenseRows(profileWith(ddsnmp.Metric{
		Name:  licenseSourceMetricName,
		Value: 1,
		Tags:  map[string]string{tagLicenseID: "stub", tagLicenseName: "Stub"},
	}), now)

	assert.Empty(t, rows)
}

func TestExtractLicenseRows_UnstampedHelperMetricDoesNotPolluteStateRow(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		signal("base", "Base Firewall", licenseValueKindStateSeverity, 2,
			map[string]string{tagLicenseStateRaw: "expired"}),
		ddsnmp.Metric{
			Name:  "_license_row_expiry",
			Value: 2, // scalar anchor value left untouched after parse rejection
			Tags: map[string]string{
				tagLicenseID:     "base",
				tagLicenseName:   "Base Firewall",
				tagLicenseType:   "subscription",
				tagLicenseImpact: "state and expiry on the device",
			},
		},
	), now)

	require.Len(t, rows, 1)
	assert.Equal(t, "base", rows[0].ID)
	assert.True(t, rows[0].HasState)
	assert.EqualValues(t, 2, rows[0].StateSeverity)
	assert.False(t, rows[0].HasExpiry)
	assert.Equal(t, licenseStateBucketBroken, rows[0].StateBucket)
}

func TestExtractLicenseRows_DropsMetricsWithoutLicenseID(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	rows := extractLicenseRows(profileWith(
		// Has identity → kept.
		signal("present", "Present", licenseValueKindStateSeverity, 0),
		// Missing _license_id → silently dropped (no safe merge key).
		ddsnmp.Metric{
			Name:  licenseSourceMetricName,
			Value: 0,
			Tags: map[string]string{
				tagLicenseName:      "Orphan",
				tagLicenseValueKind: licenseValueKindStateSeverity,
			},
		},
	), now)

	require.Len(t, rows, 1)
	assert.Equal(t, "present", rows[0].ID)
}

func TestExtractLicenseRows_DifferentTablesWithSameIDStaySeparate(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	// Same _license_id ("FortiCare") + same source, different SNMP table —
	// must NOT merge into one row. This guards the Fortinet bug from the
	// codex review where contract/version/account tables shared identifiers.
	pm := &ddsnmp.ProfileMetrics{
		Source: "vendor.yaml",
		HiddenMetrics: []ddsnmp.Metric{
			{
				Name:  licenseSourceMetricName,
				Value: 100,
				Table: "fgLicContractTable",
				Tags: map[string]string{
					tagLicenseID:        "FortiCare",
					tagLicenseValueKind: licenseValueKindCapacity,
				},
			},
			{
				Name:  licenseSourceMetricName,
				Value: 200,
				Table: "fgLicVersionTable",
				Tags: map[string]string{
					tagLicenseID:        "FortiCare",
					tagLicenseValueKind: licenseValueKindCapacity,
				},
			},
		},
	}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}

	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{pm}, now)
	require.Len(t, rows, 2)
	caps := []int64{rows[0].Capacity, rows[1].Capacity}
	assert.Contains(t, caps, int64(100))
	assert.Contains(t, caps, int64(200))
}

func TestExtractLicenseRows_MergeKeyHandlesEmbeddedNULs(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)

	pm := &ddsnmp.ProfileMetrics{
		Source: "vendor.yaml",
		HiddenMetrics: []ddsnmp.Metric{
			{
				Name:  licenseSourceMetricName,
				Value: 100,
				Table: "tableA",
				Tags: map[string]string{
					tagLicenseID:        "row\x00suffix",
					tagLicenseValueKind: licenseValueKindCapacity,
				},
			},
			{
				Name:  licenseSourceMetricName,
				Value: 200,
				Table: "tableA\x00row",
				Tags: map[string]string{
					tagLicenseID:        "suffix",
					tagLicenseValueKind: licenseValueKindCapacity,
				},
			},
		},
	}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}

	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{pm}, now)
	require.Len(t, rows, 2)
	caps := []int64{rows[0].Capacity, rows[1].Capacity}
	assert.Contains(t, caps, int64(100))
	assert.Contains(t, caps, int64(200))
}

func TestExtractLicenseRows_StaticTagsAreMerged(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{{
		Source: "vendor.yaml",
		HiddenMetrics: []ddsnmp.Metric{{
			Name:  licenseSourceMetricName,
			Value: 0,
			StaticTags: map[string]string{
				tagLicenseID:        "static",
				tagLicenseName:      "Static",
				tagLicenseValueKind: licenseValueKindStateSeverity,
			},
		}},
	}}, now)

	require.Len(t, rows, 1)
	assert.Equal(t, "static", rows[0].ID)
	assert.True(t, rows[0].HasState)
	assert.Equal(t, licenseStateBucketHealthy, rows[0].StateBucket)
}

func TestApplyVendorLicenseSanityRules_MikroTikIgnoresEpochSentinel(t *testing.T) {
	row := licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		// 1970-01-02 → year 1970 < cutoff 1971. The shouldIgnore guard
		// requires a positive timestamp so the sentinel must be > 0.
		ExpiryTS: 86400,
	}
	applyVendorLicenseSanityRules(&row)
	assert.False(t, row.HasExpiry, "epoch-like sentinel must be ignored")
	assert.EqualValues(t, 0, row.ExpiryTS)
}

func TestApplyVendorLicenseSanityRules_OnlyAppliesToUpgradeUntilSource(t *testing.T) {
	row := licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: "someOtherOID",
		HasExpiry:    true,
		ExpiryTS:     86400,
	}
	applyVendorLicenseSanityRules(&row)
	assert.True(t, row.HasExpiry, "non-upgrade-until expiry must be left alone")
}

func TestApplyVendorLicenseSanityRules_MikroTikKeepsRealExpiry(t *testing.T) {
	row := licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		ExpiryTS:     time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC).Unix(),
	}
	applyVendorLicenseSanityRules(&row)
	assert.True(t, row.HasExpiry, "real future expiry must be preserved")
	assert.NotZero(t, row.ExpiryTS)
}

func TestApplyVendorLicenseSanityRules_OnlyAppliesToMikroTik(t *testing.T) {
	row := licenseRow{
		Source:       "checkpoint",
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		ExpiryTS:     0,
	}
	applyVendorLicenseSanityRules(&row)
	assert.True(t, row.HasExpiry, "non-MikroTik rows must be left alone")
}

func TestAggregateLicenseRows_WriteToOmitsAbsentSignals(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	rows := extractLicenseRows(profileWith(
		signal("a", "A", licenseValueKindStateSeverity, 0),
	), now)

	mx := make(map[string]int64)
	aggregateLicenseRows(rows, now).writeTo(mx)
	assert.NotContains(t, mx, metricIDLicenseRemainingTime)
	assert.NotContains(t, mx, metricIDLicenseUsagePercent)
	assert.Contains(t, mx, metricIDLicenseStateHealthy)
}
