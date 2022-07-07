use std::collections::{HashMap, HashSet, VecDeque};

use rustc_hir::def_id::DefId;
use rustc_middle::{
    mir::{visit::Visitor, BasicBlock, BasicBlockData, Body, Operand, TerminatorKind},
    ty::{subst::Subst, Instance, InstanceDef, PolyFnSig, TyCtxt, TyKind::FnDef},
};
use rustc_span::{Span, Symbol};
use tracing::info;

use super::mir_body::{CallSite, resolve_callsite, check_mir_is_available};



struct BodyCallVisitor<'tcx, 'a> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    callsites: Vec<CallSite<'tcx>>,
    ensure_callee_has_mir: bool
}

pub struct CallGraph<'tcx> {
    pub all_funcs: HashSet<DefId>,
    pub calls: HashMap<DefId, HashSet<DefId>>,
    pub callsites: HashMap<DefId, Vec<CallSite<'tcx>>>,
}

impl<'tcx> CallGraph<'tcx> {
    pub fn find_callsite(&self, body_id: DefId, bb: BasicBlock) -> Option<CallSite<'tcx>> {
        let set = self.callsites.get(&body_id);

        if let Some(s) = set {
            for c in s {
                if c.bb == bb {
                    return Some(c.clone());
                }
            }
        }
        None
    }
    pub fn raw_pretty_print(&self) {
        for (caller, callees) in &self.calls {
            println!("caller {:?}, callees: {:?}", caller, callees);
        }
    }

    pub fn pretty_print(&self, tcx: TyCtxt<'tcx>) {
        for (caller, callees) in &self.calls {
            let caller_name = tcx.item_name(*caller);
            let callee_names: Vec<Symbol> =
                callees.iter().map(|callee| tcx.item_name(*callee)).collect();
            println!("caller {:?}, callees: {:?}", caller_name, callee_names);
        }
    }

    pub fn new() -> Self {
        Self { calls: HashMap::new(), callsites: HashMap::new(), all_funcs: HashSet::new() }
    }
}
pub fn callgraph_analysis<'tcx, 'a>(
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    callgraph: &mut CallGraph<'tcx>,
    ensure_callee_has_mir: bool
) {
    let body_id = body.source.def_id();
    if callgraph.calls.contains_key(&body_id) {
        // if analyzed, skip
        return;
    }
    let mut worklist: VecDeque<&'_ Body<'tcx>> = VecDeque::new();
    worklist.push_back(body);

    while let Some(body_to_visit) = worklist.pop_front() {
        let caller_def_id = body_to_visit.source.def_id();
        if callgraph.all_funcs.contains(&caller_def_id) {
            continue;
        }
        callgraph.all_funcs.insert(caller_def_id);
        let mut visiter = BodyCallVisitor { tcx, body: body_to_visit, callsites: Vec::new(), ensure_callee_has_mir };

        visiter.visit_body(body_to_visit);
        for cs in visiter.callsites {
            let callee_def_id = cs.callee.def_id();
            if callgraph.all_funcs.contains(&callee_def_id) {
                continue;
            }
            match check_mir_is_available(tcx, body_to_visit, &cs.callee) {
                Ok(()) => {}
                Err(reason) => {
                    // let callee_name = tcx.opt_item_name(callee_def_id);
                    // let name = match callee_name {
                    //     Some(ref cn) => cn.as_str(),
                    //     None => ""
                    // };
                    //info!("MIR of {} is unavailable: {}", name, reason);                    
                    continue;
                }
            }
            let body = tcx.instance_mir(cs.callee.def);
            worklist.push_back(body);

            // record call relationship
            if !callgraph.calls.contains_key(&caller_def_id) {
                callgraph.calls.insert(caller_def_id, HashSet::new());
            }

            callgraph.calls.get_mut(&caller_def_id).unwrap().insert(callee_def_id);

            // record callsite
            if !callgraph.callsites.contains_key(&caller_def_id) {
                callgraph.callsites.insert(caller_def_id, Vec::new());
            }
            callgraph.callsites.get_mut(&caller_def_id).unwrap().push(cs.clone());
        }
    }
}


impl<'tcx, 'a> Visitor<'tcx> for BodyCallVisitor<'tcx, 'a> {
    // record call graph for this body
    fn visit_basic_block_data(
        &mut self,
        block: rustc_middle::mir::BasicBlock,
        data: &rustc_middle::mir::BasicBlockData<'tcx>,
    ) {
        if data.is_cleanup {
            return self.super_basic_block_data(block, data);
        }

        let callsite = match resolve_callsite(self.tcx, self.body, block, data, self.ensure_callee_has_mir) {
            None => return self.super_basic_block_data(block, data),
            Some(it) => it,
        };

        self.callsites.push(callsite);

        self.super_basic_block_data(block, data)
    }
}

