// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import "strings"

func metricBaseIDFromName(name string) string {
	if isBGPPublicMetricName(name) {
		return "snmp_" + cleanMetricName.Replace(name)
	}
	return "snmp_device_prof_" + cleanMetricName.Replace(name)
}

func metricBaseIDFromKey(key string) string {
	if isBGPPublicMetricKey(key) {
		return "snmp_" + cleanMetricName.Replace(key)
	}
	return "snmp_device_prof_" + cleanMetricName.Replace(key)
}

func metricIDFromName(name string, subkeys ...string) string {
	id := metricBaseIDFromName(name)
	for _, subkey := range subkeys {
		id += "_" + cleanMetricName.Replace(subkey)
	}
	return id
}

func metricIDFromKey(key string, subkeys ...string) string {
	id := metricBaseIDFromKey(key)
	for _, subkey := range subkeys {
		id += "_" + cleanMetricName.Replace(subkey)
	}
	return id
}

func chartContextID(name string) string {
	if isBGPPublicMetricName(name) {
		return "snmp." + name
	}
	return "snmp.device_prof_" + cleanMetricName.Replace(name)
}

func isBGPPublicMetricName(name string) bool {
	return strings.HasPrefix(name, "bgp.")
}

func isBGPPublicMetricKey(key string) bool {
	return strings.HasPrefix(key, "bgp.")
}
