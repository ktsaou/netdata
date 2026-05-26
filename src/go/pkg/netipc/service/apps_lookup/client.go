//go:build unix

package apps_lookup

import (
	"github.com/netdata/netdata/go/plugins/pkg/netipc/protocol"
	raw "github.com/netdata/netdata/go/plugins/pkg/netipc/service/raw"
	"github.com/netdata/netdata/go/plugins/pkg/netipc/transport/posix"
)

func clientConfigToTransport(config ClientConfig) posix.ClientConfig {
	return posix.ClientConfig{
		SupportedProfiles:       config.SupportedProfiles,
		PreferredProfiles:       config.PreferredProfiles,
		MaxRequestBatchItems:    config.MaxRequestBatchItems,
		MaxResponsePayloadBytes: config.MaxResponsePayloadBytes,
		MaxResponseBatchItems:   config.MaxRequestBatchItems,
		AuthToken:               config.AuthToken,
	}
}

func serverConfigToTransport(config ServerConfig) posix.ServerConfig {
	return posix.ServerConfig{
		SupportedProfiles:       config.SupportedProfiles,
		PreferredProfiles:       config.PreferredProfiles,
		MaxRequestBatchItems:    config.MaxRequestBatchItems,
		MaxResponsePayloadBytes: config.MaxResponsePayloadBytes,
		MaxResponseBatchItems:   config.MaxRequestBatchItems,
		AuthToken:               config.AuthToken,
	}
}

type Client struct {
	inner *raw.Client
}

func NewClient(runDir, serviceName string, config ClientConfig) *Client {
	return &Client{inner: raw.NewAppsLookupClient(runDir, serviceName, clientConfigToTransport(config))}
}

func (c *Client) Refresh() bool { return c.inner.Refresh() }
func (c *Client) Ready() bool   { return c.inner.Ready() }
func (c *Client) Status() ClientStatus {
	return c.inner.Status()
}
func (c *Client) Call(pids []uint32) (*protocol.AppsLookupResponseView, error) {
	return c.inner.CallAppsLookup(pids)
}
func (c *Client) Close() { c.inner.Close() }

type Server struct {
	inner *raw.Server
}

func NewServer(runDir, serviceName string, config ServerConfig, handler Handler) *Server {
	return &Server{inner: raw.NewServer(runDir, serviceName, serverConfigToTransport(config), protocol.MethodAppsLookup, raw.AppsLookupDispatch(handler.Handle))}
}

func NewServerWithWorkers(runDir, serviceName string, config ServerConfig, handler Handler, workerCount int) *Server {
	return &Server{inner: raw.NewServerWithWorkers(runDir, serviceName, serverConfigToTransport(config), protocol.MethodAppsLookup, raw.AppsLookupDispatch(handler.Handle), workerCount)}
}

func (s *Server) Run() error { return s.inner.Run() }
func (s *Server) Stop()      { s.inner.Stop() }
