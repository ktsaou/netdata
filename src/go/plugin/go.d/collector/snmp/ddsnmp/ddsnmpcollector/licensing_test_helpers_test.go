// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func mustLoadLicensingProfile(t *testing.T, profileName string, keep func(metric ddprofiledefinition.MetricsConfig) bool) *ddsnmp.Profile {
	t.Helper()

	profiles := ddsnmp.FindProfiles("", "", []string{profileName})
	require.Len(t, profiles, 1)

	profile := profiles[0]
	profile.Definition.Metadata = nil
	profile.Definition.SysobjectIDMetadata = nil
	profile.Definition.MetricTags = nil
	profile.Definition.StaticTags = nil
	profile.Definition.VirtualMetrics = nil
	profile.Definition.Topology = nil
	profile.Definition.Licensing = nil
	profile.Definition.Metrics = slices.DeleteFunc(profile.Definition.Metrics, func(metric ddprofiledefinition.MetricsConfig) bool {
		return !keep(metric)
	})

	require.NotEmpty(t, profile.Definition.Metrics)
	assert.Equal(t, profileName+".yaml", filepath.Base(profile.SourceFile))

	return profile
}

func mustLoadTypedLicensingProfile(t *testing.T, profileName string, keep func(row ddprofiledefinition.LicensingConfig) bool) *ddsnmp.Profile {
	t.Helper()

	profiles := ddsnmp.DefaultCatalog().Resolve(ddsnmp.ResolveRequest{
		ManualProfiles: []string{profileName},
		ManualPolicy:   ddsnmp.ManualProfileFallback,
	}).Project(ddsnmp.ConsumerLicensing).Profiles()
	require.Len(t, profiles, 1)

	profile := profiles[0]
	profile.Definition.Metadata = nil
	profile.Definition.SysobjectIDMetadata = nil
	profile.Definition.MetricTags = nil
	profile.Definition.StaticTags = nil
	profile.Definition.VirtualMetrics = nil
	profile.Definition.Topology = nil
	profile.Definition.Metrics = nil
	profile.Definition.Licensing = slices.DeleteFunc(profile.Definition.Licensing, func(row ddprofiledefinition.LicensingConfig) bool {
		return !keep(row)
	})

	require.NotEmpty(t, profile.Definition.Licensing)
	assert.Equal(t, profileName+".yaml", filepath.Base(profile.SourceFile))

	return profile
}

func metricTagValue(metric ddsnmp.Metric, key string) string {
	if v := metric.Tags[key]; v != "" {
		return v
	}
	return metric.StaticTags[key]
}

func licenseMetricID(metric ddsnmp.Metric) string {
	if v := metric.StaticTags["_license_id"]; v != "" {
		return v
	}
	return metric.Tags["_license_id"]
}

func mustLicenseMetricsByIDAndKind(t *testing.T, metrics []ddsnmp.Metric) map[string]map[string]*ddsnmp.Metric {
	t.Helper()

	out := make(map[string]map[string]*ddsnmp.Metric)
	for i := range metrics {
		metric := &metrics[i]
		id := licenseMetricID(*metric)
		if id == "" {
			continue
		}
		kind := metricTagValue(*metric, "_license_value_kind")
		if kind == "" {
			continue
		}
		if out[id] == nil {
			out[id] = make(map[string]*ddsnmp.Metric)
		}
		if existing := out[id][kind]; existing != nil {
			t.Fatalf("duplicate license metric for id %q and kind %q: %s and %s", id, kind, existing.Name, metric.Name)
		}
		out[id][kind] = metric
	}
	return out
}

func licenseRowsByID(rows []ddsnmp.LicenseRow) map[string]ddsnmp.LicenseRow {
	out := make(map[string]ddsnmp.LicenseRow, len(rows))
	for _, row := range rows {
		out[row.ID] = row
	}
	return out
}

func hasMetricTable(profile *ddsnmp.Profile, oid string) bool {
	for _, metric := range profile.Definition.Metrics {
		if strings.TrimPrefix(metric.Table.OID, ".") == oid {
			return true
		}
	}
	return false
}

func hasLicensingTable(profile *ddsnmp.Profile, oid string) bool {
	for _, row := range profile.Definition.Licensing {
		if strings.TrimPrefix(row.Table.OID, ".") == oid {
			return true
		}
	}
	return false
}
