package raw

import (
	"bytes"

	"github.com/netdata/netdata/go/plugins/pkg/netipc/protocol"
)

func checkedLookupAdd(a, b int) (int, error) {
	if a < 0 || b < 0 {
		return 0, protocol.ErrOverflow
	}
	maxInt := int(^uint(0) >> 1)
	if a > maxInt-b {
		return 0, protocol.ErrOverflow
	}
	return a + b, nil
}

func checkedLookupMul(a, b int) (int, error) {
	if a < 0 || b < 0 {
		return 0, protocol.ErrOverflow
	}
	maxInt := int(^uint(0) >> 1)
	if a != 0 && b > maxInt/a {
		return 0, protocol.ErrOverflow
	}
	return a * b, nil
}

func checkedLookupAlign8(v int) (int, error) {
	if v < 0 {
		return 0, protocol.ErrOverflow
	}
	maxInt := int(^uint(0) >> 1)
	if v > maxInt-7 {
		return 0, protocol.ErrOverflow
	}
	return protocol.Align8(v), nil
}

func cgroupsLookupRequestSize(paths [][]byte) (int, error) {
	dirSize, err := checkedLookupMul(len(paths), protocol.LookupDirEntrySize)
	if err != nil {
		return 0, err
	}
	size, err := checkedLookupAdd(protocol.CgroupsLookupReqHdr, dirSize)
	if err != nil {
		return 0, err
	}
	data := size
	for _, path := range paths {
		data, err = checkedLookupAlign8(data)
		if err != nil {
			return 0, err
		}
		data, err = checkedLookupAdd(data, len(path))
		if err != nil {
			return 0, err
		}
		data, err = checkedLookupAdd(data, 1)
		if err != nil {
			return 0, err
		}
	}
	return data, nil
}

func appsLookupRequestSize(pids []uint32) (int, error) {
	dirSize, err := checkedLookupMul(len(pids), protocol.LookupDirEntrySize)
	if err != nil {
		return 0, err
	}
	keySize, err := checkedLookupMul(len(pids), protocol.AppsLookupKeySize)
	if err != nil {
		return 0, err
	}
	size, err := checkedLookupAdd(protocol.AppsLookupReqHdr, dirSize)
	if err != nil {
		return 0, err
	}
	return checkedLookupAdd(size, keySize)
}

// CallCgroupsLookup performs a blocking typed CGROUPS_LOOKUP call.
// The returned view is valid until the next typed call on this client.
func (c *Client) CallCgroupsLookup(paths [][]byte) (*protocol.CgroupsLookupResponseView, error) {
	if err := c.validateMethod(protocol.MethodCgroupsLookup); err != nil {
		return nil, err
	}

	var result *protocol.CgroupsLookupResponseView
	err := c.callWithRetry(func() error {
		reqSize, err := cgroupsLookupRequestSize(paths)
		if err != nil {
			return err
		}
		reqBuf := ensureClientScratch(&c.requestBuf, reqSize)
		reqLen, err := protocol.EncodeCgroupsLookupRequest(paths, reqBuf)
		if err != nil {
			return err
		}

		_, payload, rerr := c.doRawCall(protocol.MethodCgroupsLookup, reqBuf[:reqLen])
		if rerr != nil {
			return rerr
		}
		view, derr := protocol.DecodeCgroupsLookupResponse(payload)
		if derr != nil {
			return derr
		}
		if view.ItemCount != uint32(len(paths)) {
			return protocol.ErrBadItemCount
		}
		for i, expected := range paths {
			item, ierr := view.Item(uint32(i))
			if ierr != nil {
				return ierr
			}
			if !bytes.Equal(item.Path.Bytes(), expected) {
				return protocol.ErrBadLayout
			}
		}
		result = view
		return nil
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

// CallAppsLookup performs a blocking typed APPS_LOOKUP call.
// The returned view is valid until the next typed call on this client.
func (c *Client) CallAppsLookup(pids []uint32) (*protocol.AppsLookupResponseView, error) {
	if err := c.validateMethod(protocol.MethodAppsLookup); err != nil {
		return nil, err
	}

	var result *protocol.AppsLookupResponseView
	err := c.callWithRetry(func() error {
		reqSize, err := appsLookupRequestSize(pids)
		if err != nil {
			return err
		}
		reqBuf := ensureClientScratch(&c.requestBuf, reqSize)
		reqLen, err := protocol.EncodeAppsLookupRequest(pids, reqBuf)
		if err != nil {
			return err
		}

		_, payload, rerr := c.doRawCall(protocol.MethodAppsLookup, reqBuf[:reqLen])
		if rerr != nil {
			return rerr
		}
		view, derr := protocol.DecodeAppsLookupResponse(payload)
		if derr != nil {
			return derr
		}
		if view.ItemCount != uint32(len(pids)) {
			return protocol.ErrBadItemCount
		}
		for i, expected := range pids {
			item, ierr := view.Item(uint32(i))
			if ierr != nil {
				return ierr
			}
			if item.Pid != expected {
				return protocol.ErrBadLayout
			}
		}
		result = view
		return nil
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}
