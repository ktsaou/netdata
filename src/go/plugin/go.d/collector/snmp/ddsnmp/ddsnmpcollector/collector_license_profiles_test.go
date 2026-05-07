// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
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

func TestCollector_Collect_CheckPointLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.2620.1.6.18.1",
		[]gosnmp.SnmpPDU{
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.1.17", 17),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.2.17", 17),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.4.17", "Application Control"),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.5.17", "about-to-expire"),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.6.17", 1775152800),
			createStringPDU("1.3.6.1.4.1.2620.1.6.18.1.1.7.17", "Threat prevention coverage"),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.9.17", 100),
			createGauge32PDU("1.3.6.1.4.1.2620.1.6.18.1.1.10.17", 85),
		},
	)

	profile := mustLoadTypedLicensingProfile(t, "checkpoint", func(row ddprofiledefinition.LicensingConfig) bool {
		return strings.TrimPrefix(row.Table.OID, ".") == "1.3.6.1.4.1.2620.1.6.18.1"
	})

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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 1)

	row := pm.LicenseRows[0]
	assert.Equal(t, "17", row.ID)
	assert.Equal(t, "Application Control", row.Name)
	assert.Equal(t, "about-to-expire", row.State.Raw)
	assert.True(t, row.State.Has)
	assert.EqualValues(t, 1, row.State.Severity)
	assert.EqualValues(t, 1775152800, row.Expiry.Timestamp)
	assert.True(t, row.Expiry.Has)
	assert.Equal(t, "Threat prevention coverage", row.Impact)
	assert.EqualValues(t, 100, row.Usage.Capacity)
	assert.True(t, row.Usage.HasCapacity)
	assert.EqualValues(t, 85, row.Usage.Used)
	assert.True(t, row.Usage.HasUsed)
}

func TestCollector_Collect_FortiGateLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectFortiGateLicensingWalks(mockHandler)

	profile := mustLoadTypedLicensingProfile(t, "fortinet-fortigate", func(row ddprofiledefinition.LicensingConfig) bool {
		return strings.HasPrefix(strings.TrimPrefix(row.Table.OID, "."), "1.3.6.1.4.1.12356.101.4.6.3.")
	})

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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 3)

	byID := make(map[string]ddsnmp.LicenseRow, len(pm.LicenseRows))
	for _, row := range pm.LicenseRows {
		byID[row.ID] = row
	}

	contract := byID["FortiCare Support"]
	require.NotEmpty(t, contract)
	require.EqualValues(t, 1920585600, contract.Expiry.Timestamp)
	assert.Equal(t, "FortiCare Support", contract.Name)
	assert.Equal(t, "contract", contract.Type)
	assert.Equal(t, "device", contract.Component)

	service := byID["FortiGuard Antivirus"]
	require.NotEmpty(t, service)
	require.EqualValues(t, 1753491600, service.Expiry.Timestamp)
	assert.Equal(t, "1.00000", service.Feature)
	assert.Equal(t, "service", service.Type)
	assert.Equal(t, "fortiguard", service.Component)

	accountContract := byID["FortiCare Premium"]
	require.NotEmpty(t, accountContract)
	require.EqualValues(t, 1920585600, accountContract.Expiry.Timestamp)
	assert.Equal(t, "account_contract", accountContract.Type)
	assert.Equal(t, "account", accountContract.Component)
}

func TestCollector_Collect_MikroTikLicensingProfile(t *testing.T) {
	ctrl, mockHandler := setupMockHandler(t)
	defer ctrl.Finish()

	expectSNMPGet(mockHandler,
		[]string{
			"1.3.6.1.4.1.14988.1.1.4.2.0",
		},
		[]gosnmp.SnmpPDU{
			createDateAndTimePDU("1.3.6.1.4.1.14988.1.1.4.2.0", time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC)),
		},
	)

	profile := mustLoadTypedLicensingProfile(t, "mikrotik-router", func(row ddprofiledefinition.LicensingConfig) bool {
		return row.ID == "routeros_upgrade"
	})

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
	assert.Empty(t, pm.HiddenMetrics)
	require.Len(t, pm.LicenseRows, 1)

	row := pm.LicenseRows[0]
	assert.Equal(t, "routeros_upgrade", row.ID)
	assert.Equal(t, "RouterOS upgrade entitlement", row.Name)
	assert.Equal(t, "upgrade_entitlement", row.Type)
	assert.Equal(t, "routeros", row.Component)
	assert.True(t, row.Expiry.Has)
	assert.EqualValues(t, time.Date(2030, time.January, 1, 0, 0, 0, 0, time.UTC).Unix(), row.Expiry.Timestamp)
	assert.Equal(t, "1.3.6.1.4.1.14988.1.1.4.2.0", row.Expiry.SourceOID)
}

func expectFortiGateLicensingWalks(mockHandler *snmpmock.MockHandler) {
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.1.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.1.2.1.1.1", "FortiCare Support"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.1.2.1.2.1", "Mon 11 November 2030"),
		},
	)
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.2.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.1.2", "FortiGuard Antivirus"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.2.2", "Sat Jul 26 01:00:00 2025"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.2.2.1.3.2", "1.00000"),
		},
	)
	expectSNMPWalk(mockHandler,
		gosnmp.Version2c,
		"1.3.6.1.4.1.12356.101.4.6.3.3.2",
		[]gosnmp.SnmpPDU{
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.3.2.1.1.7", "FortiCare Premium"),
			createStringPDU("1.3.6.1.4.1.12356.101.4.6.3.3.2.1.2.7", "Mon 11 November 2030"),
		},
	)
}
