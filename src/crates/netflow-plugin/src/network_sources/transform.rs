use super::types::CompiledTransform;
use super::*;
use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
use jaq_json::Val as JaqVal;

pub(super) fn compile_transform(expression: &str) -> Result<CompiledTransform> {
    let normalized = if expression.trim().is_empty() {
        ".".to_string()
    } else {
        expression.trim().to_string()
    };

    let filter = compile_jaq_filter(&normalized)?;

    Ok(CompiledTransform {
        expression: normalized,
        filter,
    })
}

pub(super) fn run_transform(payload: Value, transform: &CompiledTransform) -> Result<Vec<Value>> {
    let input: JaqVal =
        serde_json::from_value(payload).context("failed to convert JSON payload to jaq value")?;
    let ctx = Ctx::<data::JustLut<JaqVal>>::new(&transform.filter.lut, Vars::new([]));
    let mut output = transform.filter.id.run((ctx, input)).map(unwrap_valr);
    let mut rows = Vec::new();
    while let Some(next) = output.next() {
        let value = next.map_err(|err| {
            anyhow::anyhow!(
                "failed to execute transform '{}': {}",
                transform.expression,
                err
            )
        })?;
        rows.push(jaq_value_to_json(value).with_context(|| {
            format!(
                "failed to convert transform '{}' result to JSON",
                transform.expression
            )
        })?);
    }
    if rows.is_empty() {
        anyhow::bail!("transform '{}' produced empty result", transform.expression);
    }
    Ok(rows)
}

fn compile_jaq_filter(
    expression: &str,
) -> Result<jaq_core::Filter<jaq_core::data::JustLut<JaqVal>>> {
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(
            &arena,
            File {
                code: expression,
                path: (),
            },
        )
        .map_err(|errs| {
            anyhow::anyhow!("failed to parse transform '{}': {:?}", expression, errs)
        })?;

    Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errs| anyhow::anyhow!("failed to compile transform '{}': {:?}", expression, errs))
}

fn jaq_value_to_json(value: JaqVal) -> Result<Value> {
    serde_json::from_str(&value.to_string()).context("jaq value is not representable as JSON")
}
