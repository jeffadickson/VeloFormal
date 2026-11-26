use anyhow::{anyhow, bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vf_ir::{ConstBits, Module};

use crate::smt2::sig_name;

/// Cooperative + immediate child-process cancellation token shared across portfolio workers.
/// Calling [`CancelToken::cancel`] immediately kills and reaps every registered solver subprocess
/// (`z3`, `bitwuzla`, `cvc5`, `yosys-abc`), unblocking any worker mid-query in < 1 ms with zero zombies.
#[derive(Default)]
pub struct CancelToken {
    cancelled: AtomicBool,
    children: Mutex<Vec<Arc<Mutex<Option<Child>>>>>,
}

impl CancelToken {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Register a spawned child process. If already cancelled, kills and reaps it immediately.
    pub fn register(&self, mut child: Child) -> Arc<Mutex<Option<Child>>> {
        if self.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Arc::new(Mutex::new(None));
        }
        let slot = Arc::new(Mutex::new(Some(child)));
        if let Ok(mut list) = self.children.lock() {
            if self.is_cancelled() {
                if let Ok(mut guard) = slot.lock() {
                    if let Some(mut c) = guard.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            } else {
                list.push(Arc::clone(&slot));
            }
        }
        slot
    }

    /// Signal cancellation and immediately terminate + reap all registered child processes.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut list) = self.children.lock() {
            for slot in list.drain(..) {
                if let Ok(mut guard) = slot.lock() {
                    if let Some(mut c) = guard.take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatResult {
    Sat,
    Unsat,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub step: usize,
    pub values: Vec<(String, ConstBits)>,
}

pub struct SmtSolver {
    child: Arc<Mutex<Option<Child>>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    cancel: Option<Arc<CancelToken>>,
}

impl SmtSolver {
    pub fn spawn(solver_bin: &str) -> Result<Self> {
        Self::spawn_with_cancel(solver_bin, None)
    }

    pub fn spawn_with_cancel(solver_bin: &str, cancel: Option<Arc<CancelToken>>) -> Result<Self> {
        if cancel.as_ref().is_some_and(|c| c.is_cancelled()) {
            bail!("solver cancelled before spawn");
        }
        let mut cmd = Command::new(solver_bin);
        if solver_bin.contains("bitwuzla") {
            cmd.args(["--smt2", "--incremental"]);
        } else if solver_bin.contains("cvc5") {
            cmd.args(["--incremental", "--lang", "smt2"]);
        } else {
            // Z3 default
            cmd.args(["-in", "-smt2"]);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn SMT solver '{solver_bin}'"))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("missing solver stdin"))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| anyhow!("missing solver stdout"))?,
        );
        let slot = match &cancel {
            Some(tok) => tok.register(child),
            None => Arc::new(Mutex::new(Some(child))),
        };
        Ok(Self {
            child: slot,
            stdin,
            stdout,
            cancel,
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }

    pub fn send(&mut self, smt: &str) -> Result<()> {
        self.stdin.write_all(smt.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    pub fn push(&mut self) -> Result<()> {
        self.send("(push 1)\n")
    }

    pub fn pop(&mut self) -> Result<()> {
        self.send("(pop 1)\n")
    }

    pub fn check_sat(&mut self) -> Result<SatResult> {
        self.send("(check-sat)\n")?;
        let line = self.read_nonempty_line()?;
        match line.trim() {
            "sat" => Ok(SatResult::Sat),
            "unsat" => Ok(SatResult::Unsat),
            "unknown" => Ok(SatResult::Unknown),
            other => bail!("unexpected solver response to (check-sat): {other}"),
        }
    }

    pub fn extract_trace(&mut self, m: &Module, steps: usize) -> Result<Vec<TraceFrame>> {
        let mut frames = Vec::with_capacity(steps);
        for t in 0..steps {
            let mut values = Vec::with_capacity(m.signals.len());
            if m.signals.is_empty() {
                frames.push(TraceFrame { step: t, values });
                continue;
            }
            let mut query = String::from("(get-value (");
            for s in &m.signals {
                query.push(' ');
                query.push_str(&sig_name(s.id, t));
            }
            query.push_str("))\n");
            self.send(&query)?;
            let resp = self.read_balanced_sexp()?;
            let parsed = parse_get_value(&resp)?;
            for s in &m.signals {
                let key = sig_name(s.id, t);
                let bits = parsed
                    .iter()
                    .find(|(k, _)| k == &key)
                    .map(|(_, v)| parse_smt_bv(v, s.width))
                    .transpose()?
                    .unwrap_or_else(|| ConstBits::zero(s.width));
                values.push((s.name.clone(), bits));
            }
            frames.push(TraceFrame { step: t, values });
        }
        Ok(frames)
    }

    fn read_nonempty_line(&mut self) -> Result<String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                bail!("SMT solver closed stdout unexpectedly");
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }

    fn read_balanced_sexp(&mut self) -> Result<String> {
        let mut buf = String::new();
        let mut depth: i32 = 0;
        let mut started = false;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                bail!("SMT solver closed stdout while reading S-expression");
            }
            for ch in line.chars() {
                if ch == '(' {
                    depth += 1;
                    started = true;
                } else if ch == ')' {
                    depth -= 1;
                }
            }
            buf.push_str(&line);
            if started && depth <= 0 {
                return Ok(buf);
            }
        }
    }
}

impl Drop for SmtSolver {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"(exit)\n");
        let _ = self.stdin.flush();
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}

fn parse_get_value(sexp: &str) -> Result<Vec<(String, String)>> {
    // Parse `((s0_t0 #b0000) (s1_t0 #x0f) ...)`
    let mut out = Vec::new();
    let tokens: Vec<String> = tokenize_sexp(sexp);
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "(" && i + 3 < tokens.len() && tokens[i + 1] != "(" && tokens[i + 3] == ")"
        {
            out.push((tokens[i + 1].clone(), tokens[i + 2].clone()));
            i += 4;
        } else {
            i += 1;
        }
    }
    Ok(out)
}

fn tokenize_sexp(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' | ')' => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                tokens.push(ch.to_string());
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn parse_smt_bv(lit: &str, width: u32) -> Result<ConstBits> {
    if let Some(bin) = lit.strip_prefix("#b") {
        let c = ConstBits::new(bin.len() as u32, bin);
        return Ok(c.resize_zext(width));
    }
    if let Some(hex) = lit.strip_prefix("#x") {
        let mut bits = String::with_capacity(hex.len() * 4);
        for ch in hex.chars() {
            let v = ch
                .to_digit(16)
                .ok_or_else(|| anyhow!("invalid hex digit in SMT value: {lit}"))?;
            bits.push_str(&format!("{v:04b}"));
        }
        let c = ConstBits::new(bits.len() as u32, bits);
        return Ok(c.resize_zext(width));
    }
    bail!("unsupported SMT-LIB2 bitvector literal: {lit}")
}
