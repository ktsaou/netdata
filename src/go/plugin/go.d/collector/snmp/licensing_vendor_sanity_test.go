// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"testing"
	"time"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"

	"github.com/stretchr/testify/assert"
)

func TestShouldIgnoreMikroTikUpgradeUntil(t *testing.T) {
	tests := map[string]struct {
		row  licenseRow
		want bool
	}{
		"ignores epoch-like timestamp from RouterOS upgrade field": {
			row: licenseRow{
				Source:       mikroTikLicenseSource,
				ExpirySource: mikroTikUpgradeUntilExpirySource,
				HasExpiry:    true,
				ExpiryTS:     time.Date(1970, time.January, 2, 0, 0, 0, 0, time.UTC).Unix(),
			},
			want: true,
		},
		"requires MikroTik source": {
			row: licenseRow{
				Source:       "checkpoint",
				ExpirySource: mikroTikUpgradeUntilExpirySource,
				HasExpiry:    true,
				ExpiryTS:     86400,
			},
		},
		"requires upgrade-until expiry source": {
			row: licenseRow{
				Source:       mikroTikLicenseSource,
				ExpirySource: "other",
				HasExpiry:    true,
				ExpiryTS:     86400,
			},
		},
		"keeps real expiry": {
			row: licenseRow{
				Source:       mikroTikLicenseSource,
				ExpirySource: mikroTikUpgradeUntilExpirySource,
				HasExpiry:    true,
				ExpiryTS:     time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(),
			},
		},
	}

	for name, tc := range tests {
		t.Run(name, func(t *testing.T) {
			assert.Equal(t, tc.want, shouldIgnoreMikroTikUpgradeUntil(tc.row))
		})
	}
}

func TestExtractLicenseRows_DropsMikroTikSentinelOnlyRowAfterVendorSanity(t *testing.T) {
	tests := map[string]struct {
		pm *ddsnmp.ProfileMetrics
	}{
		"sentinel-only row is dropped": {
			pm: &ddsnmp.ProfileMetrics{
				Source: "profiles/mikrotik-router.yaml",
				LicenseRows: []ddsnmp.LicenseRow{typedLicenseRow(
					"routeros_upgrade",
					"RouterOS upgrade entitlement",
					withOriginProfileID(mikroTikLicenseSource),
					withExpiry(time.Date(1970, time.January, 2, 0, 0, 0, 0, time.UTC).Unix()),
					withExpirySource(mikroTikUpgradeUntilExpirySource),
				)},
			},
		},
	}
	now := time.Date(2026, time.April, 10, 12, 0, 0, 0, time.UTC)

	for name, tc := range tests {
		t.Run(name, func(t *testing.T) {
			rows := extractLicenseRows([]*ddsnmp.ProfileMetrics{tc.pm}, now)

			assert.Empty(t, rows)
		})
	}
}
