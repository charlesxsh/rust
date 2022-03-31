// #![feature(rustc_private)]
// extern crate rustc_driver;
// extern crate rustc_errors;
// extern crate rustc_interface;
// extern crate rustc_middle;
// extern crate rustc_session;

// use std::env;

// use rf_rustc_wrapper::option::AnalysisOption;
// use rustc_session::early_error;
// use rustc_session::config::ErrorOutputType;
// use std::process;
// use log::info;
// use rf_rustc_wrapper::callback;

// /// Exit status code used for successful compilation and help output.
// pub const EXIT_SUCCESS: i32 = 0;

// /// Exit status code used for compilation failures and invalid flags.
// pub const EXIT_FAILURE: i32 = 1;

// fn main() {
//     pretty_env_logger::init();

//     let result = rustc_driver::catch_fatal_errors(move || {
//         let mut rustc_args = env::args_os()
//             .enumerate()
//             .map(|(i, arg)| {
//                 arg.into_string().unwrap_or_else(|arg| {
//                     early_error(
//                         ErrorOutputType::default(),
//                         &format!("Argument {} is not valid Unicode: {:?}", i, arg),
//                     )
//                 })
//             })
//             .collect::<Vec<_>>();

//         if let Some(sysroot) = compile_time_sysroot() {
//             let sysroot_flag = "--sysroot";
//             if !rustc_args.iter().any(|e| e == sysroot_flag) {
//                 // We need to overwrite the default that librustc would compute.
//                 rustc_args.push(sysroot_flag.to_owned());
//                 rustc_args.push(sysroot);
//             }
//         }

//         let always_encode_mir = "-Zalways_encode_mir";
//         if !rustc_args.iter().any(|e| e == always_encode_mir) {
//             // Get MIR code for all code related to the crate (including the dependencies and standard library)
//             rustc_args.push(always_encode_mir.to_owned());
//         }

//         //Add this to support analyzing no_std libraries
//         rustc_args.push("-Clink-arg=-nostartfiles".to_owned());

//         // Disable unwind to simplify the CFG
//         rustc_args.push("-Cpanic=abort".to_owned());

//         info!("{:?}", rustc_args);

//         let mut options: AnalysisOption = Default::default();

//         options.output = env::var("RF_OUT").ok();
//         if env::var("RF_UNSAFE_SPREAD").is_ok() {
//             options.analyses.insert("unsafe-spread".to_string());
//         }
//         if env::var("RF_UNSAFE_INFO").is_ok() {
//             options.analyses.insert("unsafe-info".to_string());
//         }
//         info!("Analysis Option {:?}", options);

//         let mut callbacks = callback::RFCallbacks::new(options);

//         let run_compiler = rustc_driver::RunCompiler::new(&rustc_args, &mut callbacks);
//         run_compiler.run()

//     })
//     .and_then(|result| result);

//     let exit_code = match result {
//         Ok(_) => EXIT_SUCCESS,
//         Err(_) => EXIT_FAILURE,
//     };

//     process::exit(exit_code);
// }

// /// Copied from Miri
// /// Returns the "default sysroot" if no `--sysroot` flag is set.
// /// Should be a compile-time constant.
// pub fn compile_time_sysroot() -> Option<String> {
//     if option_env!("RUSTC_STAGE").is_some() {
//         // This is being built as part of rustc, and gets shipped with rustup.
//         // We can rely on the sysroot computation in librustc.
//         return None;
//     }
//     // For builds outside rustc, we need to ensure that we got a sysroot
//     // that gets used as a default.  The sysroot computation in librustc would
//     // end up somewhere in the build dir.
//     // Taken from PR <https://github.com/Manishearth/rust-clippy/pull/911>.
//     let home = option_env!("RUSTUP_HOME").or(option_env!("MULTIRUST_HOME"));
//     let toolchain = option_env!("RUSTUP_TOOLCHAIN").or(option_env!("MULTIRUST_TOOLCHAIN"));
//     Some(match (home, toolchain) {
//         (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
//         _ => option_env!("RUST_SYSROOT")
//             .expect("To build Miri without rustup, set the `RUST_SYSROOT` env var at build time")
//             .to_owned(),
//     })
// }
