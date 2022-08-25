use std::collections::HashMap;

use rustc_hir::def_id::DefId;
use rustc_middle::mir::visit::Visitor;

use crate::utils::{alias::BodyAliasResult, callgraph::{CallGraph, callgraph_analysis}, stack_init::StackInitVisitor};

use super::{ty::RFAnalysis, find_main_func};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
struct SafeStoreAnalysisResult {}
pub struct SafeStoreAnalysis {}

impl RFAnalysis for SafeStoreAnalysis {
    fn analyze<'tcx, 'a>(&self, tcx: rustc_middle::ty::TyCtxt<'tcx>, option:&'a super::AnalysisOption) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut callgraph = CallGraph::new();
        let mut body_alias_results: HashMap<DefId, BodyAliasResult<'tcx>> = HashMap::new();

        let mut entry_func_def_id: Option<DefId> = find_main_func(tcx);

        match entry_func_def_id {
            Some(entry_id) => {
                let body = tcx.optimized_mir(entry_id);
                callgraph_analysis(tcx, body, &mut callgraph, true);
                for fn_id in &callgraph.all_funcs {
                    let body = tcx.optimized_mir(*fn_id);
                    let mut visitor = StackInitVisitor::new(tcx, body);
                    visitor.visit_body(body);

                    tcx.xsh_safe_stores().borrow_mut().insert(*fn_id, visitor.init_locs.into_iter().collect());
                }
                
            },
            None => {}
        }

        let result = SafeStoreAnalysisResult {};
        return serde_json::to_value(&result)
            .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}