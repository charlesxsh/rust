use std::{error::Error, collections::HashSet};

use rustc_hir::def_id::LocalDefId;
use rustc_middle::{ty::TyCtxt, mir::{Body, Local, visit::Visitor}};
use rustc_span::sym::s;
use serde::{Deserialize, Serialize};

use crate::utils::{callgraph::{CallGraph, callgraph_analysis}, mir_body::resolve_callsite, alias::BodyAliasVisitor};

use super::{ty::RFAnalysis, AnalysisOption};
use tracing::{debug, error, info};


#[derive(Deserialize, Serialize, Debug)]
struct Checker4RFAnalysisResult {
    // the dangerous pointer calculations
    stmts: Vec<String>
}

pub struct Checker4RFAnalysis {}

impl RFAnalysis for Checker4RFAnalysis {
    fn analyze<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        option: &'_ AnalysisOption,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut result = Checker4RFAnalysisResult { stmts: Vec::new() };

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
            callgraph_analysis(tcx, body, &mut callgraph, true);
        });



        // loop all referenced functions to record
        for fn_id in &callgraph.all_funcs {
            let body = tcx.optimized_mir(*fn_id);

            let mut partial =  check(tcx, body);

            result.stmts.append(&mut partial);
        }


        return serde_json::to_value(&result)
        .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}

fn check<'tcx>(tcx: TyCtxt<'tcx>, body:&'tcx Body<'tcx>) -> Vec<String> {
    let item_name = match tcx.opt_item_name(body.source.def_id()) {
        Some(n) => n.to_string(),
        None => "".to_string(),
    };
    info!("checking {}", item_name);
    let mut result: Vec<String>  = Vec::new();

    for (scope_data) in body.source_scopes.iter() {
        if !scope_data.fn_safety {
            info!("function is unsafe, return");
            return result;
        }
    }

    // find out number argument
    let mut num_args:HashSet<Local> = HashSet::new();
    for arg in body.args_iter() {
        let arg_ty = body.local_decls[arg].ty;
        if arg_ty.is_numeric() {
            info!("found num arg {:?}", arg);
            num_args.insert(arg);

        }
    }

    if num_args.len() == 0 {
        return result;
    }

    let mut num_args_in_cond_check: HashSet<Local> = HashSet::new();


    let mut visiter = BodyAliasVisitor::new(tcx, body, true);

    visiter.visit_body(body);

    let alias_res = visiter.get_result();

    for num_arg in num_args.clone().into_iter() {
        alias_res.local_alias.get(&num_arg)
        .into_iter()
        .flatten()
        .copied()
        .for_each(|a| {
            num_args.insert(a);
        })
    }

    for num in &num_args {
        info!("num {:?}", num);
    }

    // check if number argument has been used in condition check
    for (bb, bb_data) in body.basic_blocks().iter_enumerated() {
        for stmt in &bb_data.statements {
            match stmt.kind {
                rustc_middle::mir::StatementKind::Assign(ref data) => {
                    let lhs = data.0;
                    let rhs = &data.1;

                    match rhs {
                        rustc_middle::mir::Rvalue::BinaryOp(op, oprs) => {
                            match &oprs.0.place() {
                                Some(p) => {
                                    if num_args.contains(&p.local) {
                                        num_args_in_cond_check.insert(p.local);
                                        info!("found num arg in compare {:?}", p.local);

                                        alias_res.local_alias.get(&p.local)
                                        .into_iter()
                                        .flatten()
                                        .copied()
                                        .for_each(|a| {
                                            num_args_in_cond_check.insert(a);
                                            info!("found num arg in compare by alias {:?}", a);

                                        });
                                    }
                                    
                                },
                                None => {},
                            }

                            match &oprs.1.place() {
                                Some(p) => {
                                    if num_args.contains(&p.local) {
                                        num_args_in_cond_check.insert(p.local);
                                        info!("found num arg in compare {:?}", p.local);

                                        alias_res.local_alias.get(&p.local)
                                        .into_iter()
                                        .flatten()
                                        .copied()
                                        .for_each(|a| {
                                            num_args_in_cond_check.insert(a);
                                            info!("found num arg in compare by alias {:?}", a);

                                        });
                                    }
                                    
                                },
                                None => {},
                            }
                            
                        },
                        _ => {}
                    }
                },
                _ => {

                }
            }
        }

        // maybe we don't need to check if following locals got deference or not
        let calculated_ptr: Vec<Local> = Vec::new();

        // check call if any
        match resolve_callsite(tcx, body, bb, bb_data, true) {
            None => {}
            Some(cs) => {
                if let Some(ty) = cs.call_by_type {
                    let callee_name = match tcx.opt_item_name(cs.callee.def_id()) {
                        Some(ident) => ident.to_string(),
                        None => "".to_string(),
                    };

                    let callee_body = tcx.optimized_mir(cs.callee.def_id());
                    
                    if ty.is_unsafe_ptr() && callee_body.return_ty().is_unsafe_ptr() {
                        info!("found pointer calculation {:?}", callee_name);

                        for arg in cs.args {
                            if let Some(p) = arg.place() {
                                let offset_local = p.local;
                                info!("     arg {:?}", offset_local);

                                // if the index comes from function arguments and not used in conditional check
                                // and now used to calculate another pointer
                                if num_args.contains(&offset_local) && !num_args_in_cond_check.contains(&offset_local) {
                                    result.push(format!("{:?}", cs.span));
                                }
                            }
                        }
                    }
                }
            }
        };

        
    }
    return result;
}