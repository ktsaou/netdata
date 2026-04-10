// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"math"
	"math/big"
	"strconv"

	"github.com/gosnmp/gosnmp"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
)

func symbolMetricType(sym ddprofiledefinition.SymbolConfig, pdu gosnmp.SnmpPDU) ddprofiledefinition.ProfileMetricType {
	if sym.MetricType != "" {
		return sym.MetricType
	}
	return getMetricTypeFromPDUType(pdu)
}

func shouldApplyScaleFactorToValue(sym ddprofiledefinition.SymbolConfig, pdu gosnmp.SnmpPDU) bool {
	if sym.ScaleFactor == 0 {
		return false
	}

	_, _, deferred := deferredScaleFactor(sym, pdu)
	return !deferred
}

func deferredScaleFactor(sym ddprofiledefinition.SymbolConfig, pdu gosnmp.SnmpPDU) (mul, div int, ok bool) {
	if sym.ScaleFactor == 0 {
		return 0, 0, false
	}

	switch symbolMetricType(sym, pdu) {
	case ddprofiledefinition.ProfileMetricTypeRate, ddprofiledefinition.ProfileMetricTypeCounter:
		return scaleFactorToMulDiv(sym.ScaleFactor)
	default:
		return 0, 0, false
	}
}

func scaleFactorToMulDiv(scale float64) (mul, div int, ok bool) {
	if scale == 0 || math.IsNaN(scale) || math.IsInf(scale, 0) {
		return 0, 0, false
	}

	rat, ok := new(big.Rat).SetString(strconv.FormatFloat(scale, 'f', -1, 64))
	if !ok || rat.Sign() == 0 || !rat.Num().IsInt64() || !rat.Denom().IsInt64() {
		return 0, 0, false
	}

	n, d := rat.Num().Int64(), rat.Denom().Int64()
	if d <= 0 || int64(int(n)) != n || int64(int(d)) != d {
		return 0, 0, false
	}

	return int(n), int(d), true
}
