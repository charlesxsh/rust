use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
};

use rustc_hir::def_id::DefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{
        BasicBlock, Body, ClearCrossCrate, Local, Operand, Rvalue, Safety, Statement,
        StatementKind, TerminatorKind,
    },
    ty::TyCtxt,
};
use rustc_mir_dataflow::{Analysis, CallReturnPlaces, ResultsCursor};
use rustc_span::LineInfo;
use serde::{Deserialize, Serialize};

use super::{
    alias::BodyAliasResult,
    callgraph::{CallGraph},
    mir_body::{get_body_return_local, CallSite},
};

#[derive(Clone, Debug)]
pub struct UnsafeSpreadAnalysisBodyResult<'tcx> {
    pub unsafe_locals: HashSet<Local>,
    pub is_return_unsafe: bool,
    pub callsites_with_unsafe_args: Vec<CallSiteWithUnsafeArg<'tcx>>,
}

pub struct UnsafeSpreadAnalysis<'tcx, 'a> {
    pub tcx: TyCtxt<'tcx>,
    pub alias_result: &'a HashMap<DefId, BodyAliasResult<'tcx>>,
    pub callgraph: &'a CallGraph<'tcx>,
    pub results_cache:
        RefCell<HashMap<DefId, HashMap<Vec<usize>, UnsafeSpreadAnalysisBodyResult<'tcx>>>>,
}

impl<'tcx, 'a> UnsafeSpreadAnalysis<'tcx, 'a> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        alias_result: &'a HashMap<DefId, BodyAliasResult<'tcx>>,
        callgraph: &'a CallGraph<'tcx>,
    ) -> Self {
        Self { tcx, results_cache: RefCell::new(HashMap::new()), alias_result, callgraph }
    }

    pub fn analyze(
        &self,
        body_id: DefId,
        unsafe_arg_idxs: Vec<usize>,
    ) -> UnsafeSpreadAnalysisBodyResult {
        {
            let mut results_cache = self.results_cache.borrow_mut();
            if results_cache.contains_key(&body_id) {
                let body_results = results_cache.get(&body_id).unwrap();
                if body_results.contains_key(&unsafe_arg_idxs) {
                    return body_results.get(&unsafe_arg_idxs).unwrap().clone();
                }
            } else {
                results_cache.insert(body_id, HashMap::new());
            }
        }
        let body = self.tcx.optimized_mir(body_id);
        let mut analysis = UnsafeSpreadBodyAnalysis::new(
            self.tcx,
            body,
            self.alias_result,
            unsafe_arg_idxs.clone(),
            self,
        );
        let ares =
            analysis.into_engine(self.tcx, body).iterate_to_fixpoint().into_results_cursor(body);
        let result = ares.analysis().get_result(&ares);
        {
            let mut results_cache = self.results_cache.borrow_mut();
            results_cache.get_mut(&body_id).unwrap().insert(unsafe_arg_idxs.clone(), result);
        }
        return self
            .results_cache
            .borrow()
            .get(&body_id)
            .unwrap()
            .get(&unsafe_arg_idxs)
            .unwrap()
            .clone();
    }
}

fn is_statement_safe<'tcx, 'a>(body: &'a Body<'tcx>, statement: &'a Statement<'tcx>) -> bool {
    let scope_data = &body.source_scopes[statement.source_info.scope];
    return scope_data.safety;
}

#[derive(Clone, Debug)]
pub struct CallSiteWithUnsafeArg<'tcx> {
    pub callsite: CallSite<'tcx>,
    pub unsafe_arg_idxs: Vec<usize>,
}

struct UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    body_id: DefId,
    alias_result: &'b HashMap<DefId, BodyAliasResult<'tcx>>,
    callsites_with_unsafe_args: RefCell<Vec<CallSiteWithUnsafeArg<'tcx>>>,
    return_local: Option<Local>,
    unsafe_arg_idxs: Vec<usize>,
    callgraph: &'c CallGraph<'tcx>,
    controller: &'c UnsafeSpreadAnalysis<'tcx, 'b>,
}

impl<'tcx, 'a, 'b, 'c> UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c> {}

impl<'tcx, 'a, 'b, 'c> UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c> {
    fn new(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        alias_result: &'b HashMap<DefId, BodyAliasResult<'tcx>>,
        unsafe_arg_idxs: Vec<usize>,
        controller: &'c UnsafeSpreadAnalysis<'tcx, 'b>,
    ) -> Self {
        Self {
            tcx,
            body,
            alias_result,
            body_id: body.source.def_id(),
            callsites_with_unsafe_args: RefCell::new(Vec::new()),
            unsafe_arg_idxs,
            controller,
            callgraph: controller.callgraph,
            return_local: get_body_return_local(body),
        }
    }

    fn get_result(
        &self,
        cursor: &ResultsCursor<'_, 'tcx, Self>,
    ) -> UnsafeSpreadAnalysisBodyResult<'tcx> {
        let locals: HashSet<Local> = cursor.get().iter().collect();
        let is_return_unsafe = match self.return_local {
            Some(ret) => cursor.contains(ret),
            None => false,
        };
        UnsafeSpreadAnalysisBodyResult {
            unsafe_locals: locals,
            is_return_unsafe,
            callsites_with_unsafe_args: self.callsites_with_unsafe_args.borrow().clone(),
        }
    }
}

impl<'tcx, 'a, 'b, 'c> rustc_mir_dataflow::AnalysisDomain<'tcx>
    for UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c>
{
    const NAME: &'static str = "UnsafeSpreadBodyAnalysis";

    fn bottom_value(&self, body: &Body<'tcx>) -> Self::Domain {
        BitSet::new_empty(body.local_decls.len())
    }

    fn initialize_start_block(&self, body: &Body<'tcx>, domain: &mut Self::Domain) {
        for idx in &self.unsafe_arg_idxs {
            for (arg_idx, arg) in body.args_iter().enumerate() {
                if arg_idx == *idx as usize {
                    domain.insert(arg);
                }
            }
        }
    }

    type Domain = BitSet<Local>;

    type Direction = rustc_mir_dataflow::Forward;
}

impl<'tcx, 'a, 'b, 'c> Analysis<'tcx> for UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c> {
    fn apply_statement_effect(
        &self,
        state: &mut Self::Domain,
        statement: &rustc_middle::mir::Statement<'tcx>,
        _: rustc_middle::mir::Location,
    ) {
        let stmt_safe = is_statement_safe(self.body, statement);

        let mut is_rhs_safe = true;
        match statement.kind {
            StatementKind::Assign(ref data) => {
                let lhs = data.0;
                let rhs = &data.1;

                if !stmt_safe {
                    state.insert(lhs.local);
                    return;
                }

                match rhs {
                    Rvalue::Use(op) => match op {
                        Operand::Copy(p) => {
                            if state.contains(p.local) {
                                is_rhs_safe = false;
                            }
                        }
                        Operand::Move(p) => {
                            if state.contains(p.local) {
                                is_rhs_safe = false;
                            }
                        }
                        Operand::Constant(_) => {}
                    },
                    Rvalue::Ref(_, _, p) => {
                        if state.contains(p.local) {
                            is_rhs_safe = false;
                        }
                    }
                    _ => {}
                }
                if !is_rhs_safe {
                    state.insert(lhs.local);
                    self.alias_result
                        .get(&self.body_id)
                        .unwrap()
                        .local_alias
                        .get(&lhs.local)
                        .into_iter()
                        .flat_map(|bs| bs.iter())
                        .all(|l| state.insert(*l));
                }
            }

            _ => {}
        }
    }

    fn apply_terminator_effect(
        &self,
        state: &mut Self::Domain,
        terminator: &rustc_middle::mir::Terminator<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
    }

    fn apply_call_return_effect(
        &self,
        state: &mut Self::Domain,
        bb: BasicBlock,
        ret: CallReturnPlaces<'_, 'tcx>,
    ) {
        let callsite = self.callgraph.find_callsite(self.body_id, bb);
        // filter out some call without MIR or generated by compiler
        if callsite.is_none() {
            return;
        }
        let c = callsite.unwrap();

        let mut unsafe_arg_idxs = vec![];

        for (idx, arg) in c.args.iter().enumerate() {
            let mut is_arg_unsafe = false;
            match arg {
                Operand::Copy(p) => {
                    if state.contains(p.local) {
                        is_arg_unsafe = true;
                    }
                }
                Operand::Move(p) => {
                    if state.contains(p.local) {
                        is_arg_unsafe = true;
                    }
                }
                Operand::Constant(_) => {}
            }
            if is_arg_unsafe {
                unsafe_arg_idxs.push(idx)
            }
        }

        if unsafe_arg_idxs.len() > 0 {
            self.callsites_with_unsafe_args.borrow_mut().push(CallSiteWithUnsafeArg {
                callsite: c.clone(),
                unsafe_arg_idxs: unsafe_arg_idxs.clone(),
            });
            let result = self.controller.analyze(c.callee.def_id(), unsafe_arg_idxs);
            if result.is_return_unsafe {
                match ret {
                    CallReturnPlaces::Call(rlocal) => {
                        state.insert(rlocal.local);
                        self.alias_result
                            .get(&self.body_id)
                            .unwrap()
                            .local_alias
                            .get(&rlocal.local)
                            .into_iter()
                            .flat_map(|bs| bs.iter())
                            .all(|l| state.insert(*l));
                    }
                    CallReturnPlaces::InlineAsm(_) => {}
                }
            }
        }
    }
}
