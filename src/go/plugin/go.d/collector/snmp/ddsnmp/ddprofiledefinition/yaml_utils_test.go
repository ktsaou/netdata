// Unless explicitly stated otherwise all files in this repository are licensed
// under the Apache License Version 2.0.
// This product includes software developed at Datadog (https://www.datadoghq.com/).
// Copyright 2016-present Datadog, Inc.

package ddprofiledefinition

import (
	"testing"

	"gopkg.in/yaml.v2"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

type MyStringArray struct {
	SomeIDs StringArray `yaml:"my_field"`
}

type MySymbolStruct struct {
	SymbolField SymbolConfigCompat `yaml:"my_symbol_field"`
}

func Test_metricTagConfig_UnmarshalYAML(t *testing.T) {
	myStruct := MetricsConfig{}
	expected := MetricsConfig{MetricTags: []MetricTagConfig{{Index: 3}}}

	require.NoError(t, yaml.Unmarshal([]byte(`
metric_tags:
- index: 3
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}

func Test_metricTagConfig_onlyTags(t *testing.T) {
	myStruct := MetricsConfig{}
	expected := MetricsConfig{MetricTags: []MetricTagConfig{{SymbolTag: "aaa"}}}

	require.NoError(t, yaml.Unmarshal([]byte(`
metric_tags:
- aaa
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}

func TestStringArray_UnmarshalYAML_array(t *testing.T) {
	myStruct := MyStringArray{}
	expected := MyStringArray{SomeIDs: StringArray{"aaa", "bbb"}}

	require.NoError(t, yaml.Unmarshal([]byte(`
my_field:
 - aaa
 - bbb
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}

func TestStringArray_UnmarshalYAML_string(t *testing.T) {
	myStruct := MyStringArray{}
	expected := MyStringArray{SomeIDs: StringArray{"aaa"}}

	require.NoError(t, yaml.Unmarshal([]byte(`
my_field: aaa
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}

func TestSymbolConfig_UnmarshalYAML_symbolObject(t *testing.T) {
	myStruct := MySymbolStruct{}
	expected := MySymbolStruct{SymbolField: SymbolConfigCompat{OID: "1.2.3", Name: "aSymbol"}}

	require.NoError(t, yaml.Unmarshal([]byte(`
my_symbol_field:
  name: aSymbol
  OID: 1.2.3
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}

func TestSymbolConfig_UnmarshalYAML_symbolString(t *testing.T) {
	myStruct := MySymbolStruct{}
	expected := MySymbolStruct{SymbolField: SymbolConfigCompat{Name: "aSymbol"}}

	require.NoError(t, yaml.Unmarshal([]byte(`
my_symbol_field: aSymbol
`), &myStruct))

	assert.Equal(t, expected, myStruct)
}
