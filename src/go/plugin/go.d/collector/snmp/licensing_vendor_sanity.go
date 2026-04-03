// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import "time"

const (
	mikroTikLicenseSource             = "mikrotik-router"
	mikroTikUpgradeUntilExpirySource  = "mtxrLicUpgrUntil"
	mikroTikEpochLikeExpiryYearCutoff = 1971
)

func applyVendorLicenseSanityRules(row *licenseRow) {
	applyMikroTikLicenseSanityRules(row)
}

func applyMikroTikLicenseSanityRules(row *licenseRow) {
	if !shouldIgnoreMikroTikUpgradeUntil(*row) {
		return
	}

	row.HasExpiry = false
	row.ExpiryTS = 0
}

func shouldIgnoreMikroTikUpgradeUntil(row licenseRow) bool {
	if row.Source != mikroTikLicenseSource {
		return false
	}
	if row.ExpirySource != mikroTikUpgradeUntilExpirySource {
		return false
	}
	if !row.HasExpiry || row.ExpiryTS <= 0 {
		return false
	}

	return isEpochLikeLicenseTimestamp(row.ExpiryTS)
}

func isEpochLikeLicenseTimestamp(ts int64) bool {
	return time.Unix(ts, 0).UTC().Year() < mikroTikEpochLikeExpiryYearCutoff
}
