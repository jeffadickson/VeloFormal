mod questa_tcl;
mod report;
mod sby;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use questa_tcl::run_questa_tcl;
use report::{ReportItem, SignOffReport};
use sby::{parse_sby_tasks, run_sby_task};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use vf_apps::agent::{inspect_module, run_trust_audit, scaffold_vfc_contract, TrustAuditConfig};
use vf_apps::autocheck::{run_autocheck, AutoCheckConfig, CheckStatus};
use vf_apps::cdc::{parse_sdc_file, run_cdc_analysis, CdcConfig, CdcSeverity, GrayStatus};
use vf_apps::compose::{parse_sub_arg, run_compose, ComposeConfig};
use vf_apps::connectivity::{parse_connectivity_csv, run_connectivity, ConnStatus};
use vf_apps::covercheck::{run_covercheck, CoverCheckConfig, CoverageStatus};
use vf_apps::fault::{run_fault_analysis, FaultConfig, FaultModel, FaultStatus};
use vf_apps::power::{parse_upf_file, run_powercheck, PowerCheckConfig, PowerStatus};
use vf_apps::regcheck::{
    parse_regmap_auto, run_regcheck, RegBusConfig, RegCheckStatus, RegProtocol,
};
use vf_apps::secure::{run_secure_check, SecureConfig, SecureOutcome};
use vf_apps::vip::{render_vip_sva, render_vip_vfc, VIP_CATALOG};
use vf_apps::vplan::run_vplan_file;
use vf_apps::xcheck::{run_xcheck, XCheckConfig, XStatus};
use vf_dsl::{emit_sva, format_contract, lower_contract_to_ir, parse_contract, ContractRole};
use vf_engine::backtrace::{compute_cex_backtrace, format_agent_trace_table};
use vf_engine::bmc::{run_bmc, BmcConfig, BmcOutcome};
use vf_engine::btor_engine::{run_btor, BtorConfig, BtorSolver};
use vf_engine::cert::ProofCertificate;
use vf_engine::cover::{run_cover, CoverConfig};
use vf_engine::coverage::{
    compute_proof_core, run_mutation_analysis, MutantStatus, MutationConfig,
};
use vf_engine::deadend::{check_deadends, DeadEndConfig, DeadEndOutcome};
use vf_engine::equiv::{check_slec, parse_reset_spec, SlecConfig};
use vf_engine::kind::{run_kinduction, ProveConfig, ProveOutcome};
use vf_engine::live::{run_live, LiveConfig, LiveOutcome};
use vf_engine::pdr::{run_pdr, PdrConfig};
use vf_engine::portfolio::{parse_case_split, run_case_split, run_portfolio, PortfolioConfig};
use vf_engine::vacuity::{check_vacuity, VacuityConfig, VacuityStatus};
use vf_ir::passes::{
    apply_cutpoint, async2sync, blackbox_instance, clk2fflogic, coi_prune, has_multiple_clocks,
};
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
    /// Prove assertions unboundedly using k-induction, IC3/PDR, or parallel portfolio + case-splitting
    Prove {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum induction depth k (for k-induction)
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// Proof engine: auto, portfolio (parallel k-ind + PDR race), kind, or pdr
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Apply Cone-of-Influence (COI) pruning before proving
        #[arg(long)]
        coi: bool,
        /// Parallel case-splitting specification (e.g. `mode=0..3` or `sel=0,1,2`)
        #[arg(long)]
        case_split: Option<String>,
        /// Submodule instance prefix(es) to blackbox for assume-guarantee composition
        #[arg(long = "blackbox")]
        blackbox: Vec<String>,
        /// Signal name(s) to turn into unconstrained symbolic cutpoints
        #[arg(long = "cutpoint")]
        cutpoints: Vec<String>,
        /// Optional incremental COI proof cache directory (e.g. `.vf-cache`)
        #[arg(long)]
        cache_dir: Option<PathBuf>,
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
    /// Sequential/Combinational Logic Equivalence Checking (Questa SLEC: latency offsets, reset seq, LPGC valid-cond, safety-insertion output filter, k-ind & PDR)
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
        /// Reset initialization sequence (e.g. `rst=1` or `rst_n=0`)
        #[arg(long)]
        reset: Option<String>,
        /// Number of initial cycles to hold `--reset` active
        #[arg(long, default_value_t = 1)]
        reset_cycles: usize,
        /// Restrict comparison to specific output port(s)
        #[arg(long = "output")]
        outputs: Vec<String>,
        /// Exclude output port(s) from comparison (e.g. TMR/ECC `err_flag`)
        #[arg(long = "ignore-output")]
        ignore_outputs: Vec<String>,
        /// Low-Power Clock Gating (LPGC) / observability condition (compare only when true)
        #[arg(long)]
        valid_cond: Option<String>,
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
    /// Questa PropCheck dead-end detector: detect over-constrained assumptions causing reachable dead-end states
    Deadend {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum search depth (cycles)
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD trace leading into the dead-end state
        #[arg(long)]
        vcd: Option<PathBuf>,
    },
    /// Questa AutoCheck / Jasper Superlint: push-button formal RTL bug hunting (OOB memory/bit-select, div-by-zero, shift-overflow, unique/priority, inferred latches, comb loops, multi-drivers, signed/unsigned/mul overflow, width truncation, FSM deadlock/livelock/unreachable, clock-as-data, unresettable/const/unread regs)
    Autocheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum verification depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// Enable signed & unsigned add/sub/mul arithmetic overflow/underflow checks
        #[arg(long)]
        check_overflow: bool,
        /// Enable formal implicit width-truncation / silent data-loss checks
        #[arg(long)]
        check_truncation: bool,
        /// Check for initialized registers provably stuck constant forever
        #[arg(long)]
        check_const_regs: bool,
        /// Check formally for uninitialized registers not cleared by detected reset input
        #[arg(long)]
        check_no_reset: bool,
        /// Check for driven internal signals never read by any output, memory, or property
        #[arg(long)]
        check_unread: bool,
        /// Automatically discover FSM state registers and run deadlock/livelock/unreachable checks
        #[arg(long)]
        auto_fsm: bool,
        /// Strict mode: enable all opt-in checks (overflow, truncation, const-reg, no-reset, unread, auto-FSM)
        #[arg(long)]
        strict: bool,
        /// FSM state register name(s) to check for deadlock, livelock, and unreachable states
        #[arg(long = "fsm")]
        fsm_signals: Vec<String>,
        /// Only run specific AutoCheck category/categories (repeatable)
        #[arg(long = "enable")]
        enable_categories: Vec<String>,
        /// Skip/waive specific AutoCheck category/categories (repeatable)
        #[arg(long = "disable")]
        disable_categories: Vec<String>,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
        /// Write unified HTML or JSON sign-off report
        #[arg(long)]
        report: Option<PathBuf>,
        /// Write SARIF 2.1.0 report for GitHub Code Scanning
        #[arg(long)]
        sarif: Option<PathBuf>,
        /// Write JUnit XML report
        #[arg(long)]
        junit: Option<PathBuf>,
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
    /// Questa Register Check: verify CSR reset values, RW/RO/W1C/W1S/WO policies, masks & non-aliasing (CSV, SystemRDL .rdl, or IP-XACT .xml; Raw or AMBA APB)
    Regcheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Register specification file (`.csv`, SystemRDL `.rdl`, or IP-XACT `.xml`)
        #[arg(long)]
        spec: PathBuf,
        /// CSR bus protocol: raw or apb (AMBA APB3/APB4)
        #[arg(long, default_value = "raw")]
        protocol: String,
        /// Read data latency in clock cycles (0 = combinational read, 1 = registered read)
        #[arg(long, default_value_t = 0)]
        read_latency: usize,
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
        /// APB PSEL signal name (when `--protocol apb`)
        #[arg(long)]
        psel: Option<String>,
        /// APB PENABLE signal name (when `--protocol apb`)
        #[arg(long)]
        penable: Option<String>,
        /// Optional APB PREADY signal name (when `--protocol apb`)
        #[arg(long)]
        pready: Option<String>,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 10)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Questa CDC / RDC: structural clock-domain & reset-domain crossing analysis + SDC constraints + formal Gray-code verification
    Cdc {
        #[command(flatten)]
        src: SourceArgs,
        /// Optional SDC constraint file (`create_clock`, `set_clock_groups`, `set_false_path`)
        #[arg(long)]
        sdc: Option<PathBuf>,
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
    /// Questa Analyze Fault / ISO 26262 Functional Safety: formal SA0/SA1/SEU fault injection, SPFM & Diagnostic Coverage
    Fault {
        #[command(flatten)]
        src: SourceArgs,
        /// Target register/signal name(s) for fault injection (defaults to all user FFs)
        #[arg(long = "target")]
        targets: Vec<String>,
        /// Safety-critical output port(s) to observe (defaults to all outputs except `--alarm`)
        #[arg(long = "output")]
        outputs: Vec<String>,
        /// Safety-mechanism error/alarm output signal (e.g. TMR/ECC/parity error flag)
        #[arg(long)]
        alarm: Option<String>,
        /// Maximum cycles after output divergence within which `--alarm` must assert
        #[arg(long, default_value_t = 0)]
        detect_latency: usize,
        /// Comma-separated fault models: sa0,sa1,seu
        #[arg(long, default_value = "sa0,sa1,seu")]
        models: String,
        /// Initial settling cycles to mask before checking safety outputs
        #[arg(long, default_value_t = 0)]
        skip_cycles: usize,
        /// Maximum number of fault sites to analyze
        #[arg(long, default_value_t = 64)]
        max_faults: usize,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 12)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write Interactive HTML Sign-Off Dashboard (with FMEDA classification & waveforms)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format: text, json, junit, or sarif
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Questa Low-Power / UPF: IEEE 1801 UPF power-domain isolation clamp & crossing verifier
    Powercheck {
        #[command(flatten)]
        src: SourceArgs,
        /// Path to IEEE 1801 UPF (`.upf`) power-intent specification
        #[arg(long)]
        upf: PathBuf,
        /// Optional power-domain off signal mapping(s) (`DOMAIN=signal`) to verify power-down isolation sequencing
        #[arg(long = "pd-off")]
        pd_off: Vec<String>,
        /// Maximum k-induction depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Directory in which to write VCD traces for failed clamp/sequencing checks
        #[arg(long)]
        vcd_dir: Option<PathBuf>,
    },
    /// Verify Verilog/SV RTL against a VeloFormal Contract DSL (`.vfc`) specification
    Contract {
        #[command(flatten)]
        src: SourceArgs,
        /// Path to `.vfc` contract file
        #[arg(long)]
        spec: PathBuf,
        /// Verification mode: prove, bmc, cover, live, or all
        #[arg(long, default_value = "all")]
        mode: String,
        /// Verification role: provider (assume require, prove ensure) or caller (prove require)
        #[arg(long, default_value = "provider")]
        role: String,
        /// Proof engine: auto, kind, or pdr
        #[arg(long, default_value = "auto")]
        engine: String,
        /// Maximum depth for BMC / k-induction / cover / live
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// Run vacuity checks on all contract assertions & operation triggers
        #[arg(long)]
        vacuity: bool,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write VCD counterexample waveform on failure
        #[arg(long)]
        vcd: Option<PathBuf>,
        /// Write Interactive HTML Sign-Off Dashboard (with embedded SVG waveforms)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format: text, json, junit, or sarif
        #[arg(long, default_value = "text")]
        format: String,
        /// Emit an integrity-hashed ProofCertificate JSON file upon unbounded proof
        #[arg(long)]
        emit_cert: Option<PathBuf>,
        /// Verify an existing ProofCertificate before allowing caller/compositional checks
        #[arg(long)]
        verify_cert: Option<PathBuf>,
    },
    /// Formal Proof Core: compute COI signal coverage & minimal assumption core (detect over-constraints)
    ProofCore {
        #[command(flatten)]
        src: SourceArgs,
        /// Maximum induction depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
    },
    /// Formal Mutation Testing: inject targeted RTL bugs and measure formal specification completeness (%)
    Mutate {
        #[command(flatten)]
        src: SourceArgs,
        /// Optional `.vfc` contract file to evaluate mutation coverage against
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Maximum BMC depth per mutant
        #[arg(long, default_value_t = 10)]
        depth: usize,
        /// Maximum number of mutants to inject
        #[arg(long, default_value_t = 24)]
        max_mutants: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Write Interactive HTML Sign-Off Dashboard
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format: text, json, junit, or sarif
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Export a `.vfc` contract to a portable IEEE 1800 SystemVerilog SVA `bind` file
    EmitSva {
        /// Path to `.vfc` contract file
        spec: PathBuf,
        /// Optional RTL file(s) so port widths/signedness are inferred exactly
        #[arg(long = "rtl")]
        rtl: Vec<PathBuf>,
        /// Optional top module name when `--rtl` is provided
        #[arg(long)]
        top: Option<String>,
        /// Output `.sv` file path (prints to stdout if omitted)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// Format a `.vfc` contract file canonically (4-space LF)
    Fmt {
        /// Path to `.vfc` contract file
        file: PathBuf,
        /// Check formatting without modifying file (exits 1 if unformatted)
        #[arg(long)]
        check: bool,
        /// Format file in place
        #[arg(short = 'i', long)]
        in_place: bool,
    },
    /// Execute a Questa-Formal-compatible `.do` / Tcl verification script
    Tcl {
        /// Path to `.do` / `.tcl` script
        script: PathBuf,
        /// Write Interactive HTML Sign-Off Dashboard
        #[arg(long)]
        report: Option<PathBuf>,
        /// Output format: text, json, junit, or sarif
        #[arg(long, default_value = "text")]
        format: String,
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
    /// Inspect elaborated RTL ports, clocks, reset polarity, registers, FSMs, and bus protocols (human & AI agent introspection)
    Inspect {
        #[command(flatten)]
        src: SourceArgs,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Auto-scaffold a valid, type-safe `.vfc` v1.0 starter contract directly from elaborated Verilog/SV RTL
    Scaffold {
        #[command(flatten)]
        src: SourceArgs,
        /// Output `.vfc` file path (prints to stdout if omitted)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// 4-Gate Anti-Cheating Trustworthiness Audit (unbounded proof + dead-end check + vacuity check + mutation kill score)
    Audit {
        #[command(flatten)]
        src: SourceArgs,
        /// Optional `.vfc` contract file to audit alongside the RTL
        #[arg(long)]
        spec: Option<PathBuf>,
        /// Minimum formal mutation kill score (%) required for TRUSTWORTHY sign-off
        #[arg(long, default_value_t = 50.0)]
        min_mutation_score: f64,
        /// Maximum number of RTL mutants to test
        #[arg(long, default_value_t = 16)]
        max_mutants: usize,
        /// Maximum proof/BMC depth
        #[arg(long, default_value_t = 15)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Automated Hierarchical Assume-Guarantee Composition: prove submodule leaves (`Provider`), blackbox them in top, bind `Caller` contracts, and prove top-level SoC
    Compose {
        #[command(flatten)]
        src: SourceArgs,
        /// Top-level `.vfc` contract file
        #[arg(long)]
        spec: PathBuf,
        /// Submodule leaf spec(s): `instance=rtl.sv:spec.vfc` or `instance=rtl.sv:mod_name:spec.vfc` (repeatable)
        #[arg(long = "sub")]
        submodules: Vec<String>,
        /// Optional JSON compose manifest file (alternative to `--sub`)
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Maximum verification depth
        #[arg(long, default_value_t = 20)]
        depth: usize,
        /// SMT solver executable
        #[arg(long, default_value = "z3")]
        solver: String,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
        /// Optional interactive HTML sign-off & waveform debugger output path
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Turnkey Protocol VIP Library (AXI4-Lite, AXI4-Full, AXI4-Stream, APB, AHB-Lite, TileLink-UL, Wishbone B4, RVFI)
    Vip {
        /// Action: `list` or `init`
        action: String,
        /// Protocol id (when action is `init`, e.g. `axi4_lite`, `tilelink_ul`, `rvfi`)
        protocol: Option<String>,
        /// Target DUT module name
        #[arg(long)]
        module: Option<String>,
        /// Emit portable IEEE 1800 SVA `bind` instead of `.vfc`
        #[arg(long)]
        sva: bool,
        /// Output file path (prints to stdout if omitted)
        #[arg(short = 'o', long)]
        output: Option<PathBuf>,
    },
    /// Execute a traceable Formal Verification Plan (`vplan.json`) mapping RTL requirements to formal proofs & Trust Audits
    Vplan {
        /// Path to JSON verification plan file
        plan: PathBuf,
        /// Output format: text, markdown, or json
        #[arg(long, default_value = "text")]
        format: String,
        /// Optional HTML/JSON/Markdown sign-off matrix output file (auto-detected by extension)
        #[arg(long)]
        report: Option<PathBuf>,
        /// Optional HTML sign-off dashboard output file
        #[arg(long)]
        html: Option<PathBuf>,
        /// Optional Markdown sign-off table output file
        #[arg(long)]
        md: Option<PathBuf>,
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
                    let bt = compute_cex_backtrace(&module, Some(&property), &trace);
                    print!("{}", bt.format_text());
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
            case_split,
            blackbox,
            cutpoints,
            cache_dir,
            solver,
            vcd: vcd_path,
        } => {
            let mut module = src.compile_and_prepare()?;
            for inst in &blackbox {
                let n = blackbox_instance(&mut module, inst, true);
                eprintln!("blackboxed `{inst}` ({n} internal drivers abstracted)");
            }
            for cp in &cutpoints {
                if !apply_cutpoint(&mut module, cp) {
                    anyhow::bail!("cutpoint signal '{cp}' not found in module '{}'", module.name);
                }
            }
            if coi {
                coi_prune(&mut module);
            }

            let mut winner_note: Option<String> = None;
            let outcome = if let Some(cs) = &case_split {
                let spec = parse_case_split(cs)?;
                let pr = run_case_split(
                    &module,
                    &spec,
                    &PortfolioConfig {
                        depth,
                        solver: solver.clone(),
                        enable_pdr: engine != "kind",
                        cache_dir,
                    },
                )?;
                winner_note = Some(pr.winner);
                pr.outcome
            } else if engine == "portfolio" || cache_dir.is_some() {
                let pr = run_portfolio(
                    &module,
                    &PortfolioConfig {
                        depth,
                        solver: solver.clone(),
                        enable_pdr: engine != "kind",
                        cache_dir,
                    },
                )?;
                winner_note = Some(pr.winner);
                pr.outcome
            } else {
                match engine.as_str() {
                    "pdr" => run_pdr(
                        &module,
                        &PdrConfig {
                            smt_solver: solver,
                            ..Default::default()
                        },
                    )?,
                    "btor" | "pono" | "avr" | "btormc" => {
                        let bsolver = match engine.as_str() {
                            "pono" => BtorSolver::PonoIc3ia,
                            "avr" => BtorSolver::Avr,
                            "btormc" => BtorSolver::Btormc,
                            _ => BtorSolver::Auto,
                        };
                        run_btor(
                            &module,
                            &BtorConfig {
                                solver: bsolver,
                                depth,
                                smt_solver: solver,
                            },
                        )?
                    }
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
                }
            };
            match outcome {
                ProveOutcome::Proved { k } => {
                    if let Some(w) = winner_note {
                        println!("PROVED (unbounded, winner={w})");
                    } else if engine == "pdr" || k == 0 {
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
                    let bt = compute_cex_backtrace(&module, Some(&property), &trace);
                    print!("{}", bt.format_text());
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
            reset,
            reset_cycles,
            outputs,
            ignore_outputs,
            valid_cond,
            depth,
            solver,
            vcd: vcd_path,
        } => {
            let mut g_opts = CompileOptions::for_files(gold);
            g_opts.top = gold_top;
            let mut t_opts = CompileOptions::for_files(gate);
            t_opts.top = gate_top;
            let mut gold_mod = compile_to_ir(&g_opts)?;
            let mut gate_mod = compile_to_ir(&t_opts)?;
            if gold_mod.ffs.iter().any(|f| f.async_reset.is_some()) {
                async2sync(&mut gold_mod);
            }
            if gate_mod.ffs.iter().any(|f| f.async_reset.is_some()) {
                async2sync(&mut gate_mod);
            }
            let parsed_reset = reset.as_deref().map(parse_reset_spec).transpose()?;
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
                    reset: parsed_reset,
                    reset_cycles,
                    outputs,
                    ignore_outputs,
                    valid_cond,
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
        Commands::Deadend {
            src,
            depth,
            solver,
            vcd: vcd_path,
        } => {
            let module = src.compile_and_prepare()?;
            match check_deadends(&module, &DeadEndConfig { depth, solver })? {
                DeadEndOutcome::NoAssumptions => {
                    println!("CLEAN — design has no `assume` constraints");
                    Ok(ExitCode::SUCCESS)
                }
                DeadEndOutcome::Clean { depth } => {
                    println!(
                        "CLEAN — no assumption dead-end state reachable within {depth} cycles"
                    );
                    Ok(ExitCode::SUCCESS)
                }
                DeadEndOutcome::InitialUnsat => {
                    println!(
                        "DEAD-END — assumptions are contradictory at cycle 0 (initial state UNSAT)"
                    );
                    Ok(ExitCode::from(1))
                }
                DeadEndOutcome::DeadEndFound { step, trace } => {
                    println!(
                        "DEAD-END — reachable over-constrained state at cycle {step} (no next-cycle input satisfies `assume`)"
                    );
                    let bt = compute_cex_backtrace(&module, None, &trace);
                    print!("{}", bt.format_text());
                    if let Some(path) = vcd_path {
                        fs::write(&path, vcd::write_vcd(&module, &trace))?;
                        println!("Dead-end trace VCD written to {}", path.display());
                    }
                    Ok(ExitCode::from(1))
                }
                DeadEndOutcome::Unknown { step } => {
                    println!("UNKNOWN — solver returned unknown at cycle {step}");
                    Ok(ExitCode::from(3))
                }
            }
        }
        Commands::Autocheck {
            src,
            depth,
            check_overflow,
            check_truncation,
            check_const_regs,
            check_no_reset,
            check_unread,
            auto_fsm,
            strict,
            fsm_signals,
            enable_categories,
            disable_categories,
            format,
            report: report_path,
            sarif: sarif_path,
            junit: junit_path,
            solver,
            vcd_dir,
        } => {
            let mut cfg = if strict {
                AutoCheckConfig::strict()
            } else {
                AutoCheckConfig::default()
            };
            cfg.depth = depth;
            cfg.solver = solver;
            cfg.check_overflow |= check_overflow;
            cfg.check_truncation |= check_truncation;
            cfg.check_const_regs |= check_const_regs;
            cfg.check_no_reset |= check_no_reset;
            cfg.check_unread |= check_unread;
            cfg.auto_fsm |= auto_fsm;
            cfg.fsm_signals = fsm_signals;
            cfg.enabled_categories = enable_categories;
            cfg.disabled_categories = disable_categories;

            let (module, findings) = run_autocheck(&src.to_compile_options(), &cfg)?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }

            let mut any_bug = false;
            let mut signoff = SignOffReport::new("VeloFormal AutoCheck Sign-Off", &module.name);
            let mut json_findings = Vec::with_capacity(findings.len());

            for (i, f) in findings.iter().enumerate() {
                match &f.status {
                    CheckStatus::Safe => {
                        if format != "json" {
                            println!("SAFE [{}] `{}`", f.category, f.name);
                        }
                        signoff.items.push(ReportItem {
                            name: f.name.clone(),
                            category: format!("autocheck:{}", f.category),
                            status: "SAFE".into(),
                            engine: "autocheck".into(),
                            detail: f.detail.clone().unwrap_or_else(|| "proved safe".into()),
                            source: None,
                            trace: None,
                        });
                        json_findings.push(serde_json::json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "SAFE",
                            "detail": f.detail,
                        }));
                    }
                    CheckStatus::Bug { step, trace } => {
                        any_bug = true;
                        let msg = if let Some(d) = &f.detail {
                            d.clone()
                        } else {
                            format!("violated at cycle {step}")
                        };
                        if format != "json" {
                            if trace.is_empty() {
                                println!("BUG  [{}] `{}` — {msg}", f.category, f.name);
                            } else {
                                println!("BUG  [{}] `{}` violated at cycle {step}", f.category, f.name);
                            }
                        }
                        if let Some(dir) = &vcd_dir {
                            if !trace.is_empty() {
                                let path = dir.join(format!("autocheck_{i}.vcd"));
                                fs::write(&path, vcd::write_vcd(&module, trace))?;
                            }
                        }
                        signoff.items.push(ReportItem {
                            name: f.name.clone(),
                            category: format!("autocheck:{}", f.category),
                            status: "BUG".into(),
                            engine: "autocheck".into(),
                            detail: msg.clone(),
                            source: None,
                            trace: if trace.is_empty() {
                                None
                            } else {
                                Some(trace.clone())
                            },
                        });
                        let trace_table = if trace.is_empty() {
                            None
                        } else {
                            Some(format_agent_trace_table(&module, None, trace, 10))
                        };
                        json_findings.push(serde_json::json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "BUG",
                            "step": step,
                            "detail": msg,
                            "trace_table": trace_table,
                        }));
                    }
                    CheckStatus::Warning(msg) => {
                        if format != "json" {
                            println!("WARN [{}] `{}` — {msg}", f.category, f.name);
                        }
                        signoff.items.push(ReportItem {
                            name: f.name.clone(),
                            category: format!("autocheck:{}", f.category),
                            status: "WARN".into(),
                            engine: "autocheck".into(),
                            detail: msg.clone(),
                            source: None,
                            trace: None,
                        });
                        json_findings.push(serde_json::json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "WARN",
                            "detail": msg,
                        }));
                    }
                    CheckStatus::Unknown => {
                        if format != "json" {
                            println!("UNKNOWN [{}] `{}`", f.category, f.name);
                        }
                        signoff.items.push(ReportItem {
                            name: f.name.clone(),
                            category: format!("autocheck:{}", f.category),
                            status: "UNKNOWN".into(),
                            engine: "autocheck".into(),
                            detail: "inconclusive".into(),
                            source: None,
                            trace: None,
                        });
                        json_findings.push(serde_json::json!({
                            "category": f.category,
                            "name": f.name,
                            "status": "UNKNOWN",
                        }));
                    }
                }
            }

            if let Some(rp) = &report_path {
                let content = if rp.extension().and_then(|e| e.to_str()) == Some("json") {
                    signoff.to_json()
                } else {
                    signoff.to_html()
                };
                fs::write(rp, content)?;
            }
            if let Some(sp) = &sarif_path {
                fs::write(sp, signoff.to_sarif())?;
            }
            if let Some(jp) = &junit_path {
                fs::write(jp, signoff.to_junit_xml())?;
            }

            if format == "json" {
                let bugs = findings
                    .iter()
                    .filter(|f| matches!(f.status, CheckStatus::Bug { .. }))
                    .count();
                let warnings = findings
                    .iter()
                    .filter(|f| matches!(f.status, CheckStatus::Warning(_)))
                    .count();
                let safe = findings
                    .iter()
                    .filter(|f| matches!(f.status, CheckStatus::Safe))
                    .count();
                let out = serde_json::json!({
                    "module": module.name,
                    "summary": {
                        "total": findings.len(),
                        "safe": safe,
                        "bugs": bugs,
                        "warnings": warnings,
                    },
                    "findings": json_findings,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
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
            protocol,
            read_latency,
            rst,
            rst_active_low,
            wr_en,
            rd_en,
            addr,
            wdata,
            rdata,
            psel,
            penable,
            pready,
            depth,
            solver,
        } => {
            let module = src.compile_and_prepare()?;
            let specs = parse_regmap_auto(&spec)?;
            let proto = RegProtocol::parse(&protocol)?;
            let findings = run_regcheck(
                &module,
                &specs,
                &RegBusConfig {
                    protocol: proto,
                    read_latency,
                    rst,
                    rst_active_low,
                    wr_en,
                    rd_en,
                    addr,
                    wdata,
                    rdata,
                    psel,
                    penable,
                    pready,
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
        Commands::Fault {
            src,
            targets,
            outputs,
            alarm,
            detect_latency,
            models,
            skip_cycles,
            max_faults,
            depth,
            solver,
            report: report_path,
            format,
        } => {
            let module = src.compile_and_prepare()?;
            let parsed_models: Vec<FaultModel> = models
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(FaultModel::parse)
                .collect::<Result<_>>()?;
            let frep = run_fault_analysis(
                &module,
                &FaultConfig {
                    targets,
                    outputs,
                    alarm,
                    detect_latency,
                    models: parsed_models,
                    skip_cycles,
                    max_faults,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            let mut signoff = SignOffReport::new("ISO 26262 Formal Fault Analysis", &module.name);
            for f in &frep.findings {
                let fname = format!("{}[{}]/{}", f.signal, f.bit, f.model.as_str());
                let (status, detail, trace) = match &f.status {
                    FaultStatus::SafeMasked => (
                        "SAFE/MASKED",
                        "fault is formally masked (cannot corrupt safety outputs)".to_string(),
                        None,
                    ),
                    FaultStatus::Detected => (
                        "DETECTED",
                        "propagatable fault formally covered by safety mechanism alarm".to_string(),
                        None,
                    ),
                    FaultStatus::DangerousSpf { step, trace } => (
                        "SPF",
                        format!("DANGEROUS Single-Point Fault corrupted safety output at cycle {step} without alarm"),
                        Some(trace.clone()),
                    ),
                    FaultStatus::Unknown => (
                        "UNKNOWN",
                        "solver inconclusive within configured depth".to_string(),
                        None,
                    ),
                };
                if format == "text" {
                    println!("{status:<11} {fname} — {detail}");
                }
                signoff.items.push(ReportItem {
                    name: fname,
                    category: format!("fault_{}", f.model.as_str()),
                    status: status.into(),
                    engine: "fault-miter".into(),
                    detail,
                    source: None,
                    trace,
                });
            }
            if let Some(rp) = report_path {
                fs::write(&rp, signoff.to_html())?;
                eprintln!("HTML FMEDA sign-off dashboard written to {}", rp.display());
            }
            match format.as_str() {
                "json" => println!("{}", signoff.to_json()),
                "junit" => println!("{}", signoff.to_junit_xml()),
                "sarif" => println!("{}", signoff.to_sarif()),
                _ => {
                    println!(
                        "SUMMARY — total={} safe/masked={} detected={} dangerous_spf={} unknown={} | DC={:.1}% SPFM={:.1}%",
                        frep.total_faults,
                        frep.safe_masked,
                        frep.detected,
                        frep.dangerous_spf,
                        frep.unknown,
                        frep.diagnostic_coverage_pct,
                        frep.spfm_pct
                    );
                }
            }
            Ok(if frep.dangerous_spf > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Powercheck {
            src,
            upf: upf_path,
            pd_off,
            depth,
            solver,
            vcd_dir,
        } => {
            let module = src.compile_and_prepare()?;
            let upf_spec = parse_upf_file(&upf_path)?;
            let mut pd_off_pairs = Vec::new();
            for item in &pd_off {
                let Some((dom, sig)) = item.split_once('=') else {
                    anyhow::bail!("invalid --pd-off '{item}' (expected DOMAIN=signal)");
                };
                pd_off_pairs.push((dom.trim().to_string(), sig.trim().to_string()));
            }
            let findings = run_powercheck(
                &module,
                &upf_spec,
                &PowerCheckConfig {
                    pd_off: pd_off_pairs,
                    depth,
                    solver,
                    use_pdr: true,
                },
            )?;
            if let Some(dir) = &vcd_dir {
                fs::create_dir_all(dir)?;
            }
            let mut any_fail = false;
            for (i, f) in findings.iter().enumerate() {
                match &f.status {
                    PowerStatus::Pass => {
                        println!("PASS      [{} / {}] {} ({})", f.domain, f.rule, f.signal, f.check);
                    }
                    PowerStatus::Fail { step, trace } => {
                        println!(
                            "FAIL      [{} / {}] {} ({}) violated at cycle {step}",
                            f.domain, f.rule, f.signal, f.check
                        );
                        if let Some(dir) = &vcd_dir {
                            let path = dir.join(format!("powercheck_{i}.vcd"));
                            fs::write(&path, vcd::write_vcd(&module, trace))?;
                        }
                        any_fail = true;
                    }
                    PowerStatus::StructuralViolation(msg) => {
                        println!(
                            "VIOLATION [{} / {}] {} — {msg}",
                            f.domain, f.rule, f.signal
                        );
                        any_fail = true;
                    }
                    PowerStatus::Unknown => {
                        println!("UNKNOWN   [{} / {}] {} ({})", f.domain, f.rule, f.signal, f.check);
                        any_fail = true;
                    }
                }
            }
            Ok(if any_fail {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::Cdc {
            src,
            sdc,
            gray_signals,
            depth,
            solver,
        } => {
            let raw = src.compile_raw()?;
            let sdc_constraints = sdc.as_deref().map(parse_sdc_file).transpose()?;
            let report = run_cdc_analysis(
                &raw,
                &CdcConfig {
                    gray_signals,
                    sdc: sdc_constraints,
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
        Commands::Contract {
            src,
            spec,
            mode,
            role,
            engine,
            depth,
            vacuity,
            solver,
            vcd: vcd_path,
            report: report_html,
            format,
            emit_cert,
            verify_cert,
        } => {
            let spec_text = fs::read_to_string(&spec)?;
            let contract = parse_contract(&spec_text)?;
            if let Some(cp) = &verify_cert {
                let cert = ProofCertificate::load_and_verify(cp, Some(&spec_text))?;
                if format == "text" {
                    println!(
                        "CERT VERIFIED `{}` (module `{}`, engine `{}`, hash `{}`)",
                        cert.contract_name, cert.module_name, cert.engine, cert.rtl_hash
                    );
                }
            }
            let mut param_map: HashMap<String, i64> = HashMap::new();
            for p in &src.params {
                if let Some((k, v)) = p.split_once('=') {
                    if let Ok(iv) = v.trim().parse::<i64>() {
                        param_map.insert(k.trim().to_string(), iv);
                    }
                }
            }
            let base = src.compile_and_prepare()?;
            let c_role = match role.as_str() {
                "caller" => ContractRole::Caller,
                _ => ContractRole::Provider,
            };
            let lowered = lower_contract_to_ir(&base, &contract, c_role, &param_map)?;
            let mut rep = SignOffReport::new(
                format!("Contract {} ({})", contract.name, role),
                lowered.module.name.clone(),
            );

            let has_asserts = lowered
                .module
                .properties
                .iter()
                .any(|p| p.kind == vf_ir::PropertyKind::Assert);
            let has_covers = lowered
                .module
                .properties
                .iter()
                .any(|p| p.kind == vf_ir::PropertyKind::Cover);
            let has_lives = lowered
                .module
                .properties
                .iter()
                .any(|p| p.kind == vf_ir::PropertyKind::Live);

            // 1. Safety proof / BMC
            if has_asserts && matches!(mode.as_str(), "prove" | "bmc" | "all") {
                if mode == "bmc" {
                    match run_bmc(
                        &lowered.module,
                        &BmcConfig {
                            depth,
                            solver: solver.clone(),
                            skip: 0,
                            append: 0,
                        },
                    )? {
                        BmcOutcome::Pass { depth } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:bmc".into(),
                            status: "PASS".into(),
                            engine: "bmc".into(),
                            detail: format!("bounded-pass up to {depth} cycles (not an unbounded proof)"),
                            source: None,
                            trace: None,
                        }),
                        BmcOutcome::Fail {
                            property,
                            step,
                            trace,
                        } => {
                            if let Some(vp) = &vcd_path {
                                fs::write(vp, vcd::write_vcd(&lowered.module, &trace))?;
                            }
                            rep.items.push(ReportItem {
                                name: property,
                                category: "contract:bmc".into(),
                                status: "FAIL".into(),
                                engine: "bmc".into(),
                                detail: format!("violated at cycle {step}"),
                                source: None,
                                trace: Some(trace),
                            });
                        }
                        BmcOutcome::AssumptionsUnsat { step } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:bmc".into(),
                            status: "FAIL".into(),
                            engine: "bmc".into(),
                            detail: format!("assumptions unsatisfiable at cycle {step}"),
                            source: None,
                            trace: None,
                        }),
                        BmcOutcome::Unknown { step } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:bmc".into(),
                            status: "UNKNOWN".into(),
                            engine: "bmc".into(),
                            detail: format!("unknown at cycle {step}"),
                            source: None,
                            trace: None,
                        }),
                    }
                } else {
                    let outcome = match engine.as_str() {
                        "pdr" => run_pdr(
                            &lowered.module,
                            &PdrConfig {
                                smt_solver: solver.clone(),
                                ..Default::default()
                            },
                        )?,
                        "kind" => run_kinduction(
                            &lowered.module,
                            &ProveConfig {
                                max_k: depth,
                                solver: solver.clone(),
                            },
                        )?,
                        _ => {
                            let k = run_kinduction(
                                &lowered.module,
                                &ProveConfig {
                                    max_k: depth,
                                    solver: solver.clone(),
                                },
                            )?;
                            match k {
                                ProveOutcome::Unknown { .. } => run_pdr(
                                    &lowered.module,
                                    &PdrConfig {
                                        smt_solver: solver.clone(),
                                        ..Default::default()
                                    },
                                )
                                .unwrap_or(k),
                                other => other,
                            }
                        }
                    };
                    match outcome {
                        ProveOutcome::Proved { k } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:prove".into(),
                            status: "PROVED".into(),
                            engine: engine.clone(),
                            detail: if k == 0 {
                                "unbounded proof (PDR)".into()
                            } else {
                                format!("unbounded proof (k-induction k={k})")
                            },
                            source: None,
                            trace: None,
                        }),
                        ProveOutcome::Fail {
                            property,
                            step,
                            trace,
                        } => {
                            if let Some(vp) = &vcd_path {
                                fs::write(vp, vcd::write_vcd(&lowered.module, &trace))?;
                            }
                            rep.items.push(ReportItem {
                                name: property,
                                category: "contract:prove".into(),
                                status: "FAIL".into(),
                                engine: engine.clone(),
                                detail: format!("violated at cycle {step}"),
                                source: None,
                                trace: Some(trace),
                            });
                        }
                        ProveOutcome::AssumptionsUnsat { step } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:prove".into(),
                            status: "FAIL".into(),
                            engine: engine.clone(),
                            detail: format!("assumptions unsatisfiable at cycle {step}"),
                            source: None,
                            trace: None,
                        }),
                        ProveOutcome::Unknown { max_k } => rep.items.push(ReportItem {
                            name: contract.name.clone(),
                            category: "contract:prove".into(),
                            status: "UNKNOWN".into(),
                            engine: engine.clone(),
                            detail: format!("not decided up to k={max_k}"),
                            source: None,
                            trace: None,
                        }),
                    }
                }
            }

            // 2. Cover witnesses
            if has_covers && matches!(mode.as_str(), "cover" | "all") {
                for r in run_cover(
                    &lowered.module,
                    &CoverConfig {
                        depth,
                        solver: solver.clone(),
                        append: 0,
                    },
                )? {
                    match r.reached {
                        Some((step, trace)) => rep.items.push(ReportItem {
                            name: r.name,
                            category: "contract:cover".into(),
                            status: "REACHED".into(),
                            engine: "cover".into(),
                            detail: format!("witness found at cycle {step}"),
                            source: None,
                            trace: Some(trace),
                        }),
                        None => rep.items.push(ReportItem {
                            name: r.name,
                            category: "contract:cover".into(),
                            status: "FAIL".into(),
                            engine: "cover".into(),
                            detail: format!("unreached within {depth} cycles"),
                            source: None,
                            trace: None,
                        }),
                    }
                }
            }

            // 3. Liveness checks
            if has_lives && matches!(mode.as_str(), "live" | "all") {
                match run_live(
                    &lowered.module,
                    &LiveConfig {
                        depth,
                        solver: solver.clone(),
                    },
                )? {
                    LiveOutcome::Pass { depth } => rep.items.push(ReportItem {
                        name: format!("{}_liveness", contract.name),
                        category: "contract:live".into(),
                        status: "PASS".into(),
                        engine: "live".into(),
                        detail: format!("no fair starving lasso within {depth} cycles"),
                        source: None,
                        trace: None,
                    }),
                    LiveOutcome::Fail {
                        property,
                        loop_start,
                        loop_end,
                        trace,
                    } => rep.items.push(ReportItem {
                        name: property,
                        category: "contract:live".into(),
                        status: "FAIL".into(),
                        engine: "live".into(),
                        detail: format!("starved on fair lasso [{loop_start}..{loop_end}]"),
                        source: None,
                        trace: Some(trace),
                    }),
                    LiveOutcome::Unknown { step } => rep.items.push(ReportItem {
                        name: format!("{}_liveness", contract.name),
                        category: "contract:live".into(),
                        status: "UNKNOWN".into(),
                        engine: "live".into(),
                        detail: format!("unknown at step {step}"),
                        source: None,
                        trace: None,
                    }),
                }
            }

            // 4. Optional Vacuity analysis
            if vacuity && has_asserts {
                for vr in check_vacuity(
                    &lowered.module,
                    &VacuityConfig {
                        depth,
                        solver: solver.clone(),
                        use_pdr: true,
                    },
                )? {
                    match vr.status {
                        VacuityStatus::NonVacuous { step, .. } => rep.items.push(ReportItem {
                            name: vr.property,
                            category: "contract:vacuity".into(),
                            status: "NON-VACUOUS".into(),
                            engine: "vacuity".into(),
                            detail: format!("guard reached at cycle {step}"),
                            source: None,
                            trace: None,
                        }),
                        VacuityStatus::Vacuous => rep.items.push(ReportItem {
                            name: vr.property,
                            category: "contract:vacuity".into(),
                            status: "VACUOUS".into(),
                            engine: "vacuity".into(),
                            detail: "guard proved unreachable for all time".into(),
                            source: None,
                            trace: None,
                        }),
                        VacuityStatus::BoundedVacuous { depth } => rep.items.push(ReportItem {
                            name: vr.property,
                            category: "contract:vacuity".into(),
                            status: "VACUOUS".into(),
                            engine: "vacuity".into(),
                            detail: format!("guard not hit within {depth} cycles"),
                            source: None,
                            trace: None,
                        }),
                    }
                }
            }

            // 5. Embedded `xfree` app clauses
            if !lowered.xfree_targets.is_empty() && matches!(mode.as_str(), "prove" | "all") {
                let (_, xres) = run_xcheck(
                    &base,
                    &XCheckConfig {
                        targets: lowered.xfree_targets.clone(),
                        depth,
                        solver: solver.clone(),
                        ..Default::default()
                    },
                )?;
                for xr in xres {
                    match xr.status {
                        XStatus::XFree => rep.items.push(ReportItem {
                            name: format!("xfree_{}", xr.signal),
                            category: "contract:xcheck".into(),
                            status: "X-FREE".into(),
                            engine: "xcheck".into(),
                            detail: "proved 4-state X-free".into(),
                            source: None,
                            trace: None,
                        }),
                        XStatus::XLeak { step, trace } => rep.items.push(ReportItem {
                            name: format!("xfree_{}", xr.signal),
                            category: "contract:xcheck".into(),
                            status: "X-LEAK".into(),
                            engine: "xcheck".into(),
                            detail: format!("X reached signal at cycle {step}"),
                            source: None,
                            trace: Some(trace),
                        }),
                        XStatus::Unknown => rep.items.push(ReportItem {
                            name: format!("xfree_{}", xr.signal),
                            category: "contract:xcheck".into(),
                            status: "UNKNOWN".into(),
                            engine: "xcheck".into(),
                            detail: "inconclusive".into(),
                            source: None,
                            trace: None,
                        }),
                    }
                }
            }

            // 6. Embedded `secure` IFT app clauses
            if !lowered.secure_checks.is_empty() && matches!(mode.as_str(), "prove" | "all") {
                for sc in &lowered.secure_checks {
                    let (_, sout) = run_secure_check(
                        &base,
                        &SecureConfig {
                            sources: sc.sources.clone(),
                            targets: sc.targets.clone(),
                            declassify: sc.declassify.clone(),
                            depth,
                            solver: solver.clone(),
                            use_pdr: true,
                        },
                    )?;
                    let label = format!("{}->{}", sc.sources.join(","), sc.targets.join(","));
                    match sout {
                        SecureOutcome::Secure => rep.items.push(ReportItem {
                            name: label,
                            category: "contract:secure".into(),
                            status: "SECURE".into(),
                            engine: "secure".into(),
                            detail: "proved non-interfering".into(),
                            source: None,
                            trace: None,
                        }),
                        SecureOutcome::Leak { step, trace } => rep.items.push(ReportItem {
                            name: label,
                            category: "contract:secure".into(),
                            status: "LEAK".into(),
                            engine: "secure".into(),
                            detail: format!("information leak at cycle {step}"),
                            source: None,
                            trace: Some(trace),
                        }),
                        SecureOutcome::Unknown => rep.items.push(ReportItem {
                            name: label,
                            category: "contract:secure".into(),
                            status: "UNKNOWN".into(),
                            engine: "secure".into(),
                            detail: "inconclusive".into(),
                            source: None,
                            trace: None,
                        }),
                    }
                }
            }

            if let Some(cp) = &emit_cert {
                let all_proved = !rep.has_failures()
                    && rep.items.iter().any(|i| i.status == "PROVED");
                if all_proved {
                    let cert = ProofCertificate::create(
                        &contract.name,
                        &base,
                        &spec_text,
                        "PROVED",
                        &engine,
                    );
                    cert.save(cp)?;
                    if format == "text" {
                        println!("Proof certificate written to {}", cp.display());
                    }
                } else {
                    anyhow::bail!(
                        "cannot emit proof certificate '{}': contract did not achieve unbounded PROVED status",
                        cp.display()
                    );
                }
            }

            emit_signoff_report(&rep, &format, report_html.as_deref())?;
            Ok(if rep.has_failures() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
        Commands::ProofCore { src, depth, solver } => {
            let module = src.compile_and_prepare()?;
            let core = compute_proof_core(
                &module,
                &PortfolioConfig {
                    depth,
                    solver,
                    enable_pdr: true,
                    cache_dir: None,
                },
            )?;
            let pct = if core.total_signals == 0 {
                100.0
            } else {
                (core.coi_signals as f64 / core.total_signals as f64) * 100.0
            };
            println!(
                "COI Signal Coverage: {}/{} ({:.1}%)",
                core.coi_signals, core.total_signals, pct
            );
            if !core.uncovered_signal_names.is_empty() {
                println!("Uncovered signals: {}", core.uncovered_signal_names.join(", "));
            }
            for a in &core.assumptions {
                let tag = if a.essential { "ESSENTIAL" } else { "REDUNDANT" };
                println!("ASSUME [{tag}] `{}`", a.name);
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Mutate {
            src,
            spec,
            depth,
            max_mutants,
            solver,
            report: report_html,
            format,
        } => {
            let mut module = src.compile_and_prepare()?;
            if let Some(spec_path) = spec {
                let text = fs::read_to_string(&spec_path)?;
                let contract = parse_contract(&text)?;
                let mut param_map = HashMap::new();
                for p in &src.params {
                    if let Some((k, v)) = p.split_once('=') {
                        if let Ok(iv) = v.trim().parse::<i64>() {
                            param_map.insert(k.trim().to_string(), iv);
                        }
                    }
                }
                module = lower_contract_to_ir(&module, &contract, ContractRole::Provider, &param_map)?.module;
            }
            let mrep = run_mutation_analysis(
                &module,
                &MutationConfig {
                    depth,
                    max_mutants,
                    solver,
                },
            )?;
            let mut rep = SignOffReport::new(
                format!(
                    "Formal Mutation Coverage: {}/{} killed ({:.1}%)",
                    mrep.killed, mrep.total, mrep.score_percent
                ),
                module.name.clone(),
            );
            for m in &mrep.mutants {
                let status = match m.status {
                    MutantStatus::Killed => "PASS",
                    MutantStatus::Survived => "WARN",
                };
                rep.items.push(ReportItem {
                    name: format!("mutant_{}", m.id),
                    category: "mutation".into(),
                    status: status.into(),
                    engine: "mutate".into(),
                    detail: format!("{} — {}", m.description, m.detail),
                    source: None,
                    trace: None,
                });
            }
            if format == "text" {
                println!(
                    "Formal Mutation Score: {}/{} killed ({:.1}%)",
                    mrep.killed, mrep.total, mrep.score_percent
                );
            }
            emit_signoff_report(&rep, &format, report_html.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::EmitSva {
            spec,
            rtl,
            top,
            output,
        } => {
            let text = fs::read_to_string(&spec)?;
            let contract = parse_contract(&text)?;
            let rtl_ir = if !rtl.is_empty() {
                let mut opts = CompileOptions::for_files(rtl);
                opts.top = top;
                Some(compile_to_ir(&opts)?)
            } else {
                None
            };
            let sv = emit_sva(&contract, rtl_ir.as_ref());
            if let Some(out_path) = output {
                fs::write(&out_path, &sv)?;
                println!("Wrote IEEE 1800 SVA bind module to {}", out_path.display());
            } else {
                print!("{sv}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Fmt {
            file,
            check,
            in_place,
        } => {
            let orig = fs::read_to_string(&file)?;
            let contract = parse_contract(&orig)?;
            let formatted = format_contract(&contract);
            if check {
                if orig == formatted {
                    Ok(ExitCode::SUCCESS)
                } else {
                    eprintln!("{} is not canonically formatted", file.display());
                    Ok(ExitCode::from(1))
                }
            } else if in_place {
                fs::write(&file, &formatted)?;
                Ok(ExitCode::SUCCESS)
            } else {
                print!("{formatted}");
                Ok(ExitCode::SUCCESS)
            }
        }
        Commands::Tcl {
            script,
            report: report_html,
            format,
        } => {
            let rep = run_questa_tcl(&script)?;
            emit_signoff_report(&rep, &format, report_html.as_deref())?;
            Ok(if rep.has_failures() {
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
        Commands::Inspect { src, format } => {
            let module = src.compile_raw()?;
            let info = inspect_module(&module);
            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("Module: {}", info.module_name);
                if !info.clocks.is_empty() {
                    println!("  Clocks:    {}", info.clocks.join(", "));
                }
                if let Some(rst) = &info.reset {
                    let pol = if rst.active_low { "active-low" } else { "active-high" };
                    println!("  Reset:     {} ({pol})", rst.name);
                }
                println!(
                    "  Inputs ({}):  {}",
                    info.inputs.len(),
                    info.inputs
                        .iter()
                        .map(|p| format!("{}[{}]", p.name, p.width))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!(
                    "  Outputs ({}): {}",
                    info.outputs.len(),
                    info.outputs
                        .iter()
                        .map(|p| format!("{}[{}]", p.name, p.width))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("  Registers: {} | Memories: {}", info.registers.len(), info.memories.len());
                if !info.fsm_candidates.is_empty() {
                    println!("  FSMs:      {}", info.fsm_candidates.join(", "));
                }
                if !info.detected_protocols.is_empty() {
                    println!("  Protocols: {}", info.detected_protocols.join(", "));
                }
                println!(
                    "  Props:     assert={} assume={} cover={} live={} fair={}",
                    info.existing_properties.asserts,
                    info.existing_properties.assumes,
                    info.existing_properties.covers,
                    info.existing_properties.live,
                    info.existing_properties.fair
                );
                println!("  Recommended next checks:");
                for cmd in &info.recommended_checks {
                    println!("    - {cmd}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Scaffold { src, output } => {
            let module = src.compile_raw()?;
            let vfc = scaffold_vfc_contract(&module);
            if let Some(path) = output {
                fs::write(&path, &vfc)?;
                println!("Scaffolded starter `.vfc` contract to {}", path.display());
            } else {
                print!("{vfc}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Commands::Audit {
            src,
            spec,
            min_mutation_score,
            max_mutants,
            depth,
            solver,
            format,
        } => {
            let mut module = src.compile_and_prepare()?;
            if let Some(spec_path) = spec {
                let text = fs::read_to_string(&spec_path)?;
                let contract = parse_contract(&text)?;
                let mut param_map: HashMap<String, i64> = HashMap::new();
                for p in &src.params {
                    if let Some((k, v)) = p.split_once('=') {
                        if let Ok(n) = v.trim().parse::<i64>() {
                            param_map.insert(k.trim().to_string(), n);
                        }
                    }
                }
                module = lower_contract_to_ir(&module, &contract, ContractRole::Provider, &param_map)?
                    .module;
            }
            let report = run_trust_audit(
                &module,
                &TrustAuditConfig {
                    depth,
                    max_mutants,
                    min_mutation_score,
                    solver,
                },
            )?;
            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let verdict = if report.trustworthy {
                    "TRUSTWORTHY"
                } else {
                    "REJECTED"
                };
                println!("AUDIT {verdict} — module `{}`", report.module_name);
                println!("  Gate 1 [Safety Proof]:  {}", report.proof_status);
                println!("  Gate 2 [Dead-Ends]:     {}", report.deadend_status);
                println!(
                    "  Gate 3 [Vacuity]:       {}",
                    if report.vacuous_properties.is_empty() {
                        "NON-VACUOUS".to_string()
                    } else {
                        format!("VACUOUS ({})", report.vacuous_properties.join(", "))
                    }
                );
                println!(
                    "  Gate 4 [Mutation]:      {}/{} killed ({:.1}%, threshold {:.1}%)",
                    report.mutation_killed,
                    report.mutation_total,
                    report.mutation_score_pct,
                    min_mutation_score
                );
                for issue in &report.issues {
                    println!("  ISSUE: {issue}");
                }
                if let Some(tbl) = &report.failure_trace_table {
                    println!("\n{tbl}");
                }
            }
            Ok(if report.trustworthy {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::Compose {
            src,
            spec,
            submodules,
            manifest,
            depth,
            solver,
            format,
            report,
        } => {
            let cfg = if let Some(mpath) = manifest {
                let txt = fs::read_to_string(&mpath)?;
                serde_json::from_str::<ComposeConfig>(&txt)?
            } else {
                let mut subs = Vec::with_capacity(submodules.len());
                for raw in &submodules {
                    subs.push(parse_sub_arg(raw)?);
                }
                let mut top_params = HashMap::new();
                for p in &src.params {
                    if let Some((k, v)) = p.split_once('=') {
                        if let Ok(n) = v.trim().parse::<i64>() {
                            top_params.insert(k.trim().to_string(), n);
                        }
                    }
                }
                ComposeConfig {
                    top_rtl_files: src.files,
                    top_module: src.top,
                    top_spec_file: spec,
                    top_params,
                    submodules: subs,
                    depth,
                    solver,
                    enable_pdr: true,
                }
            };
            let res = run_compose(&cfg)?;
            if let Some(rep_path) = report {
                let mut sr = SignOffReport::new(
                    "Hierarchical Assume-Guarantee Composition",
                    &res.top_module_name,
                );
                for l in &res.leaf_results {
                    let st = if matches!(l.outcome, ProveOutcome::Proved { .. }) {
                        "PROVED"
                    } else {
                        "FAIL"
                    };
                    sr.items.push(ReportItem {
                        name: format!("leaf:{} ({})", l.instance, l.contract_name),
                        category: "compose:leaf".into(),
                        status: st.into(),
                        engine: l.winner.clone(),
                        detail: format!("{} FFs verified standalone", l.original_ffs),
                        source: None,
                        trace: None,
                    });
                }
                let (top_st, top_detail) = match &res.top_outcome {
                    ProveOutcome::Proved { k } => (
                        "PROVED",
                        format!(
                            "Unbounded proof (k={k}); FFs {} -> {} ({} abstracted)",
                            res.top_ffs_before_blackbox,
                            res.top_ffs_after_blackbox,
                            res.top_blackboxed_signals
                        ),
                    ),
                    ProveOutcome::Fail { property, step, .. } => (
                        "FAIL",
                        format!("Property `{property}` violated at cycle {step}"),
                    ),
                    ProveOutcome::AssumptionsUnsat { step } => (
                        "FAIL",
                        format!("Assumptions contradictory at cycle {step}"),
                    ),
                    ProveOutcome::Unknown { max_k } => (
                        "UNKNOWN",
                        format!("Inconclusive up to k={max_k}"),
                    ),
                };
                sr.items.push(ReportItem {
                    name: format!("top:{} ({})", res.top_module_name, res.top_contract_name),
                    category: "compose:top".into(),
                    status: top_st.into(),
                    engine: res.top_winner.clone(),
                    detail: top_detail,
                    source: None,
                    trace: res.failure_trace.clone(),
                });
                fs::write(&rep_path, sr.to_html())?;
            }
            if format == "json" {
                let leaves: Vec<serde_json::Value> = res
                    .leaf_results
                    .iter()
                    .map(|l| {
                        serde_json::json!({
                            "instance": l.instance,
                            "contract": l.contract_name,
                            "original_ffs": l.original_ffs,
                            "proved": matches!(l.outcome, ProveOutcome::Proved { .. }),
                            "winner": l.winner,
                        })
                    })
                    .collect();
                let out = serde_json::json!({
                    "composed_proved": res.composed_proved,
                    "top_module": res.top_module_name,
                    "top_contract": res.top_contract_name,
                    "top_ffs_before_blackbox": res.top_ffs_before_blackbox,
                    "top_ffs_after_blackbox": res.top_ffs_after_blackbox,
                    "top_blackboxed_signals": res.top_blackboxed_signals,
                    "top_winner": res.top_winner,
                    "leaves": leaves,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for l in &res.leaf_results {
                    let st = if matches!(l.outcome, ProveOutcome::Proved { .. }) {
                        "PROVED"
                    } else {
                        "FAILED"
                    };
                    println!(
                        "{st:<8} [compose:leaf] `{}` ({}, {} FFs) — {}",
                        l.instance, l.contract_name, l.original_ffs, l.winner
                    );
                }
                let top_st = if res.composed_proved { "PROVED" } else { "FAILED" };
                println!(
                    "{top_st:<8} [compose:top]  `{}` ({}) — FFs {} -> {} ({} signals abstracted, winner: {})",
                    res.top_module_name,
                    res.top_contract_name,
                    res.top_ffs_before_blackbox,
                    res.top_ffs_after_blackbox,
                    res.top_blackboxed_signals,
                    res.top_winner
                );
            }
            Ok(if res.composed_proved {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Commands::Vip {
            action,
            protocol,
            module,
            sva,
            output,
        } => {
            match action.as_str() {
                "list" => {
                    println!("VeloFormal Turnkey Protocol VIP Library ({} protocols):", VIP_CATALOG.len());
                    for v in VIP_CATALOG {
                        println!("  {:<14} {:<32} — {}", v.id, v.standard, v.summary);
                    }
                    Ok(ExitCode::SUCCESS)
                }
                "init" | "add" | "emit" => {
                    let proto = protocol
                        .as_deref()
                        .ok_or_else(|| anyhow::anyhow!("missing protocol name (try `veloformal vip list`)"))?;
                    let rendered = if sva {
                        render_vip_sva(proto, module.as_deref())?
                    } else {
                        render_vip_vfc(proto, module.as_deref())?
                    };
                    if let Some(path) = output {
                        fs::write(&path, &rendered)?;
                        println!("Wrote {} VIP to {}", proto, path.display());
                    } else {
                        print!("{rendered}");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                other => anyhow::bail!("unknown `vip` action '{other}' (expected `list` or `init`)"),
            }
        }
        Commands::Vplan {
            plan,
            format,
            report: report_path,
            html: html_path,
            md: md_path,
        } => {
            let rep = run_vplan_file(&plan)?;
            if let Some(rp) = &report_path {
                let ext = rp.extension().and_then(|e| e.to_str()).unwrap_or("html");
                let content = match ext {
                    "json" => rep.to_json(),
                    "md" => rep.to_markdown(),
                    _ => rep.to_html(),
                };
                fs::write(rp, content)?;
            }
            if let Some(hp) = &html_path {
                fs::write(hp, rep.to_html())?;
            }
            if let Some(mp) = &md_path {
                fs::write(mp, rep.to_markdown())?;
            }
            match format.as_str() {
                "json" => println!("{}", rep.to_json()),
                "markdown" | "md" => print!("{}", rep.to_markdown()),
                _ => {
                    println!(
                        "VPLAN {} — {}/{} requirements verified ({:.1}%)",
                        rep.project, rep.passed_requirements, rep.total_requirements, rep.coverage_pct
                    );
                    for it in &rep.items {
                        let st = if it.passed { "PASS" } else { "FAIL" };
                        println!("  {st:<5} [{}] `{}` — {} ({})", it.kind, it.id, it.title, it.detail);
                    }
                }
            }
            Ok(if rep.all_passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

fn emit_signoff_report(
    rep: &SignOffReport,
    format: &str,
    html_path: Option<&std::path::Path>,
) -> Result<()> {
    match format {
        "json" => println!("{}", rep.to_json()),
        "junit" => print!("{}", rep.to_junit_xml()),
        "sarif" => println!("{}", rep.to_sarif()),
        _ => {
            for item in &rep.items {
                println!(
                    "{:<11} [{}] `{}` — {}",
                    item.status, item.category, item.name, item.detail
                );
            }
        }
    }
    if let Some(p) = html_path {
        fs::write(p, rep.to_html())?;
        if format == "text" {
            println!("Interactive HTML sign-off report written to {}", p.display());
        }
    }
    Ok(())
}

