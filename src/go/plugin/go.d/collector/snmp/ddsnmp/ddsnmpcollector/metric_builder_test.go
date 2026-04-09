// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmpcollector

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestMetricBuilder_WithStaticTagsFillsMissingAndEmptyValues(t *testing.T) {
	metric := newMetricBuilder("testMetric", 1).
		withTags(map[string]string{
			"region":   "",
			"neighbor": "192.0.2.10",
		}).
		withStaticTags(map[string]string{
			"region": "eu-west",
			"site":   "athens",
		}).
		build()

	assert.Equal(t, map[string]string{
		"region":   "eu-west",
		"neighbor": "192.0.2.10",
		"site":     "athens",
	}, metric.Tags)
	assert.Equal(t, map[string]string{
		"region": "eu-west",
		"site":   "athens",
	}, metric.StaticTags)
}

func TestMetricBuilder_WithStaticTagsKeepsExistingNonEmptyValues(t *testing.T) {
	metric := newMetricBuilder("testMetric", 1).
		withTags(map[string]string{
			"region": "edge",
		}).
		withStaticTags(map[string]string{
			"region": "core",
		}).
		build()

	assert.Equal(t, map[string]string{"region": "edge"}, metric.Tags)
	assert.Equal(t, map[string]string{"region": "core"}, metric.StaticTags)
}
