// SPDX-License-Identifier: GPL-3.0-or-later

package cato_networks

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"os"
	"path/filepath"
	"strings"

	"github.com/netdata/netdata/go/plugins/pkg/pluginconfig"
)

type eventsMarkerStore struct {
	path string
}

func newEventsMarkerStore(configuredPath, accountID string) *eventsMarkerStore {
	path := strings.TrimSpace(configuredPath)
	if path == "" {
		base := strings.TrimSpace(pluginconfig.VarLibDir())
		if base == "" {
			return nil
		}
		sum := sha256.Sum256([]byte(accountID))
		path = filepath.Join(base, "cato_networks", hex.EncodeToString(sum[:])[:16]+".events.marker")
	}
	return &eventsMarkerStore{path: path}
}

func (s *eventsMarkerStore) read() (string, error) {
	if s == nil || s.path == "" {
		return "", nil
	}
	bs, err := os.ReadFile(s.path)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(bs)), nil
}

func (s *eventsMarkerStore) write(marker string) error {
	if s == nil || s.path == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(s.path), 0o750); err != nil {
		return err
	}
	return os.WriteFile(s.path, []byte(strings.TrimSpace(marker)+"\n"), 0o600)
}
