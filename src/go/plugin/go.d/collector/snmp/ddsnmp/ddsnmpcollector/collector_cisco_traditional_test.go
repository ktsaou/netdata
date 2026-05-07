// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/gosnmp/gosnmp"
	snmpmock "github.com/gosnmp/gosnmp/mocks"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/logger"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddprofiledefinition"
)

func TestCollector_Collect_CiscoTraditionalLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectCiscoTraditionalLicensingWalk(mockHandler,
		ciscoTraditionalRow{
			entPhysical:   1,
			storeUsed:     1,
			index:         17,
			name:          "SECURITYK9",
			version:       "1.0",
			licenseType:   5,
			remaining:     0,
			capacity:      100,
			available:     15,
			impact:        "Security subscription",
			status:        3,
			endDate:       time.Date(2030, time.November, 11, 0, 0, 0, 0, time.UTC),
			hasEndDatePDU: true,
		},
		ciscoTraditionalRow{
			entPhysical: 1,
			storeUsed:   2,
			index:       23,
			name:        "APPXK9",
			version:     "2.1",
			licenseType: 3,
			remaining:   7200,
			capacity:    10,
			available:   0,
			impact:      "Session count exhausted",
			status:      6,
		},
	)

	profile := mustLoadLicensingProfile(t, "cisco", func(metric ddprofiledefinition.MetricsConfig) bool {
		return strings.TrimPrefix(metric.Table.OID, ".") == "1.3.6.1.4.1.9.9.543.1.2.3.1"
	})
	require.Len(t, profile.Definition.Metrics, 1)

	collector := New(Config{
		SnmpClient:  mockHandler,
		Profiles:    []*ddsnmp.Profile{profile},
		Log:         logger.New(),
		SysObjectID: "",
	})

	results, err := collector.Collect()
	require.NoError(t, err)
	require.Len(t, results, 1)

	pm := results[0]
	assert.Empty(t, pm.Metrics)
	require.Len(t, pm.HiddenMetrics, 7)

	byID := mustLicenseMetricsByIDAndKind(t, pm.HiddenMetrics)

	subscription := byID["SECURITYK9"]
	require.NotNil(t, subscription)
	require.EqualValues(t, time.Date(2030, time.November, 11, 0, 0, 0, 0, time.UTC).Unix(), subscription["expiry_timestamp"].Value)
	require.EqualValues(t, 100, subscription["capacity"].Value)
	require.EqualValues(t, 15, subscription["available"].Value)
	require.EqualValues(t, 0, subscription["state_severity"].Value)
	assert.Equal(t, "SECURITYK9", metricTagValue(*subscription["state_severity"], "_license_name"))
	assert.Equal(t, "paid_subscription", metricTagValue(*subscription["state_severity"], "_license_type"))
	assert.Equal(t, "in_use", metricTagValue(*subscription["state_severity"], "_license_state_raw"))
	assert.Equal(t, "Security subscription", metricTagValue(*subscription["state_severity"], "_license_impact"))

	grace := byID["APPXK9"]
	require.NotNil(t, grace)
	require.EqualValues(t, 10, grace["capacity"].Value)
	require.EqualValues(t, 0, grace["available"].Value)
	require.EqualValues(t, 2, grace["state_severity"].Value)
	assert.Equal(t, "APPXK9", metricTagValue(*grace["state_severity"], "_license_name"))
	assert.Equal(t, "grace_period", metricTagValue(*grace["state_severity"], "_license_type"))
	assert.Equal(t, "usage_count_consumed", metricTagValue(*grace["state_severity"], "_license_state_raw"))
	assert.Equal(t, "Session count exhausted", metricTagValue(*grace["state_severity"], "_license_impact"))
	assert.NotContains(t, grace, "expiry_timestamp")
}

type ciscoTraditionalRow struct {
	entPhysical   int
	storeUsed     int
	index         int
	name          string
	version       string
	licenseType   int
	remaining     uint
	capacity      uint
	available     uint
	impact        string
	status        int
	endDate       time.Time
	hasEndDatePDU bool
}

func expectCiscoTraditionalLicensingWalk(mockHandler *snmpmock.MockHandler, rows ...ciscoTraditionalRow) {
	pdus := make([]gosnmp.SnmpPDU, 0, len(rows)*10)

	for _, row := range rows {
		idx := strconv.Itoa(row.entPhysical) + "." + strconv.Itoa(row.storeUsed) + "." + strconv.Itoa(row.index)

		pdus = append(pdus,
			createStringPDU("1.3.6.1.4.1.9.9.543.1.2.3.1.3."+idx, row.name),
			createStringPDU("1.3.6.1.4.1.9.9.543.1.2.3.1.4."+idx, row.version),
			createIntegerPDU("1.3.6.1.4.1.9.9.543.1.2.3.1.5."+idx, row.licenseType),
			createGauge32PDU("1.3.6.1.4.1.9.9.543.1.2.3.1.8."+idx, row.remaining),
			createGauge32PDU("1.3.6.1.4.1.9.9.543.1.2.3.1.10."+idx, row.capacity),
			createGauge32PDU("1.3.6.1.4.1.9.9.543.1.2.3.1.11."+idx, row.available),
			createStringPDU("1.3.6.1.4.1.9.9.543.1.2.3.1.13."+idx, row.impact),
			createIntegerPDU("1.3.6.1.4.1.9.9.543.1.2.3.1.14."+idx, row.status),
		)
		if row.hasEndDatePDU {
			pdus = append(pdus, createDateAndTimePDU("1.3.6.1.4.1.9.9.543.1.2.3.1.16."+idx, row.endDate))
		}
	}

	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.9.9.543.1.2.3.1",
		pdus,
	)
}
