pub mod analysis;
pub mod ty;
pub mod utils;

use rustc_hir::def_id::LOCAL_CRATE;
use rustc_middle::ty::TyCtxt;
use std::env;
use tracing::{debug, error, info};

pub fn run_analysis<'tcx>(tcx: TyCtxt<'tcx>) {
    let mut options: analysis::AnalysisOption = Default::default();

    options.output = env::var("RF_OUT").ok();
    if env::var("RF_UNSAFE_SPREAD").is_ok() {
        options.analyses.insert("unsafe-spread".to_string());
    }
    if env::var("RF_UNSAFE_INFO").is_ok() {
        options.analyses.insert("unsafe-info".to_string());
    }
    if env::var("RF_CHECKER_1").is_ok() {
        options.analyses.insert("checker-1".to_string());
    }
    if env::var("RF_CHECKER_4").is_ok() {
        options.analyses.insert("checker-4".to_string());
    }
    if env::var("RF_SCORE_FUZZ_ENTRY").is_ok() {
        options.analyses.insert("score-fuzz-entry".to_string());
    }
    if env::var("RF_SAFE_STORE").is_ok() {
        options.analyses.insert("safe_store".to_string());
    }
    options.unsafe_info_mode = env::var("RF_UNSAFE_INFO_MODE").ok();

    options.target_crate = env::var("RF_TARGET_CRATE").ok();

    let target_cwd = env::var("RF_TARGET_CWD").ok();
    let current_cwd = std::env::current_dir().unwrap();
    info!("option {:?}", options);

    let mut crate_name = tcx.crate_name(LOCAL_CRATE).to_string();
    crate_name = crate_name.trim_matches('\"').to_string();

    info!("current crate {}", crate_name);
    info!("current cwd {:?}", current_cwd);

    if let Some(c) = &options.target_crate {
        if *c != crate_name {
            info!("skip since target crate is {:?}", c);

            return;
        }
    }

    if let Some(c) = &target_cwd {
        let abs_target_cwd = std::fs::canonicalize(c).unwrap();

        if !current_cwd.starts_with(&abs_target_cwd) {
            info!("skip since target cwd is {:?}", abs_target_cwd);

            return;
        }
    }

    let mut dispatcher = analysis::AnalysisDispatcher::new(options.clone());
    dispatcher
        .register_analysis("unsafe-info", Box::new(analysis::unsafe_info::UnsafeInfoRFAnalysis {}));
    dispatcher.register_analysis(
        "unsafe-spread",
        Box::new(analysis::unsafe_spread::UnsafeSpreadRFAnalysis {}),
    );
    dispatcher.register_analysis(
        "checker-1",
        Box::new(analysis::checker_1::Checker1RFAnalysis {})
    );
    dispatcher.register_analysis(
        "checker-4",
        Box::new(analysis::checker_4::Checker4RFAnalysis {})
    );
    dispatcher.register_analysis(
        "safe_store",
        Box::new(analysis::safe_store::SafeStoreAnalysis {})
    );
    dispatcher.register_analysis("score-fuzz-entry", Box::new(analysis::score_fuzzentry::ScoreFuzzEntryAnalysis{}));
    match dispatcher.run(tcx) {
        Ok(result) => {
            if let Some(out) = &options.output {
                let contents = serde_json::to_string_pretty(&result).unwrap();
                match std::fs::write(out, contents) {
                    Ok(_) => {
                        info!("analysis result saved: {}", out);
                    }
                    Err(err) => {
                        error!("failed to save analysis result at {}: {:?}", out, err);
                    }
                }
            }
        }
        Err(err) => {
            error!("analysis run error: {:?}", err);
        },
    }
}
