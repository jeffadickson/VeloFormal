mod sby;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use sby::{parse_sby_tasks, run_sby_task};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_apps::cdc::{run_cdc_analysis, CdcConfig, CdcSeverity, GrayStatus};
use vf_apps::connectivity::{parse_connectivity_csv, run_connectivity, ConnStatus};
use vf_apps::covercheck::{run_covercheck, CoverCheckConfig, CoverageStatus};
use vf_apps::regcheck::{parse_regmap_csv, run_regcheck, RegBusConfig, RegCheckStatus};
use vf_apps::secure::{run_secure_check, SecureConfig, SecureOutcome};
use vf_apps::xcheck::{run_xcheck, XCheckConfig, XStatus};
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::equiv::{check_slec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::live::{run_live, LiveConfig, LiveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_engine::vacuity::{check_vacuity, VacuityConfig, VacuityStatus};
use vf_ir::passes::{async2sync, clk2fflogic, coi_prune, has_multiple_clocks};
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::{aiger, btor2, smt2, vcd};

#[derive(Parser)]
#[command(
    name = "veloformal",
    version,
    about = "Native Verilog/SystemVerilog formal verification suite (embedded Slang + RTL IR + SMT/PDR/SBY + Questa-class Formal Apps)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Clone)]
struct SourceArgs {
    /// Verilog / SystemVerilog source files
    #[arg(required = true)]
    files: Vec<PathBuf>,

    /// Top-level module name
    #[arg(long)]
    top: Option<String>,

    /// Preprocessor macro definitions (-D NAME or -D NAME=VAL)
    #[arg(short = 'D', long = "define")]
    defines: Vec<String>,

    /// Include directories
    #[arg(short = 'I', long = "include")]
    include_dirs: Vec<PathBuf>,

    /// Parameter overrides (-G PARAM=VAL)
    #[arg(short = 'G', long = "param")]
    params: Vec<String>,

    /// Enable multi-clock clk2fflogic transformation
    #[arg(long)]
    multiclock: bool,
}

impl SourceArgs {
    fn to_compile_options(&self) -> CompileOptions {
        CompileOptions {
            files: self.files.clone(),
            top: self.top.clone(),
            defines: self.defines.clone(),
            include_dirs: self.include_dirs.clone(),
            params: self.params.clone(),
            formal_mode: true,
        }
    }

    fn compile_raw(&self) -> Result<Module> {
        compile_to_ir(&self.to_compile_options())
    }

    fn compile_and_prepare(&self) -> Result<Module> {
        let mut m = self.compile_raw()?;
        if self.multiclock || has_multiple_clocks(&m) {
            clk2fflogic(&mut m);
        }
        if m.ffs.iter().any(|f| f.async_reset.is_some()) {
            async2sync(&mut m);
        }
        Ok(m)
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Run Bounded Model Checking (BMC) on Verilog/SystemVerilog assertions
    Bmc {
        #[command(flatten)]
        src: SourceArgs,
        /// Unrolling depth (cycles)
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// Skip checking assertions for the first N cycles
        #[arg(long, default_value_t = 0)]
        skip: usize,
        /// Append N extra cycles after a failing assertion in VCD trace
        #[arg(long, default_value_t = 0)]
        append: usize,
        /// SMT solver executable (z3, bitwuzla, cvc5)
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD counterexample waveform if an assertion fails
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Prove assertions unboundedly using k-induction and/or IC3/PDR (abc pdr)
    Prove {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum induction depth k (for k-induction)
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// Proof engine: kind, pdr, or auto (k-induction then PDR fallback)
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Apply Cone-of-Influence (COI) pruning before proving
        #[arg(long)]
        coi: bool,
        /// SMT solver executable (z3, bitwuzla, cvc5)
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD counterexample waveform if an assertion fails
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Check liveness & fairness properties (`s_eventually`) via bounded lasso detection
    Live {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum lasso length (cycles)
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD counterexample lasso if liveness fails
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Find shortest witness traces reaching all cover() statements
    Cover {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum search depth (cycles)
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// Append N extra cycles after each cover hit
        #[arg(long, default_value_t = 0)]
        append: usize,
        /// SMT solver executable (z3, bitwuzla, cvc5)
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Directory in which to write VCD witness waveforms
        #[arg(long)]
        vcd_dir: Option<PathBuf>,
    },
    /// Sequential/Combinational Logic Equivalence Checking (Questa SLEC: latency offsets, skip cycles, k-ind & PDR)
    Equiv {
        /// Reference (gold) Verilog/SV file(s)
        #[arg(long, required = true)]
        gold: Vec<PathBuf>,
        /// Implementation (gate) Verilog/SV file(s)
        #[arg(long, required = true)]
        gate: Vec<PathBuf>,
        /// Top module name in gold
        #[arg(long)]
        gold_top: Option<String>,
        /// Top module name in gate
        #[arg(long)]
        gate_top: Option<String>,
        /// Proof engine: auto, kind, or pdr
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Pipeline latency delay cycles applied to gold outputs (when gate has more pipeline stages)
        #[arg(long, default_value_t = 0)]
        latency_gold: usize,
        /// Pipeline latency delay cycles applied to gate outputs (when gold has more pipeline stages)
        #[arg(long, default_value_t = 0)]
        latency_gate: usize,
        /// Initial settling cycles to mask comparison
        #[arg(long, default_value_t = 0)]
        skip_cycles: usize,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD counterexample if designs diverge
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Questa PropCheck vacuity check: verify assertion antecedents/guards are actually reachable
    Vacuity {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum reachability search depth
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Questa AutoCheck: push-button formal RTL bug hunting (OOB memory, div-by-zero, unique/priority, overflow, FSM deadlock)
    Autocheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum verification depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// Enable unsigned arithmetic overflow/underflow checks
        #[arg(long)]
        check_overflow: bool,
        /// FSM state register name(s) to check for deadlock states
        #[arg(long = "fsm")]
        fsm_signals: Vec<String>,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Directory in which to write VCD traces for discovered bugs
        #[arg(long)]
        vcd_dir: Option<PathBuf>,
    },
    /// Questa CoverCheck: branch reachability witnesses & unbounded dead-code unreachability proofs
    Covercheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum reachability search depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Directory in which to write VCD witness waveforms
        #[arg(long)]
        vcd_dir: Option<PathBuf>,
    },
    /// Questa Connectivity Check: prove static/conditional pin-to-pin & bus routing rules from CSV
    Connectivity {
        #[command(flatten)]
        src: SourceArgs,
        /// CSV connectivity specification (`src,dst[,cond[,invert[,delay]]]`)
        #[arg(long)]
        spec: PathBuf,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Questa Secure Check: formal Information Flow Tracking (confidentiality & integrity via 2-copy self-composition)
    Secure {
        #[command(flatten)]
        src: SourceArgs,
        /// Secret / untrusted source signal(s)
        #[arg(long = "source", required = true)]
        sources: Vec<String>,
        /// Public / protected target signal(s) that must remain non-interfering
        #[arg(long = "target", required = true)]
        targets: Vec<String>,
        /// Optional declassification / authorized-flow condition (e.g. `auth_ok`)
        #[arg(long)]
        declassify: Option<String>,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write 2-copy counterexample VCD if an information leak is found
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Questa X-Check: 4-state dual-rail formal X-propagation verification
    Xcheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Target signal(s) to verify X-free (defaults to all module outputs)
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Additional primary input(s) allowed to inject X values
        #[arg(long = "x-source")]
        x_sources: Vec<String>,
        /// Initial settling/reset cycles to skip before asserting X-freedom
        #[arg(long, default_value_t = 0)]
        skip_cycles: usize,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Directory in which to write VCD traces for X-leaks
        #[arg(long)]
        vcd_dir: Option<PathBuf>,
    },
    /// Questa Register Check: verify CSR reset values, RW/RO/W1C/W1S/WO access policies, masks & non-aliasing from CSV
    Regcheck {
        #[command(flatten)]
        src: SourceArgs,
        /// CSV register map (`name,addr,width,reset,access[,mask]`)
        #[arg(long)]
        spec: PathBuf,
        /// Reset signal name (auto-detected if omitted)
        #[arg(long)]
        rst: Option<String>,
        /// Force active-low reset interpretation
        #[arg(long)]
        rst_active_low: Option<bool>,
        /// Write enable signal name
        #[arg(long)]
        wr_en: Option<String>,
        /// Read enable signal name
        #[arg(long)]
        rd_en: Option<String>,
        /// Address bus signal name
        #[arg(long)]
        addr: Option<String>,
        /// Write data bus signal name
        #[arg(long)]
        wdata: Option<String>,
        /// Read data bus signal name
        #[arg(long)]
        rdata: Option<String>,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 10)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Questa CDC / RDC: structural clock-domain & reset-domain crossing analysis + formal Gray-code verification
    Cdc {
        #[command(flatten)]
        src: SourceArgs,
        /// Signal(s) to formally verify as Gray-coded (`popcount(s ^ $past(s)) <= 1`)
        #[arg(long = "gray")]
        gray_signals: Vec<String>,
        /// Maximum k-induction depth for Gray-code proofs
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Execute a SymbiYosys (.sby) formal verification project (supports [tasks], [file ...], portfolio engines)
    Sby {
        /// Path to .sby configuration file
        file: PathBuf,
        /// Optional subset of task names to execute (defaults to all tasks)
        tasks: Vec<String>,
        /// Directory for SBY-compatible status and VCD outputs
        #[arg(short = 'd', long)]
        workdir: Option<PathBuf>,
    },
    /// Elaborate Verilog/SV via embedded Slang and dump the lowered word-level RTL IR
    DumpIr {
        #[command(flatten)]
        src: SourceArgs,
    },
    /// Elaborate Verilog/SV and export standard BTOR2
    DumpBtor2 {
        #[command(flatten)]
        src: SourceArgs,
    },
    /// Elaborate Verilog/SV and export SMT-LIB2 transition system
    DumpSmt2 {
        #[command(flatten)]
        src: SourceArgs,
    },
    /// Elaborate Verilog/SV and export bit-blasted binary AIGER (.aig)
    DumpAiger {
        #[command(flatten)]
        src: SourceArgs,
        /// Output `.aig` file path
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("veloformal error: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Bmc {
            src,
            depth,
            skip,
            append,
            solver,
            vcd: vcd_path,
        } => {
            let module = src.compile_and_prepare()?;
            let outcome = run_bmc(
                &module,
                &BmcConfig {
                    depth,
                    solver,
                    skip,
                    append,
                },
            )?;
            match outcome {
                BmcOutcome::Pass { depth } => {
                    println!("PASS (BMC depth {depth}) — no assertion failed within {depth} cycles");
                    Ok(ExitCode::SUCCESS)
                }
                BmcOutcome::Fail {
                    property,
                    step,
                    trace,
                } => {
                    println!("FAIL — assertion `{property}` violated at cycle {step}");
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&module, &trace))?;
                        println!("Counterexample VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                BmcOutcome::AssumptionsUnsat { step } => {
                    println!("ASSUMPTIONS_UNSAT — assumptions are contradictory at cycle {step}");
                    Ok(ExitCode::from(1))
                }
                BmcOutcome::Unknown { step } => {
                    println!("UNKNOWN — solver returned unknown at cycle {step}");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Prove {
            src,
            depth,
            engine,
            coi,
            solver,
            vcd: vcd_path,
        } => {
            let mut module = src.compile_and_prepare()?;
            if coi {
                coi_prune(&mut module);
            }
            let outcome = match engine.as_str() {
                "pdr" => run_pdr(
                    &module,
                    &PdrConfig {
                        smt_solver: solver,
                        ..Default::default()
                    },
                )?,
                "kind" => run_kinduction(
                    &module,
                    &ProveConfig {
                        max_k: depth,
                        solver,
                    },
                )?,
                _ => {
                    let k_res = run_kinduction(
                        &module,
                        &ProveConfig {
                            max_k: depth,
                            solver: solver.clone(),
                        },
                    )?;
                    match k_res {
                        ProveOutcome::Unknown { .. } => run_pdr(
                            &module,
                            &PdrConfig {
                                smt_solver: solver,
                                ..Default::default()
                            },
                        )
                        .unwrap_or(k_res),
                        other => other,
                    }
                }
            };
            match outcome {
                ProveOutcome::Proved { k } => {
                    if engine == "pdr" || k == 0 {
                        println!("PROVED (unbounded)");
                    } else {
                        println!("PROVED (unbounded, k-induction k={k})");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                } => {
                    println!("FAIL — assertion `{property}` violated at cycle {step}");
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&module, &trace))?;
                        println!("Counterexample VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                ProveOutcome::AssumptionsUnsat { step } => {
                    println!("ASSUMPTIONS_UNSAT — assumptions are contradictory at cycle {step}");
                    Ok(ExitCode::from(1))
                }
                ProveOutcome::Unknown { max_k } => {
                    println!("UNKNOWN — not proved up to k={max_k}");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Live {
            src,
            depth,
            solver,
            vcd: vcd_path,
        } => {
            let module = src.compile_and_prepare()?;
            match run_live(&module, &LiveConfig { depth, solver })? {
                LiveOutcome::Pass { depth } => {
                    println!("PASS (live depth {depth}) — no fair starving lasso found");
                    Ok(ExitCode::SUCCESS)
                }
                LiveOutcome::Fail {
                    property,
                    loop_start,
                    loop_end,
                    trace,
                } => {
                    println!(
                        "FAIL — liveness `{property}` starved on fair lasso [{loop_start}..{loop_end}]"
                    );
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&module, &trace))?;
                        println!("Lasso counterexample VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                LiveOutcome::Unknown { step } => {
                    println!("UNKNOWN — solver returned unknown at step {step}");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Cover {
            src,
            depth,
            append,
            solver,
            vcd_dir,
        } => {
            let module = src.compile_and_prepare()?;
            let results = run_cover(
                &module,
                &CoverConfig {
                    depth,
                    solver,
                    append,
                },
            )?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }
            let mut all_reached = true;
            for (i, r) in results.iter().enumerate() {
                match &r.reached {
                    Some((step, trace)) => {
                        println!("COVER REACHED — `{}` at cycle {step}", r.name);
                        if let Some(dir) = &vcd_dir {
                            let path = dir.join(format!("cover_{i}.vcd"));
                            fs::write(&path, vcd::write_vcd(&module, trace))?;
                        }
                    }
                    None => {
                        println!("COVER UNREACHED — `{}` within {depth} cycles", r.name);
                        all_reached = false;
                    }
                }
            }
            Ok(if all_reached {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::Equiv {
            gold,
            gate,
            gold_top,
            gate_top,
            engine,
            latency_gold,
            latency_gate,
            skip_cycles,
            depth,
            solver,
            vcd: vcd_path,
        } => {
            let mut g_opts = CompileOptions::for_files(gold);
            g_opts.top = gold_top;
            let mut t_opts = CompileOptions::for_files(gate);
            t_opts.top = gate_top;
            let gold_mod = compile_to_ir(&g_opts)?;
            let gate_mod = compile_to_ir(&t_opts)?;
            let (miter, outcome) = check_slec(
                &gold_mod,
                &gate_mod,
                &SlecConfig {
                    depth,
                    solver,
                    engine: engine.clone(),
                    latency_gold,
                    latency_gate,
                    skip_cycles,
                },
            )?;
            match outcome {
                ProveOutcome::Proved { k } => {
                    if engine == "pdr" || k == 0 {
                        println!("EQUIVALENT (unbounded SLEC proof)");
                    } else {
                        println!("EQUIVALENT (proved via k-induction k={k})");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                ProveOutcome::Fail {
                    property,
                    step,
                    trace,
                } => {
                    println!("NOT EQUIVALENT — `{property}` diverged at cycle {step}");
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&miter, &trace))?;
                        println!("Counterexample VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                ProveOutcome::AssumptionsUnsat { step } => {
                    println!("ASSUMPTIONS_UNSAT at cycle {step}");
                    Ok(ExitCode::from(1))
                }
                ProveOutcome::Unknown { max_k } => {
                    println!("UNKNOWN — equivalence not decided up to k={max_k}");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Vacuity { src, depth, solver } => {
            let module = src.compile_and_prepare()?;
            let results = check_vacuity(
                &module,
                &VacuityConfig {
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            let mut any_vacuous = false;
            for r in &results {
                match &r.status {
                    VacuityStatus::NonVacuous { step, .. } => {
                        println!("NON-VACUOUS — `{}` antecedent hit at cycle {step}", r.property);
                    }
                    VacuityStatus::Vacuous => {
                        println!("VACUOUS — `{}` antecedent is UNREACHABLE for all time!", r.property);
                        any_vacuous = true;
                    }
                    VacuityStatus::BoundedVacuous { depth } => {
                        println!(
                            "BOUNDED-VACUOUS — `{}` antecedent not hit within {depth} cycles",
                            r.property
                        );
                        any_vacuous = true;
                    }
                }
            }
            Ok(if any_vacuous {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Autocheck {
            src,
            depth,
            check_overflow,
            fsm_signals,
            solver,
            vcd_dir,
        } => {
            let (module, findings) = run_autocheck(
                &src.to_compile_options(),
                &AutoCheckConfig {
                    depth,
                    solver,
                    check_overflow,
                    fsm_signals,
                    use_pdr: true,
                },
            )?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }
            let mut any_bug = false;
            for (i, f) in findings.iter().enumerate() {
                match &f.status {
                    CheckStatus::Safe => {
                        println!("SAFE [{}] `{}`", f.category, f.name);
                    }
                    CheckStatus::Bug { step, trace } => {
                        println!("BUG  [{}] `{}` violated at cycle {step}", f.category, f.name);
                        if let Some(dir) = &vcd_dir {
                            let path = dir.join(format!("autocheck_{i}.vcd"));
                            fs::write(&path, vcd::write_vcd(&module, trace))?;
                        }
                        any_bug = true;
                    }
                    CheckStatus::Warning(msg) => {
                        println!("WARN [{}] `{}` — {msg}", f.category, f.name);
                    }
                    CheckStatus::Unknown => {
                        println!("UNKNOWN [{}] `{}`", f.category, f.name);
                    }
                }
            }
            Ok(if any_bug {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Covercheck {
            src,
            depth,
            solver,
            vcd_dir,
        } => {
            let (module, items) = run_covercheck(
                &src.to_compile_options(),
                &CoverCheckConfig {
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }
            let mut reachable = 0usize;
            let mut unreachable = 0usize;
            let mut undecided = 0usize;
            for (i, item) in items.iter().enumerate() {
                match &item.status {
                    CoverageStatus::Reachable { step, trace } => {
                        reachable += 1;
                        println!("REACHABLE   `{}` at cycle {step}", item.name);
                        if let Some(dir) = &vcd_dir {
                            let path = dir.join(format!("branch_{i}.vcd"));
                            fs::write(&path, vcd::write_vcd(&module, trace))?;
                        }
                    }
                    CoverageStatus::Unreachable => {
                        unreachable += 1;
                        println!("UNREACHABLE `{}` (proved dead code)", item.name);
                    }
                    CoverageStatus::Undecided => {
                        undecided += 1;
                        println!("UNDECIDED   `{}`", item.name);
                    }
                }
            }
            println!(
                "CoverCheck summary: {} total ({} reachable, {} proved unreachable, {} undecided)",
                items.len(),
                reachable,
                unreachable,
                undecided
            );
            Ok(ExitCode::SUCCESS)
        }
        Commands::Connectivity {
            src,
            spec,
            depth,
            solver,
        } => {
            let module = src.compile_and_prepare()?;
            let rules = parse_connectivity_csv(&spec)?;
            let results = run_connectivity(&module, &rules, depth, &solver)?;
            let mut all_ok = true;
            for r in &results {
                match &r.status {
                    ConnStatus::Proved => {
                        println!(
                            "PROVED `{}` ({} -> {})",
                            r.rule.name, r.rule.src, r.rule.dst
                        );
                    }
                    ConnStatus::Fail { step, .. } => {
                        println!(
                            "FAIL   `{}` ({} -> {}) diverged at cycle {step}",
                            r.rule.name, r.rule.src, r.rule.dst
                        );
                        all_ok = false;
                    }
                    ConnStatus::Unknown => {
                        println!("UNKNOWN `{}`", r.rule.name);
                        all_ok = false;
                    }
                }
            }
            Ok(if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::Secure {
            src,
            sources,
            targets,
            declassify,
            depth,
            solver,
            vcd: vcd_path,
        } => {
            let module = src.compile_and_prepare()?;
            let (miter, outcome) = run_secure_check(
                &module,
                &SecureConfig {
                    sources,
                    targets,
                    declassify,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            match outcome {
                SecureOutcome::Secure => {
                    println!("SECURE — proved non-interference (zero information flow)");
                    Ok(ExitCode::SUCCESS)
                }
                SecureOutcome::Leak { step, trace } => {
                    println!("LEAK — information flow violation detected at cycle {step}");
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&miter, &trace))?;
                        println!("2-copy counterexample VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                SecureOutcome::Unknown => {
                    println!("UNKNOWN — information flow check inconclusive");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Xcheck {
            src,
            targets,
            x_sources,
            skip_cycles,
            depth,
            solver,
            vcd_dir,
        } => {
            let module = src.compile_and_prepare()?;
            let (dual, results) = run_xcheck(
                &module,
                &XCheckConfig {
                    targets,
                    x_sources,
                    skip_cycles,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }
            let mut any_leak = false;
            for (i, r) in results.iter().enumerate() {
                match &r.status {
                    XStatus::XFree => {
                        println!("X-FREE `{}`", r.signal);
                    }
                    XStatus::XLeak { step, trace } => {
                        println!("X-LEAK `{}` reached X at cycle {step}", r.signal);
                        if let Some(dir) = &vcd_dir {
                            let path = dir.join(format!("xcheck_{i}.vcd"));
                            fs::write(&path, vcd::write_vcd(&dual, trace))?;
                        }
                        any_leak = true;
                    }
                    XStatus::Unknown => {
                        println!("UNKNOWN `{}`", r.signal);
                        any_leak = true;
                    }
                }
            }
            Ok(if any_leak {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Regcheck {
            src,
            spec,
            rst,
            rst_active_low,
            wr_en,
            rd_en,
            addr,
            wdata,
            rdata,
            depth,
            solver,
        } => {
            let module = src.compile_and_prepare()?;
            let specs = parse_regmap_csv(&spec)?;
            let findings = run_regcheck(
                &module,
                &specs,
                &RegBusConfig {
                    rst,
                    rst_active_low,
                    wr_en,
                    rd_en,
                    addr,
                    wdata,
                    rdata,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            let mut all_ok = true;
            for f in &findings {
                match &f.status {
                    RegCheckStatus::Pass => {
                        println!("PASS [{}] {}", f.register, f.check);
                    }
                    RegCheckStatus::Fail { step, .. } => {
                        println!("FAIL [{}] {} violated at cycle {step}", f.register, f.check);
                        all_ok = false;
                    }
                    RegCheckStatus::Unknown => {
                        println!("UNKNOWN [{}] {}", f.register, f.check);
                        all_ok = false;
                    }
                }
            }
            Ok(if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::Cdc {
            src,
            gray_signals,
            depth,
            solver,
        } => {
            let raw = src.compile_raw()?;
            let report = run_cdc_analysis(
                &raw,
                &CdcConfig {
                    gray_signals,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            println!("Clock domains ({}): {}", report.clock_domains.len(), report.clock_domains.join(", "));
            let mut any_err = false;
            for f in &report.findings {
                let tag = match f.severity {
                    CdcSeverity::Error => {
                        any_err = true;
                        "ERROR"
                    }
                    CdcSeverity::Warning => "WARN ",
                    CdcSeverity::Safe => "SAFE ",
                };
                println!(
                    "{tag} [{}] {} ({}) -> {} ({}): {}",
                    f.kind, f.src_signal, f.src_domain, f.dst_signal, f.dst_domain, f.detail
                );
            }
            for g in &report.gray_results {
                match &g.status {
                    GrayStatus::Proved => {
                        println!("GRAY PROVED `{}` (Hamming distance <= 1 per cycle)", g.signal);
                    }
                    GrayStatus::Fail { step, .. } => {
                        println!("GRAY FAIL   `{}` changed >1 bit at cycle {step}", g.signal);
                        any_err = true;
                    }
                    GrayStatus::Unknown => {
                        println!("GRAY UNKNOWN `{}`", g.signal);
                        any_err = true;
                    }
                }
            }
            Ok(if any_err {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Sby {
            file,
            tasks: filter_tasks,
            workdir,
        } => {
            let all_tasks = parse_sby_tasks(&file)?;
            let selected: Vec<_> = if filter_tasks.is_empty() {
                all_tasks
            } else {
                all_tasks
                    .into_iter()
                    .filter(|t| filter_tasks.contains(&t.name))
                    .collect()
            };
            let multi = selected.len() > 1;
            let mut all_ok = true;
            for task in &selected {
                let task_wd = workdir.as_ref().map(|base| {
                    if multi {
                        base.join(&task.name)
                    } else {
                        base.clone()
                    }
                });
                let res = run_sby_task(task, task_wd.as_deref())?;
                println!(
                    "SBY [{}] {} — {}{}",
                    res.task,
                    res.status,
                    res.detail,
                    if res.expected_ok {
                        ""
                    } else {
                        " (UNEXPECTED)"
                    }
                );
                for vp in &res.vcd_files {
                    println!("  VCD: {}", vp.display());
                }
                if !res.expected_ok {
                    all_ok = false;
                }
            }
            Ok(if all_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::DumpIr { src } => {
            let module = src.compile_and_prepare()?;
            print!("{}", vf_ir::fmt::dump_module(&module));
            Ok(ExitCode::SUCCESS)
        }
        Commands::DumpBtor2 { src } => {
            let module = src.compile_and_prepare()?;
            print!("{}", btor2::export_btor2(&module));
            Ok(ExitCode::SUCCESS)
        }
        Commands::DumpSmt2 { src } => {
            let module = src.compile_and_prepare()?;
            print!("{}", smt2::dump_standalone_smt2(&module));
            Ok(ExitCode::SUCCESS)
        }
        Commands::DumpAiger { src, output } => {
            let module = src.compile_and_prepare()?;
            let aig = aiger::bitblast_to_aig(&module)?;
            fs::write(&output, aig.to_binary_aig())?;
            println!(
                "Wrote binary AIGER ({} inputs, {} latches, {} bad outputs) to {}",
                aig.num_inputs,
                aig.latches.len(),
                aig.bad_outputs.len(),
                output.display()
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}
