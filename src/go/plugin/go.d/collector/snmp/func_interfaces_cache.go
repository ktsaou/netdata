// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"math"
	"sync"
	"time"

	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
)

// Interface metric names we track for the function.
var ifaceMetricNames = map[string]bool{
	"ifTraffic":          true,
	"ifPacketsUcast":     true,
	"ifPacketsBroadcast": true,
	"ifPacketsMulticast": true,
	"ifErrors":           true,
	"ifDiscards":         true,
	"ifAdminStatus":      true,
	"ifOperStatus":       true,
}

// Tag keys used to identify interfaces.
const (
	tagInterface = "interface"
	tagIfType    = "_if_type"
	tagIfTypeGrp = "_if_type_group"
)

// ifaceCache holds interface metrics between collections for function queries.
type ifaceCache struct {
	mu         sync.RWMutex
	lastUpdate time.Time
	updateTime time.Time              // current collection cycle time
	interfaces map[string]*ifaceEntry // key: interface name from m.Tags["interface"]
}

// ifaceEntry holds metrics for a single network interface.
type ifaceEntry struct {
	// Identity
	name        string // interface name (from Tags["interface"])
	ifType      string // interface type (from Tags["_if_type"])
	ifTypeGroup string // interface type group (from Tags["_if_type_group"])

	// Status (text values extracted from MultiValue)
	adminStatus string
	operStatus  string

	// Raw counter values (cumulative, stored for next delta calculation)
	counters ifaceCounters
	scales   ifaceCounterScales

	// Previous counter values (for delta calculation)
	prevCounters ifaceCounters
	prevTime     time.Time
	hasPrev      bool // true if we have previous values for delta calculation

	// Computed rates (per-second, nil if not yet calculable)
	rates ifaceRates

	// Tracking
	updated bool // true if seen in current collection cycle
}

// ifaceCounters holds raw cumulative counter values.
type ifaceCounters struct {
	trafficIn    int64
	trafficOut   int64
	ucastPktsIn  int64
	ucastPktsOut int64
	bcastPktsIn  int64
	bcastPktsOut int64
	mcastPktsIn  int64
	mcastPktsOut int64
	errorsIn     int64
	errorsOut    int64
	discardsIn   int64
	discardsOut  int64
}

type ifaceCounterScales struct {
	trafficIn    counterScale
	trafficOut   counterScale
	ucastPktsIn  counterScale
	ucastPktsOut counterScale
	bcastPktsIn  counterScale
	bcastPktsOut counterScale
	mcastPktsIn  counterScale
	mcastPktsOut counterScale
	errorsIn     counterScale
	errorsOut    counterScale
	discardsIn   counterScale
	discardsOut  counterScale
}

type counterScale struct {
	mul int
	div int
}

// ifaceRates holds computed per-second rates.
type ifaceRates struct {
	trafficIn    *float64
	trafficOut   *float64
	ucastPktsIn  *float64
	ucastPktsOut *float64
	bcastPktsIn  *float64
	bcastPktsOut *float64
	mcastPktsIn  *float64
	mcastPktsOut *float64
	errorsIn     *float64
	errorsOut    *float64
	discardsIn   *float64
	discardsOut  *float64
}

// newIfaceCache creates a new interface cache.
func newIfaceCache() *ifaceCache {
	return &ifaceCache{
		interfaces: make(map[string]*ifaceEntry),
	}
}

// isIfaceMetric returns true if the metric name is one we track for interface function.
func isIfaceMetric(name string) bool {
	return ifaceMetricNames[name]
}

// resetIfaceCache prepares the cache for a new collection cycle.
// Must be called before processing metrics.
func (c *Collector) resetIfaceCache() {
	if c.ifaceCache == nil {
		return
	}

	c.ifaceCache.mu.Lock()
	defer c.ifaceCache.mu.Unlock()

	c.ifaceCache.updateTime = time.Now()

	for _, entry := range c.ifaceCache.interfaces {
		entry.updated = false
	}
}

// updateIfaceCacheEntry updates the cache with a single interface metric.
// Called during collectProfileTableMetrics for matching metrics.
// Caller must ensure m.IsTable is true and m.Tags["interface"] is not empty.
func (c *Collector) updateIfaceCacheEntry(m ddsnmp.Metric) {
	if c.ifaceCache == nil {
		return
	}

	ifaceName := m.Tags[tagInterface]
	if ifaceName == "" {
		return
	}

	c.ifaceCache.mu.Lock()
	defer c.ifaceCache.mu.Unlock()

	entry := c.ifaceCache.interfaces[ifaceName]
	if entry == nil {
		entry = &ifaceEntry{
			name: ifaceName,
		}
		c.ifaceCache.interfaces[ifaceName] = entry
	}

	if ifType := m.Tags[tagIfType]; ifType != "" {
		entry.ifType = ifType
	}
	if ifTypeGroup := m.Tags[tagIfTypeGrp]; ifTypeGroup != "" {
		entry.ifTypeGroup = ifTypeGroup
	}

	switch m.Name {
	case "ifTraffic":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.trafficIn = v
			entry.scales.trafficIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.trafficOut = v
			entry.scales.trafficOut = scale
		}
	case "ifPacketsUcast":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.ucastPktsIn = v
			entry.scales.ucastPktsIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.ucastPktsOut = v
			entry.scales.ucastPktsOut = scale
		}
	case "ifPacketsBroadcast":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.bcastPktsIn = v
			entry.scales.bcastPktsIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.bcastPktsOut = v
			entry.scales.bcastPktsOut = scale
		}
	case "ifPacketsMulticast":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.mcastPktsIn = v
			entry.scales.mcastPktsIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.mcastPktsOut = v
			entry.scales.mcastPktsOut = scale
		}
	case "ifErrors":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.errorsIn = v
			entry.scales.errorsIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.errorsOut = v
			entry.scales.errorsOut = scale
		}
	case "ifDiscards":
		scale := newCounterScale(m)
		if v, ok := m.MultiValue["in"]; ok {
			entry.counters.discardsIn = v
			entry.scales.discardsIn = scale
		}
		if v, ok := m.MultiValue["out"]; ok {
			entry.counters.discardsOut = v
			entry.scales.discardsOut = scale
		}
	case "ifAdminStatus":
		entry.adminStatus = extractStatus(m.MultiValue)
	case "ifOperStatus":
		entry.operStatus = extractStatus(m.MultiValue)
	}

	entry.updated = true
}

// finalizeIfaceCache removes stale entries and calculates rates.
// Must be called after all metrics have been processed.
func (c *Collector) finalizeIfaceCache() {
	if c.ifaceCache == nil {
		return
	}

	c.ifaceCache.mu.Lock()
	defer c.ifaceCache.mu.Unlock()

	now := c.ifaceCache.updateTime

	for name, entry := range c.ifaceCache.interfaces {
		if !entry.updated {
			delete(c.ifaceCache.interfaces, name)
			continue
		}

		if entry.hasPrev {
			elapsed := now.Sub(entry.prevTime)
			entry.rates.trafficIn = calcScaledRate(entry.counters.trafficIn, entry.prevCounters.trafficIn, elapsed, entry.scales.trafficIn)
			entry.rates.trafficOut = calcScaledRate(entry.counters.trafficOut, entry.prevCounters.trafficOut, elapsed, entry.scales.trafficOut)
			entry.rates.ucastPktsIn = calcScaledRate(entry.counters.ucastPktsIn, entry.prevCounters.ucastPktsIn, elapsed, entry.scales.ucastPktsIn)
			entry.rates.ucastPktsOut = calcScaledRate(entry.counters.ucastPktsOut, entry.prevCounters.ucastPktsOut, elapsed, entry.scales.ucastPktsOut)
			entry.rates.bcastPktsIn = calcScaledRate(entry.counters.bcastPktsIn, entry.prevCounters.bcastPktsIn, elapsed, entry.scales.bcastPktsIn)
			entry.rates.bcastPktsOut = calcScaledRate(entry.counters.bcastPktsOut, entry.prevCounters.bcastPktsOut, elapsed, entry.scales.bcastPktsOut)
			entry.rates.mcastPktsIn = calcScaledRate(entry.counters.mcastPktsIn, entry.prevCounters.mcastPktsIn, elapsed, entry.scales.mcastPktsIn)
			entry.rates.mcastPktsOut = calcScaledRate(entry.counters.mcastPktsOut, entry.prevCounters.mcastPktsOut, elapsed, entry.scales.mcastPktsOut)
			entry.rates.errorsIn = calcScaledRate(entry.counters.errorsIn, entry.prevCounters.errorsIn, elapsed, entry.scales.errorsIn)
			entry.rates.errorsOut = calcScaledRate(entry.counters.errorsOut, entry.prevCounters.errorsOut, elapsed, entry.scales.errorsOut)
			entry.rates.discardsIn = calcScaledRate(entry.counters.discardsIn, entry.prevCounters.discardsIn, elapsed, entry.scales.discardsIn)
			entry.rates.discardsOut = calcScaledRate(entry.counters.discardsOut, entry.prevCounters.discardsOut, elapsed, entry.scales.discardsOut)
		}

		entry.prevCounters = entry.counters
		entry.prevTime = now
		entry.hasPrev = true
	}

	c.ifaceCache.lastUpdate = now
}

func newCounterScale(m ddsnmp.Metric) counterScale {
	mul, div := m.Scale()
	return counterScale{mul: mul, div: div}
}

// calcRate computes per-second rate from counter delta.
// Returns nil if rate cannot be calculated (zero or negative elapsed time).
// Handles counter wrap by treating values as unsigned.
func calcRate(current, previous int64, elapsed time.Duration) *float64 {
	if elapsed <= 0 {
		return nil
	}

	// Treat as unsigned for proper counter wrap handling
	ucurrent := uint64(current)
	uprevious := uint64(previous)

	var delta uint64
	if ucurrent >= uprevious {
		delta = ucurrent - uprevious
	} else {
		// Counter wrap - calculate wrapped delta
		delta = (math.MaxUint64 - uprevious) + ucurrent + 1
	}

	rate := float64(delta) / elapsed.Seconds()
	return &rate
}

func calcScaledRate(current, previous int64, elapsed time.Duration, scale counterScale) *float64 {
	rate := calcRate(current, previous, elapsed)
	if rate == nil {
		return nil
	}
	if scale.mul == 0 {
		scale.mul = 1
	}
	if scale.div == 0 {
		scale.div = 1
	}
	scaled := *rate * float64(scale.mul) / float64(scale.div)
	return &scaled
}

// extractStatus finds the active status from a MultiValue map.
// Returns the key where value == 1, or "unknown" if none found.
func extractStatus(mv map[string]int64) string {
	for k, v := range mv {
		if v == 1 {
			return k
		}
	}
	return "unknown"
}
