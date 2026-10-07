// SPDX-License-Identifier: GPL-3.0-or-later
// Logical emission, merge and position rules derive from pinned util/annotations; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    float_text::{FloatTextStyle, float_text},
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
    IncompatibleBinaryLayout(tokens::TokenKind),
    InvalidQuantile,
    BadBucketLabel,
    MixedClassicNativeHistograms,
    HistogramQuantileForcedMonotonicity,
    NativeHistogramQuantileNaNResult,
    NativeHistogramQuantileNaNSkew,
    NativeHistogramFractionNaNs,
}
impl AnnotationKind {
    fn severity(self) -> Severity {
        match self {
            Self::Provider(severity) => severity,
            Self::PossibleNonCounter
            | Self::HistogramIgnoredInMixedRange
            | Self::HistogramIgnoredInAggregation(_)
            | Self::MismatchedCustomBuckets(_)
            | Self::IncompatibleBinaryTypes { .. }
            | Self::HistogramQuantileForcedMonotonicity
            | Self::NativeHistogramQuantileNaNResult
            | Self::NativeHistogramQuantileNaNSkew
            | Self::NativeHistogramFractionNaNs => Severity::Info,
            Self::MixedFloatsHistograms
            | Self::NativeHistogramNotCounter
            | Self::NativeHistogramNotGauge
            | Self::MixedFloatsHistogramsAggregation
            | Self::StartTimeOverlap
            | Self::MixedExponentialCustomHistograms
            | Self::HistogramCounterResetCollision(_)
            | Self::IncompatibleBinaryLayout(_)
            | Self::InvalidQuantile
            | Self::BadBucketLabel
            | Self::MixedClassicNativeHistograms => Severity::Warning,
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
            Self::IncompatibleBinaryLayout(_) => {
                "PromQL warning: incompatible bucket layout encountered for binary operator "
            }
            Self::InvalidQuantile => "PromQL warning: quantile value should be between 0 and 1",
            Self::BadBucketLabel => {
                "PromQL warning: bucket label \"le\" is missing or has a malformed value"
            }
            Self::MixedClassicNativeHistograms => {
                "PromQL warning: vector contains a mix of classic and native histograms"
            }
            Self::HistogramQuantileForcedMonotonicity => {
                "PromQL info: input to histogram_quantile needed to be fixed for monotonicity (see https://prometheus.io/docs/prometheus/latest/querying/functions/#histogram_quantile)"
            }
            Self::NativeHistogramQuantileNaNResult => {
                crate::histogram::NativeHistogramInfo::QuantileNaNResult.prefix()
            }
            Self::NativeHistogramQuantileNaNSkew => {
                crate::histogram::NativeHistogramInfo::QuantileNaNSkew.prefix()
            }
            Self::NativeHistogramFractionNaNs => {
                crate::histogram::NativeHistogramInfo::FractionNaNs.prefix()
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
    fn optional_metric_name(self) -> bool {
        matches!(
            self,
            Self::MixedClassicNativeHistograms
                | Self::NativeHistogramQuantileNaNResult
                | Self::NativeHistogramQuantileNaNSkew
                | Self::NativeHistogramFractionNaNs
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Monotonicity {
    min_ts: i64,
    max_ts: i64,
    min_bucket: f64,
    max_bucket: f64,
    max_diff: f64,
}
impl Monotonicity {
    pub(crate) fn new(ts: i64, min_bucket: f64, max_bucket: f64, max_diff: f64) -> Self {
        Self {
            min_ts: ts,
            max_ts: ts,
            min_bucket,
            max_bucket,
            max_diff,
        }
    }
    fn merge(&mut self, incoming: &Self) {
        self.min_ts = self.min_ts.min(incoming.min_ts);
        self.max_ts = self.max_ts.max(incoming.max_ts);
        // Strict comparisons retain the old signed-zero and NaN provenance.
        if incoming.min_bucket < self.min_bucket {
            self.min_bucket = incoming.min_bucket;
        }
        if incoming.max_bucket > self.max_bucket {
            self.max_bucket = incoming.max_bucket;
        }
        if incoming.max_diff > self.max_diff {
            self.max_diff = incoming.max_diff;
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
    monotonicity: Option<AccountedBox<Monotonicity>>,
}
impl Annotation {
    fn format(&mut self, query: &[u8]) {
        self.message.clear();
        self.message.extend_from_slice(&self.raw_message);
        if !query.is_empty()
            && let Some(details) = &self.monotonicity
        {
            write!(
                self.message,
                ", from buckets {} to {}, with a max diff of {}, over {} samples from ",
                float_text(details.min_bucket, FloatTextStyle::Shortest),
                float_text(details.max_bucket, FloatTextStyle::Shortest),
                float_text(details.max_diff, FloatTextStyle::TwoSignificant),
                self.occurrences,
            )
            .expect("byte writer");
            crate::calendar::write_rfc3339(details.min_ts / 1000, &mut self.message)
                .expect("byte writer");
            self.message.extend_from_slice(b" to ");
            crate::calendar::write_rfc3339(details.max_ts / 1000, &mut self.message)
                .expect("byte writer");
        }
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
    pub(crate) fn get(&self, index: usize) -> Option<AnnotationRef<'_>> {
        self.entries.as_deref()?.get(index).map(Annotation::as_ref)
    }
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

// The non-matrix call closure and rangeEval can retain the same special error object.
pub(crate) struct CallbackAnnotations {
    values: AnnotationBuilder,
    shared: AccountedVec<bool>,
}
impl CallbackAnnotations {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            values: AnnotationBuilder::new(account),
            shared: AccountedVec::new(account, MemoryKind::Workspace),
        }
    }
    pub(crate) fn merge(
        &mut self,
        local: AnnotationBuilder,
        outer: &mut AnnotationBuilder,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        self.values.merge(local, query);
        while self.shared.len() < self.values.entries.len() {
            control.check()?;
            account.work(1);
            self.shared.push(false);
        }
        for (index, entry) in self.values.entries.iter_mut().enumerate() {
            control.check()?;
            account.work(1);
            let special = matches!(
                entry.kind,
                AnnotationKind::StartTimeOverlap
                    | AnnotationKind::HistogramQuantileForcedMonotonicity
            );
            let mut prior = None;
            for (position, previous) in outer.entries.iter().enumerate() {
                if crate::binary_labels::equal(
                    &entry.raw_message,
                    &previous.raw_message,
                    control,
                    account,
                )? {
                    prior = Some(position);
                    break;
                }
            }
            if special && self.shared[index] {
                entry.occurrences = entry.occurrences.wrapping_mul(2);
                entry.format(query);
                outer.entries[prior.expect("shared callback diagnostic")] =
                    copy_annotation(entry, control, account)?;
            } else {
                outer.merge_one(copy_annotation(entry, control, account)?, query);
                self.shared[index] = special && prior.is_none();
            }
        }
        Ok(())
    }
}

fn copy_annotation(
    source: &Annotation,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Annotation, Interrupted> {
    let mut raw_message = AccountedVec::new(account, MemoryKind::Result);
    let mut message = AccountedVec::new(account, MemoryKind::Result);
    crate::result::append_bytes_checked(&mut raw_message, &source.raw_message, account, control)?;
    crate::result::append_bytes_checked(&mut message, &source.message, account, control)?;
    Ok(Annotation {
        kind: source.kind,
        raw_message,
        message,
        span: source.span,
        occurrences: source.occurrences,
        monotonicity: source.monotonicity.as_ref().map(|details| {
            AccountedBox::new(
                **details,
                &AllocationContext::new(account, MemoryKind::Result),
            )
        }),
    })
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
        } else if let AnnotationKind::IncompatibleBinaryLayout(operator) = kind {
            raw_message
                .extend_from_slice(operator.text().expect("binary operator text").as_bytes());
        } else if let Some(operation) = kind.operation() {
            raw_message.extend_from_slice(operation.name().as_bytes());
        } else if let AnnotationKind::HistogramIgnoredInAggregation(operator) = kind {
            raw_message.extend_from_slice(operator.text().expect("aggregation text").as_bytes());
            raw_message.extend_from_slice(b" aggregation");
        } else if kind.optional_metric_name() {
            write_metric_name(metric, &mut raw_message);
        } else if kind == AnnotationKind::MixedFloatsHistogramsAggregation {
        } else {
            quote::write_bytes(metric, &mut raw_message).expect("byte writer");
        }
        self.add_generated(kind, raw_message, span, query, account, None);
    }
    pub(crate) fn add_invalid_quantile(
        &mut self,
        q: f64,
        span: Span,
        query: &[u8],
        account: &QueryAccounting,
    ) {
        let kind = AnnotationKind::InvalidQuantile;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        write!(
            raw,
            "{}, got {}",
            kind.prefix(),
            float_text(q, FloatTextStyle::Shortest),
        )
        .expect("byte writer");
        self.add_generated(kind, raw, span, query, account, None);
    }
    pub(crate) fn add_bad_bucket(
        &mut self,
        metric: &[u8],
        label: &[u8],
        span: Span,
        query: &[u8],
        account: &QueryAccounting,
    ) {
        let kind = AnnotationKind::BadBucketLabel;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        raw.extend_from_slice(kind.prefix().as_bytes());
        raw.extend_from_slice(b" of ");
        quote::write_bytes(label, &mut raw).expect("byte writer");
        write_metric_name(metric, &mut raw);
        self.add_generated(kind, raw, span, query, account, None);
    }
    pub(crate) fn add_monotonicity(
        &mut self,
        metric: &[u8],
        span: Span,
        details: Monotonicity,
        query: &[u8],
        account: &QueryAccounting,
    ) {
        let kind = AnnotationKind::HistogramQuantileForcedMonotonicity;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        raw.extend_from_slice(kind.prefix().as_bytes());
        write_metric_name(metric, &mut raw);
        self.add_generated(kind, raw, span, query, account, Some(details));
    }
    fn add_generated(
        &mut self,
        kind: AnnotationKind,
        raw_message: AccountedVec<u8>,
        span: Span,
        query: &[u8],
        account: &QueryAccounting,
        details: Option<Monotonicity>,
    ) {
        let mut entry = Annotation {
            kind,
            raw_message,
            span: Some(span),
            occurrences: 1,
            message: AccountedVec::new(account, MemoryKind::Result),
            monotonicity: details.map(|details| {
                AccountedBox::new(
                    details,
                    &AllocationContext::new(account, MemoryKind::Result),
                )
            }),
        };
        entry.format(query);
        self.merge_one(entry, query);
    }
    pub(crate) fn add_provider(
        &mut self,
        diagnostic: crate::storage::ProviderDiagnosticRef<'_>,
        query: &[u8],
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut raw_message = AccountedVec::new(account, MemoryKind::Result);
        let mut message = AccountedVec::new(account, MemoryKind::Result);
        for bytes in diagnostic.message.chunks(1024) {
            if let Some(control) = control {
                control.check()?;
            }
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
                monotonicity: None,
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
            if incoming.kind == self.entries[index].kind
                && matches!(
                    incoming.kind,
                    AnnotationKind::StartTimeOverlap
                        | AnnotationKind::HistogramQuantileForcedMonotonicity
                )
            {
                // Special merges keep the previous object's first position.
                let previous = &mut self.entries[index];
                if let Some(details) = &incoming.monotonicity {
                    previous
                        .monotonicity
                        .as_mut()
                        .expect("monotonicity payload")
                        .merge(details);
                }
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

fn write_metric_name(metric: &[u8], raw: &mut AccountedVec<u8>) {
    if !metric.is_empty() {
        raw.extend_from_slice(b" for metric name ");
        quote::write_bytes(metric, raw).expect("byte writer");
    }
}

#[cfg(feature = "fixtures")]
#[path = "annotations_fixture.rs"]
pub mod fixture;

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
    fn histogram_generic_notes_keep_last_position_and_quoted_bucket_bytes() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut builder = AnnotationBuilder::new(&account);
        for start in [0, 2] {
            let span = Span {
                start,
                end: start + 1,
            };
            builder.add_invalid_quantile(f64::NEG_INFINITY, span, b"a+b", &account);
            builder.add_bad_bucket(b"", b"\xff\n", span, b"a+b", &account);
        }
        let notes = builder.finish();
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|note| note.severity == Severity::Warning));
        assert!(
            notes
                .iter()
                .all(|note| note.span == Some(Span { start: 2, end: 3 }))
        );
        assert!(notes.iter().any(|note| note.message
            == b"PromQL warning: quantile value should be between 0 and 1, got -Inf (1:3)"));
        assert!(notes.iter().any(|note| note.message == b"PromQL warning: bucket label \"le\" is missing or has a malformed value of \"\\xff\\n\" (1:3)"));
        drop(notes);
        account.snapshot().assert_memory_released();
    }
    fn requested_result_bytes(builder: &AnnotationBuilder) -> u64 {
        let entries = builder.entries.capacity() * size_of::<Annotation>();
        let buffers: usize = builder
            .entries
            .iter()
            .map(|entry| {
                entry.raw_message.capacity()
                    + entry.message.capacity()
                    + if entry.monotonicity.is_some() {
                        AccountedBox::<Monotonicity>::allocation_bytes()
                    } else {
                        0
                    }
            })
            .sum();
        (entries + buffers) as u64
    }
    #[test]
    fn monotonicity_keeps_the_original_position_and_precise_payload_owner() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = b"a+b+c";
        let mut builder = AnnotationBuilder::new(&account);
        builder.add_monotonicity(
            b"m",
            Span { start: 0, end: 1 },
            Monotonicity::new(2001, 2.0, 5.0, 3.0),
            query,
            &account,
        );
        let original = &**builder.entries[0].monotonicity.as_ref().unwrap() as *const Monotonicity;
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            requested_result_bytes(&builder)
        );
        let mut child = AnnotationBuilder::new(&account);
        for (start, ts, min, max, diff) in [(2, -1001, 1.0, 4.0, 2.0), (4, 3001, 3.0, 6.0, 5.0)] {
            child.add_monotonicity(
                b"m",
                Span {
                    start,
                    end: start + 1,
                },
                Monotonicity::new(ts, min, max, diff),
                query,
                &account,
            );
        }
        builder.merge(child, query);
        let entry = &builder.entries[0];
        assert_eq!(
            &**entry.monotonicity.as_ref().unwrap() as *const Monotonicity,
            original
        );
        assert_eq!(entry.span, Some(Span { start: 0, end: 1 }));
        assert_eq!(entry.occurrences, 3);
        assert!(entry.message.ends_with(b", from buckets 1 to 6, with a max diff of 5, over 3 samples from 1969-12-31T23:59:59Z to 1970-01-01T00:00:03Z (1:1)"));
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            requested_result_bytes(&builder)
        );
        assert_eq!(
            size_of::<Option<AccountedBox<Monotonicity>>>(),
            size_of::<usize>()
        );
        let result = builder.finish();
        let held = account.snapshot().memory[MemoryKind::Result as usize].current;
        let retained = result.clone();
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            held
        );
        drop(result);
        drop(account);
        assert_eq!(
            engine.snapshot().memory[MemoryKind::Result as usize].current,
            held
        );
        drop(retained);
        engine.snapshot().assert_memory_released();
    }
    #[test]
    fn monotonicity_strict_comparisons_and_empty_query_retain_original_bits() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut builder = AnnotationBuilder::new(&account);
        let payload = f64::from_bits(0x7ff8_0000_0000_0042);
        for details in [
            Monotonicity::new(0, -0.0, payload, -0.0),
            Monotonicity::new(-999, 0.0, 4.0, 0.0),
        ] {
            builder.add_monotonicity(b"", Span { start: 0, end: 1 }, details, b"", &account);
        }
        let entry = &builder.entries[0];
        let details = entry.monotonicity.as_ref().unwrap();
        assert_eq!(details.min_bucket.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(details.max_bucket.to_bits(), payload.to_bits());
        assert_eq!(details.max_diff.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(entry.occurrences, 2);
        assert_eq!(&*entry.message, &*entry.raw_message);
        drop(builder);
        account.snapshot().assert_memory_released();
    }
    #[test]
    fn opaque_replacement_releases_and_restarts_monotonicity_payload() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = AnnotationBuilder::new(&account);
        let query = b"a+b+c";
        builder.add_monotonicity(
            b"m",
            Span { start: 0, end: 1 },
            Monotonicity::new(0, 1.0, 2.0, 1.0),
            query,
            &account,
        );
        let raw = builder.entries[0].raw_message.to_vec();
        builder
            .add_provider(
                crate::storage::ProviderDiagnosticRef {
                    message: &raw,
                    severity: Severity::Warning,
                    position: None,
                },
                query,
                Some(&control),
                &account,
            )
            .unwrap();
        assert!(builder.entries[0].monotonicity.is_none());
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            requested_result_bytes(&builder)
        );
        builder.add_monotonicity(
            b"m",
            Span { start: 4, end: 5 },
            Monotonicity::new(3000, 2.0, 4.0, 2.0),
            query,
            &account,
        );
        assert_eq!(builder.entries[0].span, Some(Span { start: 4, end: 5 }));
        assert_eq!(builder.entries[0].occurrences, 1);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            requested_result_bytes(&builder)
        );
        drop(builder);
        account.snapshot().assert_memory_released();
    }
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
                Some(&control),
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
        account.snapshot().assert_memory_released();
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
        account.snapshot().assert_memory_released();
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
