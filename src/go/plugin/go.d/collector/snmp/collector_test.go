// SPDX-License-Identifier: GPL-3.0-or-later

package snmp

import (
	"context"
	"errors"
	"os"
	"slices"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/netdata/netdata/go/plugins/logger"
	"github.com/netdata/netdata/go/plugins/pkg/confopt"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/collector/snmp/ddsnmp/ddsnmpcollector"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/pkg/collecttest"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/pkg/pinger"
	"github.com/netdata/netdata/go/plugins/plugin/go.d/pkg/snmputils"

	"github.com/golang/mock/gomock"
	"github.com/gosnmp/gosnmp"
	snmpmock "github.com/gosnmp/gosnmp/mocks"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

var (
	dataConfigJSON, _ = os.ReadFile("testdata/config.json")
	dataConfigYAML, _ = os.ReadFile("testdata/config.yaml")
)

func Test_testDataIsValid(t *testing.T) {
	for name, data := range map[string][]byte{
		"dataConfigJSON": dataConfigJSON,
		"dataConfigYAML": dataConfigYAML,
	} {
		require.NotNil(t, data, name)
	}
}

func TestCollector_ConfigurationSerialize(t *testing.T) {
	collecttest.TestConfigurationSerialize(t, &Collector{}, dataConfigJSON, dataConfigYAML)
}

func TestCollector_Init(t *testing.T) {
	tests := map[string]struct {
		prepareSNMP func() *Collector
		wantFail    bool
	}{
		"fail with default config": {
			wantFail: true,
			prepareSNMP: func() *Collector {
				return New()
			},
		},
		"fail when using SNMPv3 but 'user.name' not set": {
			wantFail: true,
			prepareSNMP: func() *Collector {
				collr := New()
				collr.Config = prepareV3Config()
				collr.User.Name = ""
				return collr
			},
		},
		"success when using SNMPv1 with valid config": {
			wantFail: false,
			prepareSNMP: func() *Collector {
				collr := New()
				collr.Config = prepareV1Config()
				return collr
			},
		},
		"success when using SNMPv2 with valid config": {
			wantFail: false,
			prepareSNMP: func() *Collector {
				collr := New()
				collr.Config = prepareV2Config()
				return collr
			},
		},
		"success when using SNMPv3 with valid config": {
			wantFail: false,
			prepareSNMP: func() *Collector {
				collr := New()
				collr.Config = prepareV3Config()
				return collr
			},
		},
	}

	for name, test := range tests {
		t.Run(name, func(t *testing.T) {
			collr := test.prepareSNMP()

			if test.wantFail {
				assert.Error(t, collr.Init(context.Background()))
			} else {
				assert.NoError(t, collr.Init(context.Background()))
			}
		})
	}
}

func TestCollector_InitPassesSharedPingerConfig(t *testing.T) {
	var gotCfg pinger.Config

	collr := New()
	collr.Config = prepareV2Config()
	collr.PingOnly = true
	collr.Ping.Network = "ip6"
	collr.Ping.Interface = "eth0"
	collr.Ping.Privileged = false
	collr.Ping.Packets = 4
	collr.Ping.Interval = confopt.Duration(250 * time.Millisecond)
	collr.newPinger = func(cfg pinger.Config, _ *logger.Logger) (pinger.Client, error) {
		gotCfg = cfg
		return &mockPingClient{}, nil
	}

	require.NoError(t, collr.Init(context.Background()))

	assert.Equal(t, pinger.Config{
		Probe: pinger.ProbeConfig{
			Network:    "ip6",
			Interface:  "eth0",
			Privileged: false,
			Packets:    4,
			Interval:   confopt.Duration(250 * time.Millisecond),
			Timeout:    time.Second,
		},
	}, gotCfg)
}

func TestCollector_Cleanup(t *testing.T) {
	tests := map[string]struct {
		prepareSNMP func(t *testing.T, m *snmpmock.MockHandler) *Collector
	}{
		"cleanup call does not panic if snmpClient not initialized": {
			prepareSNMP: func(t *testing.T, m *snmpmock.MockHandler) *Collector {
				collr := New()
				collr.Config = prepareV2Config()
				collr.newSnmpClient = func() gosnmp.Handler { return m }
				setMockClientInitExpect(m)

				require.NoError(t, collr.Init(context.Background()))

				collr.snmpClient = nil

				return collr
			},
		},
	}

	for name, test := range tests {
		t.Run(name, func(t *testing.T) {
			mockSNMP, cleanup := mockInit(t)
			defer cleanup()

			collr := test.prepareSNMP(t, mockSNMP)

			assert.NotPanics(t, func() { collr.Cleanup(context.Background()) })
		})
	}
}

func TestCollector_Check(t *testing.T) {
	tests := map[string]struct {
		prepare func(m *snmpmock.MockHandler) *Collector
		wantErr bool
	}{
		"success: connects and reads sysInfo": {
			wantErr: false,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				c := New()
				c.Config = prepareV2Config()
				c.CreateVnode = false
				c.Ping.Enabled = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				return c
			},
		},

		"failure: SNMP connect error": {
			wantErr: true,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientSetterExpect(m)
				m.EXPECT().Connect().Return(errors.New("connect failed")).AnyTimes()

				c := New()
				c.Config = prepareV2Config()
				c.CreateVnode = false
				c.Ping.Enabled = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				return c
			},
		},

		"failure: sysInfo walk error": {
			wantErr: true,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				// Normal init succeeds
				setMockClientInitExpect(m)
				// But sysInfo retrieval (WalkAll on system tree) fails
				// If your helper is too opinionated, stub directly:
				// The collector ultimately calls WalkAll on the system OID tree.
				m.EXPECT().
					WalkAll(gomock.Any()).
					Return(nil, errors.New("walk failed"))

				c := New()
				c.Config = prepareV2Config()
				c.CreateVnode = false
				c.Ping.Enabled = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				return c
			},
		},

		"success: ping_only with successful ping": {
			wantErr: false,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				c := New()
				c.Config = prepareV2Config()
				c.PingOnly = true
				c.CreateVnode = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				c.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
					return &mockPingClient{sample: pingSuccessSample(c.Hostname)}, nil
				}
				return c
			},
		},

		"success: ping_only with recoverable ping error": {
			wantErr: false,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				c := New()
				c.Config = prepareV2Config()
				c.PingOnly = true
				c.CreateVnode = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				c.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
					return &mockPingClient{probeErr: errors.New("host unreachable")}, nil
				}
				return c
			},
		},

		"failure: ping_only with unrecoverable ping error": {
			wantErr: true,
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				c := New()
				c.Config = prepareV2Config()
				c.PingOnly = true
				c.CreateVnode = false
				c.newSnmpClient = func() gosnmp.Handler { return m }
				c.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
					return &mockPingClient{
						probeErr: &pinger.ProbeError{Host: c.Hostname, Stage: "run", Err: syscall.EPERM},
					}, nil
				}
				return c
			},
		},
	}

	for name, tc := range tests {
		t.Run(name, func(t *testing.T) {
			ctrl := gomock.NewController(t)
			defer ctrl.Finish()

			mockSNMP := snmpmock.NewMockHandler(ctrl)

			collr := tc.prepare(mockSNMP)
			require.NoError(t, collr.Init(context.Background()))

			err := collr.Check(context.Background())
			if tc.wantErr {
				assert.Error(t, err)
			} else {
				assert.NoError(t, err)
			}
		})
	}
}

func TestCollector_CheckPingOnlyUsesReadOnlyProbing(t *testing.T) {
	ctrl := gomock.NewController(t)
	defer ctrl.Finish()

	mockSNMP := snmpmock.NewMockHandler(ctrl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	pingClient := &mockPingClient{sample: pingSuccessSample("192.0.2.1")}

	collr := New()
	collr.Config = prepareV2Config()
	collr.PingOnly = true
	collr.CreateVnode = false
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
		return pingClient, nil
	}

	require.NoError(t, collr.Init(context.Background()))
	type ctxKey struct{}
	ctx := context.WithValue(context.Background(), ctxKey{}, "check")
	require.NoError(t, collr.Check(ctx))

	calls := pingClient.probeCalls()
	require.Len(t, calls, 1)
	assert.Equal(t, "probe", calls[0].method)
	assert.Equal(t, "check", calls[0].ctx.Value(ctxKey{}))
}

func TestCollector_Collect(t *testing.T) {
	tests := map[string]struct {
		prepare func(m *snmpmock.MockHandler) *Collector
		want    map[string]int64
	}{
		"collects scalar metric": {
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				collr := New()
				collr.Config = prepareV2Config()
				collr.CreateVnode = false
				collr.Ping.Enabled = false
				collr.snmpProfiles = []*ddsnmp.Profile{{}} // non-empty to enable collectSNMP()
				collr.newSnmpClient = func() gosnmp.Handler { return m }
				collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
					return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{
						{
							Source: "test",
							Metrics: []ddsnmp.Metric{
								{
									Name:    "uptime",
									IsTable: false,
									Value:   123,
									Unit:    "s",
									Tags:    map[string]string{},
									Profile: &ddsnmp.ProfileMetrics{Tags: map[string]string{}},
								},
							},
						},
					}}
				}
				return collr
			},
			want: map[string]int64{
				// scalar → "snmp_device_prof_<name>"
				"snmp_device_prof_test_stats_errors_processing_scalar": 0,
				"snmp_device_prof_test_stats_errors_processing_table":  0,
				"snmp_device_prof_test_stats_errors_snmp":              0,
				"snmp_device_prof_test_stats_metrics_rows":             0,
				"snmp_device_prof_test_stats_metrics_licensing":        0,
				"snmp_device_prof_test_stats_metrics_scalar":           0,
				"snmp_device_prof_test_stats_metrics_table":            0,
				"snmp_device_prof_test_stats_metrics_tables":           0,
				"snmp_device_prof_test_stats_metrics_virtual":          0,
				"snmp_device_prof_test_stats_snmp_get_oids":            0,
				"snmp_device_prof_test_stats_snmp_get_requests":        0,
				"snmp_device_prof_test_stats_snmp_tables_cached":       0,
				"snmp_device_prof_test_stats_snmp_tables_walked":       0,
				"snmp_device_prof_test_stats_snmp_walk_pdus":           0,
				"snmp_device_prof_test_stats_snmp_walk_requests":       0,
				"snmp_device_prof_test_stats_table_cache_hits":         0,
				"snmp_device_prof_test_stats_table_cache_misses":       0,
				"snmp_device_prof_test_stats_timings_scalar":           0,
				"snmp_device_prof_test_stats_timings_table":            0,
				"snmp_device_prof_test_stats_timings_virtual":          0,
				"snmp_device_prof_uptime":                              123,
			},
		},
		"collects table multivalue metric": {
			prepare: func(m *snmpmock.MockHandler) *Collector {
				setMockClientInitExpect(m)
				setMockClientSysInfoExpect(m)

				collr := New()
				collr.Config = prepareV2Config()
				collr.CreateVnode = false
				collr.Ping.Enabled = false
				collr.snmpProfiles = []*ddsnmp.Profile{{}}
				collr.newSnmpClient = func() gosnmp.Handler { return m }
				collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
					return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{
						{
							Source: "test",
							Metrics: []ddsnmp.Metric{
								{
									Name:    "if_octets",
									IsTable: true,
									Unit:    "bit/s",
									Tags:    map[string]string{"ifName": "eth0"},
									Profile: &ddsnmp.ProfileMetrics{Tags: map[string]string{}},
									MultiValue: map[string]int64{
										"in":  1,
										"out": 2,
									},
								},
							},
						},
					}}
				}
				return collr
			},
			want: map[string]int64{
				// table key: "snmp_device_prof_<name>_<sorted tag values>_<subkey>"
				// here tags = {"ifName":"eth0"} → key part becomes "_eth0"
				"snmp_device_prof_test_stats_errors_processing_scalar": 0,
				"snmp_device_prof_test_stats_errors_processing_table":  0,
				"snmp_device_prof_test_stats_errors_snmp":              0,
				"snmp_device_prof_test_stats_metrics_rows":             0,
				"snmp_device_prof_test_stats_metrics_licensing":        0,
				"snmp_device_prof_test_stats_metrics_scalar":           0,
				"snmp_device_prof_test_stats_metrics_table":            0,
				"snmp_device_prof_test_stats_metrics_tables":           0,
				"snmp_device_prof_test_stats_metrics_virtual":          0,
				"snmp_device_prof_test_stats_snmp_get_oids":            0,
				"snmp_device_prof_test_stats_snmp_get_requests":        0,
				"snmp_device_prof_test_stats_snmp_tables_cached":       0,
				"snmp_device_prof_test_stats_snmp_tables_walked":       0,
				"snmp_device_prof_test_stats_snmp_walk_pdus":           0,
				"snmp_device_prof_test_stats_snmp_walk_requests":       0,
				"snmp_device_prof_test_stats_table_cache_hits":         0,
				"snmp_device_prof_test_stats_table_cache_misses":       0,
				"snmp_device_prof_test_stats_timings_scalar":           0,
				"snmp_device_prof_test_stats_timings_table":            0,
				"snmp_device_prof_test_stats_timings_virtual":          0,
				"snmp_device_prof_if_octets_eth0_in":                   1,
				"snmp_device_prof_if_octets_eth0_out":                  2,
			},
		},
	}
	tests["collects ping only metrics"] = struct {
		prepare func(m *snmpmock.MockHandler) *Collector
		want    map[string]int64
	}{
		prepare: func(m *snmpmock.MockHandler) *Collector {
			setMockClientInitExpect(m)
			setMockClientSysInfoExpect(m)

			collr := New()
			collr.Config = prepareV2Config()
			collr.PingOnly = true
			collr.CreateVnode = false
			collr.newSnmpClient = func() gosnmp.Handler { return m }
			collr.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
				return &mockPingClient{sample: pingSuccessSample(collr.Hostname)}, nil
			}

			return collr
		},
		want: map[string]int64{
			"ping_rtt_min":    (10 * time.Millisecond).Microseconds(),
			"ping_rtt_max":    (20 * time.Millisecond).Microseconds(),
			"ping_rtt_avg":    (15 * time.Millisecond).Microseconds(),
			"ping_rtt_stddev": (5 * time.Millisecond).Microseconds(),
		},
	}
	tests["collects no ping metrics when probe gets no replies"] = struct {
		prepare func(m *snmpmock.MockHandler) *Collector
		want    map[string]int64
	}{
		prepare: func(m *snmpmock.MockHandler) *Collector {
			setMockClientInitExpect(m)
			setMockClientSysInfoExpect(m)

			collr := New()
			collr.Config = prepareV2Config()
			collr.PingOnly = true
			collr.CreateVnode = false
			collr.newSnmpClient = func() gosnmp.Handler { return m }
			collr.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
				return &mockPingClient{sample: pingNoReplySample(collr.Hostname)}, nil
			}

			return collr
		},
		want: nil,
	}

	for name, tc := range tests {
		t.Run(name, func(t *testing.T) {
			mockCtl := gomock.NewController(t)
			defer mockCtl.Finish()

			mockSNMP := snmpmock.NewMockHandler(mockCtl)

			collr := tc.prepare(mockSNMP)
			require.NoError(t, collr.Init(context.Background()))

			_ = collr.Check(context.Background())

			got := collr.Collect(context.Background())
			assert.Equal(t, tc.want, got)
		})
	}
}

func TestCollector_CollectPingOnlyUsesTrackingProbing(t *testing.T) {
	ctrl := gomock.NewController(t)
	defer ctrl.Finish()

	mockSNMP := snmpmock.NewMockHandler(ctrl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	pingClient := &mockPingClient{sample: pingSuccessSample("192.0.2.1")}

	collr := New()
	collr.Config = prepareV2Config()
	collr.PingOnly = true
	collr.CreateVnode = false
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
		return pingClient, nil
	}

	require.NoError(t, collr.Init(context.Background()))
	type checkKey struct{}
	type collectKey struct{}
	checkCtx := context.WithValue(context.Background(), checkKey{}, "check")
	collectCtx := context.WithValue(context.Background(), collectKey{}, "collect")
	_ = collr.Check(checkCtx)
	got := collr.Collect(collectCtx)

	assert.Equal(t, map[string]int64{
		"ping_rtt_min":    (10 * time.Millisecond).Microseconds(),
		"ping_rtt_max":    (20 * time.Millisecond).Microseconds(),
		"ping_rtt_avg":    (15 * time.Millisecond).Microseconds(),
		"ping_rtt_stddev": (5 * time.Millisecond).Microseconds(),
	}, got)

	calls := pingClient.probeCalls()
	require.Len(t, calls, 2)
	assert.Equal(t, "probe", calls[0].method)
	assert.Equal(t, "check", calls[0].ctx.Value(checkKey{}))
	assert.Equal(t, "probe_and_track", calls[1].method)
	assert.Equal(t, "collect", calls[1].ctx.Value(collectKey{}))
}

func TestCollector_CollectMixedModeAllowsNilContext(t *testing.T) {
	ctrl := gomock.NewController(t)
	defer ctrl.Finish()

	mockSNMP := snmpmock.NewMockHandler(ctrl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	pingClient := &mockPingClient{sample: pingSuccessSample("192.0.2.1")}

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = true
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newPinger = func(cfg pinger.Config, log *logger.Logger) (pinger.Client, error) {
		return pingClient, nil
	}
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{
			{
				Source: "test",
				Metrics: []ddsnmp.Metric{
					{
						Name:    "uptime",
						IsTable: false,
						Value:   123,
						Unit:    "s",
						Tags:    map[string]string{},
						Profile: &ddsnmp.ProfileMetrics{Tags: map[string]string{}},
					},
				},
			},
		}}
	}

	require.NoError(t, collr.Init(context.Background()))
	require.NoError(t, collr.Check(context.Background()))

	got := collr.Collect(nil)

	assert.Equal(t, map[string]int64{
		"snmp_device_prof_test_stats_errors_processing_scalar": 0,
		"snmp_device_prof_test_stats_errors_processing_table":  0,
		"snmp_device_prof_test_stats_errors_snmp":              0,
		"snmp_device_prof_test_stats_metrics_rows":             0,
		"snmp_device_prof_test_stats_metrics_licensing":        0,
		"snmp_device_prof_test_stats_metrics_scalar":           0,
		"snmp_device_prof_test_stats_metrics_table":            0,
		"snmp_device_prof_test_stats_metrics_tables":           0,
		"snmp_device_prof_test_stats_metrics_virtual":          0,
		"snmp_device_prof_test_stats_snmp_get_oids":            0,
		"snmp_device_prof_test_stats_snmp_get_requests":        0,
		"snmp_device_prof_test_stats_snmp_tables_cached":       0,
		"snmp_device_prof_test_stats_snmp_tables_walked":       0,
		"snmp_device_prof_test_stats_snmp_walk_pdus":           0,
		"snmp_device_prof_test_stats_snmp_walk_requests":       0,
		"snmp_device_prof_test_stats_table_cache_hits":         0,
		"snmp_device_prof_test_stats_table_cache_misses":       0,
		"snmp_device_prof_test_stats_timings_scalar":           0,
		"snmp_device_prof_test_stats_timings_table":            0,
		"snmp_device_prof_test_stats_timings_virtual":          0,
		"snmp_device_prof_uptime":                              123,
		"ping_rtt_min":                                         (10 * time.Millisecond).Microseconds(),
		"ping_rtt_max":                                         (20 * time.Millisecond).Microseconds(),
		"ping_rtt_avg":                                         (15 * time.Millisecond).Microseconds(),
		"ping_rtt_stddev":                                      (5 * time.Millisecond).Microseconds(),
	}, got)
}

type probeCall struct {
	host   string
	method string
	ctx    context.Context
}

type mockPingClient struct {
	mu       sync.Mutex
	sample   pinger.Sample
	probeErr error
	calls    []probeCall
}

func (m *mockPingClient) Probe(ctx context.Context, host string) (pinger.Sample, error) {
	return m.recordedProbe(ctx, host, "probe")
}

func (m *mockPingClient) ProbeAndTrack(ctx context.Context, host string) (pinger.Sample, error) {
	return m.recordedProbe(ctx, host, "probe_and_track")
}

func (m *mockPingClient) recordedProbe(ctx context.Context, host, method string) (pinger.Sample, error) {
	m.mu.Lock()
	defer m.mu.Unlock()

	m.calls = append(m.calls, probeCall{host: host, method: method, ctx: ctx})
	if m.probeErr != nil {
		return pinger.Sample{}, m.probeErr
	}

	sample := m.sample
	if sample.Host == "" {
		sample.Host = host
	}
	return sample, nil
}

func (m *mockPingClient) probeCalls() []probeCall {
	m.mu.Lock()
	defer m.mu.Unlock()
	return slices.Clone(m.calls)
}

func pingSuccessSample(host string) pinger.Sample {
	return pinger.Sample{
		Host:        host,
		PacketsRecv: 5,
		PacketsSent: 5,
		RTT: pinger.RTTSummary{
			Valid:  true,
			Min:    10 * time.Millisecond,
			Max:    20 * time.Millisecond,
			Avg:    15 * time.Millisecond,
			StdDev: 5 * time.Millisecond,
		},
	}
}

func pingNoReplySample(host string) pinger.Sample {
	return pinger.Sample{
		Host:        host,
		PacketsRecv: 0,
		PacketsSent: 5,
	}
}

func TestCollector_Collect_LicensingAggregation(t *testing.T) {
	mockCtl := gomock.NewController(t)
	defer mockCtl.Finish()

	mockSNMP := snmpmock.NewMockHandler(mockCtl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	expiry := time.Now().UTC().Add(48 * time.Hour).Unix()

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = false
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		// A single Check Point license contributes four signals under the same
		// _license_id: state severity, expiry, used, capacity. licensing.go
		// merges them into one row before aggregation.
		pm := &ddsnmp.ProfileMetrics{
			Source: "checkpoint.yaml",
			HiddenMetrics: []ddsnmp.Metric{
				licenseSignal("17", "Application Control", licenseValueKindStateSeverity, 1,
					map[string]string{tagLicenseStateRaw: "about-to-expire"}),
				licenseSignal("17", "Application Control", licenseValueKindExpiryTimestamp, expiry, nil),
				licenseSignal("17", "Application Control", licenseValueKindUsage, 95, nil),
				licenseSignal("17", "Application Control", licenseValueKindCapacity, 100, nil),
			},
		}
		for i := range pm.HiddenMetrics {
			pm.HiddenMetrics[i].Profile = pm
		}
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{pm}}
	}

	require.NoError(t, collr.Init(context.Background()))
	_ = collr.Check(context.Background())

	start := time.Now().UTC()
	got := collr.Collect(context.Background())
	require.NotNil(t, got)

	assert.EqualValues(t, 0, got[metricIDLicenseStateHealthy])
	assert.EqualValues(t, 1, got[metricIDLicenseStateDegraded])
	assert.EqualValues(t, 0, got[metricIDLicenseStateBroken])
	assert.EqualValues(t, 0, got[metricIDLicenseStateIgnored])
	assert.EqualValues(t, 95, got[metricIDLicenseUsagePercent])
	expectedRemaining := expiry - start.Unix()
	assert.GreaterOrEqual(t, got[metricIDLicenseRemainingTime], expectedRemaining-30)
	assert.LessOrEqual(t, got[metricIDLicenseRemainingTime], expectedRemaining)
	assert.Contains(t, got, "snmp_device_prof_checkpoint_stats_metrics_table")
}

// licenseSignal builds one hidden _license_row metric. Multiple signals that
// share the same id belong to the same license and licensing.go merges them
// into one row before aggregation, matching the new documented contract.
func licenseSignal(id, name, kind string, value int64, extraTags map[string]string) ddsnmp.Metric {
	tags := map[string]string{
		tagLicenseID:        id,
		tagLicenseName:      name,
		tagLicenseValueKind: kind,
	}
	for k, v := range extraTags {
		tags[k] = v
	}
	return ddsnmp.Metric{
		Name:  licenseSourceMetricName,
		Value: value,
		Tags:  tags,
	}
}

func TestCollector_Collect_LicensingAggregation_CiscoSmartPartialData(t *testing.T) {
	mockCtl := gomock.NewController(t)
	defer mockCtl.Finish()

	mockSNMP := snmpmock.NewMockHandler(mockCtl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	authExpiry := time.Now().UTC().Add(48 * time.Hour).Unix()
	certExpiry := time.Now().UTC().Add(72 * time.Hour).Unix()

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = false
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		// Cisco Smart Licensing is exposed as scalar OIDs. Each licensing
		// signal is its own _license_row metric with static tags identifying
		// the logical license and a transform stamp for the value kind.
		pm := &ddsnmp.ProfileMetrics{
			Source: "cisco.yaml",
			HiddenMetrics: []ddsnmp.Metric{
				licenseSignal("smart_authorization_state", "Smart Licensing authorization state",
					licenseValueKindStateSeverity, 0, map[string]string{
						tagLicenseComponent: "smart_licensing",
						tagLicenseType:      "authorization",
					}),
				licenseSignal("smart_authorization_expiry", "Smart Licensing authorization",
					licenseValueKindAuthorizationTimestamp, authExpiry, map[string]string{
						tagLicenseComponent:           "smart_licensing",
						tagLicenseType:                "authorization",
						tagLicenseAuthorizationSource: "ciscoSlaAuthExpireTime",
					}),
				licenseSignal("smart_id_certificate_expiry", "Smart Licensing ID certificate",
					licenseValueKindCertificateTimestamp, certExpiry, map[string]string{
						tagLicenseComponent:         "smart_licensing",
						tagLicenseType:              "certificate",
						tagLicenseCertificateSource: "ciscoSlaNextCertificateExpireTime",
					}),
				licenseSignal("dna_advantage", "network-advantage",
					licenseValueKindStateSeverity, 2, map[string]string{
						tagLicenseComponent: "smart_licensing",
						tagLicenseType:      "entitlement",
						tagLicenseStateRaw:  "authorization_expired",
					}),
				licenseSignal("dna_advantage", "network-advantage",
					licenseValueKindUsage, 42, nil),
			},
		}
		for i := range pm.HiddenMetrics {
			pm.HiddenMetrics[i].Profile = pm
		}
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{pm}}
	}

	require.NoError(t, collr.Init(context.Background()))
	_ = collr.Check(context.Background())

	got := collr.Collect(context.Background())
	require.NotNil(t, got)

	// 4 logical rows: state (healthy), auth (healthy), cert (healthy), entitlement (broken).
	assert.EqualValues(t, 3, got[metricIDLicenseStateHealthy])
	assert.EqualValues(t, 0, got[metricIDLicenseStateDegraded])
	assert.EqualValues(t, 1, got[metricIDLicenseStateBroken])
	assert.EqualValues(t, 0, got[metricIDLicenseStateIgnored])
	assert.GreaterOrEqual(t, got[metricIDLicenseAuthorizationRemainingTime], int64((48*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseAuthorizationRemainingTime], int64(48*time.Hour/time.Second))
	assert.GreaterOrEqual(t, got[metricIDLicenseCertificateRemainingTime], int64((72*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseCertificateRemainingTime], int64(72*time.Hour/time.Second))
	assert.NotContains(t, got, metricIDLicenseRemainingTime)
	assert.NotContains(t, got, metricIDLicenseGraceRemainingTime)
	assert.NotContains(t, got, metricIDLicenseUsagePercent)
}

func TestCollector_Collect_LicensingAggregation_CiscoTraditional(t *testing.T) {
	mockCtl := gomock.NewController(t)
	defer mockCtl.Finish()

	mockSNMP := snmpmock.NewMockHandler(mockCtl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	securityExpiry := time.Now().UTC().Add(72 * time.Hour).Unix()

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = false
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		// "SECURITYK9" is healthy (in use, 85% used); "APPXK9" is broken
		// (usage count consumed) with 1 hour of grace left. licensing.go
		// reports the worst state and the highest usage across rows.
		pm := &ddsnmp.ProfileMetrics{
			Source: "cisco.yaml",
			HiddenMetrics: []ddsnmp.Metric{
				licenseSignal("17", "SECURITYK9", licenseValueKindExpiryTimestamp, securityExpiry,
					map[string]string{tagLicenseType: "paid_subscription", tagLicenseStateRaw: "in_use"}),
				licenseSignal("17", "SECURITYK9", licenseValueKindCapacity, 100, nil),
				licenseSignal("17", "SECURITYK9", licenseValueKindAvailable, 15, nil),
				licenseSignal("23", "APPXK9", licenseValueKindStateSeverity, 2,
					map[string]string{
						tagLicenseType:      "grace_period",
						tagLicenseStateRaw:  "usage_count_consumed",
						tagLicenseComponent: "traditional_licensing",
					}),
				licenseSignal("23", "APPXK9", licenseValueKindGraceRemaining, 3600, nil),
				licenseSignal("23", "APPXK9", licenseValueKindCapacity, 10, nil),
				licenseSignal("23", "APPXK9", licenseValueKindAvailable, 0, nil),
			},
		}
		for i := range pm.HiddenMetrics {
			pm.HiddenMetrics[i].Profile = pm
		}
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{pm}}
	}

	require.NoError(t, collr.Init(context.Background()))
	_ = collr.Check(context.Background())

	got := collr.Collect(context.Background())
	require.NotNil(t, got)

	assert.EqualValues(t, 1, got[metricIDLicenseStateHealthy])
	assert.EqualValues(t, 0, got[metricIDLicenseStateDegraded])
	assert.EqualValues(t, 1, got[metricIDLicenseStateBroken])
	assert.EqualValues(t, 0, got[metricIDLicenseStateIgnored])
	// APPXK9 is fully consumed (0 available of 10) → 100% usage, wins max.
	assert.EqualValues(t, 100, got[metricIDLicenseUsagePercent])
	assert.GreaterOrEqual(t, got[metricIDLicenseRemainingTime], int64((72*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseRemainingTime], int64(72*time.Hour/time.Second))
	assert.GreaterOrEqual(t, got[metricIDLicenseGraceRemainingTime], int64((time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseGraceRemainingTime], int64(time.Hour/time.Second))
}

func TestCollector_Collect_LicensingAggregation_SelectsWorstCaseAcrossMixedRows(t *testing.T) {
	mockCtl := gomock.NewController(t)
	defer mockCtl.Finish()

	mockSNMP := snmpmock.NewMockHandler(mockCtl)
	setMockClientInitExpect(mockSNMP)
	setMockClientSysInfoExpect(mockSNMP)

	now := time.Now().UTC()
	perpetualExpiry := now.Add(30 * time.Minute).Unix()
	earliestRealExpiry := now.Add(6 * time.Hour).Unix()
	authExpiry := now.Add(30 * time.Hour).Unix()
	certExpiry := now.Add(20 * time.Hour).Unix()
	graceExpiry := now.Add(10 * time.Hour).Unix()

	collr := New()
	collr.Config = prepareV2Config()
	collr.CreateVnode = false
	collr.Ping.Enabled = false
	collr.snmpProfiles = []*ddsnmp.Profile{{}}
	collr.newSnmpClient = func() gosnmp.Handler { return mockSNMP }
	collr.newDdSnmpColl = func(ddsnmpcollector.Config) ddCollector {
		// Mixed corpus: a perpetual base whose expiry must not count toward
		// the device-wide earliest-expiry signal, a soon-expiring contract
		// (wins), auth/cert/grace timers, an explicit broken row, and an
		// unlimited pool at 100% which must not count toward usage pressure.
		pm := &ddsnmp.ProfileMetrics{
			Source: "mixed-licensing.yaml",
			HiddenMetrics: []ddsnmp.Metric{
				licenseSignal("perpetual", "Perpetual base", licenseValueKindExpiryTimestamp, perpetualExpiry,
					map[string]string{tagLicensePerpetual: "true", tagLicenseStateRaw: "active"}),
				licenseSignal("perpetual", "Perpetual base", licenseValueKindUsage, 50, nil),
				licenseSignal("perpetual", "Perpetual base", licenseValueKindCapacity, 100, nil),

				licenseSignal("soonest_expiring", "Threat prevention", licenseValueKindExpiryTimestamp, earliestRealExpiry,
					map[string]string{tagLicenseStateRaw: "about-to-expire"}),
				licenseSignal("soonest_expiring", "Threat prevention", licenseValueKindUsage, 90, nil),
				licenseSignal("soonest_expiring", "Threat prevention", licenseValueKindCapacity, 100, nil),

				licenseSignal("auth", "Smart auth", licenseValueKindAuthorizationTimestamp, authExpiry,
					map[string]string{tagLicenseAuthorizationSource: "auth_timer"}),
				licenseSignal("cert", "Smart cert", licenseValueKindCertificateTimestamp, certExpiry,
					map[string]string{tagLicenseCertificateSource: "cert_timer"}),
				licenseSignal("grace", "Eval grace", licenseValueKindGraceTimestamp, graceExpiry,
					map[string]string{tagLicenseStateRaw: "evaluation"}),

				licenseSignal("broken", "Broken feature", licenseValueKindStateSeverity, 2, nil),

				licenseSignal("unlimited", "Unlimited pool", licenseValueKindUsagePercent, 100,
					map[string]string{tagLicenseUnlimited: "true"}),
			},
		}
		for i := range pm.HiddenMetrics {
			pm.HiddenMetrics[i].Profile = pm
		}
		return &mockDdSnmpCollector{pms: []*ddsnmp.ProfileMetrics{pm}}
	}

	require.NoError(t, collr.Init(context.Background()))
	_ = collr.Check(context.Background())

	got := collr.Collect(context.Background())
	require.NotNil(t, got)

	// 7 rows: perpetual + auth + cert + unlimited = 4 healthy,
	// soonest (about-to-expire) + grace (evaluation) = 2 degraded,
	// broken = 1 broken.
	assert.EqualValues(t, 4, got[metricIDLicenseStateHealthy])
	assert.EqualValues(t, 2, got[metricIDLicenseStateDegraded])
	assert.EqualValues(t, 1, got[metricIDLicenseStateBroken])
	assert.EqualValues(t, 0, got[metricIDLicenseStateIgnored])
	// Soonest-expiring row contributes 90% (unlimited pool skipped).
	assert.EqualValues(t, 90, got[metricIDLicenseUsagePercent])
	// Perpetual license must not contribute to the earliest-expiry signal.
	assert.GreaterOrEqual(t, got[metricIDLicenseRemainingTime], int64((6*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseRemainingTime], int64(6*time.Hour/time.Second))
	assert.GreaterOrEqual(t, got[metricIDLicenseAuthorizationRemainingTime], int64((30*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseAuthorizationRemainingTime], int64(30*time.Hour/time.Second))
	assert.GreaterOrEqual(t, got[metricIDLicenseCertificateRemainingTime], int64((20*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseCertificateRemainingTime], int64(20*time.Hour/time.Second))
	assert.GreaterOrEqual(t, got[metricIDLicenseGraceRemainingTime], int64((10*time.Hour/time.Second)-5))
	assert.LessOrEqual(t, got[metricIDLicenseGraceRemainingTime], int64(10*time.Hour/time.Second))
}

type mockDdSnmpCollector struct {
	pms  []*ddsnmp.ProfileMetrics
	meta map[string]ddsnmp.MetaTag
	err  error

	collectCalls int
}

func (m *mockDdSnmpCollector) Collect() ([]*ddsnmp.ProfileMetrics, error) {
	m.collectCalls++
	return m.pms, m.err
}

func (m *mockDdSnmpCollector) CollectDeviceMetadata() (map[string]ddsnmp.MetaTag, error) {
	return m.meta, nil
}

func prepareV3Config() Config {
	cfg := prepareV2Config()
	cfg.Options.Version = gosnmp.Version3.String()
	cfg.User = UserConfig{
		Name:          "name",
		SecurityLevel: "authPriv",
		AuthProto:     strings.ToLower(gosnmp.MD5.String()),
		AuthKey:       "auth_key",
		PrivProto:     strings.ToLower(gosnmp.AES.String()),
		PrivKey:       "priv_key",
		ContextName:   "test-context",
	}
	return cfg
}

func prepareV2Config() Config {
	cfg := prepareV1Config()
	cfg.Options.Version = gosnmp.Version2c.String()
	return cfg
}

func prepareV1Config() Config {
	return Config{
		UpdateEvery: 1,
		Hostname:    "192.0.2.1",
		Community:   "public",
		Options: OptionsConfig{
			Port:           161,
			Retries:        1,
			Timeout:        5,
			Version:        gosnmp.Version1.String(),
			MaxOIDs:        20,
			MaxRepetitions: 25,
		},
	}
}

func mockInit(t *testing.T) (*snmpmock.MockHandler, func()) {
	mockCtl := gomock.NewController(t)
	cleanup := func() { mockCtl.Finish() }
	mockSNMP := snmpmock.NewMockHandler(mockCtl)

	return mockSNMP, cleanup
}

func setMockClientInitExpect(m *snmpmock.MockHandler) {
	setMockClientSetterExpect(m)
	m.EXPECT().Connect().Return(nil).AnyTimes()
}

func setMockClientSetterExpect(m *snmpmock.MockHandler) {
	m.EXPECT().Target().AnyTimes()
	m.EXPECT().Port().AnyTimes()
	m.EXPECT().Version().AnyTimes()
	m.EXPECT().Community().AnyTimes()
	m.EXPECT().SetTarget(gomock.Any()).AnyTimes()
	m.EXPECT().SetPort(gomock.Any()).AnyTimes()
	m.EXPECT().SetRetries(gomock.Any()).AnyTimes()
	m.EXPECT().SetMaxRepetitions(gomock.Any()).AnyTimes()
	m.EXPECT().SetMaxOids(gomock.Any()).AnyTimes()
	m.EXPECT().SetLogger(gomock.Any()).AnyTimes()
	m.EXPECT().SetTimeout(gomock.Any()).AnyTimes()
	m.EXPECT().SetCommunity(gomock.Any()).AnyTimes()
	m.EXPECT().SetVersion(gomock.Any()).AnyTimes()
	m.EXPECT().SetSecurityModel(gomock.Any()).AnyTimes()
	m.EXPECT().SetMsgFlags(gomock.Any()).AnyTimes()
	m.EXPECT().SetSecurityParameters(gomock.Any()).AnyTimes()
	m.EXPECT().SetContextName(gomock.Any()).AnyTimes()
	m.EXPECT().MaxRepetitions().Return(uint32(25)).AnyTimes()
}

func setMockClientSysInfoExpect(m *snmpmock.MockHandler) {
	m.EXPECT().WalkAll(snmputils.RootOidMibSystem).Return([]gosnmp.SnmpPDU{
		{Name: snmputils.OidSysDescr, Value: []uint8("mock sysDescr"), Type: gosnmp.OctetString},
		{Name: snmputils.OidSysObject, Value: ".1.3.6.1.4.1.14988.1", Type: gosnmp.ObjectIdentifier},
		{Name: snmputils.OidSysContact, Value: []uint8("mock sysContact"), Type: gosnmp.OctetString},
		{Name: snmputils.OidSysName, Value: []uint8("mock sysName"), Type: gosnmp.OctetString},
		{Name: snmputils.OidSysLocation, Value: []uint8("mock sysLocation"), Type: gosnmp.OctetString},
	}, nil).MinTimes(1)
}
