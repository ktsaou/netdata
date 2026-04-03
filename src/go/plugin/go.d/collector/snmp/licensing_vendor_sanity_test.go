// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"strconv"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
)

func TestExtractLicenseRows_MikroTikUpgradeUntilSentinelIgnored(t *testing.T) {
	now := time.Date(2026, time.April, 3, 10, 0, 0, 0, time.UTC)

	pm := &ddsnmp.ProfileMetrics{Source: "/tmp/mikrotik-router.yaml"}
	pm.HiddenMetrics = []ddsnmp.Metric{
		{
			Name:  licenseSourceMetricName,
			Value: 6,
			StaticTags: map[string]string{
				tagLicenseName:         "RouterOS license",
				tagLicenseID:           "routeros_license",
				tagLicenseExpirySource: mikroTikUpgradeUntilExpirySource,
			},
			Tags: map[string]string{
				tagLicenseFeature:   "level_6",
				tagLicenseExpiryRaw: strconv.FormatInt(time.Date(1970, time.January, 1, 0, 1, 7, 0, time.UTC).Unix(), 10),
			},
		},
	}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}

	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{pm}, now)
	require.Len(t, rows, 1)

	row := rows[0]
	assert.Equal(t, mikroTikLicenseSource, row.Source)
	assert.Equal(t, mikroTikUpgradeUntilExpirySource, row.ExpirySource)
	assert.False(t, row.HasExpiry)
	assert.Zero(t, row.ExpiryTS)
	assert.Equal(t, licenseStateBucketIgnored, row.StateBucket)
}

func TestExtractLicenseRows_MikroTikRealUpgradeUntilKept(t *testing.T) {
	now := time.Date(2026, time.April, 3, 10, 0, 0, 0, time.UTC)

	pm := &ddsnmp.ProfileMetrics{Source: "/tmp/mikrotik-router.yaml"}
	pm.HiddenMetrics = []ddsnmp.Metric{
		{
			Name:  licenseSourceMetricName,
			Value: 6,
			StaticTags: map[string]string{
				tagLicenseName:         "RouterOS license",
				tagLicenseID:           "routeros_license",
				tagLicenseExpirySource: mikroTikUpgradeUntilExpirySource,
			},
			Tags: map[string]string{
				tagLicenseFeature:   "level_6",
				tagLicenseExpiryRaw: strconv.FormatInt(time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(), 10),
			},
		},
	}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}

	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{pm}, now)
	require.Len(t, rows, 1)

	row := rows[0]
	assert.True(t, row.HasExpiry)
	assert.EqualValues(t, time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(), row.ExpiryTS)
	assert.Equal(t, licenseStateBucketHealthy, row.StateBucket)
}

func TestExtractLicenseRows_OtherVendorsKeepEpochTimestampsUntouched(t *testing.T) {
	now := time.Date(2026, time.April, 3, 10, 0, 0, 0, time.UTC)

	pm := &ddsnmp.ProfileMetrics{Source: "/tmp/checkpoint.yaml"}
	pm.HiddenMetrics = []ddsnmp.Metric{
		{
			Name:  licenseSourceMetricName,
			Value: 1,
			StaticTags: map[string]string{
				tagLicenseName:         "Other vendor test",
				tagLicenseID:           "other_vendor_test",
				tagLicenseExpirySource: mikroTikUpgradeUntilExpirySource,
			},
			Tags: map[string]string{
				tagLicenseExpiryRaw: strconv.FormatInt(time.Date(1970, time.January, 1, 0, 1, 7, 0, time.UTC).Unix(), 10),
			},
		},
	}
	for i := range pm.HiddenMetrics {
		pm.HiddenMetrics[i].Profile = pm
	}

	rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{pm}, now)
	require.Len(t, rows, 1)

	row := rows[0]
	assert.Equal(t, "checkpoint", row.Source)
	assert.True(t, row.HasExpiry)
	assert.EqualValues(t, time.Date(1970, time.January, 1, 0, 1, 7, 0, time.UTC).Unix(), row.ExpiryTS)
}
