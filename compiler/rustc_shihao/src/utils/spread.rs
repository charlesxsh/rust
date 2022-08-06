use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque}, rc::Rc,
};

use tracing::info;
use rustc_hir::def_id::DefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{
        BasicBlock, Body, ClearCrossCrate, Local, Operand, Rvalue, Safety, Statement,
        StatementKind, TerminatorKind,
    },
    ty::TyCtxt,
};
use rustc_mir_dataflow::{Analysis, CallReturnPlaces, ResultsCursor, ResultsVisitor};
use rustc_span::{LineInfo, Span};
use serde::{Deserialize, Serialize};

use super::{
    alias::BodyAliasResult,
    callgraph::{CallGraph},
    mir_body::{get_body_return_local, CallSite, get_body_arglocal_idx_pairs},
};

#[derive(Clone, Debug)]
pub struct UnsafeSpreadAnalysisBodyResult<'tcx> {
    pub unsafe_spans: Vec<Span>,
    pub is_return_unsafe: bool,
    pub unsafe_tainted_argidxs: HashSet<usize>,
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
        let callsites_with_unsafe_args: Rc<RefCell<Vec<CallSiteWithUnsafeArg<'tcx>>>> = Rc::new(RefCell::new(Vec::new()));
        let mut analysis = UnsafeSpreadBodyAnalysis::new(
            self.tcx,
            body,
            self.alias_result,
            unsafe_arg_idxs.clone(),
            callsites_with_unsafe_args.clone(),
            self,
        );

        
        let results = analysis.into_engine(self.tcx, body).iterate_to_fixpoint();
        let mut visitor = UnsafeSpreadBodyResultVisitor::new(
            get_body_return_local(body), 
            get_body_arglocal_idx_pairs(body),
            self.alias_result.get(&body_id)
        );

        results.visit_with(&body, body.basic_blocks().iter_enumerated().map(|(i, _)| i), &mut visitor);
        //let result = ares.analysis().get_result(&ares);
        let result = visitor.get_result(callsites_with_unsafe_args.borrow().clone());
        {
            let mut results_cache = self.results_cache.borrow_mut();
            results_cache.get_mut(&body_id).unwrap().insert(unsafe_arg_idxs.clone(), result);
        }

        self.tcx.xsh_spread()
        .borrow_mut()
        .insert(body_id, Vec::new());
        
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

struct UnsafeSpreadBodyResultVisitor<'b, 'tcx> {
    spans: Vec<Span>,
    return_local: Option<Local>,
    is_return_local_unsafe: bool,
    // body's arg and index pair
    argidx_pairs: Option<Vec<(usize, Local)>>,
    // tainted body's arg (during the statements of the body)
    tainted_argidxs: HashSet<usize>,
    // find out any args(and their alias) are tainted by unsafe
    alias_result: Option<&'b BodyAliasResult<'tcx>>
}

impl<'b, 'tcx> UnsafeSpreadBodyResultVisitor<'b, 'tcx> {

    // return_local is the return local variable in the given body
    fn new(return_local: Option<Local>, argidx: Option<Vec<(usize, Local)>>, alias_result: Option<&'b BodyAliasResult<'tcx>>) -> Self  {
        return Self { 
            spans: Vec::new(), 
            return_local, 
            is_return_local_unsafe: false,
            argidx_pairs: argidx,
            tainted_argidxs: HashSet::new(),
            alias_result
        }
    }

    fn get_result(
        &self,
        callsites_with_unsafe_args: Vec<CallSiteWithUnsafeArg<'tcx>>
    ) -> UnsafeSpreadAnalysisBodyResult<'tcx> {
        UnsafeSpreadAnalysisBodyResult {
            unsafe_spans: self.spans.clone(),
            is_return_unsafe:self.is_return_local_unsafe,
            unsafe_tainted_argidxs: self.tainted_argidxs.clone(),
            callsites_with_unsafe_args,
        }
    }

    fn check_if_args_tainted() {

    }
}


impl<'mir,'tcx, 'b> ResultsVisitor< 'mir,'tcx> for  UnsafeSpreadBodyResultVisitor<'b, 'tcx> {
    type FlowState = BitSet<Local>;
    

    fn visit_statement_after_primary_effect(
        &mut self,
        _state: &Self::FlowState,
        _statement: &'mir rustc_middle::mir::Statement<'tcx>,
        _location: rustc_middle::mir::Location,
    ) {
        match _statement.kind {
            StatementKind::Assign(ref data) => {
                let lhs = data.0;
                let rhs = &data.1;
                let mut is_lhs_unsafe = false;
                let mut is_rhs_unsafe = false;

                if _state.contains(lhs.local) {
                    is_lhs_unsafe = true;
                    if let Some(ret) = self.return_local {
                        if ret == lhs.local {
                            self.is_return_local_unsafe = true;
                        }
                    }

                    if let Some(args) = &self.argidx_pairs {
                        if let Some(alias_result) = self.alias_result {
                            for idxarg in args {
                                let is_arg_alias = alias_result
                                .local_alias
                                .get(&lhs.local)
                                .into_iter()
                                .flat_map(|bs| bs.iter())
                                .any(|l| *l == idxarg.1);
    
                                if is_arg_alias {
                                    info!("visitor: found unsafe tained the arg idx {}", idxarg.0);
                                    self.tainted_argidxs.insert(idxarg.0);
                                    break;
                                }
                            }
                        }
                        
                    }
                } else {
                    match rhs {
                        Rvalue::Use(op) => match op {
                            Operand::Copy(p) => {
                                if _state.contains(p.local) {
                                    is_rhs_unsafe = true;
                                }
                                if let Some(ret) = self.return_local {
                                    if ret == p.local {
                                        self.is_return_local_unsafe = true;
                                    }
                                }
                            }
                            Operand::Move(p) => {
                                if _state.contains(p.local) {
                                    is_rhs_unsafe = true;
                                }
                                if let Some(ret) = self.return_local {
                                    if ret == p.local {
                                        self.is_return_local_unsafe = true;
                                    }
                                }
                            }
                            Operand::Constant(_) => {}
                        },
                        Rvalue::Ref(_, _, p) => {
                            if _state.contains(p.local) {
                                is_rhs_unsafe = true;
                            }
                            if let Some(ret) = self.return_local {
                                if ret == p.local {
                                    self.is_return_local_unsafe = true;
                                }
                            }
                        }
                        _ => {}
                    }
    
                }

                
                if is_rhs_unsafe || is_lhs_unsafe {
                    self.spans.push(_statement.source_info.span);
                    info!("visitor: found unsafe stmt: {:?}", _statement);
                }

                if self.is_return_local_unsafe {
                    info!("visitor: found unsafe at return");

                }
            },
            _ => {}
        }

    }

    fn visit_block_start(
        &mut self,
        _state: &Self::FlowState,
        _block_data: &'mir rustc_middle::mir::BasicBlockData<'tcx>,
        _block: BasicBlock,
    ) {
    }

    fn visit_statement_before_primary_effect(
        &mut self,
        _state: &Self::FlowState,
        _statement: &'mir rustc_middle::mir::Statement<'tcx>,
        _location: rustc_middle::mir::Location,
    ) {
    }

    fn visit_terminator_before_primary_effect(
        &mut self,
        _state: &Self::FlowState,
        _terminator: &'mir rustc_middle::mir::Terminator<'tcx>,
        _location: rustc_middle::mir::Location,
    ) {
    }

    fn visit_terminator_after_primary_effect(
        &mut self,
        _state: &Self::FlowState,
        _terminator: &'mir rustc_middle::mir::Terminator<'tcx>,
        _location: rustc_middle::mir::Location,
    ) {
        
    }

    fn visit_block_end(
        &mut self,
        _state: &Self::FlowState,
        _block_data: &'mir rustc_middle::mir::BasicBlockData<'tcx>,
        _block: BasicBlock,
    ) {
    }
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
    callsites_with_unsafe_args: Rc<RefCell<Vec<CallSiteWithUnsafeArg<'tcx>>>>,
    return_local: Option<Local>,
    unsafe_arg_idxs: Vec<usize>,
    callgraph: &'c CallGraph<'tcx>,
    controller: &'c UnsafeSpreadAnalysis<'tcx, 'b>,
}

impl<'tcx, 'a, 'b, 'c> UnsafeSpreadBodyAnalysis<'tcx, 'a, 'b, 'c> {
    fn new(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        alias_result: &'b HashMap<DefId, BodyAliasResult<'tcx>>,
        unsafe_arg_idxs: Vec<usize>,
        callsites_with_unsafe_args: Rc<RefCell<Vec<CallSiteWithUnsafeArg<'tcx>>>>,
        controller: &'c UnsafeSpreadAnalysis<'tcx, 'b>,
    ) -> Self {
        Self {
            tcx,
            body,
            alias_result,
            body_id: body.source.def_id(),
            callsites_with_unsafe_args,
            unsafe_arg_idxs,
            controller,
            callgraph: controller.callgraph,
            return_local: get_body_return_local(body),
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
                    self.alias_result
                        .get(&self.body_id)
                        .unwrap()
                        .local_alias
                        .get(&lhs.local)
                        .into_iter()
                        .flat_map(|bs| bs.iter())
                        .all(|l| state.insert(*l));
                    
                    return;
                } else {
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

        info!("checking callsite {:?}", c.callee.def_id());
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
        for argidx in result.unsafe_tainted_argidxs {
            for (idx, opr) in c.args.iter().enumerate() {
                if argidx == idx {
                    info!("checking callsite {:?}: found arg {} has been tained", c.callee.def_id(), idx);

                    match opr {
                        Operand::Copy(p) => {
                            // if an arg is passed by copy and then tainted, no harm
                        }
                        Operand::Move(p) => {
                            self.alias_result
                            .get(&self.body_id)
                            .unwrap()
                            .local_alias
                            .get(&p.local)
                            .into_iter()
                            .flat_map(|bs| bs.iter())
                            .all(|l| state.insert(*l));
                        }
                        Operand::Constant(_) => {}
                    }
                    
                    break;
                }
            }
        }
        
    }
}
