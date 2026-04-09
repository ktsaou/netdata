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
	profile.Definition.Metrics = slices.DeleteFunc(profile.Definition.Metrics, func(metric ddprofiledefinition.MetricsConfig) bool {
		return !keep(metric)
	})

	require.NotEmpty(t, profile.Definition.Metrics)
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

func licenseMetricsByIDAndKind(metrics []ddsnmp.Metric) map[string]map[string]*ddsnmp.Metric {
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
		out[id][kind] = metric
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
