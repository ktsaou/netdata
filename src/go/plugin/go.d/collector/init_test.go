// SPDX-License-Identifier: GPL-3.0-or-later

package collector

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/netdata/netdata/go/plugins/plugin/framework/collectorapi"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp"
	snmptopology "github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp_topology"
	snmptraps "github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp_traps"
	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestSNMPFamilyRegistrationUsesSharedDependencies(t *testing.T) {
	snmpCreator := requireCreator(t, "snmp")
	topologyCreator := requireCreator(t, "snmp_topology")
	trapsCreator := requireCreator(t, "snmp_traps")

	assert.NotNil(t, snmpCreator.Create)
	assert.Nil(t, snmpCreator.CreateV2)
	assert.NotNil(t, snmpCreator.Config)
	assert.NotNil(t, snmpCreator.Methods)
	assert.NotNil(t, snmpCreator.MethodHandler)
	assert.Equal(t, 10, snmpCreator.Defaults.UpdateEvery)

	assert.Nil(t, topologyCreator.Create)
	assert.NotNil(t, topologyCreator.CreateV2)
	assert.Equal(t, collectorapi.InstancePolicySingle, topologyCreator.InstancePolicy)
	assert.False(t, topologyCreator.FunctionOnly)
	assert.NotNil(t, topologyCreator.Methods)
	assert.NotNil(t, topologyCreator.MethodHandler)
	assert.Equal(t, 60, topologyCreator.Defaults.UpdateEvery)

	assert.Nil(t, trapsCreator.Create)
	assert.NotNil(t, trapsCreator.CreateV2)
	assert.NotNil(t, trapsCreator.Methods)
	assert.NotNil(t, trapsCreator.MethodHandler)
	assert.Equal(t, 1, trapsCreator.Defaults.UpdateEvery)

	snmpCollector, ok := snmpCreator.Create().(*snmp.Collector)
	require.True(t, ok)
	topologyCollector, ok := topologyCreator.CreateV2().(*snmptopology.Collector)
	require.True(t, ok)
	trapsCollector, ok := trapsCreator.CreateV2().(*snmptraps.Collector)
	require.True(t, ok)

	deviceStore := pointerField(t, snmpCollector, "deviceStore")
	require.NotZero(t, deviceStore)
	assert.Equal(t, deviceStore, interfacePointerField(t, topologyCollector, "deviceSource"))
	assert.Equal(t, deviceStore, interfacePointerField(t, trapsCollector, "deviceLookup"))

	trapEnrichment := pointerField(t, topologyCollector, "trapEnrichment")
	require.NotZero(t, trapEnrichment)
	assert.Equal(t, trapEnrichment, interfacePointerField(t, trapsCollector, "topologyEnricher"))
}

func TestCollectorConfigSchemasAllowDynCfgManagedName(t *testing.T) {
	paths, err := filepath.Glob(filepath.Join("*", "config_schema.json"))
	require.NoError(t, err)
	require.NotEmpty(t, paths)

	for _, path := range paths {
		t.Run(filepath.Dir(path), func(t *testing.T) {
			doc := loadCollectorConfigSchema(t, path)
			jsonSchema := schemaMap(t, doc, "jsonSchema")
			compiled := compileCollectorConfigSchema(t, path, jsonSchema)

			if schemaIsClosedTopLevel(jsonSchema) {
				assert.True(t, schemaExplicitlyAllowsDynCfgName(jsonSchema),
					"closed top-level schemas must explicitly allow the DynCfg-managed job name field")
			}

			payload := collectJSONSchemaDefaults(jsonSchema)
			if err := compiled.Validate(payload); err != nil {
				t.Logf("default-derived payload is not complete without user input: %v", err)
				return
			}

			payload["name"] = "local"
			assert.NoError(t, compiled.Validate(payload),
				"schema validates the default-derived payload but rejects the DynCfg-managed job name field")
		})
	}
}

func requireCreator(t *testing.T, module string) collectorapi.Creator {
	t.Helper()
	creator, ok := collectorapi.DefaultRegistry.Lookup(module)
	require.True(t, ok, "collector %q is not registered", module)
	return creator
}

func loadCollectorConfigSchema(t *testing.T, path string) map[string]any {
	t.Helper()

	bs, err := os.ReadFile(filepath.Clean(path))
	require.NoError(t, err)

	var doc map[string]any
	require.NoError(t, json.Unmarshal(bs, &doc))
	return doc
}

func compileCollectorConfigSchema(t *testing.T, path string, schemaDoc any) *jsonschema.Schema {
	t.Helper()

	compiler := jsonschema.NewCompiler()
	require.NoError(t, compiler.AddResource(path, schemaDoc))
	schema, err := compiler.Compile(path)
	require.NoError(t, err)

	return schema
}

func schemaMap(t *testing.T, parent map[string]any, key string) map[string]any {
	t.Helper()

	child, ok := parent[key].(map[string]any)
	require.Truef(t, ok, "schema key %q is %T", key, parent[key])
	return child
}

func schemaIsClosedTopLevel(schema map[string]any) bool {
	additionalProperties, ok := schema["additionalProperties"].(bool)
	return ok && !additionalProperties
}

func schemaExplicitlyAllowsDynCfgName(schema map[string]any) bool {
	props, _ := schema["properties"].(map[string]any)
	if _, ok := props["name"]; ok {
		return true
	}

	patterns, _ := schema["patternProperties"].(map[string]any)
	_, ok := patterns["^name$"]
	return ok
}

func collectJSONSchemaDefaults(schema map[string]any) map[string]any {
	props, _ := schema["properties"].(map[string]any)
	out := make(map[string]any)

	for key, raw := range props {
		prop, ok := raw.(map[string]any)
		if !ok {
			continue
		}

		if value, ok := collectJSONSchemaDefault(prop); ok {
			out[key] = value
		}
	}

	return out
}

func collectJSONSchemaDefault(schema map[string]any) (any, bool) {
	if value, ok := schema["default"]; ok {
		return value, true
	}

	if !schemaCanContainObjectDefaults(schema) {
		return nil, false
	}

	defaults := collectJSONSchemaDefaults(schema)
	if len(defaults) == 0 {
		return nil, false
	}
	return defaults, true
}

func schemaCanContainObjectDefaults(schema map[string]any) bool {
	if _, ok := schema["properties"]; !ok {
		return false
	}

	switch typ := schema["type"].(type) {
	case nil:
		return true
	case string:
		return typ == "object"
	case []any:
		for _, item := range typ {
			if item == "object" {
				return true
			}
		}
	}
	return false
}

func pointerField(t *testing.T, obj any, name string) uintptr {
	t.Helper()
	field := reflect.ValueOf(obj).Elem().FieldByName(name)
	require.True(t, field.IsValid(), "field %q not found", name)
	require.Equal(t, reflect.Pointer, field.Kind(), "field %q", name)
	require.False(t, field.IsNil(), "field %q is nil", name)
	return field.Pointer()
}

func interfacePointerField(t *testing.T, obj any, name string) uintptr {
	t.Helper()
	field := reflect.ValueOf(obj).Elem().FieldByName(name)
	require.True(t, field.IsValid(), "field %q not found", name)
	require.Equal(t, reflect.Interface, field.Kind(), "field %q", name)
	require.False(t, field.IsNil(), "field %q is nil", name)

	elem := field.Elem()
	require.Equal(t, reflect.Pointer, elem.Kind(), "field %q concrete value", name)
	require.False(t, elem.IsNil(), "field %q concrete value is nil", name)
	return elem.Pointer()
}
