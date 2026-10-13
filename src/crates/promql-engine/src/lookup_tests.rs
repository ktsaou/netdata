// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{result::OwnedValue, test_storage::FixtureProvider};

fn cardinality_work(expression: &[u8]) -> [u64; 2] {
    let mut work = [0; 2];
    for (position, count) in [128, 512].into_iter().enumerate() {
        let engine = Engine::default();
        let control = Control::default();
        let provider = FixtureProvider::new(1, 17, count);
        let query = engine
            .compile(expression, EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
        let result = report.result.as_ref().unwrap();
        assert_eq!(result.series.len(), count);
        let mut instances = std::collections::BTreeSet::new();
        for series in &*result.series {
            let instance = series
                .labels
                .0
                .iter()
                .find(|label| &*label.name == b"instance")
                .unwrap();
            assert!(instances.insert(instance.value.to_vec()));
            assert_eq!(series.points.len(), 1);
            assert_eq!(series.points[0].time_ms, 1000);
            let OwnedValue::Float(value) = series.points[0].value else {
                panic!("cardinality control must emit floats");
            };
            assert_eq!(value.to_bits(), 1.0f64.to_bits());
        }
        assert_eq!(instances.len(), count);
        work[position] = report.observations.work;
        provider.assert_closed();
        drop((report, query));
        engine.accounting.snapshot().assert_memory_released();
    }
    work
}

#[test]
fn grouped_sum_lookup_work_avoids_all_pairs() {
    let work = cardinality_work(b"sum by (instance) (counter_total)");
    assert!(work[1] <= work[0] * 6, "4x rows used work {work:?}");
}

#[test]
fn binary_matching_lookup_work_avoids_all_pairs() {
    let work = cardinality_work(b"counter_total / on (instance) counter_total");
    assert!(work[1] <= work[0] * 6, "4x rows used work {work:?}");
}
