mod sby;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use sby::{parse_sby_tasks, run_sby_task};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::equiv::check_equiv;
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::live::{run_live, LiveConfig, LiveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_ir::passes::{async2sync, clk2fflogic, has_multiple_clocks};
use vf_ir::Module;
use vf_slang::{compile_to_ir, CompileOptions};
use vf_smt::{aiger, btor2, smt2, vcd};

#[derive(Parser)]
#[command(
    name = "veloformal",
    version,
    about = "Native Verilog/SystemVerilog formal verification tool (embedded Slang + RTL IR + SMT/BTOR2/AIGER/PDR/SBY)"
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
    fn compile_and_prepare(&self) -> Result<Module> {
        let mut m = compile_to_ir(&CompileOptions {
            files: self.files.clone(),
            top: self.top.clone(),
            defines: self.defines.clone(),
            include_dirs: self.include_dirs.clone(),
            params: self.params.clone(),
            formal_mode: true,
        })?;
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
    /// Check combinational or sequential equivalence between two Verilog modules
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
            solver,
            vcd: vcd_path,
        } => {
            let module = src.compile_and_prepare()?;
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
            let (miter, outcome) = check_equiv(
                &gold_mod,
                &gate_mod,
                &ProveConfig {
                    max_k: depth,
                    solver,
                },
            )?;
            match outcome {
                ProveOutcome::Proved { k } => {
                    println!("EQUIVALENT (proved via k-induction k={k})");
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
