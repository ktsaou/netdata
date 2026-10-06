// SPDX-License-Identifier: GPL-3.0-or-later
// Logical emission, merge and position rules derive from pinned util/annotations; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::{Span, quote, token as tokens},
    shared::Strong,
    storage::{Control, Interrupted},
};
use std::fmt::{self, Write};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Info,
    Warning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistogramOperation {
    Addition,
    Subtraction,
    Aggregation,
}
impl HistogramOperation {
    fn name(self) -> &'static str {
        match self {
            Self::Addition => "addition",
            Self::Subtraction => "subtraction",
            Self::Aggregation => "aggregation",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnotationKind {
    Provider(Severity),
    PossibleNonCounter,
    NativeHistogramNotCounter,
    NativeHistogramNotGauge,
    MixedFloatsHistograms,
    MixedFloatsHistogramsAggregation,
    StartTimeOverlap,
    HistogramIgnoredInMixedRange,
    HistogramIgnoredInAggregation(tokens::TokenKind),
    MixedExponentialCustomHistograms,
    HistogramCounterResetCollision(HistogramOperation),
    MismatchedCustomBuckets(HistogramOperation),
    IncompatibleBinaryTypes {
        left_histogram: bool,
        operator: tokens::TokenKind,
        right_histogram: bool,
    },
}
impl AnnotationKind {
    fn severity(self) -> Severity {
        match self {
            Self::Provider(severity) => severity,
            Self::PossibleNonCounter
            | Self::HistogramIgnoredInMixedRange
            | Self::HistogramIgnoredInAggregation(_)
            | Self::MismatchedCustomBuckets(_)
            | Self::IncompatibleBinaryTypes { .. } => Severity::Info,
            Self::MixedFloatsHistograms
            | Self::NativeHistogramNotCounter
            | Self::NativeHistogramNotGauge
            | Self::MixedFloatsHistogramsAggregation
            | Self::StartTimeOverlap
            | Self::MixedExponentialCustomHistograms
            | Self::HistogramCounterResetCollision(_) => Severity::Warning,
        }
    }
    fn prefix(self) -> &'static str {
        match self {
            Self::Provider(_) => "",
            Self::PossibleNonCounter => {
                "PromQL info: metric might not be a counter, name does not end in _total/_sum/_count/_bucket: "
            }
            Self::NativeHistogramNotCounter => {
                "PromQL warning: this native histogram metric is not a counter: "
            }
            Self::NativeHistogramNotGauge => {
                "PromQL warning: this native histogram metric is not a gauge: "
            }
            Self::MixedFloatsHistograms => {
                "PromQL warning: encountered a mix of histograms and floats for metric name "
            }
            Self::MixedFloatsHistogramsAggregation => {
                "PromQL warning: encountered a mix of histograms and floats for aggregation"
            }
            Self::StartTimeOverlap => {
                "PromQL warning: sample has start time that overlaps with previous sample timestamp for metric "
            }
            Self::HistogramIgnoredInMixedRange => {
                "PromQL info: ignored histograms in a range containing both floats and histograms for metric name "
            }
            Self::HistogramIgnoredInAggregation(_) => "PromQL info: ignored histogram in ",
            Self::MixedExponentialCustomHistograms => {
                "PromQL warning: vector contains a mix of histograms with exponential and custom buckets schemas for metric name "
            }
            Self::HistogramCounterResetCollision(_) => {
                "PromQL warning: conflicting counter resets during histogram "
            }
            Self::MismatchedCustomBuckets(_) => {
                "PromQL info: mismatched custom buckets were reconciled during "
            }
            Self::IncompatibleBinaryTypes { .. } => {
                "PromQL info: incompatible sample types encountered for binary operator "
            }
        }
    }
    fn operation(self) -> Option<HistogramOperation> {
        match self {
            Self::HistogramCounterResetCollision(operation)
            | Self::MismatchedCustomBuckets(operation) => Some(operation),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AnnotationRef<'a> {
    pub kind: AnnotationKind,
    pub severity: Severity,
    pub raw_message: &'a [u8],
    pub message: &'a [u8],
    pub span: Option<Span>,
    pub occurrences: i64,
}

#[derive(Debug)]
struct Annotation {
    kind: AnnotationKind,
    raw_message: AccountedVec<u8>,
    message: AccountedVec<u8>,
    span: Option<Span>,
    occurrences: i64,
}
impl Annotation {
    fn format(&mut self, query: &[u8]) {
        self.message.clear();
        self.message.extend_from_slice(&self.raw_message);
        if !query.is_empty()
            && self.kind == AnnotationKind::StartTimeOverlap
            && self.occurrences > 1
        {
            write!(self.message, " ({} occurrences)", self.occurrences).expect("byte writer");
        }
        if let Some(span) = self.span {
            write_position_suffix(query, span.start as isize, &mut self.message)
                .expect("byte writer");
        }
    }
    fn as_ref(&self) -> AnnotationRef<'_> {
        AnnotationRef {
            kind: self.kind,
            severity: self.kind.severity(),
            raw_message: &self.raw_message,
            message: &self.message,
            span: self.span,
            occurrences: self.occurrences,
        }
    }
}

/// Immutable diagnostics share tracked storage independently of the provider and compiled plan.
#[derive(Clone, Debug, Default)]
pub struct Annotations {
    entries: Option<Strong<AccountedVec<Annotation>>>,
}
impl Annotations {
    pub fn iter(&self) -> impl Iterator<Item = AnnotationRef<'_>> {
        self.entries
            .as_deref()
            .map_or(&[][..], |entries| &**entries)
            .iter()
            .map(Annotation::as_ref)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_none()
    }
    pub fn len(&self) -> usize {
        self.entries.as_ref().map_or(0, |entries| entries.len())
    }
}

pub(crate) struct AnnotationBuilder {
    entries: AccountedVec<Annotation>,
}
impl AnnotationBuilder {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: AccountedVec::new(account, MemoryKind::Result),
        }
    }
    pub(crate) fn add(
        &mut self,
        kind: AnnotationKind,
        metric: &[u8],
        span: Span,
        query: &[u8],
        account: &QueryAccounting,
    ) {
        let mut raw_message = AccountedVec::new(account, MemoryKind::Result);
        raw_message.extend_from_slice(kind.prefix().as_bytes());
        if let AnnotationKind::IncompatibleBinaryTypes {
            left_histogram,
            operator,
            right_histogram,
        } = kind
        {
            let operation = operator.text().expect("binary operator text");
            quote::write_bytes(operation.as_bytes(), &mut raw_message).expect("byte writer");
            let left = if left_histogram { "histogram" } else { "float" };
            let right = if right_histogram {
                "histogram"
            } else {
                "float"
            };
            write!(raw_message, ": {left} {operation} {right}").expect("byte writer");
        } else if let Some(operation) = kind.operation() {
            raw_message.extend_from_slice(operation.name().as_bytes());
        } else if let AnnotationKind::HistogramIgnoredInAggregation(operator) = kind {
            raw_message.extend_from_slice(operator.text().expect("aggregation text").as_bytes());
            raw_message.extend_from_slice(b" aggregation");
        } else if kind == AnnotationKind::MixedFloatsHistogramsAggregation {
        } else {
            quote::write_bytes(metric, &mut raw_message).expect("byte writer");
        }
        let mut entry = Annotation {
            kind,
            raw_message,
            span: Some(span),
            occurrences: 1,
            message: AccountedVec::new(account, MemoryKind::Result),
        };
        entry.format(query);
        self.merge_one(entry, query);
    }
    pub(crate) fn add_provider(
        &mut self,
        diagnostic: crate::storage::ProviderDiagnosticRef<'_>,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut raw_message = AccountedVec::new(account, MemoryKind::Result);
        let mut message = AccountedVec::new(account, MemoryKind::Result);
        for bytes in diagnostic.message.chunks(1024) {
            control.check()?;
            account.work(bytes.len() as u64);
            raw_message.extend_from_slice(bytes);
            message.extend_from_slice(bytes);
        }
        if let Some(position) = diagnostic.position {
            write_position_suffix(query, position.start as isize, &mut message)
                .expect("byte writer");
        }
        self.merge_one(
            Annotation {
                kind: AnnotationKind::Provider(diagnostic.severity),
                raw_message,
                message,
                span: diagnostic.position,
                occurrences: 1,
            },
            query,
        );
        Ok(())
    }
    fn merge_one(&mut self, incoming: Annotation, query: &[u8]) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| *entry.raw_message == *incoming.raw_message)
        {
            if incoming.kind == AnnotationKind::StartTimeOverlap
                && self.entries[index].kind == AnnotationKind::StartTimeOverlap
            {
                // The incoming overlap merges into the previous object, keeping its first position.
                let previous = &mut self.entries[index];
                previous.occurrences = previous.occurrences.wrapping_add(incoming.occurrences);
                previous.format(query);
            } else {
                self.entries[index] = incoming;
            }
        } else {
            self.entries.push(incoming);
        }
    }
    pub(crate) fn merge(&mut self, mut child: Self, query: &[u8]) {
        child.entries.reverse();
        while let Some(entry) = child.entries.pop() {
            self.merge_one(entry, query);
        }
    }
    pub(crate) fn finish(self) -> Annotations {
        Annotations {
            entries: if self.entries.is_empty() {
                None
            } else {
                let context = self.entries.context();
                Some(Strong::new_in(self.entries, context.as_ref()))
            },
        }
    }
}

pub(crate) fn write_position_suffix(
    query: &[u8],
    start: isize,
    out: &mut impl Write,
) -> fmt::Result {
    if query.is_empty() {
        return Ok(());
    }
    if start < 0 || start as usize > query.len() {
        return out.write_str(" (invalid position)");
    }
    let start = start as usize;
    let mut line = 1;
    let mut column = start + 1;
    for (index, &byte) in query[..start].iter().enumerate() {
        if byte == b'\n' {
            line += 1;
            column = start - index;
        }
    }
    write!(out, " ({line}:{column})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;
    #[test]
    fn opaque_overlap_text_replaces_the_owner_before_a_new_generated_overlap() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = AnnotationBuilder::new(&account);
        let query = b"a+b+c";
        for start in [0, 2] {
            builder.add(
                AnnotationKind::StartTimeOverlap,
                b"m",
                Span {
                    start,
                    end: start + 1,
                },
                query,
                &account,
            );
        }
        let raw = b"PromQL warning: sample has start time that overlaps with previous sample timestamp for metric \"m\"";
        builder
            .add_provider(
                crate::storage::ProviderDiagnosticRef {
                    message: raw,
                    severity: Severity::Info,
                    position: None,
                },
                query,
                &control,
                &account,
            )
            .unwrap();
        assert_eq!(
            builder.entries[0].kind,
            AnnotationKind::Provider(Severity::Info)
        );
        assert_eq!(builder.entries[0].occurrences, 1);
        assert_eq!(&*builder.entries[0].message, raw);
        builder.add(
            AnnotationKind::StartTimeOverlap,
            b"m",
            Span { start: 4, end: 5 },
            query,
            &account,
        );
        assert_eq!(builder.entries[0].kind, AnnotationKind::StartTimeOverlap);
        assert_eq!(builder.entries[0].occurrences, 1);
        assert_eq!(builder.entries[0].span, Some(Span { start: 4, end: 5 }));
        builder.add(
            AnnotationKind::StartTimeOverlap,
            b"m",
            Span { start: 0, end: 1 },
            query,
            &account,
        );
        assert_eq!(builder.entries[0].occurrences, 2);
        assert!(
            builder.entries[0]
                .message
                .ends_with(b" (2 occurrences) (1:5)")
        );
        drop(builder);
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .all(|kind| kind.current == 0)
        );
    }
    #[test]
    fn histogram_operation_notes_deduplicate_without_the_metric_name() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut builder = AnnotationBuilder::new(&account);
        for (metric, start) in [(&b"a"[..], 0), (&b"b"[..], 2)] {
            for kind in [
                AnnotationKind::HistogramCounterResetCollision(HistogramOperation::Aggregation),
                AnnotationKind::MismatchedCustomBuckets(HistogramOperation::Aggregation),
                AnnotationKind::MixedExponentialCustomHistograms,
            ] {
                builder.add(
                    kind,
                    metric,
                    Span {
                        start,
                        end: start + 1,
                    },
                    b"a+b",
                    &account,
                );
            }
        }
        let result = builder.finish();
        let notes: Vec<_> = result.iter().collect();
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].severity, Severity::Warning);
        assert_eq!(
            notes[0].message,
            b"PromQL warning: conflicting counter resets during histogram aggregation (1:3)"
        );
        assert_eq!(notes[1].severity, Severity::Info);
        assert_eq!(
            notes[1].message,
            b"PromQL info: mismatched custom buckets were reconciled during aggregation (1:3)"
        );
        assert!(notes[2].message.ends_with(b"metric name \"a\" (1:1)"));
        assert!(notes[3].message.ends_with(b"metric name \"b\" (1:3)"));
        drop(result);
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }
    #[test]
    fn raw_dedup_keeps_last_generic_position_and_first_overlap_position() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = b"a\n\xc3\xa9b";
        let mut first = AnnotationBuilder::new(&account);
        let mut second = AnnotationBuilder::new(&account);
        for kind in [
            AnnotationKind::PossibleNonCounter,
            AnnotationKind::StartTimeOverlap,
        ] {
            first.add(kind, b"m\xff\n", Span { start: 0, end: 1 }, query, &account);
            second.add(kind, b"m\xff\n", Span { start: 4, end: 5 }, query, &account);
        }
        first.merge(second, query);
        let before_finish = account.snapshot().memory[MemoryKind::Result as usize].current;
        let result = first.finish();
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            before_finish + size_of::<crate::shared::Allocation<AccountedVec<Annotation>>>() as u64
        );
        let before_clone = account.snapshot();
        let retained = result.clone();
        assert_eq!(account.snapshot(), before_clone);
        assert_eq!(result.len(), 2);
        let generic = result
            .iter()
            .find(|entry| entry.kind == AnnotationKind::PossibleNonCounter)
            .unwrap();
        assert!(generic.message.ends_with(b"\"m\\xff\\n\" (2:3)"));
        let overlap = result
            .iter()
            .find(|entry| entry.kind == AnnotationKind::StartTimeOverlap)
            .unwrap();
        assert_eq!(overlap.occurrences, 2);
        assert!(overlap.message.ends_with(b"(2 occurrences) (1:1)"));
        let held = account.snapshot().memory[MemoryKind::Result as usize].current;
        assert!(held > 0);
        drop(result);
        drop(account);
        assert_eq!(
            engine.snapshot().memory[MemoryKind::Result as usize].current,
            held
        );
        drop(retained);
        assert_eq!(
            engine.snapshot().memory[MemoryKind::Result as usize].current,
            0
        );
    }
}
