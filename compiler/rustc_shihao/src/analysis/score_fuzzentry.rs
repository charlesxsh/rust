use std::collections::{HashMap, VecDeque, HashSet};

use log::error;
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_span::sym::HashSet;
use serde::{Deserialize, Serialize};

use crate::utils::{callgraph::{callgraph_analysis, CallGraph}, mir_body::{get_cnt_of_unsafe, get_cnt_of_ffi_call}};

use super::{ty::RFAnalysis, find_main_func};


#[derive(Deserialize, Serialize, Debug)]
struct ScoreFuzzEntryAnalysisResult {
    // the dangerous pointer calculations
    score: f32,
    total_unsafe_stmts: u32,
    total_ffi_calls: u32,
    fn_details: Vec<(String, u32, u32)>

}

pub struct ScoreFuzzEntryAnalysis {}

impl RFAnalysis for ScoreFuzzEntryAnalysis {

    /**
     * For each function, calculate
     * 
     * 
     */
    
    
    fn analyze<'tcx, 'a>(&self, tcx: rustc_middle::ty::TyCtxt<'tcx>, option:&'a super::AnalysisOption) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let fn_ids: Vec<LocalDefId> = tcx
            .mir_keys(())
            .iter()
            .filter(|id| {
                let hir = tcx.hir();
                hir.body_owner_kind(hir.local_def_id_to_hir_id(**id)).is_fn_or_closure()
            })
            .copied()
            .collect();
        let mut callgraph = CallGraph::new();

        // iterate all local functions
        fn_ids.into_iter().for_each(|fn_id| {
            let body = tcx.optimized_mir(fn_id);
            callgraph_analysis(tcx, body, &mut callgraph, false);
        });
        
        let mut result = ScoreFuzzEntryAnalysisResult { 
            score: 0.0,
            total_unsafe_stmts: 0,
            total_ffi_calls: 0,
            fn_details: Vec::new()
         };

        let mut entry_func_def_id: Option<DefId> = find_main_func(tcx);


        match &entry_func_def_id {
            Some(entry_fn_id) => {
                let body = tcx.optimized_mir(*entry_fn_id);

                let mut fn_cnts: HashMap<DefId, (u32, u32)> = HashMap::new();

                // calculate necessary counts for each function
                for fn_id in &callgraph.all_funcs {
                    if !tcx.is_mir_available(*fn_id) {
                        continue
                    }
                    let body = tcx.optimized_mir(*fn_id);
                    let unsafe_cnt = get_cnt_of_unsafe(tcx, body);
                    let callsites = callgraph.callsites.get(fn_id);
                    let mut unsafe_ffi = 0;
                    if let Some(css) = callsites {
                        unsafe_ffi = get_cnt_of_ffi_call(tcx, &css);
                    }

                    fn_cnts.insert(*fn_id, (unsafe_cnt, unsafe_ffi));

                    result.total_unsafe_stmts += unsafe_cnt;
                    result.total_ffi_calls += unsafe_ffi;

                    if let Some(item_name) = tcx.opt_item_name(*fn_id) {
                        result.fn_details.push((item_name.to_string(), unsafe_cnt, unsafe_ffi));
                    } else {
                        result.fn_details.push((format!("{:?}", body.source.def_id()), unsafe_cnt, unsafe_ffi));

                    }


                }
                let default_hs = HashSet::new();


                // find ratio of each function and times with counts calculated previously
                let fns_ratio = (callgraph.calls.get(entry_fn_id).unwrap_or(&default_hs), 1);
                let mut worklist: VecDeque<(&HashSet<DefId>, i32)> = VecDeque::new();
                worklist.push_back(fns_ratio);

                let mut fnid_ratios: HashMap<DefId, f32> = HashMap::new();

                while let Some(fr) = worklist.pop_front() {
                    for fnid in fr.0 {
                        if !fnid_ratios.contains_key(fnid) {
                            fnid_ratios.insert(*fnid, fr.1 as f32);
                            worklist.push_back((callgraph.calls.get(fnid).unwrap_or(&default_hs), fr.1/2));
                        }
                    }
                }

                for (fnid, ratio) in fnid_ratios.iter() {
                    let cnts = fn_cnts.get(fnid).unwrap();
                    result.score += ((cnts.0 as f32) + (cnts.1 as f32) *1.5) * ratio;
                }

            },
            None => {
                error!("no main function found");
            },
        }

        return return serde_json::to_value(&result)
        .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}

