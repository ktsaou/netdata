// SPDX-License-Identifier: GPL-3.0-or-later

package cato_networks

import (
	"context"
	"errors"
	"net"
	"strings"
)

const (
	operationDiscovery   = "entityLookup"
	operationSnapshot    = "accountSnapshot"
	operationMetrics     = "accountMetrics"
	operationEvents      = "eventsFeed"
	operationBGP         = "siteBgpStatus"
	operationEventMarker = "eventsMarker"
)

const (
	normalizationSurfaceSiteConnectivity = "site_connectivity"
	normalizationSurfaceSiteOperational  = "site_operational"
	normalizationSurfaceMetrics          = "metrics"
	normalizationSurfaceBGP              = "bgp"
	normalizationSurfaceEvents           = "events"
)

const (
	normalizationIssueUnknownStatus          = "unknown_status"
	normalizationIssueUnknownTimeseriesLabel = "unknown_timeseries_label"
	normalizationIssueEmptyPeer              = "empty_peer"
	normalizationIssueParseInt               = "parse_int"
	normalizationIssueAccountError           = "account_error"
	normalizationIssueCardinalityLimit       = "cardinality_limit"
	normalizationIssuePageCap                = "page_cap"
	normalizationIssueMarkerStalled          = "marker_stalled"
	normalizationIssueEmptyEventType         = "empty_event_type"
	normalizationIssueEmptyEventSubType      = "empty_event_sub_type"
	normalizationIssueEmptyEventSeverity     = "empty_event_severity"
	normalizationIssueEmptyEventStatus       = "empty_event_status"
	normalizationIssueComplexEventField      = "complex_event_field"
)

type collectorHealth struct {
	CollectionSuccess bool
	DiscoveredSites   int64

	MarkerPersistenceAvailable bool
	BGPSitesPerCollection      int64
	BGPFullScanSeconds         int64
	BGPCachedSites             int64

	LastOperations          map[string]operationHealth
	OperationFailures       map[operationFailureKey]int64
	OperationAffectedSites  map[operationFailureKey]int64
	CollectionFailureTotals map[string]int64
	NormalizationIssues     map[normalizationIssueKey]int64
}

type operationHealth struct {
	Success    bool
	ErrorClass string
}

type operationFailureKey struct {
	Operation  string
	ErrorClass string
}

type normalizationIssueKey struct {
	Surface string
	Issue   string
}

func (c *Collector) beginHealthCycle() {
	c.ensureHealth()
	c.health.CollectionSuccess = false
	c.health.DiscoveredSites = int64(len(c.discovery.siteIDs))
	c.health.MarkerPersistenceAvailable = !c.eventsEnabled() || c.markerStoreAvailable
	// LastOperations is stateful because the chart reports last observed status.
}

func (c *Collector) markOperationSuccess(operation string) {
	c.ensureHealth()
	c.health.LastOperations[operation] = operationHealth{Success: true, ErrorClass: "none"}
}

func (c *Collector) markOperationFailure(operation string, err error) {
	c.ensureHealth()
	class := classifyCatoError(err)
	c.health.LastOperations[operation] = operationHealth{ErrorClass: class}
	c.health.OperationFailures[operationFailureKey{Operation: operation, ErrorClass: class}]++
}

func (c *Collector) markOperationAffectedSites(operation string, err error, count int) {
	if count <= 0 {
		return
	}
	c.ensureHealth()
	class := classifyCatoError(err)
	c.health.OperationAffectedSites[operationFailureKey{Operation: operation, ErrorClass: class}] += int64(count)
}

func (c *Collector) markCollectionFailure(err error) {
	c.ensureHealth()
	c.health.CollectionFailureTotals[classifyCatoError(err)]++
}

func (c *Collector) markNormalizationIssue(surface, issue string) {
	c.ensureHealth()
	c.health.NormalizationIssues[normalizationIssueKey{Surface: surface, Issue: issue}]++
}

func (c *Collector) ensureHealth() {
	if c.health.LastOperations == nil {
		c.health.LastOperations = make(map[string]operationHealth)
	}
	if c.health.OperationFailures == nil {
		c.health.OperationFailures = make(map[operationFailureKey]int64)
	}
	if c.health.OperationAffectedSites == nil {
		c.health.OperationAffectedSites = make(map[operationFailureKey]int64)
	}
	if c.health.CollectionFailureTotals == nil {
		c.health.CollectionFailureTotals = make(map[string]int64)
	}
	if c.health.NormalizationIssues == nil {
		c.health.NormalizationIssues = make(map[normalizationIssueKey]int64)
	}
}

func classifyCatoError(err error) string {
	if err == nil {
		return "none"
	}
	if errors.Is(err, context.Canceled) {
		return "canceled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return "timeout"
	}

	var netErr net.Error
	if errors.As(err, &netErr) && netErr.Timeout() {
		return "timeout"
	}

	msg := strings.ToLower(err.Error())
	switch {
	case containsAny(msg, "rate limit", "rate-limit", "ratelimit", "too many requests"),
		containsHTTPStatus(msg, "429"):
		return "rate_limit"
	case containsAny(msg, "unauthorized", "forbidden", "invalid api key"),
		containsHTTPStatus(msg, "401"),
		containsHTTPStatus(msg, "403"):
		return "auth"
	case containsAny(msg, "json", "decode", "unmarshal"):
		return "decode"
	case containsAny(msg, "deadline", "timeout", "i/o timeout"):
		return "timeout"
	case containsAny(msg, "proxyconnect", "proxy error", "proxy url", "proxy"):
		return "proxy"
	case containsAny(msg, "x509", "certificate", "tls", "handshake"):
		return "tls"
	case containsAny(msg, "no such host", "connection refused", "connection reset", "network is unreachable",
		"temporary failure in name resolution", "server misbehaving", "dial tcp", "eof"):
		return "network"
	case containsAny(msg, "no cato sites", "no sites"):
		return "empty"
	case strings.Contains(msg, "pagination"):
		return "pagination"
	case strings.Contains(msg, "graphql"):
		return "graphql"
	default:
		return "error"
	}
}

func containsAny(s string, substrings ...string) bool {
	for _, substring := range substrings {
		if strings.Contains(s, substring) {
			return true
		}
	}
	return false
}

func containsHTTPStatus(msg, code string) bool {
	return strings.Contains(msg, "http status "+code) ||
		strings.Contains(msg, "http "+code) ||
		strings.Contains(msg, "status code "+code) ||
		strings.Contains(msg, "status "+code) ||
		strings.Contains(msg, " "+code+" ")
}
