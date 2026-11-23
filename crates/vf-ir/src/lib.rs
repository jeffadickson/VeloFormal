pub mod fmt;
pub mod opt;
pub mod passes;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignalId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExprId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemId(pub usize);

/// Arbitrary-width two-state bitvector constant, stored MSB-first as `'0'`/`'1'`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConstBits {
    pub width: u32,
    /// MSB-first binary string of length `width`, containing only `'0'` and `'1'`.
    pub bits: String,
}

impl ConstBits {
    pub fn new(width: u32, bits: impl Into<String>) -> Self {
        let bits = bits.into();
        assert_eq!(
            bits.len() as u32,
            width,
            "ConstBits length {} does not match width {}",
            bits.len(),
            width
        );
        assert!(
            bits.bytes().all(|b| b == b'0' || b == b'1'),
            "ConstBits must contain only 0/1, got {bits:?}"
        );
        Self { width, bits }
    }

    pub fn zero(width: u32) -> Self {
        Self {
            width,
            bits: "0".repeat(width as usize),
        }
    }

    pub fn one(width: u32) -> Self {
        Self::from_u64(width, 1)
    }

    pub fn from_u64(width: u32, value: u64) -> Self {
        let mut s = String::with_capacity(width as usize);
        for i in (0..width).rev() {
            let bit = if i < 64 && ((value >> i) & 1) != 0 {
                '1'
            } else {
                '0'
            };
            s.push(bit);
        }
        Self { width, bits: s }
    }

    pub fn from_u128(width: u32, value: u128) -> Self {
        let mut s = String::with_capacity(width as usize);
        for i in (0..width).rev() {
            let bit = if i < 128 && ((value >> i) & 1) != 0 {
                '1'
            } else {
                '0'
            };
            s.push(bit);
        }
        Self { width, bits: s }
    }

    pub fn from_i64(width: u32, value: i64) -> Self {
        let sign_bit = if value < 0 { '1' } else { '0' };
        let u = value as u64;
        let mut s = String::with_capacity(width as usize);
        for i in (0..width).rev() {
            let bit = if i < 64 {
                if ((u >> i) & 1) != 0 { '1' } else { '0' }
            } else {
                sign_bit
            };
            s.push(bit);
        }
        Self { width, bits: s }
    }

    pub fn is_zero(&self) -> bool {
        self.bits.bytes().all(|b| b == b'0')
    }

    pub fn is_all_ones(&self) -> bool {
        self.bits.bytes().all(|b| b == b'1')
    }

    pub fn to_u64(&self) -> Option<u64> {
        if self.width > 64 {
            return None;
        }
        u64::from_str_radix(&self.bits, 2).ok()
    }

    pub fn to_u128(&self) -> Option<u128> {
        if self.width > 128 {
            return None;
        }
        u128::from_str_radix(&self.bits, 2).ok()
    }

    pub fn resize_zext(&self, new_width: u32) -> Self {
        if new_width == self.width {
            return self.clone();
        }
        if new_width > self.width {
            let pad = "0".repeat((new_width - self.width) as usize);
            Self::new(new_width, format!("{pad}{}", self.bits))
        } else {
            let start = (self.width - new_width) as usize;
            Self::new(new_width, &self.bits[start..])
        }
    }

    pub fn resize_sext(&self, new_width: u32) -> Self {
        if new_width == self.width {
            return self.clone();
        }
        if new_width > self.width {
            let sign = self.bits.chars().next().unwrap_or('0');
            let pad: String = std::iter::repeat_n(sign, (new_width - self.width) as usize).collect();
            Self::new(new_width, format!("{pad}{}", self.bits))
        } else {
            let start = (self.width - new_width) as usize;
            Self::new(new_width, &self.bits[start..])
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalKind {
    Input,
    Output,
    Wire,
    Reg,
    /// Unconstrained symbolic value that may change every cycle (`$anyseq`).
    AnySeq,
    /// Unconstrained symbolic value fixed across all cycles (`$anyconst`).
    AnyConst,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub name: String,
    pub width: u32,
    pub signed: bool,
    pub kind: SignalKind,
    pub init: Option<ConstBits>,
    pub is_clock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemId,
    pub name: String,
    pub addr_width: u32,
    pub data_width: u32,
    pub depth: u32,
    pub init: Vec<(u32, ConstBits)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExprKind {
    Const(ConstBits),
    Signal(SignalId),

    Not(ExprId),
    Neg(ExprId),
    RedAnd(ExprId),
    RedOr(ExprId),
    RedXor(ExprId),

    Add(ExprId, ExprId),
    Sub(ExprId, ExprId),
    Mul(ExprId, ExprId),
    UDiv(ExprId, ExprId),
    SDiv(ExprId, ExprId),
    UMod(ExprId, ExprId),
    SMod(ExprId, ExprId),

    And(ExprId, ExprId),
    Or(ExprId, ExprId),
    Xor(ExprId, ExprId),

    Shl(ExprId, ExprId),
    Lshr(ExprId, ExprId),
    Ashr(ExprId, ExprId),

    Eq(ExprId, ExprId),
    Ne(ExprId, ExprId),
    Ult(ExprId, ExprId),
    Ule(ExprId, ExprId),
    Slt(ExprId, ExprId),
    Sle(ExprId, ExprId),

    Mux {
        cond: ExprId,
        then_val: ExprId,
        else_val: ExprId,
    },
    Concat(Vec<ExprId>),
    /// Inclusive bit slice `[hi:lo]` (0 = LSB).
    Slice {
        expr: ExprId,
        hi: u32,
        lo: u32,
    },
    Zext {
        expr: ExprId,
        width: u32,
    },
    Sext {
        expr: ExprId,
        width: u32,
    },
    MemRead {
        mem: MemId,
        addr: ExprId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExprNode {
    pub width: u32,
    pub kind: ExprKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClockEdge {
    PosEdge,
    NegEdge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClockSpec {
    pub signal: SignalId,
    pub edge: ClockEdge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncReset {
    pub cond: ExprId,
    pub reset_val: ExprId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ff {
    pub signal: SignalId,
    pub next: ExprId,
    pub init: Option<ConstBits>,
    pub clock: Option<ClockSpec>,
    pub async_reset: Option<AsyncReset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombAssign {
    pub signal: SignalId,
    pub value: ExprId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemWrite {
    pub mem: MemId,
    pub addr: ExprId,
    pub data: ExprId,
    pub enable: ExprId,
    pub clock: Option<ClockSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyKind {
    Assert,
    Assume,
    Cover,
    /// Liveness property (`assert (s_eventually cond)`): must hold infinitely often on fair paths.
    Live,
    /// Fairness constraint (`assume (s_eventually cond)`): restricts liveness loops to visit `cond`.
    Fair,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub kind: PropertyKind,
    /// 1-bit predicate to assert / assume / cover.
    pub cond: ExprId,
    /// 1-bit enable guard (e.g. from enclosing `if` condition).
    pub en: ExprId,
    pub source: Option<String>,
}

/// Flattened, word-level synchronous RTL transition system (Yosys RTLIL equivalent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Module {
    pub name: String,
    pub signals: Vec<Signal>,
    pub signal_by_name: HashMap<String, SignalId>,
    pub memories: Vec<Memory>,
    pub mem_by_name: HashMap<String, MemId>,
    pub exprs: Vec<ExprNode>,
    #[serde(skip)]
    expr_intern: HashMap<(u32, ExprKind), ExprId>,
    pub comb: Vec<CombAssign>,
    pub ffs: Vec<Ff>,
    pub mem_writes: Vec<MemWrite>,
    pub properties: Vec<Property>,
}

impl Module {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn add_signal(
        &mut self,
        name: impl Into<String>,
        width: u32,
        signed: bool,
        kind: SignalKind,
    ) -> SignalId {
        let name = name.into();
        assert!(width >= 1, "signal {name} must have width >= 1");
        if let Some(&existing) = self.signal_by_name.get(&name) {
            return existing;
        }
        let id = SignalId(self.signals.len());
        self.signals.push(Signal {
            id,
            name: name.clone(),
            width,
            signed,
            kind,
            init: None,
            is_clock: false,
        });
        self.signal_by_name.insert(name, id);
        id
    }

    pub fn signal(&self, id: SignalId) -> &Signal {
        &self.signals[id.0]
    }

    pub fn signal_mut(&mut self, id: SignalId) -> &mut Signal {
        &mut self.signals[id.0]
    }

    pub fn find_signal(&self, name: &str) -> Option<SignalId> {
        self.signal_by_name.get(name).copied()
    }

    pub fn add_memory(
        &mut self,
        name: impl Into<String>,
        addr_width: u32,
        data_width: u32,
        depth: u32,
    ) -> MemId {
        let name = name.into();
        if let Some(&existing) = self.mem_by_name.get(&name) {
            return existing;
        }
        let id = MemId(self.memories.len());
        self.memories.push(Memory {
            id,
            name: name.clone(),
            addr_width,
            data_width,
            depth,
            init: Vec::new(),
        });
        self.mem_by_name.insert(name, id);
        id
    }

    pub fn memory(&self, id: MemId) -> &Memory {
        &self.memories[id.0]
    }

    pub fn memory_mut(&mut self, id: MemId) -> &mut Memory {
        &mut self.memories[id.0]
    }

    pub fn expr(&self, id: ExprId) -> &ExprNode {
        &self.exprs[id.0]
    }

    pub fn width(&self, id: ExprId) -> u32 {
        self.exprs[id.0].width
    }

    pub fn intern(&mut self, width: u32, kind: ExprKind) -> ExprId {
        assert!(width >= 1, "expression width must be >= 1");
        if let Some(&id) = self.expr_intern.get(&(width, kind.clone())) {
            return id;
        }
        let id = ExprId(self.exprs.len());
        self.exprs.push(ExprNode {
            width,
            kind: kind.clone(),
        });
        self.expr_intern.insert((width, kind), id);
        id
    }

    pub fn c_bits(&mut self, c: ConstBits) -> ExprId {
        let w = c.width;
        self.intern(w, ExprKind::Const(c))
    }

    pub fn c_u64(&mut self, width: u32, val: u64) -> ExprId {
        self.c_bits(ConstBits::from_u64(width, val))
    }

    pub fn c_bool(&mut self, val: bool) -> ExprId {
        self.c_u64(1, u64::from(val))
    }

    pub fn sig_expr(&mut self, id: SignalId) -> ExprId {
        let w = self.signals[id.0].width;
        self.intern(w, ExprKind::Signal(id))
    }

    pub fn resize(&mut self, expr: ExprId, target_width: u32, signed: bool) -> ExprId {
        let w = self.width(expr);
        if w == target_width {
            expr
        } else if target_width < w {
            self.intern(
                target_width,
                ExprKind::Slice {
                    expr,
                    hi: target_width - 1,
                    lo: 0,
                },
            )
        } else if signed {
            self.intern(
                target_width,
                ExprKind::Sext {
                    expr,
                    width: target_width,
                },
            )
        } else {
            self.intern(
                target_width,
                ExprKind::Zext {
                    expr,
                    width: target_width,
                },
            )
        }
    }

    pub fn to_bool(&mut self, expr: ExprId) -> ExprId {
        if self.width(expr) == 1 {
            expr
        } else {
            self.intern(1, ExprKind::RedOr(expr))
        }
    }

    pub fn not(&mut self, expr: ExprId) -> ExprId {
        let w = self.width(expr);
        self.intern(w, ExprKind::Not(expr))
    }

    pub fn logic_not(&mut self, expr: ExprId) -> ExprId {
        let b = self.to_bool(expr);
        self.intern(1, ExprKind::Not(b))
    }

    pub fn logic_and(&mut self, a: ExprId, b: ExprId) -> ExprId {
        let a = self.to_bool(a);
        let b = self.to_bool(b);
        self.intern(1, ExprKind::And(a, b))
    }

    pub fn logic_or(&mut self, a: ExprId, b: ExprId) -> ExprId {
        let a = self.to_bool(a);
        let b = self.to_bool(b);
        self.intern(1, ExprKind::Or(a, b))
    }

    pub fn eq(&mut self, a: ExprId, b: ExprId, signed: bool) -> ExprId {
        let w = self.width(a).max(self.width(b));
        let a = self.resize(a, w, signed);
        let b = self.resize(b, w, signed);
        self.intern(1, ExprKind::Eq(a, b))
    }

    pub fn mux(&mut self, cond: ExprId, then_val: ExprId, else_val: ExprId) -> ExprId {
        let cond = self.to_bool(cond);
        let w = self.width(then_val).max(self.width(else_val));
        let then_val = self.resize(then_val, w, false);
        let else_val = self.resize(else_val, w, false);
        if then_val == else_val {
            return then_val;
        }
        if let ExprKind::Const(c) = &self.expr(cond).kind {
            return if c.is_zero() { else_val } else { then_val };
        }
        self.intern(
            w,
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            },
        )
    }

    pub fn concat(&mut self, parts: Vec<ExprId>) -> ExprId {
        assert!(!parts.is_empty(), "concat requires at least one part");
        if parts.len() == 1 {
            return parts[0];
        }
        let total: u32 = parts.iter().map(|&p| self.width(p)).sum();
        self.intern(total, ExprKind::Concat(parts))
    }

    pub fn slice(&mut self, expr: ExprId, hi: u32, lo: u32) -> ExprId {
        let w = self.width(expr);
        assert!(lo <= hi && hi < w, "invalid slice [{hi}:{lo}] on width {w}");
        if lo == 0 && hi == w - 1 {
            return expr;
        }
        match self.expr(expr).kind.clone() {
            ExprKind::Slice {
                expr: inner,
                lo: inner_lo,
                ..
            } => return self.slice(inner, inner_lo + hi, inner_lo + lo),
            ExprKind::Concat(parts) => {
                let mut cur_hi = w;
                let mut out = Vec::new();
                for p in parts {
                    let pw = self.width(p);
                    let cur_lo = cur_hi - pw;
                    let part_hi = cur_hi - 1;
                    // Check overlap between [part_hi, cur_lo] and [hi, lo]
                    if part_hi >= lo && cur_lo <= hi {
                        let o_hi = part_hi.min(hi) - cur_lo;
                        let o_lo = cur_lo.max(lo) - cur_lo;
                        out.push(self.slice(p, o_hi, o_lo));
                    }
                    cur_hi = cur_lo;
                }
                return self.concat(out);
            }
            ExprKind::Const(c) => {
                let start = (c.width - 1 - hi) as usize;
                let end = (c.width - lo) as usize;
                return self.c_bits(ConstBits::new(hi - lo + 1, &c.bits[start..end]));
            }
            _ => {}
        }
        self.intern(hi - lo + 1, ExprKind::Slice { expr, hi, lo })
    }

    /// Replace a constant bit slice `[hi:lo]` inside `base` with `replacement`.
    pub fn update_slice(&mut self, base: ExprId, hi: u32, lo: u32, replacement: ExprId) -> ExprId {
        let w = self.width(base);
        let rep = self.resize(replacement, hi - lo + 1, false);
        let mut parts = Vec::new();
        if hi + 1 < w {
            parts.push(self.slice(base, w - 1, hi + 1));
        }
        parts.push(rep);
        if lo > 0 {
            parts.push(self.slice(base, lo - 1, 0));
        }
        self.concat(parts)
    }

    /// Ensure combinational definitions are topologically sorted (no forward ref issues in SMT2/BTOR2).
    pub fn topo_sort_comb(&mut self) -> Result<(), String> {
        let comb_map: HashMap<SignalId, usize> = self
            .comb
            .iter()
            .enumerate()
            .map(|(i, c)| (c.signal, i))
            .collect();
        let n = self.comb.len();
        let mut state = vec![0u8; n]; // 0=unvisited, 1=visiting, 2=done
        let mut order = Vec::with_capacity(n);

        fn dfs(
            idx: usize,
            module: &Module,
            comb_map: &HashMap<SignalId, usize>,
            state: &mut [u8],
            order: &mut Vec<usize>,
        ) -> Result<(), String> {
            match state[idx] {
                2 => return Ok(()),
                1 => {
                    return Err(format!(
                        "combinational loop detected involving signal '{}'",
                        module.signal(module.comb[idx].signal).name
                    ));
                }
                _ => {}
            }
            state[idx] = 1;
            let mut deps = Vec::new();
            collect_signals(module, module.comb[idx].value, &mut deps);
            for sig in deps {
                if let Some(&dep_idx) = comb_map.get(&sig) {
                    dfs(dep_idx, module, comb_map, state, order)?;
                }
            }
            state[idx] = 2;
            order.push(idx);
            Ok(())
        }

        for i in 0..n {
            if state[i] == 0 {
                dfs(i, self, &comb_map, &mut state, &mut order)?;
            }
        }
        let old = std::mem::take(&mut self.comb);
        self.comb = order.into_iter().map(|i| old[i].clone()).collect();
        Ok(())
    }
}

pub fn collect_signals(module: &Module, expr: ExprId, out: &mut Vec<SignalId>) {
    let mut stack = vec![expr];
    let mut visited = std::collections::HashSet::new();
    while let Some(e) = stack.pop() {
        if !visited.insert(e) {
            continue;
        }
        match &module.expr(e).kind {
            ExprKind::Const(_) => {}
            ExprKind::Signal(s) => out.push(*s),
            ExprKind::Not(a)
            | ExprKind::Neg(a)
            | ExprKind::RedAnd(a)
            | ExprKind::RedOr(a)
            | ExprKind::RedXor(a)
            | ExprKind::Slice { expr: a, .. }
            | ExprKind::Zext { expr: a, .. }
            | ExprKind::Sext { expr: a, .. }
            | ExprKind::MemRead { addr: a, .. } => stack.push(*a),
            ExprKind::Add(a, b)
            | ExprKind::Sub(a, b)
            | ExprKind::Mul(a, b)
            | ExprKind::UDiv(a, b)
            | ExprKind::SDiv(a, b)
            | ExprKind::UMod(a, b)
            | ExprKind::SMod(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b)
            | ExprKind::Xor(a, b)
            | ExprKind::Shl(a, b)
            | ExprKind::Lshr(a, b)
            | ExprKind::Ashr(a, b)
            | ExprKind::Eq(a, b)
            | ExprKind::Ne(a, b)
            | ExprKind::Ult(a, b)
            | ExprKind::Ule(a, b)
            | ExprKind::Slt(a, b)
            | ExprKind::Sle(a, b) => {
                stack.push(*a);
                stack.push(*b);
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                stack.push(*cond);
                stack.push(*then_val);
                stack.push(*else_val);
            }
            ExprKind::Concat(parts) => {
                for &p in parts {
                    stack.push(p);
                }
            }
        }
    }
}
