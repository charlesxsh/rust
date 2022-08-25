use std::collections::HashSet;

use rustc_middle::{mir::{visit::Visitor, Location, Rvalue, Operand, Body, Local, ProjectionElem, Place}, ty::TyCtxt};



pub struct StackInitVisitor<'tcx, 'a> {
    pub tcx: TyCtxt<'tcx>,
    pub body: &'a Body<'tcx>,
    pub init_locs: HashSet<Location>,
    args: HashSet<Local>
}

impl<'tcx, 'a> StackInitVisitor<'tcx, 'a> {
    pub fn new(tcx: TyCtxt<'tcx>, body: &'a Body<'tcx>) -> Self {
        let mut args = HashSet::new();
        for arg in body.args_iter() {
            args.insert(arg);
        }
        Self { tcx, body, init_locs: HashSet::new(), args }
    }
}


impl<'tcx, 'a> Visitor<'tcx> for StackInitVisitor<'tcx, 'a> {
    fn visit_assign(
        &mut self,
        place: &Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
        location: rustc_middle::mir::Location,
    ) {
        match rvalue {
            Rvalue::Use(op) => match op {
                Operand::Copy(p) => {
                    if !self.args.contains(&place.local) {
                        self.init_locs.insert(location);
                    }
                }
                Operand::Move(p) => {
                    if !self.args.contains(&place.local) {
                        self.init_locs.insert(location);
                    }
                },
                Operand::Constant(_) => {
                    if !self.args.contains(&place.local) {
                        self.init_locs.insert(location);
                    }

                }
            },
            Rvalue::Ref(_, _, p) => {
                if !self.args.contains(&place.local) {
                    self.init_locs.insert(location);
                }
            },
            _ => {}
        }

        self.super_assign(place, rvalue, location)
    }
}