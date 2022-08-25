pub mod ty;
pub mod unsafe_info;
pub mod unsafe_spread;
pub mod checker_1;
pub mod checker_4;
pub mod score_fuzzentry;
pub mod safe_store;
use crate::analysis::ty::RFAnalysis;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_middle::ty::TyCtxt;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use tracing::debug;

pub struct RFCallbacks {
    analysis_option: AnalysisOption,
}

impl RFCallbacks {
    pub fn new(analysis_option: AnalysisOption) -> Self {
        Self { analysis_option }
    }
}

#[derive(Clone, Debug)]
pub struct AnalysisOption {
    pub entry_point: String,
    pub entry_def_id_index: Option<u32>,
    pub target_crate: Option<String>,
    pub output: Option<String>,
    pub analyses: HashSet<String>,

    // unsafe-info flags
    pub unsafe_info_mode: Option<String>,
}

impl Default for AnalysisOption {
    fn default() -> Self {
        Self {
            entry_point: String::from("main"),
            entry_def_id_index: None,
            output: None,
            target_crate: None,
            analyses: HashSet::new(),
            unsafe_info_mode: None,
        }
    }
}

pub struct AnalysisDispatcher {
    pub option: AnalysisOption,
    pub analyses: HashMap<&'static str, Box<dyn RFAnalysis>>,
}

impl AnalysisDispatcher {
    pub fn new(option: AnalysisOption) -> Self {
        Self { option, analyses: Default::default() }
    }

    pub fn register_analysis(&mut self, name: &'static str, analysis: Box<dyn RFAnalysis>) {
        self.analyses.insert(name, analysis);
    }

    pub fn run<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Result<serde_json::Value, Box<dyn Error>> {
        let mut result: HashMap<String, serde_json::Value> = HashMap::new();
        for (name, a) in &self.analyses {
            if self.option.analyses.contains(&name.to_string()) {
                let res = a.analyze(tcx, &self.option)?;
                result.insert(name.to_string(), res);
            }
        }

        return serde_json::to_value(result)
            .or_else(|err| Err(Box::new(err) as Box<dyn std::error::Error>));
    }
}

pub fn find_main_func<'tcx>(tcx: TyCtxt) -> Option<DefId> {
    let mut entry_func_def_id: Option<DefId> = None;
    for def_id in tcx.hir().body_owners() {
        let def_kind = tcx.def_kind(def_id);
        // Find the DefId for the entry point, note that the entry point must be a function
        if def_kind == DefKind::Fn || def_kind == DefKind::AssocFn {
            let item_name = tcx.item_name(def_id.to_def_id());
            if item_name.to_string() == "main" {
                entry_func_def_id = Some(def_id.to_def_id());
                debug!("Entry Point: {:?}, DefId: {:?}", item_name, def_id);
            }
        }
    }
    entry_func_def_id
}

#[derive(Debug, Clone)]
pub struct NoEntryFound;

impl Error for NoEntryFound {}

impl fmt::Display for NoEntryFound {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "no entry function found")
    }
}
