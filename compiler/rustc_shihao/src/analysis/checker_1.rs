use super::AnalysisOption;
use crate::analysis::RFAnalysis;
use crate::utils::{
    callgraph::{callgraph_analysis, CallGraph},
    mir_body::resolve_callsite,
};
use rustc_hir::{
    def::DefKind,
    def_id::{DefId, LocalDefId},
};
use rustc_middle::mir::{ProjectionElem, Place, Local, Location};
use rustc_middle::{
    mir::{visit::Visitor, Body, ClearCrossCrate, Safety},
    ty::{self, Ty, TyCtxt},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{collections::HashSet, error::Error};
use tracing::{debug, error, info};

#[derive(Deserialize, Serialize, Debug)]
struct Checker1RFAnalysisResult {
    items: Vec<Checker1Item>,
    items_without_new: Vec<Checker1Item>,
    items_without_safe_new: Vec<Checker1Item>
}

#[derive(Deserialize, Serialize, Debug)]
struct Checker1Item {
    ty: String,
    deref_pos: String
}
pub struct Checker1RFAnalysis {}

impl RFAnalysis for Checker1RFAnalysis {
    fn analyze<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        option: &'_ AnalysisOption,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let fn_ids: Vec<LocalDefId> = tcx
            .mir_keys(())
            .iter()
            .filter(|id| {
                let hir = tcx.hir();
                hir.body_owner_kind(**id).is_fn_or_closure()
            })
            .copied()
            .collect();
        let mut callgraph = CallGraph::new();

        // iterate all local functions
        fn_ids.into_iter().for_each(|fn_id| {
            let body = tcx.optimized_mir(fn_id);
            callgraph_analysis(tcx, body, &mut callgraph, true);
        });

        let mut result = Checker1RFAnalysisResult { items: Vec::new(), items_without_safe_new: Vec::new(), items_without_new: Vec::new() };

        let mut all_referenced_types: HashSet<Ty> = HashSet::new();
        let mut ty2funcs: HashMap<Ty, HashSet<DefId>> = HashMap::new();
        let mut ty2news: HashMap<Ty, HashSet<DefId>> = HashMap::new();

        // loop all referenced functions to record
        // 1. all referenced types
        // 2. type -> member functions (first arg is that type)
        for fn_id in &callgraph.all_funcs {
            let item_name = match tcx.opt_item_name(*fn_id) {
                Some(n) => n.to_string(),
                None => "".to_string(),
            };

            info!("func: {}", item_name);

            let body = tcx.optimized_mir(*fn_id);
            // find out all referenced types

            for arg in body.args_iter() {
                all_referenced_types.insert(body.local_decls[arg].ty);
            }

            if body.arg_count > 0 {
                let first_arg = body.args_iter().next().unwrap();
                let argt = body.local_decls[first_arg].ty;
                if argt.is_ref() {
                    match argt.kind() {
                        ty::TyKind::Ref(_, actualTy, _) => {
                            if !ty2funcs.contains_key(actualTy) {
                                ty2funcs.insert(*actualTy, HashSet::new());
                            }
                            ty2funcs.get_mut(actualTy).unwrap().insert(*fn_id);
                        }
                        _ => {}
                    }
                }
            }

            let return_ty = body.return_ty();

            all_referenced_types.insert(return_ty);

            if item_name.starts_with("new") {
                if !ty2news.contains_key(&return_ty) {
                    ty2news.insert(return_ty, HashSet::new());
                }

                ty2news.get_mut(&return_ty).unwrap().insert(*fn_id);

                info!("found new func '{:?}' for type {:?}", item_name, return_ty);
            }
        }

        let mut types_with_rawptr: HashSet<Ty> = HashSet::new();
        for t in all_referenced_types {
            info!("checking {:?} has pointer?", t);
            let found_child_with_ptr = match t.kind() {
                ty::TyKind::Adt(adtdef, substs) => {
                    //info!("is adt!");

                    let mut found = false;
                    for (_, vardef) in adtdef.variants().iter_enumerated() {
                        for field in &vardef.fields {
                            //info!("checking field {:?}", field);
                            if field.ty(tcx, substs).is_unsafe_ptr() {
                                found = true;
                                info!("has pointer!");
                                break;
                            }
                        }
                    }

                    found
                }
                _ => {
                    //info!("is not adt!");
                    false
                }
            };

            if found_child_with_ptr {
                types_with_rawptr.insert(t);
            }
        }

        for t in types_with_rawptr {
            //info!("checking {:?}", t);

            if !ty2funcs.contains_key(&t) {
                info!("no member function found for {:?}, skip", t);

                continue;
            }

            let funcs = ty2funcs.get(&t).unwrap();
            for f in funcs {
                let body = tcx.optimized_mir(*f);

                // check if there is any class method that have deref ptr without check
                let mut partial_results = check_body_deref_ptr_without_check(tcx, body, t);

                if partial_results.len() == 0 {
                    continue;
                }
                if ty2news.contains_key(&t) {
                    // if this type did has a least one class method that have deref ptr without check, 
                    // make sure it have safe constructor
                    let new_funcs = ty2news.get(&t).unwrap();

                    info!("{} new functions found for {:?}", new_funcs.len(),  t);
                    
                    let mut found = false;
                    for new_func in new_funcs {
                        let new_body = tcx.optimized_mir(*new_func);


                        if check_body_safe_and_ptr_arg_without_check(tcx, new_body, t) {
                            result.items.append(&mut partial_results);
                            found = true;
                        } 
                    }

                    if !found {
                        result.items_without_safe_new.append(&mut partial_results);
                    }
                } else {
                    info!("no new function found for {:?}", t);

                    result.items_without_new.append(&mut partial_results);

                }
            }
        }

        return serde_json::to_value(&result)
            .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}

fn remove_reference_if(t: Ty) -> Option<Ty> {
    if t.is_ref() {
        match t.kind() {
            ty::TyKind::Ref(_, actualTy, _) => {
                return Some(*actualTy);
            }
            _ => {}
        }
    } else {
        return Some(t);
    }
    None
}

fn check_body_deref_ptr_without_check<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body,
    target_ty: Ty,
) -> Vec<Checker1Item> {
    let item_name = match tcx.opt_item_name(body.source.def_id()) {
        Some(n) => n.to_string(),
        None => "".to_string(),
    };
    let ty_name = target_ty.to_string();

    info!("check_body_deref_ptr_without_check: {}", item_name);
    let mut checked_null_places:HashSet<Place> = HashSet::new();
    let mut results: Vec<Checker1Item> = Vec::new();
    for (bb, bb_data) in body.basic_blocks().iter_enumerated() {
        for stmt in &bb_data.statements {
            match stmt.kind {
                rustc_middle::mir::StatementKind::Assign(ref data) => {
                    let lhs = data.0;
                    let rhs = &data.1;
                    info!("{:?} is assign, rhs is {:?}", stmt, rhs);
                    match rhs {
                        rustc_middle::mir::Rvalue::Use(op) => match op {
                            rustc_middle::mir::Operand::Copy(place) => {
                                info!("rhs is copy");
                                
                            }
                            rustc_middle::mir::Operand::Move(op) => {
                                info!("rhs is move");
                            }
                            rustc_middle::mir::Operand::Constant(_) => {
                                info!("rhs is constant");

                            }
                        },
                        rustc_middle::mir::Rvalue::Ref(_, _, place) => {
                            info!("rhs is ref");
                            if checked_null_places.contains(place) {
                                info!("{:?} has been checked, skip", place);
                                continue;
                            }

                            if place.projection.len() > 0 {
                                let base = place.local;

                                let base_ty = body.local_decls[base].ty;

                                if remove_reference_if(base_ty).unwrap() == target_ty {
                                    if base_ty.is_ref() {
                                        // *((*_1).0: *mut xxx) suppose 0 is the pointer field
                                        // projection 0 is deref _1
                                        // projection 1 is field .0
                                        // projection 2 is deref
                                        if place.projection.len() < 3 {
                                            continue;
                                        }
                                        if ProjectionElem::Deref != place.projection[0] {
                                            continue;
                                        }
                                        if let ProjectionElem::Field(f, t) = place.projection[1] {
                                            if !t.is_unsafe_ptr() {
                                                continue;
                                            }
                                            if ProjectionElem::Deref != place.projection[2] {
                                                continue;
                                            }

                                            // found deref pointer here
                                            info!("found deref pointer from target type!");
                                            results.push(Checker1Item {
                                                ty: ty_name.clone(),
                                                deref_pos: format!("{:?}", stmt.source_info.span),
                                            })
                                        } else {
                                            continue;
                                        }

                                    } else {
                                        // *(_1.0: *mut xxx)
                                        if place.projection.len() < 2 {
                                            continue;
                                        }
                                        if let ProjectionElem::Field(f, t) = place.projection[1] {
                                            if !t.is_unsafe_ptr() {
                                                continue;
                                            }
                                            if ProjectionElem::Deref != place.projection[2] {
                                                continue;
                                            }

                                            // found deref pointer here
                                            info!("found deref pointer from target type!");
                                            results.push(Checker1Item {
                                                ty: ty_name.clone(),
                                                deref_pos: format!("{:?}", stmt.source_info.span),
                                            })
                                        } else {
                                            continue;
                                        }


                                    }
                                   
                                }
                            } else {
                                info!("rhs has no projection, skip");
                            }
                        }
                        _ => {}
                    }
                },
                
                _ => {}
            }
        }

        // check call if any
        match resolve_callsite(tcx, body, bb, bb_data, true) {
            None => {}
            Some(cs) => {
                if let Some(ty) = cs.call_by_type {
                    let callee_name = match tcx.opt_item_name(cs.callee.def_id()) {
                        Some(ident) => ident.to_string(),
                        None => "".to_string(),
                    };
                    if ty.is_unsafe_ptr() && (callee_name.starts_with("is_null") || callee_name.starts_with("as_ref")) {
                        let ptr_op = cs.args.first().unwrap();
                        if let Some(p) = ptr_op.place() {
                            checked_null_places.insert(p);
                        }
                    }
                }
            }
        };

       
    }
    return results;
}

// body has to be safe
// body has ptr assignment wihout check
fn check_body_safe_and_ptr_arg_without_check<'tcx>(tcx: TyCtxt<'tcx>, body: &'tcx Body,  target_ty: Ty) -> bool {
    let item_name = match tcx.opt_item_name(body.source.def_id()) {
        Some(n) => n.to_string(),
        None => "".to_string(),
    };
    let ty_name = target_ty.to_string();
    info!("check_body_safe_and_ptr_arg_without_check: {} for type {}", item_name, ty_name);

    let mut checked_null_places:HashSet<Place> = HashSet::new();

    for (scope_data) in body.source_scopes.iter() {
        if !scope_data.fn_safety {
            info!("function is unsafe, return");
            return false;
        }
    }

    let mut body_args:HashSet<Local> = HashSet::new();
    for arg in body.args_iter() {
        body_args.insert(arg);
    }

    for (bb, bb_data) in body.basic_blocks().iter_enumerated() {
        for stmt in &bb_data.statements {
            match stmt.kind {
                rustc_middle::mir::StatementKind::Assign(ref data) => {
                    let lhs = data.0;
                    let rhs = &data.1;

                    // if assignment is for pointer
                    if lhs.ty(body, tcx).ty.is_unsafe_ptr() {
                        // and if right value does not null checked before
                        match rhs {
                            rustc_middle::mir::Rvalue::Use(opr) => {
                                match opr {
                                    rustc_middle::mir::Operand::Copy(p) => {
                                        if !checked_null_places.contains(p){
                                            if body_args.contains(&p.local) && p.ty(body, tcx).ty.is_unsafe_ptr() {
                                                info!("found arg ptr assign without check!");
                                                return true
                                            }
                                        }
                                    },
                                    rustc_middle::mir::Operand::Move(_) => {},
                                    rustc_middle::mir::Operand::Constant(_) => {},
                                }
                            },
                            rustc_middle::mir::Rvalue::BinaryOp(op, oprs) => {
                                match &oprs.0.place() {
                                    Some(p) => {
                                            checked_null_places.insert(*p);
                                        
                                        
                                    },
                                    None => {},
                                }
            
                                match &oprs.1.place() {
                                    Some(p) => {
                                        checked_null_places.insert(*p);
                                        
                                        
                                    },
                                    None => {},
                                }
                                
                            },
                            _ => {}
                        }
                    }
                },
                
                _ => {}
            }
        }

        match resolve_callsite(tcx, body, bb, bb_data, true) {
            None => {}
            Some(cs) => {
                if let Some(ty) = cs.call_by_type {
                    let callee_name = match tcx.opt_item_name(cs.callee.def_id()) {
                        Some(ident) => ident.to_string(),
                        None => "".to_string(),
                    };
                    if ty.is_unsafe_ptr() && (callee_name.starts_with("is_null") || callee_name.starts_with("as_ref")) {
                        let ptr_op = cs.args.first().unwrap();
                        if let Some(p) = ptr_op.place() {
                            checked_null_places.insert(p);
                        }
                    }
                }
            }
        };
    }
    return true;
}
