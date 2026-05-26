package protocol

import "bytes"

type LookupLabelView struct {
	Key   CStringView
	Value CStringView
}

type CgroupsLookupRequestView struct {
	ItemCount uint32
	payload   []byte
}

type AppsLookupRequestView struct {
	ItemCount uint32
	payload   []byte
}

type CgroupsLookupResponseView struct {
	LayoutVersion uint16
	Flags         uint16
	ItemCount     uint32
	Generation    uint64
	payload       []byte
}

type CgroupsLookupItemView struct {
	Status           uint16
	Orchestrator     uint16
	Path             CStringView
	Name             CStringView
	LabelCount       uint16
	item             []byte
	labelTableOffset int
}

type AppsLookupResponseView struct {
	LayoutVersion uint16
	Flags         uint16
	ItemCount     uint32
	Generation    uint64
	payload       []byte
}

type AppsLookupItemView struct {
	Status           uint16
	Orchestrator     uint16
	CgroupStatus     uint16
	Pid              uint32
	Ppid             uint32
	Uid              uint32
	Starttime        uint64
	Comm             CStringView
	CgroupPath       CStringView
	CgroupName       CStringView
	LabelCount       uint16
	item             []byte
	labelTableOffset int
}

func invalidSourceString(data []byte, requireNonEmpty bool) bool {
	return (requireNonEmpty && len(data) == 0) || bytes.IndexByte(data, 0) >= 0
}

func maxIntValue() int {
	return int(^uint(0) >> 1)
}

func checkedU32Int(value int) (uint32, bool) {
	if value < 0 || uint64(value) > uint64(^uint32(0)) {
		return 0, false
	}
	return uint32(value), true
}

func checkedU16Int(value int) (uint16, bool) {
	if value < 0 || value > int(^uint16(0)) {
		return 0, false
	}
	return uint16(value), true
}

func checkedWireU32Int(buf []byte, off int) (int, error) {
	value, ok := checkedInt(uint64(ne.Uint32(buf[off : off+4])))
	if !ok {
		return 0, ErrOutOfBounds
	}
	return value, nil
}

func lookupDirEntry(buf []byte, base int) (int, int, error) {
	off, err := checkedWireU32Int(buf, base)
	if err != nil {
		return 0, 0, err
	}
	length, err := checkedWireU32Int(buf, base+4)
	if err != nil {
		return 0, 0, err
	}
	return off, length, nil
}

func lookupPayloadSlice(buf []byte, start int, off int, length int) ([]byte, error) {
	abs, ok := checkedAddInt(start, off)
	if !ok {
		return nil, ErrOutOfBounds
	}
	end, ok := checkedAddInt(abs, length)
	if !ok || end > len(buf) {
		return nil, ErrOutOfBounds
	}
	return buf[abs:end], nil
}

func lookupBuilderDataOffset(hdrSize int, maxItems uint32) (int, bool) {
	dirSize, ok := checkedInt(uint64(maxItems) * uint64(LookupDirEntrySize))
	if !ok {
		return 0, false
	}
	return checkedAddInt(hdrSize, dirSize)
}

func lookupDirOffset(hdrSize int, index uint32) (int, bool) {
	dirOff, ok := checkedInt(uint64(index) * uint64(LookupDirEntrySize))
	if !ok {
		return 0, false
	}
	return checkedAddInt(hdrSize, dirOff)
}

func checkedInt(value uint64) (int, bool) {
	maxInt := uint64(maxIntValue())
	if value > maxInt {
		return 0, false
	}
	return int(value), true
}

func checkedAddInt(a, b int) (int, bool) {
	if a < 0 || b < 0 {
		return 0, false
	}
	maxInt := maxIntValue()
	if a > maxInt-b {
		return 0, false
	}
	return a + b, true
}

func checkedMulInt(a, b int) (int, bool) {
	if a < 0 || b < 0 {
		return 0, false
	}
	maxInt := maxIntValue()
	if a != 0 && b > maxInt/a {
		return 0, false
	}
	return a * b, true
}

func checkedAlign8(v int) (int, bool) {
	if v < 0 {
		return 0, false
	}
	maxInt := maxIntValue()
	if v > maxInt-7 {
		return 0, false
	}
	return Align8(v), true
}

func validateLookupDir(buf []byte, dirStart int, itemCount uint32, packedAreaLen int, minLen int, exactLen int) error {
	dirSize64 := uint64(itemCount) * uint64(LookupDirEntrySize)
	dirEnd64 := uint64(dirStart) + dirSize64
	dirEnd, ok := checkedInt(dirEnd64)
	if !ok {
		return ErrBadItemCount
	}
	if dirEnd > len(buf) {
		return ErrTruncated
	}

	prevEnd := uint64(0)
	for i := uint32(0); i < itemCount; i++ {
		base := dirStart + int(i)*LookupDirEntrySize
		off := ne.Uint32(buf[base : base+4])
		length := ne.Uint32(buf[base+4 : base+8])
		if off%uint32(Alignment) != 0 {
			return ErrBadAlignment
		}
		if exactLen >= 0 {
			if length != uint32(exactLen) {
				return ErrBadLayout
			}
		} else if length < uint32(minLen) {
			return ErrBadLayout
		}
		end := uint64(off) + uint64(length)
		if end > uint64(packedAreaLen) {
			return ErrOutOfBounds
		}
		if i > 0 && uint64(off) < prevEnd {
			return ErrBadLayout
		}
		prevEnd = end
	}
	return nil
}

func lookupString(item []byte, hdrSize int, off int, length int) (CStringView, int, error) {
	if off < hdrSize {
		return CStringView{}, 0, ErrOutOfBounds
	}
	nul64 := uint64(off) + uint64(length)
	nul, ok := checkedInt(nul64)
	if !ok || nul >= len(item) {
		return CStringView{}, 0, ErrOutOfBounds
	}
	if item[nul] != 0 {
		return CStringView{}, 0, ErrMissingNul
	}
	if bytes.Contains(item[off:nul], []byte{0}) {
		return CStringView{}, 0, ErrBadLayout
	}
	return NewCStringView(item[off:nul+1], uint32(length)), nul + 1, nil
}

func overlap(aStart, aEnd, bStart, bEnd int) bool {
	return aStart < bEnd && bStart < aEnd
}

func validateLabels(item []byte, hdrSize int, labelCount uint16, fixedEnd int) (int, error) {
	if labelCount == 0 {
		if fixedEnd != len(item) {
			return 0, ErrBadLayout
		}
		return fixedEnd, nil
	}

	tableStart, ok := checkedAlign8(fixedEnd)
	if !ok {
		return 0, ErrOutOfBounds
	}
	if tableStart > len(item) {
		return 0, ErrOutOfBounds
	}
	for _, b := range item[fixedEnd:tableStart] {
		if b != 0 {
			return 0, ErrBadLayout
		}
	}

	tableBytes64 := uint64(labelCount) * uint64(LookupLabelEntrySize)
	expected64 := uint64(tableStart) + tableBytes64
	expected, ok := checkedInt(expected64)
	if !ok || expected > len(item) {
		return 0, ErrOutOfBounds
	}

	for i := uint16(0); i < labelCount; i++ {
		entryRel, ok := checkedInt(uint64(i) * uint64(LookupLabelEntrySize))
		if !ok {
			return 0, ErrOutOfBounds
		}
		base, ok := checkedAddInt(tableStart, entryRel)
		if !ok {
			return 0, ErrOutOfBounds
		}
		keyOff, err := checkedWireU32Int(item, base)
		if err != nil {
			return 0, err
		}
		keyLen, err := checkedWireU32Int(item, base+4)
		if err != nil {
			return 0, err
		}
		valueOff, err := checkedWireU32Int(item, base+8)
		if err != nil {
			return 0, err
		}
		valueLen, err := checkedWireU32Int(item, base+12)
		if err != nil {
			return 0, err
		}
		if keyLen == 0 || keyOff != expected {
			return 0, ErrBadLayout
		}
		_, keyEnd, err := lookupString(item, hdrSize, keyOff, keyLen)
		if err != nil {
			return 0, err
		}
		expected = keyEnd
		if valueOff != expected {
			return 0, ErrBadLayout
		}
		_, valueEnd, err := lookupString(item, hdrSize, valueOff, valueLen)
		if err != nil {
			return 0, err
		}
		expected = valueEnd
	}
	if expected != len(item) {
		return 0, ErrBadLayout
	}
	return tableStart, nil
}

func lookupLabelAt(item []byte, hdrSize int, labelCount uint16, tableOffset int, index uint32) (LookupLabelView, error) {
	if index >= uint32(labelCount) {
		return LookupLabelView{}, ErrOutOfBounds
	}
	entryRel, ok := checkedInt(uint64(index) * uint64(LookupLabelEntrySize))
	if !ok {
		return LookupLabelView{}, ErrOutOfBounds
	}
	base, ok := checkedAddInt(tableOffset, entryRel)
	if !ok {
		return LookupLabelView{}, ErrOutOfBounds
	}
	keyOff, err := checkedWireU32Int(item, base)
	if err != nil {
		return LookupLabelView{}, err
	}
	keyLen, err := checkedWireU32Int(item, base+4)
	if err != nil {
		return LookupLabelView{}, err
	}
	valueOff, err := checkedWireU32Int(item, base+8)
	if err != nil {
		return LookupLabelView{}, err
	}
	valueLen, err := checkedWireU32Int(item, base+12)
	if err != nil {
		return LookupLabelView{}, err
	}
	key, _, err := lookupString(item, hdrSize, keyOff, keyLen)
	if err != nil {
		return LookupLabelView{}, err
	}
	value, _, err := lookupString(item, hdrSize, valueOff, valueLen)
	if err != nil {
		return LookupLabelView{}, err
	}
	return LookupLabelView{Key: key, Value: value}, nil
}

func EncodeCgroupsLookupRequest(paths [][]byte, buf []byte) (int, error) {
	count := len(paths)
	if uint64(count) > uint64(^uint32(0)) {
		return 0, ErrOverflow
	}
	dirSize, ok := checkedMulInt(count, LookupDirEntrySize)
	if !ok {
		return 0, ErrOverflow
	}
	packedStart, ok := checkedAddInt(CgroupsLookupReqHdr, dirSize)
	if !ok {
		return 0, ErrOverflow
	}
	if len(buf) < packedStart {
		return 0, ErrOverflow
	}
	data := packedStart
	for i, path := range paths {
		if invalidSourceString(path, true) {
			return 0, ErrBadLayout
		}
		aligned, ok := checkedAlign8(data)
		if !ok {
			return 0, ErrOverflow
		}
		keyLen, ok := checkedAddInt(len(path), 1)
		if !ok {
			return 0, ErrOverflow
		}
		end, ok := checkedAddInt(aligned, keyLen)
		if !ok {
			return 0, ErrOverflow
		}
		if end > len(buf) {
			return 0, ErrOverflow
		}
		clear(buf[data:aligned])
		offset32, ok := checkedU32Int(aligned - packedStart)
		if !ok {
			return 0, ErrOverflow
		}
		keyLen32, ok := checkedU32Int(keyLen)
		if !ok {
			return 0, ErrOverflow
		}
		base := CgroupsLookupReqHdr + i*LookupDirEntrySize
		ne.PutUint32(buf[base:base+4], offset32)
		ne.PutUint32(buf[base+4:base+8], keyLen32)
		copy(buf[aligned:], path)
		buf[aligned+len(path)] = 0
		data = end
	}
	ne.PutUint16(buf[0:2], 1)
	ne.PutUint16(buf[2:4], 0)
	ne.PutUint32(buf[4:8], uint32(count))
	ne.PutUint32(buf[8:12], 0)
	ne.PutUint32(buf[12:16], 0)
	return data, nil
}

func DecodeCgroupsLookupRequest(buf []byte) (*CgroupsLookupRequestView, error) {
	if len(buf) < CgroupsLookupReqHdr {
		return nil, ErrTruncated
	}
	if ne.Uint16(buf[0:2]) != 1 || ne.Uint16(buf[2:4]) != 0 ||
		ne.Uint32(buf[8:12]) != 0 || ne.Uint32(buf[12:16]) != 0 {
		return nil, ErrBadLayout
	}
	itemCount := ne.Uint32(buf[4:8])
	dirSize64 := uint64(itemCount) * uint64(LookupDirEntrySize)
	dirEnd, ok := checkedInt(uint64(CgroupsLookupReqHdr) + dirSize64)
	if !ok {
		return nil, ErrBadItemCount
	}
	if dirEnd > len(buf) {
		return nil, ErrTruncated
	}
	if err := validateLookupDir(buf, CgroupsLookupReqHdr, itemCount, len(buf)-dirEnd, 2, -1); err != nil {
		return nil, err
	}
	for i := uint32(0); i < itemCount; i++ {
		base := CgroupsLookupReqHdr + int(i)*LookupDirEntrySize
		off, length, err := lookupDirEntry(buf, base)
		if err != nil {
			return nil, err
		}
		key, err := lookupPayloadSlice(buf, dirEnd, off, length)
		if err != nil {
			return nil, err
		}
		if key[length-1] != 0 {
			return nil, ErrMissingNul
		}
		if bytes.Contains(key[:length-1], []byte{0}) {
			return nil, ErrBadLayout
		}
	}
	return &CgroupsLookupRequestView{ItemCount: itemCount, payload: buf}, nil
}

func (v *CgroupsLookupRequestView) Item(index uint32) (CStringView, error) {
	if index >= v.ItemCount {
		return CStringView{}, ErrOutOfBounds
	}
	dirEnd, ok := lookupBuilderDataOffset(CgroupsLookupReqHdr, v.ItemCount)
	if !ok {
		return CStringView{}, ErrOverflow
	}
	base, ok := lookupDirOffset(CgroupsLookupReqHdr, index)
	if !ok {
		return CStringView{}, ErrOverflow
	}
	off, length, err := lookupDirEntry(v.payload, base)
	if err != nil {
		return CStringView{}, err
	}
	item, err := lookupPayloadSlice(v.payload, dirEnd, off, length)
	if err != nil {
		return CStringView{}, err
	}
	return NewCStringView(item, uint32(length-1)), nil
}

func EncodeAppsLookupRequest(pids []uint32, buf []byte) (int, error) {
	count := len(pids)
	if uint64(count) > uint64(^uint32(0)) {
		return 0, ErrOverflow
	}
	dirSize, ok := checkedMulInt(count, LookupDirEntrySize)
	if !ok {
		return 0, ErrOverflow
	}
	keySize, ok := checkedMulInt(count, AppsLookupKeySize)
	if !ok {
		return 0, ErrOverflow
	}
	packedStart, ok := checkedAddInt(AppsLookupReqHdr, dirSize)
	if !ok {
		return 0, ErrOverflow
	}
	total, ok := checkedAddInt(packedStart, keySize)
	if !ok {
		return 0, ErrOverflow
	}
	if total > len(buf) {
		return 0, ErrOverflow
	}
	for i, pid := range pids {
		offset32, ok := checkedU32Int(i * AppsLookupKeySize)
		if !ok {
			return 0, ErrOverflow
		}
		base := AppsLookupReqHdr + i*LookupDirEntrySize
		ne.PutUint32(buf[base:base+4], offset32)
		ne.PutUint32(buf[base+4:base+8], AppsLookupKeySize)
		key := packedStart + i*AppsLookupKeySize
		ne.PutUint32(buf[key:key+4], pid)
		ne.PutUint32(buf[key+4:key+8], 0)
	}
	ne.PutUint16(buf[0:2], 1)
	ne.PutUint16(buf[2:4], 0)
	ne.PutUint32(buf[4:8], uint32(count))
	ne.PutUint32(buf[8:12], 0)
	ne.PutUint32(buf[12:16], 0)
	return total, nil
}

func DecodeAppsLookupRequest(buf []byte) (*AppsLookupRequestView, error) {
	if len(buf) < AppsLookupReqHdr {
		return nil, ErrTruncated
	}
	if ne.Uint16(buf[0:2]) != 1 || ne.Uint16(buf[2:4]) != 0 ||
		ne.Uint32(buf[8:12]) != 0 || ne.Uint32(buf[12:16]) != 0 {
		return nil, ErrBadLayout
	}
	itemCount := ne.Uint32(buf[4:8])
	dirSize64 := uint64(itemCount) * uint64(LookupDirEntrySize)
	dirEnd, ok := checkedInt(uint64(AppsLookupReqHdr) + dirSize64)
	if !ok {
		return nil, ErrBadItemCount
	}
	if dirEnd > len(buf) {
		return nil, ErrTruncated
	}
	if err := validateLookupDir(buf, AppsLookupReqHdr, itemCount, len(buf)-dirEnd, 0, AppsLookupKeySize); err != nil {
		return nil, err
	}
	for i := uint32(0); i < itemCount; i++ {
		base := AppsLookupReqHdr + int(i)*LookupDirEntrySize
		off, _, err := lookupDirEntry(buf, base)
		if err != nil {
			return nil, err
		}
		key, err := lookupPayloadSlice(buf, dirEnd, off, AppsLookupKeySize)
		if err != nil {
			return nil, err
		}
		if ne.Uint32(key[4:8]) != 0 {
			return nil, ErrBadLayout
		}
	}
	return &AppsLookupRequestView{ItemCount: itemCount, payload: buf}, nil
}

func (v *AppsLookupRequestView) Item(index uint32) (uint32, error) {
	if index >= v.ItemCount {
		return 0, ErrOutOfBounds
	}
	dirEnd, ok := lookupBuilderDataOffset(AppsLookupReqHdr, v.ItemCount)
	if !ok {
		return 0, ErrOverflow
	}
	base, ok := lookupDirOffset(AppsLookupReqHdr, index)
	if !ok {
		return 0, ErrOverflow
	}
	off, _, err := lookupDirEntry(v.payload, base)
	if err != nil {
		return 0, err
	}
	key, err := lookupPayloadSlice(v.payload, dirEnd, off, AppsLookupKeySize)
	if err != nil {
		return 0, err
	}
	return ne.Uint32(key[0:4]), nil
}

func DecodeCgroupsLookupResponse(buf []byte) (*CgroupsLookupResponseView, error) {
	if len(buf) < CgroupsLookupRespHdr {
		return nil, ErrTruncated
	}
	if ne.Uint16(buf[0:2]) != 1 || ne.Uint16(buf[2:4]) != 0 {
		return nil, ErrBadLayout
	}
	itemCount := ne.Uint32(buf[4:8])
	dirEnd, ok := checkedInt(uint64(CgroupsLookupRespHdr) + uint64(itemCount)*uint64(LookupDirEntrySize))
	if !ok {
		return nil, ErrBadItemCount
	}
	if dirEnd > len(buf) {
		return nil, ErrTruncated
	}
	if err := validateLookupDir(buf, CgroupsLookupRespHdr, itemCount, len(buf)-dirEnd, CgroupsLookupItemHdr, -1); err != nil {
		return nil, err
	}
	for i := uint32(0); i < itemCount; i++ {
		base := CgroupsLookupRespHdr + int(i)*LookupDirEntrySize
		off, length, err := lookupDirEntry(buf, base)
		if err != nil {
			return nil, err
		}
		item, err := lookupPayloadSlice(buf, dirEnd, off, length)
		if err != nil {
			return nil, err
		}
		if _, err := decodeCgroupsLookupItem(item); err != nil {
			return nil, err
		}
	}
	return &CgroupsLookupResponseView{
		LayoutVersion: 1,
		Flags:         0,
		ItemCount:     itemCount,
		Generation:    ne.Uint64(buf[8:16]),
		payload:       buf,
	}, nil
}

func (v *CgroupsLookupResponseView) Item(index uint32) (*CgroupsLookupItemView, error) {
	if index >= v.ItemCount {
		return nil, ErrOutOfBounds
	}
	dirEnd, ok := lookupBuilderDataOffset(CgroupsLookupRespHdr, v.ItemCount)
	if !ok {
		return nil, ErrOverflow
	}
	base, ok := lookupDirOffset(CgroupsLookupRespHdr, index)
	if !ok {
		return nil, ErrOverflow
	}
	off, length, err := lookupDirEntry(v.payload, base)
	if err != nil {
		return nil, err
	}
	item, err := lookupPayloadSlice(v.payload, dirEnd, off, length)
	if err != nil {
		return nil, err
	}
	return decodeCgroupsLookupItem(item)
}

func decodeCgroupsLookupItem(item []byte) (*CgroupsLookupItemView, error) {
	if len(item) < CgroupsLookupItemHdr {
		return nil, ErrTruncated
	}
	status := ne.Uint16(item[2:4])
	orchestrator := ne.Uint16(item[4:6])
	pathOff, err := checkedWireU32Int(item, 8)
	if err != nil {
		return nil, err
	}
	pathLen, err := checkedWireU32Int(item, 12)
	if err != nil {
		return nil, err
	}
	nameOff, err := checkedWireU32Int(item, 16)
	if err != nil {
		return nil, err
	}
	nameLen, err := checkedWireU32Int(item, 20)
	if err != nil {
		return nil, err
	}
	labelCount := ne.Uint16(item[24:26])
	if ne.Uint16(item[0:2]) != 1 || ne.Uint16(item[6:8]) != 0 || ne.Uint16(item[26:28]) != 0 {
		return nil, ErrBadLayout
	}
	if status != CgroupLookupKnown && status != CgroupLookupUnknownRetryLater && status != CgroupLookupUnknownPermanent {
		return nil, ErrBadLayout
	}
	if pathLen == 0 {
		return nil, ErrBadLayout
	}
	if status != CgroupLookupKnown && (orchestrator != 0 || nameLen != 0 || labelCount != 0) {
		return nil, ErrBadLayout
	}
	path, pathEnd, err := lookupString(item, CgroupsLookupItemHdr, pathOff, pathLen)
	if err != nil {
		return nil, err
	}
	name, nameEnd, err := lookupString(item, CgroupsLookupItemHdr, nameOff, nameLen)
	if err != nil {
		return nil, err
	}
	if overlap(pathOff, pathEnd, nameOff, nameEnd) {
		return nil, ErrBadLayout
	}
	table, err := validateLabels(item, CgroupsLookupItemHdr, labelCount, max(pathEnd, nameEnd))
	if err != nil {
		return nil, err
	}
	return &CgroupsLookupItemView{
		Status:           status,
		Orchestrator:     orchestrator,
		Path:             path,
		Name:             name,
		LabelCount:       labelCount,
		item:             item,
		labelTableOffset: table,
	}, nil
}

func (v *CgroupsLookupItemView) Label(index uint32) (LookupLabelView, error) {
	return lookupLabelAt(v.item, CgroupsLookupItemHdr, v.LabelCount, v.labelTableOffset, index)
}

type CgroupsLookupBuilder struct {
	buf        []byte
	generation uint64
	itemCount  uint32
	maxItems   uint32
	dataOffset int
	err        error
}

func NewCgroupsLookupBuilder(buf []byte, maxItems uint32, generation uint64) *CgroupsLookupBuilder {
	minRequired, ok := lookupBuilderDataOffset(CgroupsLookupRespHdr, maxItems)
	if !ok {
		panic("CgroupsLookupBuilder buffer too small")
	}
	if len(buf) < minRequired {
		panic("CgroupsLookupBuilder buffer too small")
	}
	return &CgroupsLookupBuilder{buf: buf, generation: generation, maxItems: maxItems, dataOffset: minRequired}
}

func (b *CgroupsLookupBuilder) SetGeneration(generation uint64) {
	b.generation = generation
}

func (b *CgroupsLookupBuilder) Add(status, orchestrator uint16, path, name []byte, labels []struct{ Key, Value []byte }) error {
	if b.itemCount >= b.maxItems {
		b.err = ErrOverflow
		return ErrOverflow
	}
	if status != CgroupLookupKnown && status != CgroupLookupUnknownRetryLater && status != CgroupLookupUnknownPermanent {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	if invalidSourceString(path, true) || invalidSourceString(name, false) {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	if status != CgroupLookupKnown && (orchestrator != 0 || len(name) != 0 || len(labels) != 0) {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	labelCount, ok := checkedU16Int(len(labels))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemStart, ok := checkedAlign8(b.dataOffset)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	pathOff := CgroupsLookupItemHdr
	nameOff, ok := checkedAddInt(pathOff, len(path))
	if ok {
		nameOff, ok = checkedAddInt(nameOff, 1)
	}
	fixedEnd, ok := checkedAddInt(nameOff, len(name))
	if ok {
		fixedEnd, ok = checkedAddInt(fixedEnd, 1)
	}
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	tableStart, tableBytes, itemSize, err := labelLayoutGo(fixedEnd, labels)
	if err != nil {
		b.err = err
		return err
	}
	itemEnd, ok := checkedAddInt(itemStart, itemSize)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	if itemEnd > len(b.buf) {
		b.err = ErrOverflow
		return ErrOverflow
	}
	pathOff32, ok := checkedU32Int(pathOff)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	pathLen32, ok := checkedU32Int(len(path))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	nameOff32, ok := checkedU32Int(nameOff)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	nameLen32, ok := checkedU32Int(len(name))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemStart32, ok := checkedU32Int(itemStart)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemSize32, ok := checkedU32Int(itemSize)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	clear(b.buf[b.dataOffset:itemStart])
	item := b.buf[itemStart:itemEnd]
	ne.PutUint16(item[0:2], 1)
	ne.PutUint16(item[2:4], status)
	ne.PutUint16(item[4:6], orchestrator)
	ne.PutUint16(item[6:8], 0)
	ne.PutUint32(item[8:12], pathOff32)
	ne.PutUint32(item[12:16], pathLen32)
	ne.PutUint32(item[16:20], nameOff32)
	ne.PutUint32(item[20:24], nameLen32)
	ne.PutUint16(item[24:26], labelCount)
	ne.PutUint16(item[26:28], 0)
	copy(item[pathOff:], path)
	item[pathOff+len(path)] = 0
	copy(item[nameOff:], name)
	item[nameOff+len(name)] = 0
	if len(labels) > 0 {
		clear(item[fixedEnd:tableStart])
		next, ok := checkedAddInt(tableStart, tableBytes)
		if !ok {
			return ErrOverflow
		}
		for i, label := range labels {
			if invalidSourceString(label.Key, true) || invalidSourceString(label.Value, false) {
				b.err = ErrBadLayout
				return ErrBadLayout
			}
			keyOff32, ok := checkedU32Int(next)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			keyLen32, ok := checkedU32Int(len(label.Key))
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueOff, ok := checkedAddInt(next, len(label.Key))
			if ok {
				valueOff, ok = checkedAddInt(valueOff, 1)
			}
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueOff32, ok := checkedU32Int(valueOff)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueLen32, ok := checkedU32Int(len(label.Value))
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			entryRel, ok := checkedMulInt(i, LookupLabelEntrySize)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			entry, ok := checkedAddInt(tableStart, entryRel)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			ne.PutUint32(item[entry:entry+4], keyOff32)
			ne.PutUint32(item[entry+4:entry+8], keyLen32)
			ne.PutUint32(item[entry+8:entry+12], valueOff32)
			ne.PutUint32(item[entry+12:entry+16], valueLen32)
			copy(item[next:], label.Key)
			item[next+len(label.Key)] = 0
			next = valueOff
			copy(item[next:], label.Value)
			item[next+len(label.Value)] = 0
			next, ok = checkedAddInt(next, len(label.Value))
			if ok {
				next, ok = checkedAddInt(next, 1)
			}
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
		}
		itemSize = next
	}
	dir, ok := lookupDirOffset(CgroupsLookupRespHdr, b.itemCount)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	ne.PutUint32(b.buf[dir:dir+4], itemStart32)
	ne.PutUint32(b.buf[dir+4:dir+8], itemSize32)
	b.dataOffset = itemStart + itemSize
	b.itemCount++
	return nil
}

func (b *CgroupsLookupBuilder) Finish() int {
	return finishLookupResponse(b.buf, CgroupsLookupRespHdr, b.itemCount, b.dataOffset, b.generation)
}

func (b *CgroupsLookupBuilder) Error() error {
	return b.err
}

func (b *CgroupsLookupBuilder) ItemCount() uint32 {
	return b.itemCount
}

func labelLayoutGo(fixedEnd int, labels []struct{ Key, Value []byte }) (int, int, int, error) {
	if len(labels) == 0 {
		return fixedEnd, 0, fixedEnd, nil
	}
	tableStart, ok := checkedAlign8(fixedEnd)
	if !ok {
		return 0, 0, 0, ErrOverflow
	}
	tableBytes, ok := checkedMulInt(len(labels), LookupLabelEntrySize)
	if !ok {
		return 0, 0, 0, ErrOverflow
	}
	itemSize, ok := checkedAddInt(tableStart, tableBytes)
	if !ok {
		return 0, 0, 0, ErrOverflow
	}
	for _, label := range labels {
		if invalidSourceString(label.Key, true) || invalidSourceString(label.Value, false) {
			return 0, 0, 0, ErrBadLayout
		}
		keySize, ok := checkedAddInt(len(label.Key), 1)
		if ok {
			valueSize, okValue := checkedAddInt(len(label.Value), 1)
			if okValue {
				keySize, ok = checkedAddInt(keySize, valueSize)
			} else {
				ok = false
			}
		}
		if ok {
			itemSize, ok = checkedAddInt(itemSize, keySize)
		}
		if !ok {
			return 0, 0, 0, ErrOverflow
		}
	}
	return tableStart, tableBytes, itemSize, nil
}

func finishLookupResponse(buf []byte, hdrSize int, itemCount uint32, dataOffset int, generation uint64) int {
	ne.PutUint16(buf[0:2], 1)
	ne.PutUint16(buf[2:4], 0)
	ne.PutUint32(buf[4:8], itemCount)
	ne.PutUint64(buf[8:16], generation)
	if itemCount == 0 {
		return hdrSize
	}
	dirSize, ok := checkedInt(uint64(itemCount) * uint64(LookupDirEntrySize))
	if !ok {
		return 0
	}
	count, ok := checkedInt(uint64(itemCount))
	if !ok {
		return 0
	}
	finalPackedStart, ok := checkedAddInt(hdrSize, dirSize)
	if !ok {
		return 0
	}
	firstItemAbs, ok := checkedInt(uint64(ne.Uint32(buf[hdrSize : hdrSize+4])))
	if !ok {
		return 0
	}
	if dataOffset < firstItemAbs {
		return 0
	}
	packedDataLen := dataOffset - firstItemAbs
	if finalPackedStart < firstItemAbs {
		copy(buf[finalPackedStart:], buf[firstItemAbs:firstItemAbs+packedDataLen])
	}
	for i := 0; i < count; i++ {
		entry := hdrSize + i*LookupDirEntrySize
		abs, ok := checkedInt(uint64(ne.Uint32(buf[entry : entry+4])))
		if !ok || abs < firstItemAbs {
			return 0
		}
		rel, ok := checkedU32Int(abs - firstItemAbs)
		if !ok {
			return 0
		}
		ne.PutUint32(buf[entry:entry+4], rel)
	}
	return finalPackedStart + packedDataLen
}

func DecodeAppsLookupResponse(buf []byte) (*AppsLookupResponseView, error) {
	if len(buf) < AppsLookupRespHdr {
		return nil, ErrTruncated
	}
	if ne.Uint16(buf[0:2]) != 1 || ne.Uint16(buf[2:4]) != 0 {
		return nil, ErrBadLayout
	}
	itemCount := ne.Uint32(buf[4:8])
	dirEnd, ok := checkedInt(uint64(AppsLookupRespHdr) + uint64(itemCount)*uint64(LookupDirEntrySize))
	if !ok {
		return nil, ErrBadItemCount
	}
	if dirEnd > len(buf) {
		return nil, ErrTruncated
	}
	if err := validateLookupDir(buf, AppsLookupRespHdr, itemCount, len(buf)-dirEnd, AppsLookupItemHdr, -1); err != nil {
		return nil, err
	}
	for i := uint32(0); i < itemCount; i++ {
		base := AppsLookupRespHdr + int(i)*LookupDirEntrySize
		off, length, err := lookupDirEntry(buf, base)
		if err != nil {
			return nil, err
		}
		item, err := lookupPayloadSlice(buf, dirEnd, off, length)
		if err != nil {
			return nil, err
		}
		if _, err := decodeAppsLookupItem(item); err != nil {
			return nil, err
		}
	}
	return &AppsLookupResponseView{
		LayoutVersion: 1,
		Flags:         0,
		ItemCount:     itemCount,
		Generation:    ne.Uint64(buf[8:16]),
		payload:       buf,
	}, nil
}

func (v *AppsLookupResponseView) Item(index uint32) (*AppsLookupItemView, error) {
	if index >= v.ItemCount {
		return nil, ErrOutOfBounds
	}
	dirEnd, ok := lookupBuilderDataOffset(AppsLookupRespHdr, v.ItemCount)
	if !ok {
		return nil, ErrOverflow
	}
	base, ok := lookupDirOffset(AppsLookupRespHdr, index)
	if !ok {
		return nil, ErrOverflow
	}
	off, length, err := lookupDirEntry(v.payload, base)
	if err != nil {
		return nil, err
	}
	item, err := lookupPayloadSlice(v.payload, dirEnd, off, length)
	if err != nil {
		return nil, err
	}
	return decodeAppsLookupItem(item)
}

func decodeAppsLookupItem(item []byte) (*AppsLookupItemView, error) {
	if len(item) < AppsLookupItemHdr {
		return nil, ErrTruncated
	}
	status := ne.Uint16(item[2:4])
	orchestrator := ne.Uint16(item[4:6])
	cgroupStatus := ne.Uint16(item[6:8])
	pid := ne.Uint32(item[8:12])
	ppid := ne.Uint32(item[12:16])
	uid := ne.Uint32(item[16:20])
	starttime := ne.Uint64(item[24:32])
	commOff, err := checkedWireU32Int(item, 32)
	if err != nil {
		return nil, err
	}
	commLen, err := checkedWireU32Int(item, 36)
	if err != nil {
		return nil, err
	}
	pathOff, err := checkedWireU32Int(item, 40)
	if err != nil {
		return nil, err
	}
	pathLen, err := checkedWireU32Int(item, 44)
	if err != nil {
		return nil, err
	}
	nameOff, err := checkedWireU32Int(item, 48)
	if err != nil {
		return nil, err
	}
	nameLen, err := checkedWireU32Int(item, 52)
	if err != nil {
		return nil, err
	}
	labelCount := ne.Uint16(item[56:58])
	if ne.Uint16(item[0:2]) != 1 || ne.Uint32(item[20:24]) != 0 || ne.Uint16(item[58:60]) != 0 {
		return nil, ErrBadLayout
	}
	if status != PidLookupKnown && status != PidLookupUnknown {
		return nil, ErrBadLayout
	}
	if cgroupStatus != AppsCgroupKnown && cgroupStatus != AppsCgroupUnknownRetryLater &&
		cgroupStatus != AppsCgroupUnknownPermanent && cgroupStatus != AppsCgroupHostRoot {
		return nil, ErrBadLayout
	}
	if commLen > 15 {
		return nil, ErrBadLayout
	}
	if status == PidLookupUnknown {
		if orchestrator != 0 || cgroupStatus != 0 || ppid != 0 || uid != NipcUIDUnset ||
			starttime != 0 || commLen != 0 || pathLen != 0 || nameLen != 0 || labelCount != 0 {
			return nil, ErrBadLayout
		}
	} else if commLen == 0 {
		return nil, ErrBadLayout
	} else {
		switch cgroupStatus {
		case AppsCgroupKnown:
			if pathLen == 0 {
				return nil, ErrBadLayout
			}
		case AppsCgroupUnknownRetryLater, AppsCgroupUnknownPermanent:
			if pathLen == 0 || orchestrator != 0 || nameLen != 0 || labelCount != 0 {
				return nil, ErrBadLayout
			}
		case AppsCgroupHostRoot:
			if orchestrator != 0 || pathLen != 0 || nameLen != 0 || labelCount != 0 {
				return nil, ErrBadLayout
			}
		}
	}
	comm, commEnd, err := lookupString(item, AppsLookupItemHdr, commOff, commLen)
	if err != nil {
		return nil, err
	}
	path, pathEnd, err := lookupString(item, AppsLookupItemHdr, pathOff, pathLen)
	if err != nil {
		return nil, err
	}
	name, nameEnd, err := lookupString(item, AppsLookupItemHdr, nameOff, nameLen)
	if err != nil {
		return nil, err
	}
	if overlap(commOff, commEnd, pathOff, pathEnd) || overlap(commOff, commEnd, nameOff, nameEnd) ||
		overlap(pathOff, pathEnd, nameOff, nameEnd) {
		return nil, ErrBadLayout
	}
	table, err := validateLabels(item, AppsLookupItemHdr, labelCount, max(commEnd, max(pathEnd, nameEnd)))
	if err != nil {
		return nil, err
	}
	return &AppsLookupItemView{
		Status:           status,
		Orchestrator:     orchestrator,
		CgroupStatus:     cgroupStatus,
		Pid:              pid,
		Ppid:             ppid,
		Uid:              uid,
		Starttime:        starttime,
		Comm:             comm,
		CgroupPath:       path,
		CgroupName:       name,
		LabelCount:       labelCount,
		item:             item,
		labelTableOffset: table,
	}, nil
}

func (v *AppsLookupItemView) Label(index uint32) (LookupLabelView, error) {
	return lookupLabelAt(v.item, AppsLookupItemHdr, v.LabelCount, v.labelTableOffset, index)
}

type AppsLookupBuilder struct {
	buf        []byte
	generation uint64
	itemCount  uint32
	maxItems   uint32
	dataOffset int
	err        error
}

func NewAppsLookupBuilder(buf []byte, maxItems uint32, generation uint64) *AppsLookupBuilder {
	minRequired, ok := lookupBuilderDataOffset(AppsLookupRespHdr, maxItems)
	if !ok {
		panic("AppsLookupBuilder buffer too small")
	}
	if len(buf) < minRequired {
		panic("AppsLookupBuilder buffer too small")
	}
	return &AppsLookupBuilder{buf: buf, generation: generation, maxItems: maxItems, dataOffset: minRequired}
}

func (b *AppsLookupBuilder) SetGeneration(generation uint64) {
	b.generation = generation
}

func (b *AppsLookupBuilder) Add(status, cgroupStatus, orchestrator uint16, pid, ppid, uid uint32, starttime uint64, comm, cgroupPath, cgroupName []byte, labels []struct{ Key, Value []byte }) error {
	if b.itemCount >= b.maxItems {
		b.err = ErrOverflow
		return ErrOverflow
	}
	if status != PidLookupKnown && status != PidLookupUnknown {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	if cgroupStatus != AppsCgroupKnown && cgroupStatus != AppsCgroupUnknownRetryLater &&
		cgroupStatus != AppsCgroupUnknownPermanent && cgroupStatus != AppsCgroupHostRoot {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	if len(comm) > 15 || invalidSourceString(comm, status == PidLookupKnown) ||
		invalidSourceString(cgroupPath, false) || invalidSourceString(cgroupName, false) {
		b.err = ErrBadLayout
		return ErrBadLayout
	}
	if status == PidLookupUnknown {
		if orchestrator != 0 || cgroupStatus != 0 || ppid != 0 || uid != NipcUIDUnset ||
			starttime != 0 || len(comm) != 0 || len(cgroupPath) != 0 || len(cgroupName) != 0 || len(labels) != 0 {
			b.err = ErrBadLayout
			return ErrBadLayout
		}
	} else {
		switch cgroupStatus {
		case AppsCgroupKnown:
			if len(cgroupPath) == 0 {
				b.err = ErrBadLayout
				return ErrBadLayout
			}
		case AppsCgroupUnknownRetryLater, AppsCgroupUnknownPermanent:
			if len(cgroupPath) == 0 || orchestrator != 0 || len(cgroupName) != 0 || len(labels) != 0 {
				b.err = ErrBadLayout
				return ErrBadLayout
			}
		case AppsCgroupHostRoot:
			if orchestrator != 0 || len(cgroupPath) != 0 || len(cgroupName) != 0 || len(labels) != 0 {
				b.err = ErrBadLayout
				return ErrBadLayout
			}
		}
	}
	labelCount, ok := checkedU16Int(len(labels))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemStart, ok := checkedAlign8(b.dataOffset)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	commOff := AppsLookupItemHdr
	pathOff, ok := checkedAddInt(commOff, len(comm))
	if ok {
		pathOff, ok = checkedAddInt(pathOff, 1)
	}
	nameOff, ok := checkedAddInt(pathOff, len(cgroupPath))
	if ok {
		nameOff, ok = checkedAddInt(nameOff, 1)
	}
	fixedEnd, ok := checkedAddInt(nameOff, len(cgroupName))
	if ok {
		fixedEnd, ok = checkedAddInt(fixedEnd, 1)
	}
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	tableStart, tableBytes, itemSize, err := labelLayoutGo(fixedEnd, labels)
	if err != nil {
		b.err = err
		return err
	}
	itemEnd, ok := checkedAddInt(itemStart, itemSize)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	if itemEnd > len(b.buf) {
		b.err = ErrOverflow
		return ErrOverflow
	}
	commOff32, ok := checkedU32Int(commOff)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	commLen32, ok := checkedU32Int(len(comm))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	pathOff32, ok := checkedU32Int(pathOff)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	pathLen32, ok := checkedU32Int(len(cgroupPath))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	nameOff32, ok := checkedU32Int(nameOff)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	nameLen32, ok := checkedU32Int(len(cgroupName))
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemStart32, ok := checkedU32Int(itemStart)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	itemSize32, ok := checkedU32Int(itemSize)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	clear(b.buf[b.dataOffset:itemStart])
	item := b.buf[itemStart:itemEnd]
	ne.PutUint16(item[0:2], 1)
	ne.PutUint16(item[2:4], status)
	ne.PutUint16(item[4:6], orchestrator)
	ne.PutUint16(item[6:8], cgroupStatus)
	ne.PutUint32(item[8:12], pid)
	ne.PutUint32(item[12:16], ppid)
	ne.PutUint32(item[16:20], uid)
	ne.PutUint32(item[20:24], 0)
	ne.PutUint64(item[24:32], starttime)
	ne.PutUint32(item[32:36], commOff32)
	ne.PutUint32(item[36:40], commLen32)
	ne.PutUint32(item[40:44], pathOff32)
	ne.PutUint32(item[44:48], pathLen32)
	ne.PutUint32(item[48:52], nameOff32)
	ne.PutUint32(item[52:56], nameLen32)
	ne.PutUint16(item[56:58], labelCount)
	ne.PutUint16(item[58:60], 0)
	copy(item[commOff:], comm)
	item[commOff+len(comm)] = 0
	copy(item[pathOff:], cgroupPath)
	item[pathOff+len(cgroupPath)] = 0
	copy(item[nameOff:], cgroupName)
	item[nameOff+len(cgroupName)] = 0
	if len(labels) > 0 {
		clear(item[fixedEnd:tableStart])
		next, ok := checkedAddInt(tableStart, tableBytes)
		if !ok {
			return ErrOverflow
		}
		for i, label := range labels {
			keyOff32, ok := checkedU32Int(next)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			keyLen32, ok := checkedU32Int(len(label.Key))
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueOff, ok := checkedAddInt(next, len(label.Key))
			if ok {
				valueOff, ok = checkedAddInt(valueOff, 1)
			}
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueOff32, ok := checkedU32Int(valueOff)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			valueLen32, ok := checkedU32Int(len(label.Value))
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			entryRel, ok := checkedMulInt(i, LookupLabelEntrySize)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			entry, ok := checkedAddInt(tableStart, entryRel)
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
			ne.PutUint32(item[entry:entry+4], keyOff32)
			ne.PutUint32(item[entry+4:entry+8], keyLen32)
			ne.PutUint32(item[entry+8:entry+12], valueOff32)
			ne.PutUint32(item[entry+12:entry+16], valueLen32)
			copy(item[next:], label.Key)
			item[next+len(label.Key)] = 0
			next = valueOff
			copy(item[next:], label.Value)
			item[next+len(label.Value)] = 0
			next, ok = checkedAddInt(next, len(label.Value))
			if ok {
				next, ok = checkedAddInt(next, 1)
			}
			if !ok {
				b.err = ErrOverflow
				return ErrOverflow
			}
		}
		itemSize = next
	}
	dir, ok := lookupDirOffset(AppsLookupRespHdr, b.itemCount)
	if !ok {
		b.err = ErrOverflow
		return ErrOverflow
	}
	ne.PutUint32(b.buf[dir:dir+4], itemStart32)
	ne.PutUint32(b.buf[dir+4:dir+8], itemSize32)
	b.dataOffset = itemStart + itemSize
	b.itemCount++
	return nil
}

func (b *AppsLookupBuilder) Finish() int {
	return finishLookupResponse(b.buf, AppsLookupRespHdr, b.itemCount, b.dataOffset, b.generation)
}

func (b *AppsLookupBuilder) Error() error {
	return b.err
}

func (b *AppsLookupBuilder) ItemCount() uint32 {
	return b.itemCount
}

func DispatchCgroupsLookup(req []byte, resp []byte, handler func(*CgroupsLookupRequestView, *CgroupsLookupBuilder) bool) (int, error) {
	request, err := DecodeCgroupsLookupRequest(req)
	if err != nil {
		return 0, err
	}
	minRequired, ok := lookupBuilderDataOffset(CgroupsLookupRespHdr, request.ItemCount)
	if !ok || len(resp) < minRequired {
		return 0, ErrOverflow
	}
	builder := NewCgroupsLookupBuilder(resp, request.ItemCount, 0)
	if !handler(request, builder) {
		if builder.Error() != nil {
			return 0, builder.Error()
		}
		return 0, ErrBadLayout
	}
	if builder.Error() != nil {
		return 0, builder.Error()
	}
	if builder.itemCount != request.ItemCount {
		return 0, ErrBadItemCount
	}
	n := builder.Finish()
	if n == 0 {
		return 0, ErrOverflow
	}
	return n, nil
}

func DispatchAppsLookup(req []byte, resp []byte, handler func(*AppsLookupRequestView, *AppsLookupBuilder) bool) (int, error) {
	request, err := DecodeAppsLookupRequest(req)
	if err != nil {
		return 0, err
	}
	minRequired, ok := lookupBuilderDataOffset(AppsLookupRespHdr, request.ItemCount)
	if !ok || len(resp) < minRequired {
		return 0, ErrOverflow
	}
	builder := NewAppsLookupBuilder(resp, request.ItemCount, 0)
	if !handler(request, builder) {
		if builder.Error() != nil {
			return 0, builder.Error()
		}
		return 0, ErrBadLayout
	}
	if builder.Error() != nil {
		return 0, builder.Error()
	}
	if builder.itemCount != request.ItemCount {
		return 0, ErrBadItemCount
	}
	n := builder.Finish()
	if n == 0 {
		return 0, ErrOverflow
	}
	return n, nil
}
