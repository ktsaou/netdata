// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

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

func TestAggregateLicenseRows_IgnoredRowsDoNotDriveSignalAggregation(t *testing.T) {
	now := time.Date(2026, 4, 9, 12, 0, 0, 0, time.UTC)
	ignoredExpiry := now.Add(1 * time.Hour).Unix()
	healthyExpiry := now.Add(24 * time.Hour).Unix()

	rows := extractLicenseRows(profileWith(
		signal("ignored", "Ignored", licenseValueKindExpiryTimestamp, ignoredExpiry,
			map[string]string{tagLicenseStateRaw: "not applicable"}),
		signal("ignored", "Ignored", licenseValueKindUsagePercent, 99,
			map[string]string{tagLicenseStateRaw: "not applicable"}),
		signal("healthy", "Healthy", licenseValueKindExpiryTimestamp, healthyExpiry),
		signal("healthy", "Healthy", licenseValueKindUsagePercent, 60),
	), now)

	agg := aggregateLicenseRows(rows, now)
	assert.True(t, agg.hasRemainingTime)
	assert.Equal(t, healthyExpiry-now.Unix(), agg.remainingTime)
	assert.True(t, agg.hasUsagePercent)
	assert.EqualValues(t, 60, agg.usagePercent)
	assert.True(t, agg.hasStateCounts)
	assert.EqualValues(t, 1, agg.stateHealthy)
	assert.EqualValues(t, 1, agg.stateIgnored)
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
