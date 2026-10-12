// SPDX-License-Identifier: GPL-3.0-or-later
//! AST-site selections and retained metadata; raw values remain cursor/page scoped.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    evaluator::{EvaluationError, EvaluationOptions},
    frontend::ast::{Expr, Matcher, NodeId, ParsedExpression},
    query_bounds,
    query_preparation::Prepared,
    storage::{
        Control, QueryProvider, ResourceOwner, SelectionRequest, Series, SeriesSelection,
        StorageError, matches_checked, validate_labels_checked,
    },
};
use std::cell::RefCell;

type SelectedSeries<'q, Q> = <<Q as QueryProvider>::Selection<'q> as SeriesSelection>::Series;
struct Site<'q, Q: QueryProvider + 'q> {
    // Retained descendants release before the provider selection and query.
    series: Option<AccountedVec<ResourceOwner<SelectedSeries<'q, Q>>>>,
    selection: Option<ResourceOwner<Q::Selection<'q>>>,
    failure: Option<AccountedVec<u8>>,
}
pub(crate) struct Catalog<'q, Q: QueryProvider + 'q> {
    expression: &'q ParsedExpression,
    query: &'q Q,
    sites: RefCell<AccountedVec<Option<Site<'q, Q>>>>,
    info_sites: RefCell<AccountedVec<Site<'q, Q>>>,
}
#[derive(Clone, Copy)]
pub(crate) struct InfoSite(usize);
#[derive(Clone, Copy)]
enum ExpansionMode {
    Ordinary,
    Info,
}
pub(crate) struct ExpansionFailure {
    pub(crate) error: EvaluationError,
    pub(crate) annotations: AnnotationBuilder,
}

impl<'q, Q: QueryProvider + 'q> Catalog<'q, Q> {
    pub(crate) fn new(
        query: &'q Q,
        expression: &'q ParsedExpression,
        prepared: &Prepared,
        options: EvaluationOptions,
        no_step_interval_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let requests = query_bounds::selection_sites(
            expression,
            options,
            prepared,
            no_step_interval_ms,
            control,
            account,
        )?;
        let mut sites = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in &expression.arena.nodes {
            control.check()?;
            account.work(1);
            sites.push(None);
        }
        for request in requests {
            control.check()?;
            account.provider_call();
            let Expr::Selector(selector) = &expression.arena[request.node].expression else {
                unreachable!("selection site")
            };
            let selection = query
                .select(
                    SelectionRequest {
                        matchers: &selector.matchers,
                        bounds: request.bounds,
                        hints: request.hints,
                        accounting: Some(account),
                    },
                    control,
                )
                .map_err(|error| error.accounted(account));
            let (selection, failure) = match selection {
                Ok(value) => {
                    let resources = value.resources();
                    (Some(ResourceOwner::new(value, resources, account)), None)
                }
                Err(StorageError::Provider(bytes)) => (None, Some(bytes)),
                Err(error) => return Err(error.into()),
            };
            sites[request.node.0] = Some(Site {
                series: None,
                selection,
                failure,
            });
        }
        Ok(Self {
            expression,
            query,
            sites: RefCell::new(sites),
            info_sites: RefCell::new(AccountedVec::new(account, MemoryKind::Workspace)),
        })
    }

    pub(crate) fn expand(
        &self,
        node: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AnnotationBuilder, ExpansionFailure> {
        let mut sites = self.sites.borrow_mut();
        let site = sites[node.0].as_mut().expect("upfront selector site");
        let Expr::Selector(selector) = &self.expression.arena[node].expression else {
            unreachable!("upfront selector site")
        };
        self.expand_site(
            site,
            ExpansionMode::Ordinary,
            Some(&selector.matchers),
            control,
            account,
        )
    }

    pub(crate) fn select_info(
        &self,
        request: SelectionRequest<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(InfoSite, AnnotationBuilder), EvaluationError> {
        control.check()?;
        account.provider_call();
        let selection = self
            .query
            .select(request, control)
            .map_err(|error| error.accounted(account))?;
        let resources = selection.resources();
        let mut site = Site {
            series: None,
            selection: Some(ResourceOwner::new(selection, resources, account)),
            failure: None,
        };
        let annotations = self
            .expand_site(
                &mut site,
                ExpansionMode::Info,
                Some(request.matchers),
                control,
                account,
            )
            .map_err(|failure| failure.error)?;
        let mut sites = self.info_sites.borrow_mut();
        let id = InfoSite(sites.len());
        sites.push(site);
        Ok((id, annotations))
    }

    fn expand_site(
        &self,
        site: &mut Site<'q, Q>,
        mode: ExpansionMode,
        matchers: Option<&[Option<Matcher>]>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<AnnotationBuilder, ExpansionFailure> {
        let mut annotations = AnnotationBuilder::new(account);
        if site
            .series
            .as_ref()
            .is_some_and(|series| !series.is_empty())
        {
            return Ok(annotations);
        }
        if let Some(message) = site.failure.take() {
            return Err(ExpansionFailure {
                error: expansion_error(message, mode, account),
                annotations,
            });
        }
        let selection = site.selection.as_mut().expect("live selector site");
        let mut series = AccountedVec::new(account, MemoryKind::Workspace);
        let mut failure = None;
        loop {
            if let Err(error) = control.check() {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations,
                });
            }
            account.provider_call();
            let result = selection
                .value
                .next(control)
                .map_err(|error| error.accounted(account));
            selection.lease.update(selection.value.resources());
            match result {
                Ok(Some(value)) => {
                    let resources = value.resources();
                    let value = ResourceOwner::new(value, resources, account);
                    if let Err(error) = control.check() {
                        return Err(ExpansionFailure {
                            error: error.into(),
                            annotations,
                        });
                    }
                    if let Err(error) = value
                        .value
                        .label_views_checked(account, control)
                        .map_err(StorageError::from)
                        .and_then(|labels| validate_labels_checked(&labels, account, control))
                    {
                        return Err(ExpansionFailure {
                            error: error.into(),
                            annotations,
                        });
                    }
                    if let Some(matchers) = matchers {
                        let result = value
                            .value
                            .label_views_checked(account, control)
                            .map_err(EvaluationError::from)
                            .and_then(|labels| {
                                matches_checked(&labels, matchers, Some(account), control)
                                    .map_err(EvaluationError::from)
                            });
                        match result {
                            Ok(false) => continue,
                            Ok(true) => (),
                            Err(error) => return Err(ExpansionFailure { error, annotations }),
                        }
                    }
                    series.push(value);
                }
                Ok(None) => break,
                Err(StorageError::Provider(bytes)) => {
                    failure = Some(bytes);
                    break;
                }
                Err(error) => {
                    return Err(ExpansionFailure {
                        error: error.into(),
                        annotations,
                    });
                }
            }
        }
        if failure.is_none()
            && let Err(error) = control.check()
        {
            return Err(ExpansionFailure {
                error: error.into(),
                annotations,
            });
        }
        account.provider_call();
        let count = selection.value.diagnostic_count();
        selection.lease.update(selection.value.resources());
        for index in 0..count {
            if failure.is_none()
                && let Err(error) = control.check()
            {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations,
                });
            }
            account.work(1);
            account.provider_call();
            let result = selection.value.diagnostic(index, |diagnostic| {
                let _view = account.hold(
                    MemoryKind::BorrowedView,
                    (size_of_val(&diagnostic) + diagnostic.message.len()) as u64,
                );
                annotations.add_provider(
                    diagnostic,
                    &self.expression.source,
                    failure.is_none().then_some(control),
                    account,
                )
            });
            selection.lease.update(selection.value.resources());
            if let Err(error) = result {
                return Err(ExpansionFailure {
                    error: error.into(),
                    annotations: AnnotationBuilder::new(account),
                });
            }
        }
        if let Some(message) = failure {
            return Err(ExpansionFailure {
                error: expansion_error(message, mode, account),
                annotations,
            });
        }
        site.series = Some(series);
        Ok(annotations)
    }

    pub(crate) fn len(&self, node: NodeId) -> usize {
        self.sites.borrow()[node.0]
            .as_ref()
            .expect("selector site")
            .series
            .as_ref()
            .expect("expanded selector site")
            .len()
    }
    /// Backing factories do not borrow selector tables across a nested replay.
    pub(crate) fn query(&self) -> &'q Q {
        self.query
    }
    pub(crate) fn with_series<R>(
        &self,
        node: NodeId,
        ordinal: usize,
        visit: impl for<'s> FnOnce(&'s SelectedSeries<'q, Q>) -> R,
    ) -> Option<R> {
        let sites = self.sites.borrow();
        let series = sites[node.0]
            .as_ref()
            .expect("selector site")
            .series
            .as_ref()
            .expect("expanded selector site");
        series.get(ordinal).map(|series| visit(&series.value))
    }

    pub(crate) fn info_len(&self, site: InfoSite) -> usize {
        self.info_sites.borrow()[site.0]
            .series
            .as_ref()
            .expect("expanded info site")
            .len()
    }

    pub(crate) fn with_info_series<R>(
        &self,
        site: InfoSite,
        ordinal: usize,
        visit: impl for<'s> FnOnce(&'s SelectedSeries<'q, Q>) -> R,
    ) -> Option<R> {
        let sites = self.info_sites.borrow();
        sites[site.0]
            .series
            .as_ref()
            .expect("expanded info site")
            .get(ordinal)
            .map(|series| visit(&series.value))
    }
}

fn expansion_error(
    message: AccountedVec<u8>,
    mode: ExpansionMode,
    account: &QueryAccounting,
) -> EvaluationError {
    if matches!(mode, ExpansionMode::Info) {
        return StorageError::Provider(message).into();
    }
    let mut output = AccountedVec::new(account, MemoryKind::Result);
    output.extend_from_slice(b"expanding series: ");
    for bytes in message.chunks(1024) {
        // Formatting an already returned failure cannot replace it with later cancellation.
        account.work(bytes.len() as u64);
        output.extend_from_slice(bytes);
    }
    EvaluationError::Expansion(output)
}
