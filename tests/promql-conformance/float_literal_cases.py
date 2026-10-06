"""Independent float parser inputs; no candidate output is used.

The original inputs come from golang/go@862c888e612ac346c7c4d99c9392bdfd265f33b0,
src/internal/strconv/atof_test.go. Seeded additions cover the public float grammar.
"""
import base64
import math
import random
import struct


def generate(original):
    requests = list(original)
    counts = {"original_go": len(requests)}
    seen = {r["id"] for r in requests}

    def add(kind, value):
        raw = value.encode() if isinstance(value, str) else value
        ident = f"{kind}-{counts.get(kind, 0):05d}"
        if ident in seen:
            raise ValueError("duplicate generated float identity")
        seen.add(ident)
        counts[kind] = counts.get(kind, 0) + 1
        requests.append({"id": ident, "input": base64.b64encode(raw).decode(), "origin": kind})

    for text in ["010", "-0", "+0", "0x10", "0x1p0", "0x_1p0", "0X_A.BP+1_2", "-NaN", "+NaN",
                 "infi", "infinite", "nan(1)", "1e999999x", "0x1p999999x", "0x1p+_999999", " 1", "1 ",
                 "\t1", "1\n", "\x001", "1\x00", "1_", "_1", "0x_", "0x_.1p0", "0x1_p0", "0x1p0_",
                 "0x1p0__1", "0x1p0_1", "0x1p0a", "0x1e1", "0b1", "0o1", "1.e1", ".1e1", ".", "+", "-"]:
        add("direct", text)
    for special in ["nan", "NaN", "NAN", "nAn", "inf", "INF", "infinity", "Infinity", "INFINITY"]:
        for sign in ["", "+", "-"]:
            for suffix in ["", "x", "\x00", " "]:
                add("special", sign + special + suffix)
    for byte in range(256):
        for prefix, suffix in [(b"", b""), (b"1", b""), (b"0x1", b"p0"), (b"0x1p", b"0")]:
            add("byte", prefix + bytes([byte]) + suffix)
    exponents = sorted(set(range(-1100, -1008)) | set(range(968, 1038)) | {-999999999, -4000, -2000, -1000, -53, -1, 0, 1, 52, 100, 500, 999999999})
    for exponent in exponents:
        for mantissa in ["0", "1", "2", "3", "0.1", ".1", "1.fffffffffffff", "1.fffffffffffff7fff",
                         "1.fffffffffffff8", "1.00000000000008", "1.000000000000081", "1.00000000000018",
                         "0.00000000000008", "00000.00000001", "0.ffffffffffffe7", "000_01.f_fff"]:
            for sign in ["", "-"]:
                add("hex-boundary", f"{sign}0x{mantissa}p{exponent}")
    rng = random.Random(602815)
    for _ in range(1536):
        value = struct.unpack(">d", rng.getrandbits(64).to_bytes(8, "big"))[0]
        if math.isfinite(value):
            add("hex-roundtrip", value.hex())
            add("decimal-roundtrip", repr(value))
    for _ in range(1024):
        length = rng.randint(1, 192)
        digits = "".join(rng.choice("0123456789abcdef") for _ in range(length))
        point = rng.randint(0, length)
        sign = rng.choice(["", "+", "-"])
        exponent = rng.randint(-4000, 4000)
        add("hex-long", f"{sign}0x{digits[:point]}.{digits[point:]}p{exponent}")
        digits = "".join(rng.choice("0123456789") for _ in range(length))
        add("decimal-long", f"{sign}{digits[:point]}.{digits[point:]}e{exponent}")
    for text in ["0x1.00000000000008", "0x1.fffffffffffff7fff", "0x0.00000000000008", "-0x1.00000000000008"]:
        for length in [15, 64, 1024, 10000]:
            for tail in ["", "1"]:
                add("hex-sticky", text + "0" * length + tail + "p0")
    add("long-zero", "-0x" + "0" * 400000 + "p999999999999999999999999999999999")
    add("long-zero", "-" + "0" * 400000 + "e999999999999999999999999999999999")

    # Original Go freezes exponent prefixes and routes ambiguous 19-digit values to an
    # independent 800-digit fallback. Preserve the original panel before these additions.
    add("decimal-exponent-cancel", "1" + "0" * 10000 + "e-100000")
    for length in [799, 800, 801, 9998, 9999, 10000, 10001, 10002, 99990, 99998, 99999, 100000]:
        for exponent in [length, length + 1, 10000, 10001, 100000, 999990]:
            for sign in ["", "-"]:
                add("decimal-zero-tail-cancel", sign + "1" + "0" * length + "e-" + str(exponent))
    for length in [799, 800, 801, 9999, 10000, 10001]:
        for lead in ["1000000000000000111", "1000000000000000333", "1999999999999999888", "1234567890123456789"]:
            for tail in ["0", "1", "5", "9"]:
                add("decimal-sticky-cancel", lead + "0" * (length - len(lead) + 1) + tail + "e-100000")
    for zeros in [9999, 10000, 10001, 10002, 99999]:
        for exponent in [10000, 100000, 100009, 100010, 999990]:
            for sign in ["", "-"]:
                add("decimal-fraction-cancel", sign + "0." + "0" * zeros + "1e" + str(exponent))
    add("decimal-fallback-cancel", "10000000000000001111" + "0" * 781 + "e-800")
    add("decimal-fallback-cancel", "10000000000000001111" + "0" * 10000 + "e-10019")
    for length in [18, 19, 20, 798, 799, 800, 801, 802, 9999, 10000, 10001]:
        for lead in ["1000000000000000111", "1000000000000000112", "1000000000000000333", "1999999999999999888"]:
            for tail in ["0", "1", "5", "9"]:
                if length >= len(lead):
                    text = lead + "0" * (length - len(lead)) + tail
                    for sign in ["", "-"]:
                        add("decimal-fallback-no-dot", sign + text + "e-" + str(len(text) - 1))
                        add("decimal-fallback-dot", sign + text[0] + "." + text[1:])
    return requests, counts
