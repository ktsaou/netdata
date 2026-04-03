// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import "github.com/netdata/netdata/go/plugins/plugin/framework/collectorapi"

const (
	prioLicenseRemainingTime = prioPingStdDev + 1 + iota
	prioLicenseAuthorizationRemainingTime
	prioLicenseCertificateRemainingTime
	prioLicenseGraceRemainingTime
	prioLicenseUsagePercent
	prioLicenseState
)

var (
	licenseCharts = collectorapi.Charts{
		licenseRemainingTimeChart.Copy(),
		licenseAuthorizationRemainingTimeChart.Copy(),
		licenseCertificateRemainingTimeChart.Copy(),
		licenseGraceRemainingTimeChart.Copy(),
		licenseUsagePercentChart.Copy(),
		licenseStateChart.Copy(),
	}
	licenseRemainingTimeChart = collectorapi.Chart{
		ID:       "snmp_device_license_remaining_time",
		Title:    "License remaining time",
		Units:    "seconds",
		Fam:      "Licensing/Time",
		Ctx:      "snmp.license.remaining_time",
		Priority: prioLicenseRemainingTime,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: "snmp_device_license_remaining_time", Name: "remaining_time"},
		},
	}
	licenseAuthorizationRemainingTimeChart = collectorapi.Chart{
		ID:       "snmp_device_license_authorization_remaining_time",
		Title:    "License authorization remaining time",
		Units:    "seconds",
		Fam:      "Licensing/Time",
		Ctx:      "snmp.license.authorization_remaining_time",
		Priority: prioLicenseAuthorizationRemainingTime,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: "snmp_device_license_authorization_remaining_time", Name: "remaining_time"},
		},
	}
	licenseCertificateRemainingTimeChart = collectorapi.Chart{
		ID:       "snmp_device_license_certificate_remaining_time",
		Title:    "License certificate remaining time",
		Units:    "seconds",
		Fam:      "Licensing/Time",
		Ctx:      "snmp.license.certificate_remaining_time",
		Priority: prioLicenseCertificateRemainingTime,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: "snmp_device_license_certificate_remaining_time", Name: "remaining_time"},
		},
	}
	licenseGraceRemainingTimeChart = collectorapi.Chart{
		ID:       "snmp_device_license_grace_remaining_time",
		Title:    "License grace remaining time",
		Units:    "seconds",
		Fam:      "Licensing/Time",
		Ctx:      "snmp.license.grace_remaining_time",
		Priority: prioLicenseGraceRemainingTime,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: "snmp_device_license_grace_remaining_time", Name: "remaining_time"},
		},
	}
	licenseUsagePercentChart = collectorapi.Chart{
		ID:       "snmp_device_license_usage_percent",
		Title:    "License usage pressure",
		Units:    "percentage",
		Fam:      "Licensing/Usage",
		Ctx:      "snmp.license.usage_percent",
		Priority: prioLicenseUsagePercent,
		Type:     collectorapi.Area,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: "snmp_device_license_usage_percent", Name: "usage_percent"},
		},
	}
	licenseStateChart = collectorapi.Chart{
		ID:       "snmp_device_license_state",
		Title:    "License state counts",
		Units:    "licenses",
		Fam:      "Licensing/State",
		Ctx:      "snmp.license.state",
		Priority: prioLicenseState,
		Type:     collectorapi.Stacked,
		SkipGaps: true,
		Dims: collectorapi.Dims{
			{ID: metricIDLicenseStateHealthy, Name: string(licenseStateBucketHealthy)},
			{ID: metricIDLicenseStateDegraded, Name: string(licenseStateBucketDegraded)},
			{ID: metricIDLicenseStateBroken, Name: string(licenseStateBucketBroken)},
			{ID: metricIDLicenseStateIgnored, Name: string(licenseStateBucketIgnored)},
		},
	}
)

func (c *Collector) addLicenseCharts() {
	if c.Charts().Get(licenseRemainingTimeChart.ID) != nil {
		return
	}

	charts := licenseCharts.Copy()

	labels := c.chartBaseLabels()
	labels["component"] = "licensing"

	for _, chart := range *charts {
		for k, v := range labels {
			chart.Labels = append(chart.Labels, collectorapi.Label{Key: k, Value: v})
		}
	}

	if err := c.Charts().Add(*charts...); err != nil {
		c.Warningf("failed to add license charts: %v", err)
	}
}
