// SPDX-License-Identifier: GPL-3.0-or-later

package cato_networks

import (
	"fmt"
	"sort"
	"time"

	"github.com/netdata/netdata/go/plugins/pkg/topology"
)

const (
	topologySchemaVersion = "v1"
	topologySource        = "cato_networks"
	topologyLayer         = "wan"

	actorTypeSite    = "cato_site"
	actorTypePop     = "cato_pop"
	actorTypeBGPPeer = "bgp_peer"

	linkTypeTunnel = "cato_tunnel"
	linkTypeBGP    = "bgp_session"
)

func buildTopology(accountID string, sites map[string]*siteState, order []string, collectedAt time.Time) *topology.Data {
	actors := make([]topology.Actor, 0, len(order)*2)
	links := make([]topology.Link, 0, len(order)*2)
	popSeen := make(map[string]bool)

	for _, siteID := range order {
		site := sites[siteID]
		if site == nil {
			continue
		}

		siteActorID := catoSiteActorID(site.ID)
		actors = append(actors, topology.Actor{
			ActorID:   siteActorID,
			ActorType: actorTypeSite,
			Layer:     topologyLayer,
			Source:    topologySource,
			Match: topology.Match{
				CloudAccountID:  accountID,
				CloudInstanceID: site.ID,
				Hostnames:       []string{site.Name},
			},
			Attributes: map[string]any{
				"name":                site.Name,
				"site_id":             site.ID,
				"connectivity_status": site.ConnectivityStatus,
				"operational_status":  site.OperationalStatus,
				"pop_name":            site.PopName,
				"site_type":           site.SiteType,
				"connection_type":     site.ConnectionType,
				"country_code":        site.CountryCode,
				"country_name":        site.CountryName,
				"region":              site.Region,
				"host_count":          site.HostCount,
			},
			Labels: map[string]string{
				"site_id":   site.ID,
				"site_name": site.Name,
				"pop_name":  site.PopName,
			},
			Tables: siteTopologyTables(site),
		})

		if site.PopName != "" {
			popActorID := catoPopActorID(site.PopName)
			if !popSeen[site.PopName] {
				popSeen[site.PopName] = true
				actors = append(actors, topology.Actor{
					ActorID:   popActorID,
					ActorType: actorTypePop,
					Layer:     topologyLayer,
					Source:    topologySource,
					Match: topology.Match{
						CloudAccountID:  accountID,
						CloudInstanceID: site.PopName,
					},
					Attributes: map[string]any{
						"name": site.PopName,
					},
					Labels: map[string]string{
						"pop_name": site.PopName,
					},
				})
			}

			links = append(links, topology.Link{
				Layer:      topologyLayer,
				Protocol:   "cato",
				LinkType:   linkTypeTunnel,
				Direction:  "bidirectional",
				State:      site.ConnectivityStatus,
				SrcActorID: siteActorID,
				DstActorID: popActorID,
				Src: topology.LinkEndpoint{Match: topology.Match{
					CloudAccountID:  accountID,
					CloudInstanceID: site.ID,
				}},
				Dst: topology.LinkEndpoint{Match: topology.Match{
					CloudAccountID:  accountID,
					CloudInstanceID: site.PopName,
				}},
				LastSeen: &collectedAt,
				Metrics: map[string]any{
					"bytes_upstream_max":    site.Metrics.BytesUpstreamMax,
					"bytes_downstream_max":  site.Metrics.BytesDownstreamMax,
					"lost_upstream_percent": site.Metrics.LostUpstreamPercent,
					"rtt_ms":                site.Metrics.RTTMS,
				},
			})
		}

		for _, peer := range site.BGPPeers {
			if peer.RemoteIP == "" && peer.RemoteASN == "" {
				continue
			}
			peerActorID := catoBGPPeerActorID(site.ID, peer.RemoteIP, peer.RemoteASN)
			actors = append(actors, topology.Actor{
				ActorID:   peerActorID,
				ActorType: actorTypeBGPPeer,
				Layer:     topologyLayer,
				Source:    topologySource,
				Match: topology.Match{
					IPAddresses: []string{peer.RemoteIP},
				},
				Attributes: map[string]any{
					"remote_ip":   peer.RemoteIP,
					"remote_asn":  peer.RemoteASN,
					"local_ip":    peer.LocalIP,
					"local_asn":   peer.LocalASN,
					"bgp_session": peer.BGPSession,
				},
				Labels: map[string]string{
					"site_id":  site.ID,
					"peer_ip":  peer.RemoteIP,
					"peer_asn": peer.RemoteASN,
				},
			})

			links = append(links, topology.Link{
				Layer:      topologyLayer,
				Protocol:   "bgp",
				LinkType:   linkTypeBGP,
				Direction:  "bidirectional",
				State:      peer.BGPSession,
				SrcActorID: siteActorID,
				DstActorID: peerActorID,
				Src: topology.LinkEndpoint{Match: topology.Match{
					CloudAccountID:  accountID,
					CloudInstanceID: site.ID,
				}},
				Dst: topology.LinkEndpoint{Match: topology.Match{
					IPAddresses: []string{peer.RemoteIP},
				}},
				LastSeen: &collectedAt,
				Metrics: map[string]any{
					"routes":                    peer.RoutesCount,
					"routes_limit":              peer.RoutesCountLimit,
					"routes_limit_exceeded":     peer.RoutesCountLimitExceeded,
					"rib_out_routes":            peer.RIBOutRoutes,
					"incoming_connection_state": peer.IncomingState,
					"outgoing_connection_state": peer.OutgoingState,
				},
			})
		}
	}

	return &topology.Data{
		SchemaVersion: topologySchemaVersion,
		Source:        topologySource,
		Layer:         topologyLayer,
		AgentID:       accountID,
		CollectedAt:   collectedAt,
		View:          "summary",
		Actors:        actors,
		Links:         links,
		Stats: map[string]any{
			"sites": len(order),
			"links": len(links),
		},
	}
}

func siteTopologyTables(site *siteState) map[string][]map[string]any {
	tables := make(map[string][]map[string]any)
	ifaceKeys := make([]string, 0, len(site.Interfaces))
	for key := range site.Interfaces {
		ifaceKeys = append(ifaceKeys, key)
	}
	sort.Slice(ifaceKeys, func(i, j int) bool {
		left := site.Interfaces[ifaceKeys[i]]
		right := site.Interfaces[ifaceKeys[j]]
		var leftName, rightName string
		if left != nil {
			leftName = left.Name
		}
		if right != nil {
			rightName = right.Name
		}
		if leftName != rightName {
			return leftName < rightName
		}
		return ifaceKeys[i] < ifaceKeys[j]
	})
	for _, key := range ifaceKeys {
		iface := site.Interfaces[key]
		if iface == nil {
			continue
		}
		tables["interfaces"] = append(tables["interfaces"], map[string]any{
			"name":                 iface.Name,
			"type":                 iface.Type,
			"connected":            iface.Connected || iface.LinkUp,
			"pop_name":             iface.PopName,
			"tunnel_remote_ip":     iface.TunnelRemoteIP,
			"tunnel_uptime":        iface.TunnelUptime,
			"upstream_bandwidth":   iface.UpstreamBandwidth,
			"downstream_bandwidth": iface.DownstreamBandwidth,
		})
	}
	devices := append([]deviceState(nil), site.Devices...)
	sort.Slice(devices, func(i, j int) bool {
		if devices[i].Name != devices[j].Name {
			return devices[i].Name < devices[j].Name
		}
		return devices[i].ID < devices[j].ID
	})
	for _, dev := range devices {
		tables["devices"] = append(tables["devices"], map[string]any{
			"name":           dev.Name,
			"type":           dev.Type,
			"connected":      dev.Connected,
			"ha_role":        dev.HaRole,
			"socket_serial":  dev.SocketSerial,
			"socket_version": dev.SocketVersion,
			"internal_ip":    dev.InternalIP,
		})
	}
	return tables
}

func catoSiteActorID(siteID string) string {
	return "cato:site:" + siteID
}

func catoPopActorID(popName string) string {
	return "cato:pop:" + popName
}

func catoBGPPeerActorID(siteID, remoteIP, remoteASN string) string {
	return fmt.Sprintf("cato:bgp:%s:%s:%s", siteID, remoteIP, remoteASN)
}
