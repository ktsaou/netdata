"""Deterministic metadata inputs; expected permutations come only from original Go."""
import itertools
import struct


def bits(value):
    return struct.pack(">d", float(value)).hex()


def generate():
    cases, counts = [], {}

    def add(family, name, values):
        cases.append({"id": f"{family}:{name}", "bounds": list(values)})
        counts[family] = counts.get(family, 0) + 1

    special = [bits(-1), bits(1), "0000000000000000", "8000000000000000",
               "fff0000000000000", "7ff0000000000000", "7ff8000000000063",
               "fff8000000000027", "7ff0000000000044"]
    for length in range(5):
        for index, values in enumerate(itertools.product(special, repeat=length)):
            add("product", f"{length}:{index:04d}", values)
    state = 0x9183aeff5910496b

    def random():
        nonlocal state
        state ^= (state << 13) & 0xffffffffffffffff
        state ^= state >> 7
        state ^= (state << 17) & 0xffffffffffffffff
        return state

    for length in range(5, 13):
        for index in range(128):
            add("insertion", f"{length}:{index:03d}",
                (special[random() % len(special)] for _ in range(length)))
    lengths = sorted(set(range(17)) | {31, 32, 33, 49, 50, 51} |
                     {2**power + offset for power in range(6, 14) for offset in (-1, 0, 1)})
    for length in lengths:
        ascending = [bits(index) for index in range(length)]
        for name, values in {
            "ascending": ascending,
            "descending": reversed(ascending),
            "equal": [bits(1)] * length,
            "zeros": [special[2 + index % 2] for index in range(length)],
            "saw": [bits(index % 31) for index in range(length)],
            "organ": [bits(min(index, length - 1 - index)) for index in range(length)],
            "rotated": ascending[length // 3:] + ascending[:length // 3],
            "nan": [special[6 + index % 3] for index in range(length)],
            "special": [special[index % len(special)] for index in range(length)],
            "random": [special[random() % len(special)] for _ in range(length)],
        }.items():
            add("boundary", f"{length:05d}:{name}", values)
        for position in sorted({0, length // 4, length // 2, 3 * length // 4, length - 1}):
            if 0 <= position < length:
                for nan in special[6:]:
                    values = ascending.copy()
                    values[position] = nan
                    add("nan-pivot", f"{length:05d}:{position:05d}:{nan}", values)
    length = 100000
    for name, values in {
        "descending": [bits(length - index) for index in range(length)],
        "organ": [bits(min(index, length - 1 - index)) for index in range(length)],
        "special": [special[index % len(special)] for index in range(length)],
        "random": [bits(random() % 16381) for _ in range(length)],
    }.items():
        add("large", name, values)
    assert len({case["id"] for case in cases}) == len(cases)
    return cases, counts
