// SPDX-License-Identifier: GPL-3.0-or-later

package ddsnmp

import (
	"bytes"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

// runLicenseTransform compiles a transform body and applies it to the given
// metric. It mirrors the minimal "execute the template against {Metric: m}"
// contract used by ddsnmpcollector.applyTransform, without crossing package
// boundaries just for the test.
func runLicenseTransform(t *testing.T, body string, m *Metric) {
	t.Helper()
	require.NoError(t, executeLicenseTransform(body, m))
}

func executeLicenseTransform(body string, m *Metric) error {
	tmpl, err := compileTransform(body)
	if err != nil {
		return err
	}
	var buf bytes.Buffer
	return tmpl.Execute(&buf, struct{ Metric *Metric }{Metric: m})
}

func TestSetTagTransform_StampsValueKindOnTagsMap(t *testing.T) {
	m := &Metric{Value: 42, Tags: map[string]string{}}
	runLicenseTransform(t, `{{- setTag .Metric "_license_value_kind" "expiry_timestamp" -}}`, m)

	assert.Equal(t, "expiry_timestamp", m.Tags["_license_value_kind"])
	assert.EqualValues(t, 42, m.Value)
}

func TestSetTagTransform_AllocatesTagsWhenNil(t *testing.T) {
	m := &Metric{Value: 1}
	runLicenseTransform(t, `{{- setTag .Metric "_license_value_kind" "state_severity" -}}`, m)

	require.NotNil(t, m.Tags)
	assert.Equal(t, "state_severity", m.Tags["_license_value_kind"])
}

func TestIsTextDateNoValue(t *testing.T) {
	noValues := []string{"", "0", "-1", "never", "perpetual", "permanent", "n/a", "na", "none", "unlimited", "4294967295"}
	for _, raw := range noValues {
		assert.True(t, IsTextDateNoValue(raw), "raw=%q", raw)
	}

	values := []string{"1", "1798675200", "2026-12-31", "not-a-date"}
	for _, raw := range values {
		assert.False(t, IsTextDateNoValue(raw), "raw=%q", raw)
	}
}
