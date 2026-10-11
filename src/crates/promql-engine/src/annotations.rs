// SPDX-License-Identifier: GPL-3.0-or-later
// Logical emission, merge and position rules derive from pinned util/annotations; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    float_text::{FloatTextStyle, float_text},
    frontend::{Span, token as tokens},
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
    InvalidRatio,
    BadBucketLabel,
    MixedClassicNativeHistograms,
    HistogramQuantileForcedMonotonicity,
    NativeHistogramQuantileNaNResult,
    NativeHistogramQuantileNaNSkew,
    NativeHistogramFractionNaNs,
    SortInRangeQuery,
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
            | Self::InvalidRatio
            | Self::BadBucketLabel
            | Self::MixedClassicNativeHistograms
            | Self::SortInRangeQuery => Severity::Warning,
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
            Self::InvalidRatio => "PromQL warning: ratio value should be between -1 and 1",
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
            Self::SortInRangeQuery => {
                "PromQL warning: sort is ineffective for range queries since results are always ordered by labels"
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
    fn format(
        &mut self,
        query: &[u8],
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut message = AccountedVec::new(account, MemoryKind::Result);
        append(&mut message, &self.raw_message, control, account)?;
        write_with(&mut message, control, account, |message| {
            if !query.is_empty()
                && let Some(details) = &self.monotonicity
            {
                write!(
                    message,
                    ", from buckets {} to {}, with a max diff of {}, over {} samples from ",
                    float_text(details.min_bucket, FloatTextStyle::Shortest),
                    float_text(details.max_bucket, FloatTextStyle::Shortest),
                    float_text(details.max_diff, FloatTextStyle::TwoSignificant),
                    self.occurrences,
                )?;
                crate::calendar::write_rfc3339(details.min_ts / 1000, message)?;
                message.write_str(" to ")?;
                crate::calendar::write_rfc3339(details.max_ts / 1000, message)?;
            }
            if !query.is_empty()
                && self.kind == AnnotationKind::StartTimeOverlap
                && self.occurrences > 1
            {
                write!(message, " ({} occurrences)", self.occurrences)?;
            }
            Ok(())
        })?;
        if let Some(span) = self.span {
            write_position_suffix_with(query, span.start as isize, &mut message, control, account)?;
        }
        if let Some(control) = control {
            control.check()?;
        }
        self.message = message;
        Ok(())
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
        self.values.merge(local, query, Some(control), account)?;
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
                let occurrences = entry.occurrences;
                entry.occurrences = occurrences.wrapping_mul(2);
                if let Err(error) = entry.format(query, Some(control), account) {
                    entry.occurrences = occurrences;
                    return Err(error);
                }
                outer.entries[prior.expect("shared callback diagnostic")] =
                    copy_annotation(entry, control, account)?;
            } else {
                let shares_incoming = special
                    && prior.is_none_or(|position| outer.entries[position].kind != entry.kind);
                outer.merge_one(
                    copy_annotation(entry, control, account)?,
                    query,
                    Some(control),
                    account,
                )?;
                self.shared[index] = shares_incoming;
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
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut raw_message = AccountedVec::new(account, MemoryKind::Result);
        append(
            &mut raw_message,
            kind.prefix().as_bytes(),
            Some(control),
            account,
        )?;
        if let AnnotationKind::IncompatibleBinaryTypes {
            left_histogram,
            operator,
            right_histogram,
        } = kind
        {
            let operation = operator.text().expect("binary operator text");
            crate::binary_labels::quoted(&mut raw_message, operation.as_bytes(), control, account)?;
            let left = if left_histogram { "histogram" } else { "float" };
            let right = if right_histogram {
                "histogram"
            } else {
                "float"
            };
            write_with(&mut raw_message, Some(control), account, |out| {
                write!(out, ": {left} {operation} {right}")
            })?;
        } else if let AnnotationKind::IncompatibleBinaryLayout(operator) = kind {
            append(
                &mut raw_message,
                operator.text().expect("binary operator text").as_bytes(),
                Some(control),
                account,
            )?;
        } else if let Some(operation) = kind.operation() {
            append(
                &mut raw_message,
                operation.name().as_bytes(),
                Some(control),
                account,
            )?;
        } else if let AnnotationKind::HistogramIgnoredInAggregation(operator) = kind {
            append(
                &mut raw_message,
                operator.text().expect("aggregation text").as_bytes(),
                Some(control),
                account,
            )?;
            append(&mut raw_message, b" aggregation", Some(control), account)?;
        } else if kind.optional_metric_name() {
            write_metric_name(metric, &mut raw_message, control, account)?;
        } else if !matches!(
            kind,
            AnnotationKind::MixedFloatsHistogramsAggregation | AnnotationKind::SortInRangeQuery
        ) {
            crate::binary_labels::quoted(&mut raw_message, metric, control, account)?;
        }
        self.add_generated(kind, raw_message, span, query, control, account, None)
    }
    pub(crate) fn add_invalid_quantile(
        &mut self,
        q: f64,
        span: Span,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let kind = AnnotationKind::InvalidQuantile;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        write_with(&mut raw, Some(control), account, |out| {
            write!(
                out,
                "{}, got {}",
                kind.prefix(),
                float_text(q, FloatTextStyle::Shortest)
            )
        })?;
        self.add_generated(kind, raw, span, query, control, account, None)
    }
    pub(crate) fn add_invalid_ratio(
        &mut self,
        ratio: f64,
        capped: f64,
        span: Span,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let kind = AnnotationKind::InvalidRatio;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        write_with(&mut raw, Some(control), account, |out| {
            write!(
                out,
                "{}, got {}, capping to {}",
                kind.prefix(),
                float_text(ratio, FloatTextStyle::Shortest),
                float_text(capped, FloatTextStyle::Shortest)
            )
        })?;
        self.add_generated(kind, raw, span, query, control, account, None)
    }
    pub(crate) fn add_bad_bucket(
        &mut self,
        metric: &[u8],
        label: &[u8],
        span: Span,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let kind = AnnotationKind::BadBucketLabel;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        append(&mut raw, kind.prefix().as_bytes(), Some(control), account)?;
        append(&mut raw, b" of ", Some(control), account)?;
        crate::binary_labels::quoted(&mut raw, label, control, account)?;
        write_metric_name(metric, &mut raw, control, account)?;
        self.add_generated(kind, raw, span, query, control, account, None)
    }
    pub(crate) fn add_monotonicity(
        &mut self,
        metric: &[u8],
        span: Span,
        details: Monotonicity,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let kind = AnnotationKind::HistogramQuantileForcedMonotonicity;
        let mut raw = AccountedVec::new(account, MemoryKind::Result);
        append(&mut raw, kind.prefix().as_bytes(), Some(control), account)?;
        write_metric_name(metric, &mut raw, control, account)?;
        self.add_generated(kind, raw, span, query, control, account, Some(details))
    }
    #[allow(clippy::too_many_arguments)]
    fn add_generated(
        &mut self,
        kind: AnnotationKind,
        raw_message: AccountedVec<u8>,
        span: Span,
        query: &[u8],
        control: &Control,
        account: &QueryAccounting,
        details: Option<Monotonicity>,
    ) -> Result<(), Interrupted> {
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
        entry.format(query, Some(control), account)?;
        self.merge_one(entry, query, Some(control), account)
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
        append(&mut raw_message, diagnostic.message, control, account)?;
        append(&mut message, diagnostic.message, control, account)?;
        if let Some(position) = diagnostic.position {
            write_position_suffix_with(
                query,
                position.start as isize,
                &mut message,
                control,
                account,
            )?;
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
            control,
            account,
        )
    }
    fn merge_one(
        &mut self,
        incoming: Annotation,
        query: &[u8],
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(control) = control {
            control.check()?;
        }
        let mut found = None;
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(control) = control {
                control.check()?;
            }
            account.work(1);
            let equal = if let Some(control) = control {
                crate::binary_labels::equal(
                    &entry.raw_message,
                    &incoming.raw_message,
                    control,
                    account,
                )?
            } else {
                account.work(entry.raw_message.len().min(incoming.raw_message.len()) as u64);
                *entry.raw_message == *incoming.raw_message
            };
            if equal {
                found = Some(index);
                break;
            }
        }
        if let Some(control) = control {
            control.check()?;
        }
        if let Some(index) = found {
            if incoming.kind == self.entries[index].kind
                && matches!(
                    incoming.kind,
                    AnnotationKind::StartTimeOverlap
                        | AnnotationKind::HistogramQuantileForcedMonotonicity
                )
            {
                // Special merges retain the previous object's first position.
                let previous = &mut self.entries[index];
                let occurrences = previous.occurrences;
                let details = previous.monotonicity.as_ref().map(|details| **details);
                if let Some(details) = &incoming.monotonicity {
                    previous
                        .monotonicity
                        .as_mut()
                        .expect("monotonicity payload")
                        .merge(details);
                }
                previous.occurrences = occurrences.wrapping_add(incoming.occurrences);
                if let Err(error) = previous.format(query, control, account) {
                    previous.occurrences = occurrences;
                    if let Some(details) = details {
                        **previous.monotonicity.as_mut().unwrap() = details;
                    }
                    return Err(error);
                }
            } else {
                self.entries[index] = incoming;
            }
        } else {
            self.entries.push(incoming);
        }
        Ok(())
    }
    pub(crate) fn merge(
        &mut self,
        child: Self,
        query: &[u8],
        control: Option<&Control>,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(control) = control {
            control.check()?;
        }
        for entry in child.entries {
            if let Some(control) = control {
                control.check()?;
            }
            account.work(1);
            self.merge_one(entry, query, control, account)?;
        }
        if let Some(control) = control {
            control.check()?;
        }
        Ok(())
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

fn append(
    out: &mut AccountedVec<u8>,
    bytes: &[u8],
    control: Option<&Control>,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    if let Some(control) = control {
        control.check()?;
        crate::result::append_bytes_checked(out, bytes, account, control)?;
        control.check()?;
    } else {
        account.work(bytes.len() as u64);
        out.extend_from_slice(bytes);
    }
    Ok(())
}
fn write_with(
    out: &mut AccountedVec<u8>,
    control: Option<&Control>,
    account: &QueryAccounting,
    write: impl FnOnce(&mut crate::binary_labels::Writer<'_>) -> fmt::Result,
) -> Result<(), Interrupted> {
    crate::binary_labels::write_with_control(out, control, account, write)
}
fn write_metric_name(
    metric: &[u8],
    raw: &mut AccountedVec<u8>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    if !metric.is_empty() {
        append(raw, b" for metric name ", Some(control), account)?;
        crate::binary_labels::quoted(raw, metric, control, account)?;
    }
    Ok(())
}

#[cfg(feature = "fixtures")]
#[path = "annotations_fixture.rs"]
pub mod fixture;

pub(crate) fn write_position_suffix(
    query: &[u8],
    start: isize,
    out: &mut impl Write,
) -> fmt::Result {
    render_position(
        position(query, start, None).expect("uncontrolled position"),
        out,
    )
}
fn position(
    query: &[u8],
    start: isize,
    observation: Option<(&Control, &QueryAccounting)>,
) -> Result<Option<(usize, usize)>, Interrupted> {
    if let Some((control, _)) = observation {
        control.check()?;
    }
    if query.is_empty() {
        return Ok(None);
    }
    if start < 0 || start as usize > query.len() {
        return Ok(Some((0, 0)));
    }
    let start = start as usize;
    let (mut line, mut column) = (1, start + 1);
    for (chunk_index, bytes) in query[..start].chunks(1024).enumerate() {
        if let Some((control, account)) = observation {
            control.check()?;
            account.work(bytes.len() as u64);
        }
        for (index, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                line += 1;
                column = start - (chunk_index * 1024 + index);
            }
        }
    }
    if let Some((control, _)) = observation {
        control.check()?;
    }
    Ok(Some((line, column)))
}
fn render_position(position: Option<(usize, usize)>, out: &mut impl Write) -> fmt::Result {
    match position {
        None => Ok(()),
        Some((0, 0)) => out.write_str(" (invalid position)"),
        Some((line, column)) => write!(out, " ({line}:{column})"),
    }
}
fn write_position_suffix_with(
    query: &[u8],
    start: isize,
    out: &mut AccountedVec<u8>,
    control: Option<&Control>,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    let position = position(query, start, control.map(|control| (control, account)))?;
    write_with(out, control, account, |writer| {
        render_position(position, writer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::EngineAccounting;
    #[test]
    fn cancelled_empty_provider_diagnostic_does_not_publish_a_note() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = AnnotationBuilder::new(&account);
        control.cancellation.cancel();
        let result = builder.add_provider(
            crate::storage::ProviderDiagnosticRef {
                message: b"",
                severity: Severity::Info,
                position: None,
            },
            b"m",
            Some(&control),
            &account,
        );
        assert_eq!(result, Err(Interrupted::Cancelled));
        assert!(builder.entries.is_empty());
        drop(builder);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn callback_diagnostic_merge_stops_during_long_equal_prefix_search() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut local = AnnotationBuilder::new(&account);
        for index in 0..16 {
            let mut metric = vec![b'x'; 16_384];
            metric.extend_from_slice(index.to_string().as_bytes());
            local
                .add(
                    AnnotationKind::NativeHistogramNotCounter,
                    &metric,
                    Span { start: 0, end: 1 },
                    b"m",
                    &control,
                    &account,
                )
                .unwrap();
        }
        let mut callbacks = CallbackAnnotations::new(&account);
        let mut outer = AnnotationBuilder::new(&account);
        let result = crate::test_storage::interrupt_during(&account, &control, || {
            callbacks.merge(local, &mut outer, b"m", &control, &account)
        });
        assert_eq!(result, Err(Interrupted::Cancelled));
        assert!(
            callbacks.values.entries.len() < 16,
            "the unchecked prelude must not merge the entire diagnostic roster"
        );
        assert!(outer.entries.is_empty());
        drop((callbacks, outer));
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn provider_position_scan_honors_control_even_when_message_is_empty() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut builder = AnnotationBuilder::new(&account);
        let query = vec![b' '; 32_768];
        let result = crate::test_storage::interrupt_during(&account, &control, || {
            builder.add_provider(
                crate::storage::ProviderDiagnosticRef {
                    message: b"",
                    severity: Severity::Info,
                    position: Some(Span {
                        start: query.len(),
                        end: query.len(),
                    }),
                },
                &query,
                Some(&control),
                &account,
            )
        });
        assert_eq!(result, Err(Interrupted::Cancelled));
        assert!(builder.entries.is_empty());
        drop(builder);
        account.snapshot().assert_memory_released();
    }

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
            builder
                .add_invalid_quantile(
                    f64::NEG_INFINITY,
                    span,
                    b"a+b",
                    &Control::default(),
                    &account,
                )
                .unwrap();
            builder
                .add_bad_bucket(b"", b"\xff\n", span, b"a+b", &Control::default(), &account)
                .unwrap();
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
        builder
            .add_monotonicity(
                b"m",
                Span { start: 0, end: 1 },
                Monotonicity::new(2001, 2.0, 5.0, 3.0),
                query,
                &Control::default(),
                &account,
            )
            .unwrap();
        let original = &**builder.entries[0].monotonicity.as_ref().unwrap() as *const Monotonicity;
        assert_eq!(
            account.snapshot().memory[MemoryKind::Result as usize].current,
            requested_result_bytes(&builder)
        );
        let mut child = AnnotationBuilder::new(&account);
        for (start, ts, min, max, diff) in [(2, -1001, 1.0, 4.0, 2.0), (4, 3001, 3.0, 6.0, 5.0)] {
            child
                .add_monotonicity(
                    b"m",
                    Span {
                        start,
                        end: start + 1,
                    },
                    Monotonicity::new(ts, min, max, diff),
                    query,
                    &Control::default(),
                    &account,
                )
                .unwrap();
        }
        builder.merge(child, query, None, &account).unwrap();
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
            builder
                .add_monotonicity(
                    b"",
                    Span { start: 0, end: 1 },
                    details,
                    b"",
                    &Control::default(),
                    &account,
                )
                .unwrap();
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
        builder
            .add_monotonicity(
                b"m",
                Span { start: 0, end: 1 },
                Monotonicity::new(0, 1.0, 2.0, 1.0),
                query,
                &Control::default(),
                &account,
            )
            .unwrap();
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
        builder
            .add_monotonicity(
                b"m",
                Span { start: 4, end: 5 },
                Monotonicity::new(3000, 2.0, 4.0, 2.0),
                query,
                &Control::default(),
                &account,
            )
            .unwrap();
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
            builder
                .add(
                    AnnotationKind::StartTimeOverlap,
                    b"m",
                    Span {
                        start,
                        end: start + 1,
                    },
                    query,
                    &Control::default(),
                    &account,
                )
                .unwrap();
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
        builder
            .add(
                AnnotationKind::StartTimeOverlap,
                b"m",
                Span { start: 4, end: 5 },
                query,
                &Control::default(),
                &account,
            )
            .unwrap();
        assert_eq!(builder.entries[0].kind, AnnotationKind::StartTimeOverlap);
        assert_eq!(builder.entries[0].occurrences, 1);
        assert_eq!(builder.entries[0].span, Some(Span { start: 4, end: 5 }));
        builder
            .add(
                AnnotationKind::StartTimeOverlap,
                b"m",
                Span { start: 0, end: 1 },
                query,
                &Control::default(),
                &account,
            )
            .unwrap();
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
                builder
                    .add(
                        kind,
                        metric,
                        Span {
                            start,
                            end: start + 1,
                        },
                        b"a+b",
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
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
            first
                .add(
                    kind,
                    b"m\xff\n",
                    Span { start: 0, end: 1 },
                    query,
                    &Control::default(),
                    &account,
                )
                .unwrap();
            second
                .add(
                    kind,
                    b"m\xff\n",
                    Span { start: 4, end: 5 },
                    query,
                    &Control::default(),
                    &account,
                )
                .unwrap();
        }
        first.merge(second, query, None, &account).unwrap();
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
