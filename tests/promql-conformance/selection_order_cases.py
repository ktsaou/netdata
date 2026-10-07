"""Declared ordering inputs; expected operations and permutations come from original Go."""
import itertools

from go_sort_cases import bits


def generate():
    cases = []
    special = [bits(-1), bits(1), bits(0), bits(-0.0), bits(float("inf")),
               bits(float("-inf")), "7ff8000000000063", "fff8000000000027", "7ff0000000000044"]

    def add(name, values, action="sort", bottom=False, reverse=False, **extra):
        cases.append({"id": f"{name}:{action}:{int(bottom)}:{int(reverse)}",
                      "values": list(values), "action": action, "bottom": bottom,
                      "reverse": reverse, **extra})

    for length in range(4):
        for index, values in enumerate(itertools.product(special, repeat=length)):
            for bottom, reverse in itertools.product((False, True), repeat=2):
                add(f"product:{length}:{index}", values, bottom=bottom, reverse=reverse)
    state = 0x9183aeff5910496b

    def random():
        nonlocal state
        state ^= (state << 13) & 0xffffffffffffffff
        state ^= state >> 7
        state ^= (state << 17) & 0xffffffffffffffff
        return state

    lengths = sorted(set(range(17)) | {31, 32, 33, 49, 50, 51} |
                     {2**power + delta for power in range(6, 11) for delta in (-1, 0, 1)})
    for length in lengths:
        ascending = [bits(index) for index in range(length)]
        patterns = {"ascending": ascending, "descending": ascending[::-1],
                    "equal": [bits(1)] * length,
                    "zeros": [bits(-0.0 if index % 2 else 0) for index in range(length)],
                    "organ": [bits(min(index, length - index - 1)) for index in range(length)],
                    "rotated": ascending[length // 3:] + ascending[:length // 3],
                    "nan": [special[6 + index % 3] for index in range(length)],
                    "special": [special[index % len(special)] for index in range(length)],
                    "random": [special[random() % len(special)] for _ in range(length)]}
        for name, values in patterns.items():
            for bottom, reverse in itertools.product((False, True), repeat=2):
                add(f"boundary:{length}:{name}", values, bottom=bottom, reverse=reverse)
            for bottom in (False, True):
                add(f"heap:{length}:{name}", values, "push", bottom)
                if length:
                    for index in sorted({0, length // 2, length - 1}):
                        for value in (bits(-2), bits(2), special[6]):
                            add(f"fix:{length}:{name}:{index}:{value}", values, "fix", bottom,
                                index=index, replacement=value)
            if length:
                for action in ("insertion", "partial", "heap", "pivot", "reverse", "patterns"):
                    add(f"phase:{length}:{name}", values, action)
                for action in ("partition", "equal"):
                    add(f"phase:{length}:{name}", values, action, pivot=length // 2)
    assert len(cases) == len({case["id"] for case in cases})
    return cases
