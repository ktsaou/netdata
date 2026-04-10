// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"testing"

	"github.com/gosnmp/gosnmp"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
)

func TestScaleFactorDeferral(t *testing.T) {
	tests := map[string]struct {
		symbol    ddprofiledefinition.SymbolConfig
		pdu       gosnmp.SnmpPDU
		wantApply bool
		wantMul   int
		wantDiv   int
		wantOK    bool
	}{
		"inferred counter rate defers octets to bits scale": {
			symbol: ddprofiledefinition.SymbolConfig{
				ScaleFactor: 8,
			},
			pdu: gosnmp.SnmpPDU{
				Type: gosnmp.Counter32,
			},
			wantMul: 8,
			wantDiv: 1,
			wantOK:  true,
		},
		"explicit deprecated counter defers fractional scale": {
			symbol: ddprofiledefinition.SymbolConfig{
				MetricType:  ddprofiledefinition.ProfileMetricTypeCounter,
				ScaleFactor: 0.01,
			},
			pdu: gosnmp.SnmpPDU{
				Type: gosnmp.Gauge32,
			},
			wantMul: 1,
			wantDiv: 100,
			wantOK:  true,
		},
		"gauge applies scale directly": {
			symbol: ddprofiledefinition.SymbolConfig{
				MetricType:  ddprofiledefinition.ProfileMetricTypeGauge,
				ScaleFactor: 8,
			},
			pdu: gosnmp.SnmpPDU{
				Type: gosnmp.Counter32,
			},
			wantApply: true,
		},
		"rate with non-representable scale still never pre-scales": {
			symbol: ddprofiledefinition.SymbolConfig{
				MetricType:  ddprofiledefinition.ProfileMetricTypeRate,
				ScaleFactor: 1e100,
			},
			pdu: gosnmp.SnmpPDU{
				Type: gosnmp.Counter64,
			},
		},
		"zero scale does not apply or defer": {
			symbol: ddprofiledefinition.SymbolConfig{
				MetricType: ddprofiledefinition.ProfileMetricTypeRate,
			},
			pdu: gosnmp.SnmpPDU{
				Type: gosnmp.Counter64,
			},
		},
	}

	for name, tc := range tests {
		t.Run(name, func(t *testing.T) {
			require.Equal(t, tc.wantApply, shouldApplyScaleFactorToValue(tc.symbol, tc.pdu))
			mul, div, ok := deferredScaleFactor(tc.symbol, tc.pdu)
			require.Equal(t, tc.wantOK, ok)
			require.Equal(t, tc.wantMul, mul)
			require.Equal(t, tc.wantDiv, div)
		})
	}
}

func TestNumericValueProcessor_ProcessValue_DefersCounterScaleFactor(t *testing.T) {
	processor := newValueProcessor()

	value, err := processor.processValue(
		ddprofiledefinition.SymbolConfig{ScaleFactor: 8},
		gosnmp.SnmpPDU{Type: gosnmp.Counter32, Value: uint(100)},
	)
	require.NoError(t, err)
	require.Equal(t, int64(100), value)

	value, err = processor.processValue(
		ddprofiledefinition.SymbolConfig{MetricType: ddprofiledefinition.ProfileMetricTypeGauge, ScaleFactor: 8},
		gosnmp.SnmpPDU{Type: gosnmp.Counter32, Value: uint(100)},
	)
	require.NoError(t, err)
	require.Equal(t, int64(800), value)
}
