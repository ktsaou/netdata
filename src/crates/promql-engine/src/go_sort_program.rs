// SPDX-License-Identifier: GPL-3.0-or-later
// Resumable Go 1.27.1 sort/zsortinterface.go and container/heap/heap.go; see NOTICE.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Operation {
    Step,
    Less(u64, u64),
    Swap(u64, u64),
    Done,
}

pub(crate) fn physical_index(index: u64, length: usize) -> usize {
    let index = usize::try_from(index).expect("sort index exceeds physical address width");
    assert!(index < length, "sort index exceeds physical buffer");
    index
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}
#[derive(Clone, Copy, Debug)]
enum Return {
    Unit,
    Boolean(bool),
    Index(u64),
    Partition(u64, bool),
    Pivot(u64, Hint),
    Median(u64, usize),
}
#[derive(Clone, Copy)]
struct Pdq {
    a: u64,
    b: u64,
    limit: u32,
    balanced: bool,
    partitioned: bool,
    pivot: u64,
    hint: Hint,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Insertion {
    a: u64,
    b: u64,
    i: u64,
    j: u64,
    waiting: bool,
}
#[derive(Clone, Copy)]
struct Partition {
    a: u64,
    pivot: u64,
    i: u64,
    j: u64,
    equal: bool,
    untouched: bool,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Partial {
    a: u64,
    b: u64,
    i: u64,
    j: u64,
    attempts: u8,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Pivot {
    a: u64,
    b: u64,
    i: u64,
    j: u64,
    k: u64,
    swaps: usize,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Median {
    a: u64,
    b: u64,
    c: u64,
    swaps: usize,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Heap {
    a: u64,
    hi: u64,
    i: u64,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Sift {
    first: u64,
    root: u64,
    hi: u64,
    child: u64,
    phase: u8,
}
#[derive(Clone, Copy)]
struct Patterns {
    a: u64,
    length: u64,
    position: usize,
    random: u64,
}
#[derive(Clone, Copy)]
struct Down {
    initial: u64,
    root: u64,
    length: u64,
    child: u64,
    phase: u8,
}
#[derive(Clone, Copy)]
enum Frame {
    Idle,
    Pdq(Pdq),
    Insertion(Insertion),
    Partition(Partition),
    Partial(Partial),
    Pivot(Pivot),
    Median(Median),
    Heap(Heap),
    Sift(Sift),
    Patterns(Patterns),
    Reverse { i: u64, j: u64 },
    Up { j: u64, waiting: bool },
    Down(Down),
    Fix { root: u64, length: u64, phase: u8 },
}
impl Frame {
    fn pdq(a: u64, b: u64, limit: u32) -> Self {
        Self::Pdq(Pdq {
            a,
            b,
            limit,
            balanced: true,
            partitioned: true,
            pivot: 0,
            hint: Hint::Unknown,
            phase: 0,
        })
    }
    fn insertion(a: u64, b: u64) -> Self {
        Self::Insertion(Insertion {
            a,
            b,
            i: a + 1,
            j: a + 1,
            waiting: false,
        })
    }
    fn partition(a: u64, b: u64, pivot: u64, equal: bool) -> Self {
        Self::Partition(Partition {
            a,
            pivot,
            i: a + 1,
            j: b - 1,
            equal,
            untouched: true,
            phase: 0,
        })
    }
    fn partial(a: u64, b: u64) -> Self {
        Self::Partial(Partial {
            a,
            b,
            i: a + 1,
            j: 0,
            attempts: 0,
            phase: 0,
        })
    }
    fn pivot(a: u64, b: u64) -> Self {
        let quarter = (b - a) / 4;
        Self::Pivot(Pivot {
            a,
            b,
            i: a + quarter,
            j: a + quarter * 2,
            k: a + quarter * 3,
            swaps: 0,
            phase: 0,
        })
    }
    fn median(a: u64, b: u64, c: u64) -> Self {
        Self::Median(Median {
            a,
            b,
            c,
            swaps: 0,
            phase: 0,
        })
    }
    fn heap(a: u64, b: u64) -> Self {
        Self::Heap(Heap {
            a,
            hi: b - a,
            i: 0,
            phase: 0,
        })
    }
    fn sift(first: u64, root: u64, hi: u64) -> Self {
        Self::Sift(Sift {
            first,
            root,
            hi,
            child: 0,
            phase: 0,
        })
    }
    fn reverse(a: u64, b: u64) -> Self {
        Self::Reverse {
            i: a,
            j: b.saturating_sub(1),
        }
    }
    fn patterns(a: u64, b: u64) -> Self {
        Self::Patterns(Patterns {
            a,
            length: b - a,
            position: 0,
            random: b - a,
        })
    }
}

// Every recursive PDQ child has at most half its parent's length; helpers add at most two frames.
const FRAMES: usize = 64 + 3;
pub(crate) struct Program {
    frames: [Frame; FRAMES],
    depth: usize,
    returned: Return,
    waiting: bool,
}
impl Program {
    fn new(frame: Frame) -> Self {
        let mut frames = [Frame::Idle; FRAMES];
        frames[0] = frame;
        Self {
            frames,
            depth: 1,
            returned: Return::Unit,
            waiting: false,
        }
    }
    pub(crate) fn sort(length: u64) -> Self {
        Self::new(Frame::pdq(0, length, u64::BITS - length.leading_zeros()))
    }
    pub(crate) fn heap_push(length: u64) -> Self {
        assert!(length > 0, "heap Push follows an append");
        Self::new(Frame::Up {
            j: length - 1,
            waiting: false,
        })
    }
    pub(crate) fn heap_fix(root: u64, length: u64) -> Self {
        assert!(root < length, "heap Fix needs an existing item");
        Self::new(Frame::Fix {
            root,
            length,
            phase: 0,
        })
    }
    fn save(&mut self, frame: Frame) {
        self.frames[self.depth - 1] = frame;
    }
    fn step(&mut self, frame: Frame) -> Operation {
        self.save(frame);
        Operation::Step
    }
    fn less(&mut self, frame: Frame, a: u64, b: u64) -> Operation {
        self.save(frame);
        self.waiting = true;
        Operation::Less(a, b)
    }
    fn swap(&mut self, frame: Frame, a: u64, b: u64) -> Operation {
        self.save(frame);
        Operation::Swap(a, b)
    }
    fn call(&mut self, parent: Frame, child: Frame) -> Operation {
        self.save(parent);
        assert!(
            self.depth < FRAMES,
            "PDQ control depth exceeds its word-size proof"
        );
        self.frames[self.depth] = child;
        self.depth += 1;
        Operation::Step
    }
    fn finish(&mut self, result: Return) -> Operation {
        self.depth -= 1;
        self.frames[self.depth] = Frame::Idle;
        self.returned = result;
        Operation::Step
    }
    /// The caller polls each transition, services Less once, and applies Swap before resuming.
    pub(crate) fn next(&mut self, reply: Option<bool>) -> Operation {
        assert_eq!(self.waiting, reply.is_some(), "comparison reply lifecycle");
        self.waiting = false;
        if self.depth == 0 {
            return Operation::Done;
        }
        match self.frames[self.depth - 1] {
            Frame::Idle => unreachable!("active control frame"),
            Frame::Pdq(mut f) => match f.phase {
                0 => {
                    let length = f.b - f.a;
                    if length <= 12 {
                        f.phase = 9;
                        self.call(Frame::Pdq(f), Frame::insertion(f.a, f.b))
                    } else if f.limit == 0 {
                        f.phase = 9;
                        self.call(Frame::Pdq(f), Frame::heap(f.a, f.b))
                    } else if !f.balanced {
                        f.phase = 1;
                        self.call(Frame::Pdq(f), Frame::patterns(f.a, f.b))
                    } else {
                        f.phase = 2;
                        self.call(Frame::Pdq(f), Frame::pivot(f.a, f.b))
                    }
                }
                1 => {
                    f.limit -= 1;
                    f.phase = 2;
                    self.call(Frame::Pdq(f), Frame::pivot(f.a, f.b))
                }
                2 => {
                    let Return::Pivot(pivot, hint) = self.returned else {
                        unreachable!()
                    };
                    f.pivot = pivot;
                    f.hint = hint;
                    if hint == Hint::Decreasing {
                        f.phase = 3;
                        self.call(Frame::Pdq(f), Frame::reverse(f.a, f.b))
                    } else {
                        f.phase = 4;
                        self.step(Frame::Pdq(f))
                    }
                }
                3 => {
                    f.pivot = (f.b - 1) - (f.pivot - f.a);
                    f.hint = Hint::Increasing;
                    f.phase = 4;
                    self.step(Frame::Pdq(f))
                }
                4 => {
                    if f.balanced && f.partitioned && f.hint == Hint::Increasing {
                        f.phase = 5;
                        self.call(Frame::Pdq(f), Frame::partial(f.a, f.b))
                    } else {
                        f.phase = 6;
                        self.step(Frame::Pdq(f))
                    }
                }
                5 => {
                    let Return::Boolean(sorted) = self.returned else {
                        unreachable!()
                    };
                    if sorted {
                        self.finish(Return::Unit)
                    } else {
                        f.phase = 6;
                        self.step(Frame::Pdq(f))
                    }
                }
                6 => {
                    if f.a > 0 {
                        f.phase = 7;
                        self.less(Frame::Pdq(f), f.a - 1, f.pivot)
                    } else {
                        f.phase = 8;
                        self.call(Frame::Pdq(f), Frame::partition(f.a, f.b, f.pivot, false))
                    }
                }
                7 => {
                    let equal = !reply.unwrap();
                    f.phase = if equal { 10 } else { 8 };
                    self.call(Frame::Pdq(f), Frame::partition(f.a, f.b, f.pivot, equal))
                }
                8 => {
                    let Return::Partition(mid, untouched) = self.returned else {
                        unreachable!()
                    };
                    f.partitioned = untouched;
                    let (left, right) = (mid - f.a, f.b - mid);
                    let threshold = (f.b - f.a) / 8;
                    let child = if left < right {
                        f.balanced = left >= threshold;
                        let child = Frame::pdq(f.a, mid, f.limit);
                        f.a = mid + 1;
                        child
                    } else {
                        f.balanced = right >= threshold;
                        let child = Frame::pdq(mid + 1, f.b, f.limit);
                        f.b = mid;
                        child
                    };
                    f.phase = 11;
                    self.call(Frame::Pdq(f), child)
                }
                9 => self.finish(Return::Unit),
                10 => {
                    let Return::Index(mid) = self.returned else {
                        unreachable!()
                    };
                    f.a = mid;
                    f.phase = 0;
                    self.step(Frame::Pdq(f))
                }
                11 => {
                    f.phase = 0;
                    self.step(Frame::Pdq(f))
                }
                _ => unreachable!(),
            },
            Frame::Insertion(mut f) => {
                if f.waiting {
                    f.waiting = false;
                    if reply.unwrap() {
                        let j = f.j;
                        f.j -= 1;
                        return self.swap(Frame::Insertion(f), j, j - 1);
                    }
                    f.i += 1;
                    f.j = f.i;
                    return self.step(Frame::Insertion(f));
                }
                if f.i >= f.b {
                    self.finish(Return::Unit)
                } else if f.j > f.a {
                    f.waiting = true;
                    self.less(Frame::Insertion(f), f.j, f.j - 1)
                } else {
                    f.i += 1;
                    f.j = f.i;
                    self.step(Frame::Insertion(f))
                }
            }
            Frame::Partition(mut f) => match f.phase {
                0 => {
                    f.phase = 1;
                    self.swap(Frame::Partition(f), f.a, f.pivot)
                }
                1 => {
                    if f.i <= f.j {
                        f.phase = 2;
                        let (a, b) = if f.equal { (f.a, f.i) } else { (f.i, f.a) };
                        self.less(Frame::Partition(f), a, b)
                    } else {
                        f.phase = 3;
                        self.step(Frame::Partition(f))
                    }
                }
                2 => {
                    if reply.unwrap() != f.equal {
                        f.i += 1;
                        f.phase = 1;
                    } else {
                        f.phase = 3;
                    }
                    self.step(Frame::Partition(f))
                }
                3 => {
                    if f.i <= f.j {
                        f.phase = 4;
                        let (a, b) = if f.equal { (f.a, f.j) } else { (f.j, f.a) };
                        self.less(Frame::Partition(f), a, b)
                    } else {
                        f.phase = 5;
                        self.step(Frame::Partition(f))
                    }
                }
                4 => {
                    if reply.unwrap() == f.equal {
                        f.j -= 1;
                        f.phase = 3;
                    } else {
                        f.phase = 5;
                    }
                    self.step(Frame::Partition(f))
                }
                5 => {
                    if f.i > f.j {
                        if f.equal {
                            self.finish(Return::Index(f.i))
                        } else {
                            f.phase = 6;
                            self.swap(Frame::Partition(f), f.j, f.a)
                        }
                    } else {
                        let (i, j) = (f.i, f.j);
                        f.i += 1;
                        f.j -= 1;
                        f.untouched = false;
                        f.phase = 1;
                        self.swap(Frame::Partition(f), i, j)
                    }
                }
                6 => self.finish(Return::Partition(f.j, f.untouched)),
                _ => unreachable!(),
            },
            Frame::Partial(mut f) => match f.phase {
                0 => {
                    if f.attempts == 5 {
                        self.finish(Return::Boolean(false))
                    } else if f.i < f.b {
                        f.phase = 1;
                        self.less(Frame::Partial(f), f.i, f.i - 1)
                    } else {
                        f.phase = 2;
                        self.step(Frame::Partial(f))
                    }
                }
                1 => {
                    if !reply.unwrap() {
                        f.i += 1;
                        f.phase = 0;
                    } else {
                        f.phase = 2;
                    }
                    self.step(Frame::Partial(f))
                }
                2 => {
                    if f.i == f.b {
                        self.finish(Return::Boolean(true))
                    } else if f.b - f.a < 50 {
                        self.finish(Return::Boolean(false))
                    } else {
                        f.phase = 3;
                        self.swap(Frame::Partial(f), f.i, f.i - 1)
                    }
                }
                3 => {
                    if f.i - f.a >= 2 {
                        f.j = f.i - 1;
                        f.phase = 4;
                    } else {
                        f.phase = 6;
                    }
                    self.step(Frame::Partial(f))
                }
                4 => {
                    if f.j >= 1 {
                        f.phase = 5;
                        self.less(Frame::Partial(f), f.j, f.j - 1)
                    } else {
                        f.phase = 6;
                        self.step(Frame::Partial(f))
                    }
                }
                5 => {
                    if reply.unwrap() {
                        let j = f.j;
                        f.j -= 1;
                        f.phase = 4;
                        self.swap(Frame::Partial(f), j, j - 1)
                    } else {
                        f.phase = 6;
                        self.step(Frame::Partial(f))
                    }
                }
                6 => {
                    if f.b - f.i >= 2 {
                        f.j = f.i + 1;
                        f.phase = 7;
                    } else {
                        f.attempts += 1;
                        f.phase = 0;
                    }
                    self.step(Frame::Partial(f))
                }
                7 => {
                    if f.j < f.b {
                        f.phase = 8;
                        self.less(Frame::Partial(f), f.j, f.j - 1)
                    } else {
                        f.attempts += 1;
                        f.phase = 0;
                        self.step(Frame::Partial(f))
                    }
                }
                8 => {
                    if reply.unwrap() {
                        let j = f.j;
                        f.j += 1;
                        f.phase = 7;
                        self.swap(Frame::Partial(f), j, j - 1)
                    } else {
                        f.attempts += 1;
                        f.phase = 0;
                        self.step(Frame::Partial(f))
                    }
                }
                _ => unreachable!(),
            },
            Frame::Pivot(mut f) => match f.phase {
                0 => {
                    if f.b - f.a < 8 {
                        self.finish(Return::Pivot(f.j, Hint::Increasing))
                    } else if f.b - f.a >= 50 {
                        f.phase = 1;
                        self.call(Frame::Pivot(f), Frame::median(f.i - 1, f.i, f.i + 1))
                    } else {
                        f.phase = 4;
                        self.call(Frame::Pivot(f), Frame::median(f.i, f.j, f.k))
                    }
                }
                1..=4 => {
                    let Return::Median(index, swaps) = self.returned else {
                        unreachable!()
                    };
                    f.swaps += swaps;
                    match f.phase {
                        1 => {
                            f.i = index;
                            f.phase = 2;
                            self.call(Frame::Pivot(f), Frame::median(f.j - 1, f.j, f.j + 1))
                        }
                        2 => {
                            f.j = index;
                            f.phase = 3;
                            self.call(Frame::Pivot(f), Frame::median(f.k - 1, f.k, f.k + 1))
                        }
                        3 => {
                            f.k = index;
                            f.phase = 4;
                            self.call(Frame::Pivot(f), Frame::median(f.i, f.j, f.k))
                        }
                        _ => self.finish(Return::Pivot(
                            index,
                            match f.swaps {
                                0 => Hint::Increasing,
                                12 => Hint::Decreasing,
                                _ => Hint::Unknown,
                            },
                        )),
                    }
                }
                _ => unreachable!(),
            },
            Frame::Median(mut f) => match f.phase {
                0 => {
                    f.phase = 1;
                    self.less(Frame::Median(f), f.b, f.a)
                }
                1 => {
                    if reply.unwrap() {
                        std::mem::swap(&mut f.a, &mut f.b);
                        f.swaps += 1;
                    }
                    f.phase = 2;
                    self.less(Frame::Median(f), f.c, f.b)
                }
                2 => {
                    if reply.unwrap() {
                        std::mem::swap(&mut f.b, &mut f.c);
                        f.swaps += 1;
                    }
                    f.phase = 3;
                    self.less(Frame::Median(f), f.b, f.a)
                }
                3 => {
                    if reply.unwrap() {
                        std::mem::swap(&mut f.a, &mut f.b);
                        f.swaps += 1;
                    }
                    self.finish(Return::Median(f.b, f.swaps))
                }
                _ => unreachable!(),
            },
            Frame::Heap(mut f) => match f.phase {
                0 => {
                    if f.hi == 0 {
                        self.finish(Return::Unit)
                    } else {
                        f.i = (f.hi - 1) / 2;
                        f.phase = 1;
                        self.call(Frame::Heap(f), Frame::sift(f.a, f.i, f.hi))
                    }
                }
                1 => {
                    if f.i > 0 {
                        f.i -= 1;
                        self.call(Frame::Heap(f), Frame::sift(f.a, f.i, f.hi))
                    } else {
                        f.i = f.hi;
                        f.phase = 2;
                        self.step(Frame::Heap(f))
                    }
                }
                2 => {
                    if f.i == 0 {
                        self.finish(Return::Unit)
                    } else {
                        f.i -= 1;
                        f.phase = 3;
                        self.swap(Frame::Heap(f), f.a, f.a + f.i)
                    }
                }
                3 => {
                    f.phase = 2;
                    self.call(Frame::Heap(f), Frame::sift(f.a, 0, f.i))
                }
                _ => unreachable!(),
            },
            Frame::Sift(mut f) => match f.phase {
                0 => {
                    f.child = 2 * f.root + 1;
                    if f.child >= f.hi {
                        self.finish(Return::Unit)
                    } else if f.child + 1 < f.hi {
                        f.phase = 1;
                        self.less(Frame::Sift(f), f.first + f.child, f.first + f.child + 1)
                    } else {
                        f.phase = 2;
                        self.step(Frame::Sift(f))
                    }
                }
                1 => {
                    if reply.unwrap() {
                        f.child += 1;
                    }
                    f.phase = 2;
                    self.step(Frame::Sift(f))
                }
                2 => {
                    f.phase = 3;
                    self.less(Frame::Sift(f), f.first + f.root, f.first + f.child)
                }
                3 => {
                    if !reply.unwrap() {
                        self.finish(Return::Unit)
                    } else {
                        let root = f.root;
                        f.root = f.child;
                        f.phase = 0;
                        self.swap(Frame::Sift(f), f.first + root, f.first + f.child)
                    }
                }
                _ => unreachable!(),
            },
            Frame::Patterns(mut f) => {
                if f.length < 8 || f.position == 3 {
                    self.finish(Return::Unit)
                } else {
                    f.random ^= f.random << 13;
                    f.random ^= f.random >> 7;
                    f.random ^= f.random << 17;
                    let shift = u64::BITS - f.length.leading_zeros();
                    let modulus = 1u64.checked_shl(shift).unwrap_or(0);
                    let mut other = f.random & modulus.wrapping_sub(1);
                    if other >= f.length {
                        other -= f.length;
                    }
                    let index = f.a + (f.length / 4) * 2 - 1 + f.position as u64;
                    f.position += 1;
                    self.swap(Frame::Patterns(f), index, f.a + other)
                }
            }
            Frame::Reverse { mut i, mut j } => {
                if i >= j {
                    self.finish(Return::Unit)
                } else {
                    let pair = (i, j);
                    i += 1;
                    j -= 1;
                    self.swap(Frame::Reverse { i, j }, pair.0, pair.1)
                }
            }
            Frame::Up { mut j, waiting } => {
                if waiting {
                    if !reply.unwrap() {
                        self.finish(Return::Unit)
                    } else {
                        let parent = (j - 1) / 2;
                        let pair = (parent, j);
                        j = parent;
                        self.swap(Frame::Up { j, waiting: false }, pair.0, pair.1)
                    }
                } else if j == 0 {
                    self.finish(Return::Unit)
                } else {
                    self.less(Frame::Up { j, waiting: true }, j, (j - 1) / 2)
                }
            }
            Frame::Down(mut f) => match f.phase {
                0 => {
                    f.child = 2 * f.root + 1;
                    if f.child >= f.length {
                        self.finish(Return::Boolean(f.root > f.initial))
                    } else if f.child + 1 < f.length {
                        f.phase = 1;
                        self.less(Frame::Down(f), f.child + 1, f.child)
                    } else {
                        f.phase = 2;
                        self.step(Frame::Down(f))
                    }
                }
                1 => {
                    if reply.unwrap() {
                        f.child += 1;
                    }
                    f.phase = 2;
                    self.step(Frame::Down(f))
                }
                2 => {
                    f.phase = 3;
                    self.less(Frame::Down(f), f.child, f.root)
                }
                3 => {
                    if !reply.unwrap() {
                        self.finish(Return::Boolean(f.root > f.initial))
                    } else {
                        let root = f.root;
                        f.root = f.child;
                        f.phase = 0;
                        self.swap(Frame::Down(f), root, f.child)
                    }
                }
                _ => unreachable!(),
            },
            Frame::Fix {
                root,
                length,
                phase,
            } => match phase {
                0 => self.call(
                    Frame::Fix {
                        root,
                        length,
                        phase: 1,
                    },
                    Frame::Down(Down {
                        initial: root,
                        root,
                        length,
                        child: 0,
                        phase: 0,
                    }),
                ),
                1 => {
                    let Return::Boolean(moved) = self.returned else {
                        unreachable!()
                    };
                    if moved {
                        self.finish(Return::Unit)
                    } else {
                        self.call(
                            Frame::Fix {
                                root,
                                length,
                                phase: 2,
                            },
                            Frame::Up {
                                j: root,
                                waiting: false,
                            },
                        )
                    }
                }
                2 => self.finish(Return::Unit),
                _ => unreachable!(),
            },
        }
    }
    #[cfg(any(test, feature = "fixtures"))]
    pub(crate) fn phase(name: &str, length: u64, pivot: u64) -> Self {
        Self::new(match name {
            "insertion" => Frame::insertion(0, length),
            "partial" => Frame::partial(0, length),
            "partition" => Frame::partition(0, length, pivot, false),
            "equal" => Frame::partition(0, length, pivot, true),
            "heap" => Frame::heap(0, length),
            "pivot" => Frame::pivot(0, length),
            "reverse" => Frame::reverse(0, length),
            "patterns" => Frame::patterns(0, length),
            _ => unreachable!("declared original ordering phase"),
        })
    }
    #[cfg(feature = "fixtures")]
    pub(crate) fn result(&self) -> Option<Vec<u64>> {
        assert_eq!(self.depth, 0);
        match self.returned {
            Return::Unit => None,
            Return::Boolean(value) => Some(vec![u64::from(value)]),
            Return::Index(index) => Some(vec![index]),
            Return::Partition(index, value) => Some(vec![index, u64::from(value)]),
            Return::Pivot(index, hint) => Some(vec![
                index,
                match hint {
                    Hint::Unknown => 0,
                    Hint::Increasing => 1,
                    Hint::Decreasing => 2,
                },
            ]),
            Return::Median(_, _) => unreachable!("internal median return"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{AccountedBox, AccountedVec, AllocationContext, EngineAccounting, MemoryKind},
        storage::{Control, Interrupted},
    };
    use std::collections::BTreeMap;

    type VirtualTrace = Vec<(Operation, Option<bool>)>;

    // Captured unchanged Go 1.27.1 helpers use only word-bounded state for these inputs.
    fn virtual_trace(
        mut program: Program,
        length: u64,
        mut values: BTreeMap<u64, i128>,
    ) -> (Return, VirtualTrace, BTreeMap<u64, i128>) {
        let mut trace = Vec::new();
        let mut reply = None;
        for _ in 0..512 {
            let operation = program.next(reply.take());
            match operation {
                Operation::Step => (),
                Operation::Less(a, b) => {
                    assert!(a < length && b < length);
                    let left = values.get(&a).copied().unwrap_or(i128::from(a));
                    let right = values.get(&b).copied().unwrap_or(i128::from(b));
                    reply = Some(left < right);
                    trace.push((operation, reply));
                }
                Operation::Swap(a, b) => {
                    assert!(a < length && b < length);
                    let left = values.get(&a).copied().unwrap_or(i128::from(a));
                    let right = values.get(&b).copied().unwrap_or(i128::from(b));
                    values.insert(a, right);
                    values.insert(b, left);
                    trace.push((operation, None));
                }
                Operation::Done => return (program.returned, trace, values),
            }
            assert!(values.len() <= 64 && trace.len() <= 256);
        }
        panic!("original virtual control exceeded its declared transition bound");
    }

    #[test]
    fn wide_heap_fix_preserves_original_child_addresses_and_operations() {
        for length in [
            17,
            4_294_967_301,
            1_099_511_627_781,
            1_152_921_504_606_846_981,
        ] {
            let root = (length - 3) / 2;
            let replacement = i128::from(length + 1);
            let (result, trace, values) = virtual_trace(
                Program::heap_fix(root, length),
                length,
                BTreeMap::from([(root, replacement)]),
            );
            assert!(matches!(result, Return::Unit));
            assert_eq!(
                trace,
                vec![
                    (Operation::Less(length - 1, length - 2), Some(false)),
                    (Operation::Less(length - 2, root), Some(true)),
                    (Operation::Swap(root, length - 2), None),
                ]
            );
            assert_eq!(
                values,
                BTreeMap::from([(root, i128::from(length - 2)), (length - 2, replacement),])
            );
        }
    }

    #[test]
    fn wide_pivot_preserves_original_comparisons_and_returned_index() {
        let (result, trace, values) = virtual_trace(
            Program::phase("pivot", 8_589_934_597, 0),
            8_589_934_597,
            BTreeMap::new(),
        );
        assert!(matches!(
            result,
            Return::Pivot(4_294_967_298, Hint::Increasing)
        ));
        assert!(values.is_empty());
        let pairs = [
            (2_147_483_649, 2_147_483_648),
            (2_147_483_650, 2_147_483_649),
            (2_147_483_649, 2_147_483_648),
            (4_294_967_298, 4_294_967_297),
            (4_294_967_299, 4_294_967_298),
            (4_294_967_298, 4_294_967_297),
            (6_442_450_947, 6_442_450_946),
            (6_442_450_948, 6_442_450_947),
            (6_442_450_947, 6_442_450_946),
            (4_294_967_298, 2_147_483_649),
            (6_442_450_947, 4_294_967_298),
            (4_294_967_298, 2_147_483_649),
        ];
        assert_eq!(
            trace,
            pairs
                .into_iter()
                .map(|(a, b)| (Operation::Less(a, b), Some(false)))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn wide_pattern_masks_preserve_original_random_swap_addresses() {
        for (length, pairs) in [
            (
                4_294_967_301,
                [
                    (2_147_483_649, 1_082_827_077),
                    (2_147_483_650, 1_006_978_370),
                    (2_147_483_651, 1_470_037_512),
                ],
            ),
            (
                1_099_511_627_781,
                [
                    (549_755_813_889, 14_001_283_392),
                    (549_755_813_890, 129_788_626_247),
                    (549_755_813_891, 41_885_562_376),
                ],
            ),
            (
                1_152_921_504_606_846_981,
                [
                    (576_460_752_303_423_489, 9_007_204_666_089_792),
                    (576_460_752_303_423_490, 287_102_390_584_647),
                    (576_460_752_303_423_491, 1_105_024_694_045_308_429),
                ],
            ),
        ] {
            let (result, trace, values) = virtual_trace(
                Program::phase("patterns", length, 0),
                length,
                BTreeMap::new(),
            );
            assert!(matches!(result, Return::Unit));
            assert_eq!(
                trace,
                pairs
                    .into_iter()
                    .map(|(a, b)| (Operation::Swap(a, b), None))
                    .collect::<Vec<_>>()
            );
            assert_eq!(values.len(), 6);
            for (a, b) in pairs {
                assert_eq!(values[&a], i128::from(b));
                assert_eq!(values[&b], i128::from(a));
            }
        }
    }

    #[test]
    fn wide_heap_push_preserves_original_ancestor_path_with_bounded_state() {
        let length = 4_294_967_301;
        let (result, trace, values) = virtual_trace(
            Program::heap_push(length),
            length,
            BTreeMap::from([(length - 1, -1)]),
        );
        let path = [
            4_294_967_300,
            2_147_483_649,
            1_073_741_824,
            536_870_911,
            268_435_455,
            134_217_727,
            67_108_863,
            33_554_431,
            16_777_215,
            8_388_607,
            4_194_303,
            2_097_151,
            1_048_575,
            524_287,
            262_143,
            131_071,
            65_535,
            32_767,
            16_383,
            8_191,
            4_095,
            2_047,
            1_023,
            511,
            255,
            127,
            63,
            31,
            15,
            7,
            3,
            1,
            0,
        ];
        assert!(matches!(result, Return::Unit));
        assert_eq!(
            trace,
            path.windows(2)
                .flat_map(|pair| [
                    (Operation::Less(pair[0], pair[1]), Some(true)),
                    (Operation::Swap(pair[1], pair[0]), None),
                ])
                .collect::<Vec<_>>()
        );
        assert_eq!(values.len(), 33);
        assert_eq!(values[&0], -1);
        for pair in path.windows(2) {
            assert_eq!(values[&pair[0]], i128::from(pair[1]));
        }
    }

    #[test]
    fn physical_buffer_boundaries_reject_wide_indices_without_truncation() {
        assert_eq!(physical_index(2, 4), 2);
        for index in [4, 1 << 32, u64::MAX] {
            assert!(std::panic::catch_unwind(|| physical_index(index, 4)).is_err());
        }
    }

    #[test]
    fn comparison_answers_are_required_once_and_do_not_change_indices_while_suspended() {
        let mut program = Program::sort(2);
        while program.next(None) == Operation::Step {}
        assert!(program.waiting);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| program.next(None))).is_err()
        );
        assert!(program.waiting);
        assert_eq!(program.next(Some(true)), Operation::Swap(1, 0));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| program.next(Some(false))))
                .is_err()
        );
        assert!(!program.waiting);
        while program.next(None) != Operation::Done {}
        assert_eq!(program.next(None), Operation::Done);
    }

    #[test]
    fn suspended_sort_and_heap_phases_release_exact_state_and_identity_owners_on_interruption() {
        for phase in [
            "sort",
            "push",
            "fix",
            "insertion",
            "partial",
            "partition",
            "equal",
            "heap",
            "pivot",
            "reverse",
            "patterns",
        ] {
            for stop in 1..=128 {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let context = AllocationContext::new(&account, MemoryKind::Workspace);
                let mut data = AccountedVec::new(&account, MemoryKind::Workspace);
                for ordinal in 0..64 {
                    data.push((64 - ordinal, ordinal));
                }
                let state = match phase {
                    "sort" => Program::sort(64),
                    "push" => Program::heap_push(64),
                    "fix" => Program::heap_fix(0, 64),
                    _ => Program::phase(phase, 64, 32),
                };
                let mut program = AccountedBox::new(state, &context);
                let memory = account.snapshot().memory;
                let control = Control::default();
                let (mut visits, mut reply) = (0, None);
                let result = loop {
                    visits += 1;
                    if visits == stop {
                        control.cancellation.cancel();
                    }
                    if let Err(error) = control.check() {
                        break Err(error);
                    }
                    account.work(1);
                    match program.next(reply.take()) {
                        Operation::Step => (),
                        Operation::Less(a, b) => {
                            let (a, b) =
                                (physical_index(a, data.len()), physical_index(b, data.len()));
                            reply = Some(data[a].0 < data[b].0);
                        }
                        Operation::Swap(a, b) => {
                            let (a, b) =
                                (physical_index(a, data.len()), physical_index(b, data.len()));
                            data.swap(a, b);
                        }
                        Operation::Done => break Ok(()),
                    }
                };
                if result.is_err() {
                    assert_eq!(result, Err(Interrupted::Cancelled));
                    assert_eq!(visits, stop);
                    assert_eq!(account.snapshot().work, (visits - 1) as u64);
                }
                assert_eq!(account.snapshot().memory, memory);
                let mut seen = 0u64;
                for &(value, ordinal) in &data {
                    assert_eq!(value, 64 - ordinal);
                    assert_eq!(seen & (1 << ordinal), 0);
                    seen |= 1 << ordinal;
                }
                assert_eq!(seen, u64::MAX);
                drop((program, data, context));
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }
}
