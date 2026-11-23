pub mod ast;
pub mod lower;

use anyhow::{anyhow, bail, Result};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use vf_ir::Module;

extern "C" {
    fn vf_slang_elaborate(
        argc: i32,
        argv: *const *const c_char,
        out_json: *mut *mut c_char,
        out_diag: *mut *mut c_char,
    ) -> i32;
    fn vf_slang_free_string(ptr: *mut c_char);
}

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    pub files: Vec<PathBuf>,
    pub top: Option<String>,
    pub defines: Vec<String>,
    pub include_dirs: Vec<PathBuf>,
    pub params: Vec<String>,
    /// Define `FORMAL` preprocessor macro automatically (default: true).
    pub formal_mode: bool,
}

impl CompileOptions {
    pub fn for_files(files: Vec<PathBuf>) -> Self {
        Self {
            files,
            formal_mode: true,
            ..Default::default()
        }
    }
}

pub fn elaborate_json(opts: &CompileOptions) -> Result<(String, String)> {
    let mut args = Vec::new();
    args.push("veloformal".to_string());
    if opts.formal_mode {
        args.push("-DFORMAL".to_string());
    }
    for d in &opts.defines {
        args.push(format!("-D{d}"));
    }
    for inc in &opts.include_dirs {
        args.push("-I".to_string());
        args.push(inc.to_string_lossy().into_owned());
    }
    for p in &opts.params {
        args.push("-G".to_string());
        args.push(p.clone());
    }
    if let Some(top) = &opts.top {
        args.push("--top".to_string());
        args.push(top.clone());
    }
    for f in &opts.files {
        args.push(f.to_string_lossy().into_owned());
    }

    let c_args: Vec<CString> = args
        .iter()
        .map(|s| CString::new(s.as_str()).map_err(|e| anyhow!("invalid argument: {e}")))
        .collect::<Result<_>>()?;
    let ptrs: Vec<*const c_char> = c_args.iter().map(|s| s.as_ptr()).collect();

    let mut out_json: *mut c_char = std::ptr::null_mut();
    let mut out_diag: *mut c_char = std::ptr::null_mut();

    let rc = unsafe {
        vf_slang_elaborate(
            ptrs.len() as i32,
            ptrs.as_ptr(),
            &mut out_json,
            &mut out_diag,
        )
    };

    let diag = if !out_diag.is_null() {
        let s = unsafe { CStr::from_ptr(out_diag) }
            .to_string_lossy()
            .into_owned();
        unsafe { vf_slang_free_string(out_diag) };
        s
    } else {
        String::new()
    };

    let json = if !out_json.is_null() {
        let s = unsafe { CStr::from_ptr(out_json) }
            .to_string_lossy()
            .into_owned();
        unsafe { vf_slang_free_string(out_json) };
        s
    } else {
        String::new()
    };

    if rc != 0 {
        bail!(
            "slang elaboration failed (code {rc}):\n{}",
            if diag.is_empty() {
                "(no diagnostics)"
            } else {
                &diag
            }
        );
    }

    Ok((json, diag))
}

pub use lower::InstrumentOptions;

pub fn compile_to_ir(opts: &CompileOptions) -> Result<Module> {
    compile_to_ir_with_options(opts, InstrumentOptions::default())
}

pub fn compile_to_ir_with_options(
    opts: &CompileOptions,
    inst: InstrumentOptions,
) -> Result<Module> {
    let (json, _diag) = elaborate_json(opts)?;
    let design: ast::ElaboratedDesign =
        serde_json::from_str(&json).map_err(|e| anyhow!("failed to parse slang bridge JSON: {e}"))?;
    lower::lower_to_ir_with_options(&design, inst)
}
