// SPDX-License-Identifier: GPL-3.0-or-later

package ddprofiledefinition

import "slices"

type LicenseSignalKind string

const (
	LicenseSignalExpiryTimestamp        LicenseSignalKind = "expiry_timestamp"
	LicenseSignalExpiryRemaining        LicenseSignalKind = "expiry_remaining"
	LicenseSignalAuthorizationTimestamp LicenseSignalKind = "authorization_timestamp"
	LicenseSignalAuthorizationRemaining LicenseSignalKind = "authorization_remaining"
	LicenseSignalCertificateTimestamp   LicenseSignalKind = "certificate_timestamp"
	LicenseSignalCertificateRemaining   LicenseSignalKind = "certificate_remaining"
	LicenseSignalGraceTimestamp         LicenseSignalKind = "grace_timestamp"
	LicenseSignalGraceRemaining         LicenseSignalKind = "grace_remaining"
	LicenseSignalUsageUsed              LicenseSignalKind = "usage_used"
	LicenseSignalUsageCapacity          LicenseSignalKind = "usage_capacity"
	LicenseSignalUsageAvailable         LicenseSignalKind = "usage_available"
	LicenseSignalUsagePercent           LicenseSignalKind = "usage_percent"
	LicenseSignalStateSeverity          LicenseSignalKind = "state_severity"
)

var validLicenseSignalKinds = map[LicenseSignalKind]struct{}{
	LicenseSignalExpiryTimestamp:        {},
	LicenseSignalExpiryRemaining:        {},
	LicenseSignalAuthorizationTimestamp: {},
	LicenseSignalAuthorizationRemaining: {},
	LicenseSignalCertificateTimestamp:   {},
	LicenseSignalCertificateRemaining:   {},
	LicenseSignalGraceTimestamp:         {},
	LicenseSignalGraceRemaining:         {},
	LicenseSignalUsageUsed:              {},
	LicenseSignalUsageCapacity:          {},
	LicenseSignalUsageAvailable:         {},
	LicenseSignalUsagePercent:           {},
	LicenseSignalStateSeverity:          {},
}

func IsValidLicenseSignalKind(kind LicenseSignalKind) bool {
	_, ok := validLicenseSignalKinds[kind]
	return ok
}

type LicenseSentinelPolicy string

const (
	LicenseSentinelTimerZeroOrNegative LicenseSentinelPolicy = "timer_zero_or_negative"
	LicenseSentinelTimerU32Max         LicenseSentinelPolicy = "timer_u32_max"
	LicenseSentinelTimerPre1971        LicenseSentinelPolicy = "timer_pre_1971"
)

var validLicenseSentinelPolicies = map[LicenseSentinelPolicy]struct{}{
	LicenseSentinelTimerZeroOrNegative: {},
	LicenseSentinelTimerU32Max:         {},
	LicenseSentinelTimerPre1971:        {},
}

func IsValidLicenseSentinelPolicy(policy LicenseSentinelPolicy) bool {
	_, ok := validLicenseSentinelPolicies[policy]
	return ok
}

type LicenseStatePolicy string

const (
	LicenseStatePolicyDefault  LicenseStatePolicy = "default"
	LicenseStatePolicySophos   LicenseStatePolicy = "sophos"
	LicenseStatePolicyMikroTik LicenseStatePolicy = "mikrotik"
)

var validLicenseStatePolicies = map[LicenseStatePolicy]struct{}{
	LicenseStatePolicyDefault:  {},
	LicenseStatePolicySophos:   {},
	LicenseStatePolicyMikroTik: {},
}

func IsValidLicenseStatePolicy(policy LicenseStatePolicy) bool {
	_, ok := validLicenseStatePolicies[policy]
	return ok
}

type LicensingConfig struct {
	OriginProfileID string       `yaml:"-" json:"-"`
	ID              string       `yaml:"id,omitempty" json:"id,omitempty"`
	MIB             string       `yaml:"MIB,omitempty" json:"MIB,omitempty"`
	Table           SymbolConfig `yaml:"table,omitempty" json:"table,omitempty"`

	Identity    LicenseIdentityConfig    `yaml:"identity,omitempty" json:"identity,omitempty"`
	Descriptors LicenseDescriptorsConfig `yaml:"descriptors,omitempty" json:"descriptors,omitempty"`
	State       LicenseStateConfig       `yaml:"state,omitempty" json:"state,omitempty"`
	Signals     LicenseSignalsConfig     `yaml:"signals,omitempty" json:"signals,omitempty"`

	StaticTags []StaticMetricTagConfig `yaml:"static_tags,omitempty" json:"-"`
	MetricTags MetricTagConfigList     `yaml:"metric_tags,omitempty" json:"metric_tags,omitempty"`
}

func (c LicensingConfig) Clone() LicensingConfig {
	return LicensingConfig{
		OriginProfileID: c.OriginProfileID,
		ID:              c.ID,
		MIB:             c.MIB,
		Table:           c.Table.Clone(),
		Identity:        c.Identity.Clone(),
		Descriptors:     c.Descriptors.Clone(),
		State:           c.State.Clone(),
		Signals:         c.Signals.Clone(),
		StaticTags:      slices.Clone(c.StaticTags),
		MetricTags:      cloneSlice(c.MetricTags),
	}
}

type LicenseIdentityConfig struct {
	ID        LicenseValueConfig `yaml:"id,omitempty" json:"id,omitempty"`
	Name      LicenseValueConfig `yaml:"name,omitempty" json:"name,omitempty"`
	Feature   LicenseValueConfig `yaml:"feature,omitempty" json:"feature,omitempty"`
	Component LicenseValueConfig `yaml:"component,omitempty" json:"component,omitempty"`
}

func (c LicenseIdentityConfig) Clone() LicenseIdentityConfig {
	return LicenseIdentityConfig{
		ID:        c.ID.Clone(),
		Name:      c.Name.Clone(),
		Feature:   c.Feature.Clone(),
		Component: c.Component.Clone(),
	}
}

type LicenseDescriptorsConfig struct {
	Type      LicenseValueConfig `yaml:"type,omitempty" json:"type,omitempty"`
	Impact    LicenseValueConfig `yaml:"impact,omitempty" json:"impact,omitempty"`
	Perpetual LicenseValueConfig `yaml:"perpetual,omitempty" json:"perpetual,omitempty"`
	Unlimited LicenseValueConfig `yaml:"unlimited,omitempty" json:"unlimited,omitempty"`
}

func (c LicenseDescriptorsConfig) Clone() LicenseDescriptorsConfig {
	return LicenseDescriptorsConfig{
		Type:      c.Type.Clone(),
		Impact:    c.Impact.Clone(),
		Perpetual: c.Perpetual.Clone(),
		Unlimited: c.Unlimited.Clone(),
	}
}

type LicenseStateConfig struct {
	LicenseValueConfig `yaml:",inline" json:",inline"`
	Policy             LicenseStatePolicy `yaml:"policy,omitempty" json:"policy,omitempty"`
}

func (c LicenseStateConfig) Clone() LicenseStateConfig {
	return LicenseStateConfig{
		LicenseValueConfig: c.LicenseValueConfig.Clone(),
		Policy:             c.Policy,
	}
}

type LicenseSignalsConfig struct {
	Expiry        LicenseTimerSignalsConfig `yaml:"expiry,omitempty" json:"expiry,omitempty"`
	Authorization LicenseTimerSignalsConfig `yaml:"authorization,omitempty" json:"authorization,omitempty"`
	Certificate   LicenseTimerSignalsConfig `yaml:"certificate,omitempty" json:"certificate,omitempty"`
	Grace         LicenseTimerSignalsConfig `yaml:"grace,omitempty" json:"grace,omitempty"`
	Usage         LicenseUsageSignalsConfig `yaml:"usage,omitempty" json:"usage,omitempty"`
}

func (c LicenseSignalsConfig) Clone() LicenseSignalsConfig {
	return LicenseSignalsConfig{
		Expiry:        c.Expiry.Clone(),
		Authorization: c.Authorization.Clone(),
		Certificate:   c.Certificate.Clone(),
		Grace:         c.Grace.Clone(),
		Usage:         c.Usage.Clone(),
	}
}

type LicenseTimerSignalsConfig struct {
	LicenseValueConfig `yaml:",inline" json:",inline"`
	Timestamp          LicenseValueConfig `yaml:"timestamp,omitempty" json:"timestamp,omitempty"`
	Remaining          LicenseValueConfig `yaml:"remaining,omitempty" json:"remaining,omitempty"`
}

func (c LicenseTimerSignalsConfig) Clone() LicenseTimerSignalsConfig {
	return LicenseTimerSignalsConfig{
		LicenseValueConfig: c.LicenseValueConfig.Clone(),
		Timestamp:          c.Timestamp.Clone(),
		Remaining:          c.Remaining.Clone(),
	}
}

type LicenseUsageSignalsConfig struct {
	Used      LicenseValueConfig `yaml:"used,omitempty" json:"used,omitempty"`
	Capacity  LicenseValueConfig `yaml:"capacity,omitempty" json:"capacity,omitempty"`
	Available LicenseValueConfig `yaml:"available,omitempty" json:"available,omitempty"`
	Percent   LicenseValueConfig `yaml:"percent,omitempty" json:"percent,omitempty"`
}

func (c LicenseUsageSignalsConfig) Clone() LicenseUsageSignalsConfig {
	return LicenseUsageSignalsConfig{
		Used:      c.Used.Clone(),
		Capacity:  c.Capacity.Clone(),
		Available: c.Available.Clone(),
		Percent:   c.Percent.Clone(),
	}
}

type LicenseValueConfig struct {
	Value string `yaml:"value,omitempty" json:"value,omitempty"`
	From  string `yaml:"from,omitempty" json:"from,omitempty"`

	Index          uint                   `yaml:"index,omitempty" json:"index,omitempty"`
	IndexTransform []MetricIndexTransform `yaml:"index_transform,omitempty" json:"index_transform,omitempty"`

	Symbol SymbolConfig `yaml:"symbol,omitempty" json:"symbol,omitempty"`
	OID    string       `yaml:"OID,omitempty" json:"OID,omitempty" jsonschema:"-"`
	Name   string       `yaml:"name,omitempty" json:"name,omitempty" jsonschema:"-"`

	Format   string                  `yaml:"format,omitempty" json:"format,omitempty"`
	Mapping  MappingConfig           `yaml:"mapping,omitempty" json:"mapping,omitempty"`
	Sentinel []LicenseSentinelPolicy `yaml:"sentinel,omitempty" json:"sentinel,omitempty"`
	Kind     LicenseSignalKind       `yaml:"kind,omitempty" json:"kind,omitempty"`
}

func (c LicenseValueConfig) IsSet() bool {
	return c.Value != "" ||
		c.From != "" ||
		c.Index != 0 ||
		len(c.IndexTransform) > 0 ||
		c.Symbol.OID != "" ||
		c.Symbol.Name != "" ||
		c.OID != "" ||
		c.Name != "" ||
		c.Format != "" ||
		c.Mapping.HasItems() ||
		c.Mapping.Mode != "" ||
		len(c.Sentinel) > 0 ||
		c.Kind != ""
}

func (c LicenseValueConfig) Clone() LicenseValueConfig {
	return LicenseValueConfig{
		Value:          c.Value,
		From:           c.From,
		Index:          c.Index,
		IndexTransform: slices.Clone(c.IndexTransform),
		Symbol:         c.Symbol.Clone(),
		OID:            c.OID,
		Name:           c.Name,
		Format:         c.Format,
		Mapping:        c.Mapping.Clone(),
		Sentinel:       slices.Clone(c.Sentinel),
		Kind:           c.Kind,
	}
}
