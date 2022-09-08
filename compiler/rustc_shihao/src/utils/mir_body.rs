use rustc_middle::{mir::{Body, Local, LocalKind, BasicBlockData, BasicBlock, TerminatorKind, Operand}, ty::{Instance, InstanceDef, Ty, TyCtxt, PolyFnSig, TyKind::{Closure, FnPtr}}};
use rustc_span::Span;
use rustc_middle::ty::FnDef;
use rustc_middle::ty::subst::Subst;
use tracing::info;

pub fn get_body_return_local<'tcx>(body: &Body<'tcx>) -> Option<Local> {
    body.local_decls
        .iter_enumerated()
        .filter(|(l, _)| body.local_kind(*l) == LocalKind::ReturnPointer)
        .map(|(l, _)| l)
        .next()
}

pub fn get_body_arglocal_idx_pairs<'tcx>(body: &Body<'tcx>) -> Option<Vec<(usize, Local)>> {
    if body.arg_count > 0 {
        return Some(body.args_iter().enumerate().collect());
    }


    return None;
}
#[derive(Clone, Debug)]
pub struct CallSite<'tcx> {
    pub bb: BasicBlock,
    pub span: Span,
    pub callee: Instance<'tcx>,
    pub fn_sig: PolyFnSig<'tcx>,
    pub args: Vec<Operand<'tcx>>,
    pub call_by_type: Option<Ty<'tcx>>,
    pub in_unsafe: bool
}

pub fn resolve_callsite<'tcx>(
    tcx: TyCtxt<'tcx>,
    caller_body: &Body<'tcx>,
    bb: BasicBlock,
    bb_data: &BasicBlockData<'tcx>,
    ensure_callee_has_mir: bool
) -> Option<CallSite<'tcx>> {
    if ensure_callee_has_mir && bb_data.is_cleanup  {
        return None
    }
    // Only consider direct calls to functions
    let terminator = bb_data.terminator();
    let scope_data = &caller_body.source_scopes[terminator.source_info.scope];
    if let TerminatorKind::Call { ref args, ref func, ref destination, ref fn_span, .. } = terminator.kind {
        info!("terminator call {:?}", terminator);

        let func_ty = func.ty(caller_body, tcx);
        info!("ty {:?}", func_ty);

        if let FnDef(def_id, substs) = *func_ty.kind() {

            info!("found fn def {:?}", def_id);
            // To resolve an instance its substs have to be fully normalized.
            let param_env =
                tcx.param_env_reveal_all_normalized(caller_body.source.def_id());
            let substs = tcx.normalize_erasing_regions(param_env, substs);
            let callee =
                Instance::resolve(tcx, param_env, def_id, substs).ok().flatten()?;

            info!("callee def {:?}", callee.def);

            if let InstanceDef::Virtual(..) | InstanceDef::Intrinsic(_) = callee.def {
                
                return None;
            }

            if ensure_callee_has_mir && !check_mir_is_available(tcx, caller_body, &callee).is_ok() {
                info!("{:?} mir is unavailable", def_id);
                return None
            }

            let fn_sig = tcx.fn_sig(def_id).subst(tcx, substs);
            let mut call_by_type = None;
            if args.len() != substs.len() && args.len() > 0 {
                let caller_obj = &args[0];
                let caller_ty = caller_obj.ty(caller_body, tcx);
                call_by_type = Some(caller_ty);
            }

            
            return Some(CallSite {
                callee,
                fn_sig,
                args: args.clone(),
                bb,
                span: *fn_span,
                call_by_type,
                in_unsafe:!scope_data.safety
            });
        }


        if let FnPtr(sig) = *func_ty.kind() {
            info!("found fnptr {:?}", sig);
            //TODO: find candidates from all functions & closures
        }

    }
    None
}

pub fn check_mir_is_available<'tcx>(
    tcx: TyCtxt<'tcx>,
    caller_body: &Body<'tcx>,
    callee: &Instance<'tcx>,
) -> Result<(), &'static str> {
    if callee.def_id() == caller_body.source.def_id() {
        return Err("self-recursion");
    }

    match callee.def {
        InstanceDef::Item(_) => {
            // If there is no MIR available (either because it was not in metadata or
            // because it has no MIR because it's an extern function), then the inliner
            // won't cause cycles on this.
            if !tcx.is_mir_available(callee.def_id()) {
                return Err("item MIR unavailable");
            }
        }
        // These have no own callable MIR.
        InstanceDef::Intrinsic(_) | InstanceDef::Virtual(..) => {
            return Err("instance without MIR (intrinsic / virtual)");
        }
        // This cannot result in an immediate cycle since the callee MIR is a shim, which does
        // not get any optimizations run on it. Any subsequent inlining may cause cycles, but we
        // do not need to catch this here, we can wait until the inliner decides to continue
        // inlining a second time.
        InstanceDef::VtableShim(_)
        | InstanceDef::ReifyShim(_)
        | InstanceDef::FnPtrShim(..)  // TODO: debug here
        | InstanceDef::ClosureOnceShim { .. }
        | InstanceDef::DropGlue(..)
        | InstanceDef::CloneShim(..) => return Err("ignore Shim and DropGlue"),
    }

    Ok(())
}

pub fn get_cnt_of_unsafe<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> u32 {
    let mut cnt = 0;
    for bb in body.basic_blocks() {
        for stmt in &bb.statements {
            let scope = &body.source_scopes[stmt.source_info.scope];
            if !scope.safety {
                cnt += 1;
            }

        }
    }
    return cnt;
}

pub fn get_cnt_of_ffi_call<'tcx>(tcx: TyCtxt<'tcx>, callsites: &Vec<CallSite>) -> u32 {
    let mut cnt = 0;
    for cs in callsites {
        if !tcx.is_mir_available(cs.callee.def_id()) && cs.in_unsafe {
            cnt += 1;
        }
    }
    return cnt;
}