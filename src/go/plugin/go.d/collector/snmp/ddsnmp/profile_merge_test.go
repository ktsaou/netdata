// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmp

import (
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/pkg/multipath"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
)

func TestProfile_MultipleExtends_TableSymbolLaterOverrideEarlierByNameWithinTable(t *testing.T) {
	tmp := t.TempDir()

	writeTableBase(t, filepath.Join(tmp, "_base1.yaml"), "1.3.6.1.2.1.2.2", "ifTable", "1.3.6.1.2.1.2.2.1.10", "ifInOctets", "base1")
	writeTableBase(t, filepath.Join(tmp, "_base2.yaml"), "1.3.6.1.2.1.2.2", "ifTable", "1.3.6.1.2.1.2.2.1.16", "ifInOctets", "base2")
	writeYAML(t, filepath.Join(tmp, "device.yaml"), ddprofiledefinition.ProfileDefinition{
		Extends: []string{"_base1.yaml", "_base2.yaml"},
	})

	prof, err := loadProfile(filepath.Join(tmp, "device.yaml"), multipath.New(tmp))
	require.NoError(t, err)
	require.Len(t, prof.Definition.Metrics, 1)
	require.Len(t, prof.Definition.Metrics[0].Symbols, 1)

	sym := prof.Definition.Metrics[0].Symbols[0]
	assert.Equal(t, "ifInOctets", sym.Name)
	assert.Equal(t, "1.3.6.1.2.1.2.2.1.16", sym.OID)
	assert.Equal(t, "base2", sym.ChartMeta.Description)
}

func TestProfile_MultipleExtends_TableSymbolsPreserveSameOIDDifferentNames(t *testing.T) {
	tmp := t.TempDir()

	writeTableBase(t, filepath.Join(tmp, "_base1.yaml"), "1.3.6.1.4.1.999.1", "eventTable", "1.3.6.1.4.1.999.1.1.5", "eventCode", "base1")
	writeTableBase(t, filepath.Join(tmp, "_base2.yaml"), "1.3.6.1.4.1.999.1", "eventTable", "1.3.6.1.4.1.999.1.1.5", "eventSubCode", "base2")
	writeYAML(t, filepath.Join(tmp, "device.yaml"), ddprofiledefinition.ProfileDefinition{
		Extends: []string{"_base1.yaml", "_base2.yaml"},
	})

	prof, err := loadProfile(filepath.Join(tmp, "device.yaml"), multipath.New(tmp))
	require.NoError(t, err)
	require.Len(t, prof.Definition.Metrics, 2)

	got := make(map[string]string)
	for _, metric := range prof.Definition.Metrics {
		require.Len(t, metric.Symbols, 1)
		got[metric.Symbols[0].Name] = metric.Symbols[0].OID
	}

	assert.Equal(t, map[string]string{
		"eventCode":    "1.3.6.1.4.1.999.1.1.5",
		"eventSubCode": "1.3.6.1.4.1.999.1.1.5",
	}, got)
}

func writeTableBase(t *testing.T, path, tableOID, tableName, symbolOID, symbolName, description string) {
	t.Helper()

	writeYAML(t, path, ddprofiledefinition.ProfileDefinition{
		Metrics: []ddprofiledefinition.MetricsConfig{
			{
				Table: ddprofiledefinition.SymbolConfig{
					OID:  tableOID,
					Name: tableName,
				},
				Symbols: []ddprofiledefinition.SymbolConfig{
					{
						OID:  symbolOID,
						Name: symbolName,
						ChartMeta: ddprofiledefinition.ChartMeta{
							Description: description,
						},
					},
				},
				MetricTags: []ddprofiledefinition.MetricTagConfig{
					{
						Tag: "row",
						IndexTransform: []ddprofiledefinition.MetricIndexTransform{
							{Start: 0, End: 0},
						},
					},
				},
			},
		},
	})
}
