"""Independent binary64 formatting inputs, without candidate-derived expectations."""
import random
import struct


def generate(original):
    requests = list(original)
    counts = {"original_go": len(requests)}
    seen = {r["id"] for r in requests}

    def add(kind, bits):
        for mode in ("shortest", "two", "openmetrics"):
            ident = f"{kind}-{counts.get(kind, 0):05d}"
            if ident in seen:
                raise ValueError("duplicate float formatting identity")
            seen.add(ident)
            counts[kind] = counts.get(kind, 0) + 1
            requests.append({"id": ident, "bits": f"{bits:016x}", "mode": mode, "origin": kind})

    for bits in [0, 1 << 63, 1, (1 << 63) | 1, 0x000fffffffffffff, 0x0010000000000000,
                 0x0010000000000001, 0x3ff0000000000000, 0xbff0000000000000,
                 0x7fefffffffffffff, 0xffefffffffffffff, 0x7ff0000000000000, 0xfff0000000000000,
                 0x7ff8000000000000, 0x7ff8000000000001, 0xfff8000000000042,
                 0x7ff0000000000001, 0xfff0000000000044, 0x7fffffffffffffff, 0xffffffffffffffff]:
        add("special", bits)
    for exponent in range(2047):
        for mantissa in (0, 1, (1 << 51) - 1, (1 << 52) - 1):
            for sign in (0, 1 << 63):
                add("binary-exponent", sign | exponent << 52 | mantissa)
    for exponent in range(-324, 309):
        center = int.from_bytes(struct.pack(">d", float(f"1e{exponent}")), "big")
        for delta in range(-2, 3):
            bits = center + delta
            if 0 <= bits <= 0x7fefffffffffffff:
                for sign in (0, 1 << 63):
                    add("decade", sign | bits)
    for value in (0.0000995, 0.0009995, 0.00995, 0.0995, 0.995, 1.25, 1.35,
                  9.95, 12.5, 13.5, 99.5, 125.0, 135.0, 995.0, 999500.0):
        center = int.from_bytes(struct.pack(">d", value), "big")
        for delta in range(-2, 3):
            for sign in (0, 1 << 63):
                add("ties-carry", sign | (center + delta))
    rng = random.Random(60281500)
    for _ in range(4096):
        add("seeded-bits", rng.getrandbits(64))
    return requests, counts
