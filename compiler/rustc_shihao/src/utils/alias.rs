use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use rustc_hir::def_id::DefId;
use rustc_index::bit_set::BitSet;
use rustc_middle::{
    mir::{
        visit::Visitor, BasicBlock, BasicBlockData, Body, ClearCrossCrate, Local, LocalKind,
        Operand, Place, Rvalue, Safety, Statement, StatementKind, TerminatorKind,
    },
    ty::{subst::Subst, Instance, InstanceDef, PolyFnSig, TyCtxt, TyKind::FnDef},
};
use rustc_mir_dataflow::{Analysis, CallReturnPlaces, GenKillAnalysis};
use rustc_span::{LineInfo, Symbol};

use super::{mir_body::{get_body_return_local, CallSite}};

#[derive(Clone, Debug)]
pub struct BodyAliasResult<'tcx> {
    // local alias
    pub local_alias: HashMap<Local, HashSet<Local>>,

    // escape part
    pub arg_idx_returned: HashSet<Local>,
    pub local_as_args: HashMap<Local, CallSite<'tcx>>,
}

pub struct BodyAliasVisitor<'tcx, 'a> {
    pub tcx: TyCtxt<'tcx>,
    pub body: &'a Body<'tcx>,
    pub track_copy: bool,
    pub local_alias: HashMap<Local, HashSet<Local>>,
    pub arg_idx_returned: HashSet<Local>,
    pub local_as_args: HashMap<Local, CallSite<'tcx>>,
}

fn set_alias(local_alias: &mut HashMap<Local, HashSet<Local>>, a: Local, b: Local) {
    if a == b {
        return;
    }
    if !local_alias.contains_key(&a) {
        local_alias.insert(a, HashSet::new());
    }

    if !local_alias.contains_key(&b) {
        local_alias.insert(b, HashSet::new());
    }

    let a_aliases: Vec<Local> =
        local_alias.get(&a).iter().flat_map(|s| s.iter()).copied().collect();

    for aa in a_aliases {
        if !local_alias.contains_key(&aa) {
            local_alias.insert(aa, HashSet::new());
        }
        local_alias.get_mut(&aa).unwrap().insert(b);
    }

    let b_aliases: Vec<Local> =
        local_alias.get(&b).iter().flat_map(|s| s.iter()).copied().collect();

    for bb in b_aliases {
        if !local_alias.contains_key(&bb) {
            local_alias.insert(bb, HashSet::new());
        }
        local_alias.get_mut(&bb).unwrap().insert(a);
    }

    local_alias.get_mut(&a).unwrap().insert(b);
    local_alias.get_mut(&b).unwrap().insert(a);
}
impl<'tcx, 'a> BodyAliasVisitor<'tcx, 'a> {

    pub fn new(tcx: TyCtxt<'tcx>, body: &'tcx Body<'tcx>, track_copy: bool) -> Self {
        return Self {
            tcx,
            body,
            track_copy,
            local_alias: HashMap::new(),
            arg_idx_returned: HashSet::new(),
            local_as_args: HashMap::new(),
        };
    }
    pub fn get_result(&self) -> BodyAliasResult<'tcx> {
        return BodyAliasResult {
            local_alias: self.local_alias.clone(),
            arg_idx_returned: self.arg_idx_returned.clone(),
            local_as_args: self.local_as_args.clone(),
        };
    }

    fn set_alias(&mut self, a: Local, b: Local) {
        set_alias(&mut self.local_alias, a, b)
    }

    fn is_alias_of(&self, a: Local, b: Local) -> bool {
        if let Some(alias_set) = self.local_alias.get(&a) {
            return alias_set.contains(&b);
        }
        return false;
    }

    fn pretty_print(&self) {
        let fname = self.tcx.item_name(self.body.source.def_id());
        println!("alias analyis for {:?}", fname);
        for (key, value) in &self.local_alias {
            println!("{:?}: {:?}", key, value);
        }
    }
}

impl<'tcx, 'a> Visitor<'tcx> for BodyAliasVisitor<'tcx, 'a> {
    // record local alias
    fn visit_assign(
        &mut self,
        place: &Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
        match rvalue {
            Rvalue::Use(op) => match op {
                Operand::Copy(p) => {
                    if self.track_copy {
                        self.set_alias(place.local, p.local)
                    }
                }
                Operand::Move(p) => self.set_alias(place.local, p.local),
                Operand::Constant(_) => {}
            },
            Rvalue::Ref(_, _, p) => self.set_alias(place.local, p.local),
            _ => {}
        }

        self.super_assign(place, rvalue, location)
    }

    // escape analysis for args
    fn visit_terminator(
        &mut self,
        terminator: &rustc_middle::mir::Terminator<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
        if let Some(return_local) = get_body_return_local(self.body) {
            match terminator.kind {
                TerminatorKind::Return => {
                    for arg in self.body.args_iter() {
                        if self.is_alias_of(arg, return_local) {
                            self.arg_idx_returned.insert(arg);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}
