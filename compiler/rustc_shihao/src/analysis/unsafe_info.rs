use std::error::Error;

use rustc_hir::{
    def::DefKind,
    def_id::{DefId, LocalDefId},
};
use rustc_middle::{
    mir::{visit::Visitor, Body, ClearCrossCrate, Safety},
    ty::TyCtxt,
};
use serde::{Deserialize, Serialize};

use crate::utils::range::span_str;

use super::AnalysisOption;
use super::{find_main_func, ty::RFAnalysis};
use crate::utils::callgraph::{callgraph_analysis, CallGraph};
use tracing::{debug, error, info};

use std::collections::{HashSet, VecDeque};

#[derive(Deserialize, Serialize)]
struct UnsafeInfoRFAnalysisResult {
    spans: Vec<String>,
}
pub struct UnsafeInfoRFAnalysis {}

impl RFAnalysis for UnsafeInfoRFAnalysis {
    fn analyze<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        option: &'_ AnalysisOption,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut result = UnsafeInfoRFAnalysisResult { spans: Vec::new() };
        let fn_ids: Vec<LocalDefId> = tcx
            .mir_keys(())
            .iter()
            .filter(|id| {
                let hir = tcx.hir();
                hir.body_owner_kind(**id).is_fn_or_closure()
            })
            .copied()
            .collect();
        let mut mode = "local";
        if let Some(m) = &option.unsafe_info_mode {
            mode = m;
        }

        match mode {
            "local" => {
                // iterate all local functions
                fn_ids.into_iter().for_each(|fn_id| {
                    let body = tcx.optimized_mir(fn_id);
                    let mut visitor = UnsafeInfoVisitor { tcx, unsafe_spans: Vec::new(), body };
                    visitor.visit_body(body);
                    result.spans.append(&mut visitor.unsafe_spans);
                });
            }
            "callgraph" => {
                let mut entry_func_def_id: Option<DefId> = find_main_func(tcx);
                let mut callgraph = CallGraph::new();

                // iterate all local functions
                fn_ids.into_iter().for_each(|fn_id| {
                    let body = tcx.optimized_mir(fn_id);
                    callgraph_analysis(tcx, body, &mut callgraph, true);
                });
                match entry_func_def_id {
                    Some(entry_id) => {
                        let body = tcx.optimized_mir(entry_id);
                        callgraph_analysis(tcx, body, &mut callgraph, true);
                        for fn_id in &callgraph.all_funcs {
                            let item_name = tcx.opt_item_name(*fn_id);
                            match item_name {
                                Some(n) => {
                                    info!("visiting {:?}", item_name);
                                }
                                None => {
                                    info!("visiting {:?} without item name", fn_id);
                                }
                            }

                            let body = tcx.optimized_mir(*fn_id);
                            let mut visitor =
                                UnsafeInfoVisitor { tcx, unsafe_spans: Vec::new(), body };
                            visitor.visit_body(body);
                            result.spans.append(&mut visitor.unsafe_spans);
                        }
                    }
                    None => {
                        error!("No Entry Point Found");
                    }
                }
            }
            _ => {}
        }

        return serde_json::to_value(&result)
            .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}

struct UnsafeInfoVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    unsafe_spans: Vec<String>,
}

impl<'tcx> Visitor<'tcx> for UnsafeInfoVisitor<'tcx> {
    fn visit_statement(
        &mut self,
        statement: &rustc_middle::mir::Statement<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
        let scope_data = &self.body.source_scopes[statement.source_info.scope];

        if !scope_data.safety {
            if let Some(str) = span_str(&statement.source_info.span, true) {
                self.unsafe_spans.push(str);
            };
        }

        self.super_statement(statement, location)
    }
}
