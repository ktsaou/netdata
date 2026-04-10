// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
)

// Licensing pipeline contract — see src/go/plugin/go.d/collector/snmp/profile-format.md §"Licensing rows".
//
// Profiles describe each licensing signal as a hidden ("_"-prefixed) metric
// named "_license_row". The integer value carries a single signal (expiry
// epoch, used count, severity, etc). Profiles stamp the "_license_value_kind"
// tag, usually with the generic setTag transform, so this pipeline knows how
// to interpret the value. Identifying fields (id, name, component, ...) are set
// as tags in the profile, using the documented tag-fallback pattern.
//
// Rows are grouped by (profile source, SNMP table, _license_id) so a single
// license can carry both an expiry and a usage signal in separate metrics
// without colliding with another table that happens to share the same id.

const (
	licenseSourceMetricName = "_license_row"

	tagLicenseID                  = "_license_id"
	tagLicenseName                = "_license_name"
	tagLicenseFeature             = "_license_feature"
	tagLicenseComponent           = "_license_component"
	tagLicenseType                = "_license_type"
	tagLicenseImpact              = "_license_impact"
	tagLicenseStateRaw            = "_license_state_raw"
	tagLicenseUnlimited           = "_license_unlimited"
	tagLicensePerpetual           = "_license_perpetual"
	tagLicenseExpirySource        = "_license_expiry_source"
	tagLicenseAuthorizationSource = "_license_authorization_source"
	tagLicenseCertificateSource   = "_license_certificate_source"
	tagLicenseGraceSource         = "_license_grace_source"
	tagLicenseValueKind           = "_license_value_kind"

	// Value kinds accepted in the profile-stamped "_license_value_kind" tag.
	licenseValueKindExpiryTimestamp        = "expiry_timestamp"
	licenseValueKindExpiryRemaining        = "expiry_remaining"
	licenseValueKindAuthorizationTimestamp = "authorization_timestamp"
	licenseValueKindAuthorizationRemaining = "authorization_remaining"
	licenseValueKindCertificateTimestamp   = "certificate_timestamp"
	licenseValueKindCertificateRemaining   = "certificate_remaining"
	licenseValueKindGraceTimestamp         = "grace_timestamp"
	licenseValueKindGraceRemaining         = "grace_remaining"
	licenseValueKindUsage                  = "usage"
	licenseValueKindCapacity               = "capacity"
	licenseValueKindAvailable              = "available"
	licenseValueKindUsagePercent           = "usage_percent"
	licenseValueKindStateSeverity          = "state_severity"

	metricIDLicenseRemainingTime              = "snmp_device_license_remaining_time"
	metricIDLicenseAuthorizationRemainingTime = "snmp_device_license_authorization_remaining_time"
	metricIDLicenseCertificateRemainingTime   = "snmp_device_license_certificate_remaining_time"
	metricIDLicenseGraceRemainingTime         = "snmp_device_license_grace_remaining_time"
	metricIDLicenseUsagePercent               = "snmp_device_license_usage_percent"
	metricIDLicenseStateHealthy               = "snmp_device_license_state_healthy"
	metricIDLicenseStateDegraded              = "snmp_device_license_state_degraded"
	metricIDLicenseStateBroken                = "snmp_device_license_state_broken"
	metricIDLicenseStateIgnored               = "snmp_device_license_state_ignored"
)

type licenseRow struct {
	ID     string
	Source string
	Table  string
	Name   string

	Feature   string
	Component string
	Type      string
	Impact    string

	StateRaw      string
	StateSeverity int64
	HasState      bool
	StateBucket   licenseStateBucket

	ExpiryTS             int64
	HasExpiry            bool
	AuthorizationExpiry  int64
	HasAuthorizationTime bool
	CertificateExpiry    int64
	HasCertificateTime   bool
	GraceExpiry          int64
	HasGraceTime         bool

	Usage        int64
	HasUsage     bool
	Capacity     int64
	HasCapacity  bool
	Available    int64
	HasAvailable bool
	UsagePercent float64
	HasUsagePct  bool

	IsUnlimited bool
	IsPerpetual bool

	ExpirySource string
	AuthSource   string
	CertSource   string
	GraceSource  string

	OriginalMetric string
}

type licenseRowMergeKey struct {
	Source string
	Table  string
	ID     string
}

type licenseCache struct {
	mu         sync.RWMutex
	lastUpdate time.Time
	rows       []licenseRow
}

func newLicenseCache() *licenseCache {
	return &licenseCache{}
}

func (c *licenseCache) store(ts time.Time, rows []licenseRow) {
	c.mu.Lock()
	defer c.mu.Unlock()

	c.lastUpdate = ts
	c.rows = slices.Clone(rows)
}

func (c *licenseCache) snapshot() (time.Time, []licenseRow) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	return c.lastUpdate, slices.Clone(c.rows)
}

// extractLicenseRows walks the HiddenMetrics of each profile, groups
// _license_row signals by (source, table, _license_id), and populates a
// licenseRow per group. Per-row vendor sanity rules and state-bucket
// normalization run once per merged row at the end.
//
// Metrics that do not declare a _license_id are dropped: there is no safe
// fallback for grouping, and silently collapsing them would corrupt
// device-level aggregates.
func extractLicenseRows(pms []*ddsnmp.ProfileMetrics, now time.Time) []licenseRow {
	var rows []licenseRow
	rowIdx := make(map[licenseRowMergeKey]int)

	for _, pm := range pms {
		if pm == nil {
			continue
		}
		source := stripFileNameExt(pm.Source)

		for _, metric := range pm.HiddenMetrics {
			if !isLicenseSourceMetric(metric.Name) {
				continue
			}
			tags := mergeLicenseTags(metric)
			id := strings.TrimSpace(tags[tagLicenseID])
			if id == "" {
				// No identity → cannot safely merge or stand alone.
				continue
			}
			// metric.Table differentiates rows that come from different SNMP
			// tables but happen to share the same _license_id (e.g., Fortinet
			// fgLicContractTable vs. fgLicVersionTable both keying by
			// description). Scalars have an empty Table so they continue to
			// merge across metric definitions, which is what Cisco Smart
			// Licensing relies on.
			key := licenseRowMergeKey{
				Source: source,
				Table:  metric.Table,
				ID:     id,
			}

			idx, seen := rowIdx[key]
			if !seen {
				rows = append(rows, newLicenseRow(source, metric.Table, id, metric.Name, tags))
				idx = len(rows) - 1
				rowIdx[key] = idx
			}
			mergeLicenseSignal(&rows[idx], metric.Value, tags[tagLicenseValueKind], now)
		}
	}

	// Drop rows that produced no merged signal at all. These come from
	// transforms that failed to parse a vendor format (e.g.,
	// licenseDateFromTag rejecting an unknown date string), where the row was
	// allocated but no value_kind ever stamped its value. Keeping them would
	// silently inflate the "ignored" bucket on the device-level chart.
	rows = slices.DeleteFunc(rows, func(row licenseRow) bool {
		return !licenseRowHasAnySignal(row)
	})

	for i := range rows {
		applyVendorLicenseSanityRules(&rows[i])
		rows[i].StateBucket = normalizeLicenseStateBucket(rows[i], now)
	}

	return rows
}

func isLicenseSourceMetric(name string) bool {
	return strings.HasPrefix(name, licenseSourceMetricName)
}

// licenseRowHasAnySignal returns true when at least one merged signal
// (state, expiry, auth/cert/grace timer, usage shape) is set on the row.
func licenseRowHasAnySignal(row licenseRow) bool {
	return row.HasState ||
		row.HasExpiry ||
		row.HasAuthorizationTime ||
		row.HasCertificateTime ||
		row.HasGraceTime ||
		row.HasUsage ||
		row.HasCapacity ||
		row.HasAvailable ||
		row.HasUsagePct
}

// newLicenseRow allocates a row populated entirely from the profile-declared
// tags. Identity (_license_id, _license_name) and any optional descriptive
// fields must come from YAML — the documented "tag fallback (first non-empty
// wins)" pattern is the right place to express vendor preference order.
func newLicenseRow(source, table, id, metricName string, tags map[string]string) licenseRow {
	return licenseRow{
		Source:         source,
		Table:          table,
		ID:             id,
		Name:           tags[tagLicenseName],
		Feature:        tags[tagLicenseFeature],
		Component:      tags[tagLicenseComponent],
		Type:           tags[tagLicenseType],
		Impact:         tags[tagLicenseImpact],
		StateRaw:       tags[tagLicenseStateRaw],
		ExpirySource:   tags[tagLicenseExpirySource],
		AuthSource:     tags[tagLicenseAuthorizationSource],
		CertSource:     tags[tagLicenseCertificateSource],
		GraceSource:    tags[tagLicenseGraceSource],
		OriginalMetric: metricName,
		IsUnlimited:    parseLicenseBool(tags[tagLicenseUnlimited]),
		IsPerpetual:    parseLicenseBool(tags[tagLicensePerpetual]),
	}
}

// licenseTimestampSentinelMaxUint32 is the 0xFFFFFFFF "never expires"
// sentinel some vendors emit as a Gauge32. Treating it as a real epoch would
// give a fake "remaining time = ~80 years" signal that pollutes aggregates,
// so it is filtered out at the merge boundary for every *_timestamp kind.
const licenseTimestampSentinelMaxUint32 = int64(4294967295)

// isLicenseTimestampSentinel returns true for values that look like a
// vendor "no expiry" sentinel rather than a real unix epoch.
func isLicenseTimestampSentinel(value int64) bool {
	return value <= 0 || value == licenseTimestampSentinelMaxUint32
}

// mergeLicenseSignal folds one metric's numeric value into the licenseRow,
// using the profile-declared value kind to decide which field it belongs to.
// Signals encountered twice do not overwrite an already-populated field.
func mergeLicenseSignal(row *licenseRow, value int64, kind string, now time.Time) {
	switch kind {
	case licenseValueKindStateSeverity:
		if row.HasState {
			return
		}
		if value < 0 {
			value = 0
		}
		if value > 2 {
			value = 2
		}
		row.StateSeverity = value
		row.HasState = true

	case licenseValueKindExpiryTimestamp:
		if row.HasExpiry || isLicenseTimestampSentinel(value) {
			return
		}
		row.ExpiryTS = value
		row.HasExpiry = true
	case licenseValueKindExpiryRemaining:
		if row.HasExpiry {
			return
		}
		row.ExpiryTS = now.Unix() + value
		row.HasExpiry = true

	case licenseValueKindAuthorizationTimestamp:
		if row.HasAuthorizationTime || isLicenseTimestampSentinel(value) {
			return
		}
		row.AuthorizationExpiry = value
		row.HasAuthorizationTime = true
	case licenseValueKindAuthorizationRemaining:
		if row.HasAuthorizationTime {
			return
		}
		row.AuthorizationExpiry = now.Unix() + value
		row.HasAuthorizationTime = true

	case licenseValueKindCertificateTimestamp:
		if row.HasCertificateTime || isLicenseTimestampSentinel(value) {
			return
		}
		row.CertificateExpiry = value
		row.HasCertificateTime = true
	case licenseValueKindCertificateRemaining:
		if row.HasCertificateTime {
			return
		}
		row.CertificateExpiry = now.Unix() + value
		row.HasCertificateTime = true

	case licenseValueKindGraceTimestamp:
		if row.HasGraceTime || isLicenseTimestampSentinel(value) {
			return
		}
		row.GraceExpiry = value
		row.HasGraceTime = true
	case licenseValueKindGraceRemaining:
		if row.HasGraceTime {
			return
		}
		row.GraceExpiry = now.Unix() + value
		row.HasGraceTime = true

	case licenseValueKindUsage:
		if row.HasUsage {
			return
		}
		row.Usage = value
		row.HasUsage = true
	case licenseValueKindCapacity:
		if row.HasCapacity {
			return
		}
		row.Capacity = value
		row.HasCapacity = true
	case licenseValueKindAvailable:
		if row.HasAvailable {
			return
		}
		row.Available = value
		row.HasAvailable = true
	case licenseValueKindUsagePercent:
		if row.HasUsagePct {
			return
		}
		row.UsagePercent = float64(value)
		row.HasUsagePct = true
	}

	if !row.HasUsage && row.HasCapacity && row.HasAvailable && row.Available >= 0 && row.Available <= row.Capacity {
		row.Usage = row.Capacity - row.Available
		row.HasUsage = true
	}
	if !row.HasUsagePct && row.HasUsage && row.HasCapacity && row.Capacity > 0 && !row.IsUnlimited {
		row.UsagePercent = float64(row.Usage) * 100 / float64(row.Capacity)
		row.HasUsagePct = true
	}
}

func mergeLicenseTags(metric ddsnmp.Metric) map[string]string {
	switch {
	case len(metric.StaticTags) == 0:
		return metric.Tags
	case len(metric.Tags) == 0:
		return metric.StaticTags
	}

	tags := make(map[string]string, len(metric.StaticTags)+len(metric.Tags))
	for k, v := range metric.StaticTags {
		tags[k] = v
	}
	for k, v := range metric.Tags {
		tags[k] = v
	}
	return tags
}

func parseLicenseBool(raw string) bool {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "1", "true", "yes", "y", "enabled", "active", "perpetual", "unlimited":
		return true
	default:
		return false
	}
}

func firstNonBlank(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}
