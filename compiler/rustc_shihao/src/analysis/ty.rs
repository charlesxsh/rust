use std::error::Error;

use rustc_middle::{ty::TyCtxt};

use super::AnalysisOption;
pub trait RFAnalysis {
    fn analyze<'tcx, 'a>(&self, tcx: TyCtxt<'tcx>, option:&'a AnalysisOption) -> Result<serde_json::Value, Box<dyn Error>>;
}