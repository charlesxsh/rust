use std::{
    collections::{HashMap, HashSet, VecDeque},
    error::Error,
};

use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_middle::{
    mir::{visit::Visitor, Body, Operand, Rvalue, StatementKind},
    ty::TyCtxt,
};
use serde::{Deserialize, Serialize};

use crate::utils::{
    alias::{BodyAliasResult, BodyAliasVisitor},
    callgraph::{callgraph_analysis, CallGraph},
    range::span_str,
    spread::UnsafeSpreadAnalysis,
};

use super::{find_main_func, AnalysisOption};
use super::{ty::RFAnalysis, NoEntryFound};

#[derive(Deserialize, Serialize)]
struct UnsafeSpreadRFAnalysisResult {
    fns: Vec<UnsafeSpansWithFuncContext>,
}

#[derive(Deserialize, Serialize)]
struct UnsafeSpansWithFuncContext {
    fn_name: Option<String>,
    def_span: Option<String>,
    unsafe_args: Vec<usize>,
    spans: Vec<String>,
    // call span, callee_def_span, unsafe_args
    calls_with_unsafe_args: Vec<(String, String, Vec<usize>)>,
}

pub struct UnsafeSpreadRFAnalysis {}

impl RFAnalysis for UnsafeSpreadRFAnalysis {
    fn analyze<'tcx, 'a>(
        &self,
        tcx: TyCtxt<'tcx>,
        option: &'a AnalysisOption,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut callgraph = CallGraph::new();
        let mut body_alias_results: HashMap<DefId, BodyAliasResult<'tcx>> = HashMap::new();

        let mut entry_func_def_id: Option<DefId> = find_main_func(tcx);

        match entry_func_def_id {
            Some(entry_id) => {
                let body = tcx.optimized_mir(entry_id);
                callgraph_analysis(tcx, body, &mut callgraph, true);

                // alias analysis
                for fn_id in &callgraph.all_funcs {
                    let body = tcx.optimized_mir(*fn_id);

                    let mut visiter = BodyAliasVisitor::new(tcx, body, false);

                    visiter.visit_body(body);

                    body_alias_results.insert(*fn_id, visiter.get_result());
                }

                let analysis = UnsafeSpreadAnalysis::new(tcx, &body_alias_results, &callgraph);
                for fn_id in &callgraph.all_funcs {
                    analysis.analyze(*fn_id, vec![]);
                }
                let result = unsafe_spread_analysis_to_result(tcx, &analysis);
                return serde_json::to_value(&result)
                    .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
            }
            None => return Err(Box::new(NoEntryFound {})),
        }
    }
}

fn unsafe_spread_analysis_to_result<'tcx>(
    tcx: TyCtxt<'tcx>,
    analysis: &UnsafeSpreadAnalysis,
) -> UnsafeSpreadRFAnalysisResult {
    let mut result = UnsafeSpreadRFAnalysisResult { fns: Vec::new() };
    for (func_id, func_results) in analysis.results_cache.borrow().iter() {
        let body: &Body = tcx.optimized_mir(*func_id);
        let fn_name = match tcx.opt_item_name(*func_id) {
            Some(item) => Some(item.as_str().to_string()),
            None => None,
        };

        for (unsafe_args, func_result) in func_results {
            let mut func_ctx = UnsafeSpansWithFuncContext {
                fn_name: fn_name.clone(),
                def_span: span_str(&body.span, true),
                unsafe_args: unsafe_args.clone(),
                spans: Vec::new(),
                calls_with_unsafe_args: Vec::new(),
            };

            for (bb, bb_data) in body.basic_blocks().iter_enumerated() {
                for stmt in &bb_data.statements {
                    let mut unsafe_local_involved: bool = match stmt.kind {
                        StatementKind::Assign(ref data) => {
                            let lhs = data.0;
                            let rhs = &data.1;
                            if func_result.unsafe_locals.contains(&lhs.local) {
                                true
                            } else {
                                match rhs {
                                    Rvalue::Use(op) => match op {
                                        Operand::Copy(p) => {
                                            func_result.unsafe_locals.contains(&p.local)
                                        }

                                        Operand::Move(p) => {
                                            func_result.unsafe_locals.contains(&p.local)
                                        }
                                        Operand::Constant(_) => false,
                                    },
                                    Rvalue::Ref(_, _, p) => false,
                                    _ => false,
                                }
                            }
                        }
                        _ => false,
                    };
                    if unsafe_local_involved {
                        let stmt_span_str = span_str(&stmt.source_info.span, true).unwrap();
                        func_ctx.spans.push(stmt_span_str);
                    }
                }
            }

            for call in &func_result.callsites_with_unsafe_args {
                let callee_span = &tcx.optimized_mir(call.callsite.callee.def_id()).span;
                func_ctx.calls_with_unsafe_args.push((
                    span_str(&call.callsite.span, true).unwrap(),
                    span_str(callee_span, true).unwrap(),
                    call.unsafe_arg_idxs.clone(),
                ))
            }

            result.fns.push(func_ctx);
        }
    }
    return result;
}
