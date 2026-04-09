// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

func TestShouldIgnoreMikroTikUpgradeUntil_EpochLikeTimestamp(t *testing.T) {
	row := licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		ExpiryTS:     time.Date(1970, time.January, 2, 0, 0, 0, 0, time.UTC).Unix(),
	}

	assert.True(t, shouldIgnoreMikroTikUpgradeUntil(row))
}

func TestShouldIgnoreMikroTikUpgradeUntil_RequiresMatchingSourceAndField(t *testing.T) {
	assert.False(t, shouldIgnoreMikroTikUpgradeUntil(licenseRow{
		Source:       "checkpoint",
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		ExpiryTS:     86400,
	}))
	assert.False(t, shouldIgnoreMikroTikUpgradeUntil(licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: "other",
		HasExpiry:    true,
		ExpiryTS:     86400,
	}))
}

func TestShouldIgnoreMikroTikUpgradeUntil_KeepsRealExpiry(t *testing.T) {
	row := licenseRow{
		Source:       mikroTikLicenseSource,
		ExpirySource: mikroTikUpgradeUntilExpirySource,
		HasExpiry:    true,
		ExpiryTS:     time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(),
	}

	assert.False(t, shouldIgnoreMikroTikUpgradeUntil(row))
}
