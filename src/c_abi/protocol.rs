// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{accounting::AccountedVec, frontend::http, query_time::Timestamp};
use std::fmt::Write;

pub(super) enum Error {
    Parse(AccountedVec<u8>),
    Interrupted(Interrupted),
}
pub(super) fn status(value: &Result<(), Error>) -> u32 {
    match value {
        Ok(()) => types::OK,
        Err(Error::Parse(_)) => types::PARSE_ERROR,
        Err(Error::Interrupted(value)) => interrupted_status(*value),
    }
}

unsafe fn parse<T: Copy + Default>(
    raw_engine: *const types::Engine,
    source: types::Bytes,
    raw_control: *const types::Control,
    out: *mut T,
    out_report: *mut *mut types::Report,
    operation: impl FnOnce(
        &[u8],
        &QueryAccounting,
        &CoreControl,
    ) -> Result<Result<T, AccountedVec<u8>>, Interrupted>,
) -> Result<u32, Invalid> {
    checked::distinct_outputs(out, out_report)?;
    unsafe {
        out.write(T::default());
        out_report.write(std::ptr::null_mut());
    }
    let engine = unsafe { borrow::<Engine, _>(raw_engine) }?;
    let source = unsafe { checked::bytes(source) }?;
    let control = unsafe { control(raw_control, &engine.core.accounting) }?;
    let account = engine.core.accounting.query();
    let mut report = Report::new(&account, true);
    let timing = account.compilation();
    let observation = control.observe(&account);
    let value = match operation(source, &account, &control) {
        Ok(Ok(value)) => {
            unsafe { out.write(value) };
            Ok(())
        }
        Ok(Err(error)) => Err(Error::Parse(error)),
        Err(error) => Err(Error::Interrupted(error)),
    };
    drop(observation);
    drop(timing);
    let status = status(&value);
    let slot = report.get_mut().expect("unpublished protocol report");
    slot.outcome = Outcome::Protocol(value);
    slot.observations = account.snapshot();
    unsafe { out_report.write(report.into_raw().cast()) };
    Ok(status)
}

impl Write for types::NumericText {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let begin = self.len as usize;
        let end = begin.checked_add(value.len()).ok_or(std::fmt::Error)?;
        let target = self.bytes.get_mut(begin..end).ok_or(std::fmt::Error)?;
        target.copy_from_slice(value.as_bytes());
        self.len = end as u32;
        Ok(())
    }
}

export! {
    fn netdata_promql_api_parse_time(engine: *const types::Engine, source: types::Bytes, raw_control: *const types::Control, out: *mut types::Timestamp, out_report: *mut *mut types::Report) {
        unsafe { parse(engine, source, raw_control, out, out_report, |source, account, control| {
            Ok(http::parse_time(source, account, control)?.map(|Timestamp { unix_seconds, nanoseconds }| types::Timestamp { unix_seconds, nanoseconds, reserved: 0 }))
        }) }
    }
    fn netdata_promql_api_parse_duration(engine: *const types::Engine, source: types::Bytes, raw_control: *const types::Control, out: *mut i64, out_report: *mut *mut types::Report) {
        unsafe { parse(engine, source, raw_control, out, out_report, http::parse_duration) }
    }
    fn netdata_promql_api_parse_limit(engine: *const types::Engine, source: types::Bytes, raw_control: *const types::Control, out: *mut i64, out_report: *mut *mut types::Report) {
        unsafe { parse(engine, source, raw_control, out, out_report, http::parse_limit) }
    }
    fn netdata_promql_api_float_text(value_bits: u64, out: *mut types::NumericText) {
        unsafe { checked::output(out) }?;
        let text = crate::float_text::float_text(f64::from_bits(value_bits), crate::float_text::FloatTextStyle::PrometheusApi);
        let mut value = types::NumericText::default();
        value.bytes[..text.as_bytes().len()].copy_from_slice(text.as_bytes());
        value.len = text.as_bytes().len() as u32;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_api_scalar_float_text(value_bits: u64, out: *mut types::NumericText) {
        unsafe { checked::output(out) }?;
        let mut value = types::NumericText::default();
        value.len = crate::float_text::fixed_text_into(f64::from_bits(value_bits), &mut value.bytes).expect("fixed float capacity") as u32;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_api_scalar_timestamp_text(time_ms: i64, out: *mut types::NumericText) {
        unsafe { checked::output(out) }?;
        let text = crate::float_text::float_text(time_ms as f64 / 1000.0, crate::float_text::FloatTextStyle::PrometheusApi);
        let mut value = types::NumericText::default();
        value.bytes[..text.as_bytes().len()].copy_from_slice(text.as_bytes());
        value.len = text.as_bytes().len() as u32;
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_api_timestamp_text(time_ms: i64, out: *mut types::NumericText) {
        unsafe { checked::output(out) }?;
        let mut text = types::NumericText::default();
        let time = if time_ms < 0 {
            text.write_str("-").expect("fixed timestamp capacity");
            time_ms.wrapping_neg()
        } else { time_ms };
        write!(&mut text, "{}", time / 1000).expect("fixed timestamp capacity");
        let fraction = time % 1000;
        if fraction != 0 {
            text.write_str(".").expect("fixed timestamp capacity");
            if fraction < 100 { text.write_str("0").expect("fixed timestamp capacity"); }
            if fraction < 10 { text.write_str("0").expect("fixed timestamp capacity"); }
            write!(&mut text, "{fraction}").expect("fixed timestamp capacity");
        }
        unsafe { out.write(text) };
        Ok(types::OK)
    }
}
