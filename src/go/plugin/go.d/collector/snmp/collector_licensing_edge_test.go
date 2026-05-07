// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"context"
	"testing"

	"github.com/golang/mock/gomock"
	"github.com/gosnmp/gosnmp"
	snmpmock "github.com/gosnmp/gosnmp/mocks"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	ddsnmpcollector "github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddsnmpcollector"
)

func TestCollector_Collect_LicensingAggregation_ReadsTypedRowsAndIgnoresHiddenMetrics(t *testing.T) {
	mockCtl := gomock.NewController(t)
	defer mockCtl.Finish()

	mockSNMP := snmpmock.NewMockHandler(mockCtl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = false
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		pm := &ddsnmp.ProfileMetrics{
			Source: "noise-licensing.yaml",
			LicenseRows: []ddsnmp.LicenseRow{
				typedLicenseRow("healthy", "Healthy license", withState(0, "active")),
			},
			HiddenMetrics: []ddsnmp.Metric{
				{
					Name:  "_license_row",
					Value: 0,
					Tags: map[string]string{
						"_license_id":         "hidden-healthy",
						"_license_name":       "Hidden healthy license",
						"_license_state_raw":  "active",
						"_license_value_kind": "state_severity",
					},
				},
				{
					Name:  "_license_row",
					Value: 123,
					Tags: map[string]string{
						"_license_id":   "noise",
						"_license_name": "Noise without kind",
					},
				},
				{
					Name:  "_license_row_expiry",
					Value: 999,
					Tags: map[string]string{
						"_license_id":   "bad-expiry",
						"_license_name": "Bad expiry helper",
					},
				},
			},
		}
		for i := range pm.HiddenMetrics {
			pm.HiddenMetrics[i].Profile = pm
		}
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{pm}}
	}

	require.NoError(t, collr.Init(context.Background()))
	_ = collr.Check(context.Background())

	got := collr.Collect(context.Background())
	require.NotNil(t, got)

	assert.EqualValues(t, 1, got[metricIDLicenseStateHealthy])
	assert.EqualValues(t, 0, got[metricIDLicenseStateInformational])
	assert.EqualValues(t, 0, got[metricIDLicenseStateDegraded])
	assert.EqualValues(t, 0, got[metricIDLicenseStateBroken])
	assert.EqualValues(t, 0, got[metricIDLicenseStateIgnored])
	assert.NotContains(t, got, metricIDLicenseRemainingTime)
	assert.NotContains(t, got, metricIDLicenseAuthorizationRemainingTime)
	assert.NotContains(t, got, metricIDLicenseCertificateRemainingTime)
	assert.NotContains(t, got, metricIDLicenseGraceRemainingTime)
	assert.NotContains(t, got, metricIDLicenseUsagePercent)
}
