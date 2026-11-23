use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};
use vf_ir::{ExprId, ExprKind, MemId, Module, PropertyKind, SignalId, SignalKind};

/// AIGER literal: even = non-inverted (`2*var`), odd = inverted (`2*var + 1`).
/// `0` = constant FALSE, `1` = constant TRUE.
pub type Lit = u32;

pub const FALSE: Lit = 0;
pub const TRUE: Lit = 1;

#[inline]
pub fn neg(l: Lit) -> Lit {
    l ^ 1
}

#[derive(Debug, Clone)]
pub struct AigCircuit {
    pub num_inputs: u32,
    pub latches: Vec<Lit>,
    pub bad_outputs: Vec<(String, Lit)>,
    /// AND gates: index `i` has variable `1 + num_inputs + latches.len() + i`?
    /// Stored internally with arbitrary node ids during construction, then renumbered
    /// into canonical AIGER binary topological order in `to_binary_aig()`.
    nodes: Vec<AigNode>,
    and_table: HashMap<(Lit, Lit), Lit>,
}

#[derive(Debug, Clone, Copy)]
enum AigNode {
    Const,
    Input(u32),
    Latch(u32),
    And(Lit, Lit),
}

impl AigCircuit {
    fn new() -> Self {
        Self {
            num_inputs: 0,
            latches: Vec::new(),
            bad_outputs: Vec::new(),
            nodes: vec![AigNode::Const], // var 0 = FALSE
            and_table: HashMap::new(),
        }
    }

    fn new_input(&mut self) -> Lit {
        let idx = self.num_inputs;
        self.num_inputs += 1;
        let var = self.nodes.len() as u32;
        self.nodes.push(AigNode::Input(idx));
        var << 1
    }

    fn new_latch(&mut self) -> (usize, Lit) {
        let idx = self.latches.len();
        self.latches.push(FALSE);
        let var = self.nodes.len() as u32;
        self.nodes.push(AigNode::Latch(idx as u32));
        (idx, var << 1)
    }

    fn set_latch_next(&mut self, idx: usize, next: Lit) {
        self.latches[idx] = next;
    }

    fn and(&mut self, mut a: Lit, mut b: Lit) -> Lit {
        if a == FALSE || b == FALSE {
            return FALSE;
        }
        if a == TRUE {
            return b;
        }
        if b == TRUE {
            return a;
        }
        if a == b {
            return a;
        }
        if a == neg(b) {
            return FALSE;
        }
        if a < b {
            std::mem::swap(&mut a, &mut b);
        }
        if let Some(&existing) = self.and_table.get(&(a, b)) {
            return existing;
        }
        let var = self.nodes.len() as u32;
        self.nodes.push(AigNode::And(a, b));
        let lit = var << 1;
        self.and_table.insert((a, b), lit);
        lit
    }

    fn or(&mut self, a: Lit, b: Lit) -> Lit {
        neg(self.and(neg(a), neg(b)))
    }

    fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        let t1 = self.and(a, neg(b));
        let t2 = self.and(neg(a), b);
        self.or(t1, t2)
    }

    fn mux(&mut self, cond: Lit, then_v: Lit, else_v: Lit) -> Lit {
        if cond == TRUE {
            return then_v;
        }
        if cond == FALSE {
            return else_v;
        }
        if then_v == else_v {
            return then_v;
        }
        let t = self.and(cond, then_v);
        let e = self.and(neg(cond), else_v);
        self.or(t, e)
    }

    /// Encode into standard binary AIGER 1.0 format (`aig M I L O A`) with all latches
    /// zero-initialized and all assumptions folded into the bad outputs.
    pub fn to_binary_aig(&self) -> Vec<u8> {
        let num_i = self.num_inputs;
        let num_l = self.latches.len() as u32;

        // Map internal node variable -> canonical AIGER variable:
        //   0 -> 0 (FALSE)
        //   Input(k) -> 1 + k
        //   Latch(k) -> 1 + num_i + k
        //   And -> assigned in DFS topological post-order starting at 1 + num_i + num_l
        let mut var_map: Vec<Option<u32>> = vec![None; self.nodes.len()];
        var_map[0] = Some(0);
        for (v, node) in self.nodes.iter().enumerate() {
            match *node {
                AigNode::Const => var_map[v] = Some(0),
                AigNode::Input(k) => var_map[v] = Some(1 + k),
                AigNode::Latch(k) => var_map[v] = Some(1 + num_i + k),
                AigNode::And(_, _) => {}
            }
        }

        let mut next_and_var = 1 + num_i + num_l;
        let mut ordered_ands: Vec<(Lit, Lit)> = Vec::new();

        fn visit(
            lit: Lit,
            nodes: &[AigNode],
            var_map: &mut [Option<u32>],
            next_and_var: &mut u32,
            ordered_ands: &mut Vec<(Lit, Lit)>,
        ) -> Lit {
            let v = (lit >> 1) as usize;
            let inv = lit & 1;
            if let Some(mapped_v) = var_map[v] {
                return (mapped_v << 1) | inv;
            }
            let AigNode::And(a, b) = nodes[v] else {
                unreachable!()
            };
            let ma = visit(a, nodes, var_map, next_and_var, ordered_ands);
            let mb = visit(b, nodes, var_map, next_and_var, ordered_ands);
            let (rhs0, rhs1) = if ma >= mb { (ma, mb) } else { (mb, ma) };
            let mv = *next_and_var;
            *next_and_var += 1;
            var_map[v] = Some(mv);
            ordered_ands.push((rhs0, rhs1));
            (mv << 1) | inv
        }

        // Visit all latch next-states and bad outputs to collect reachable AND gates.
        let mut mapped_latches = Vec::with_capacity(self.latches.len());
        for &l in &self.latches {
            mapped_latches.push(visit(
                l,
                &self.nodes,
                &mut var_map,
                &mut next_and_var,
                &mut ordered_ands,
            ));
        }
        let mut mapped_outputs = Vec::with_capacity(self.bad_outputs.len());
        for (_, l) in &self.bad_outputs {
            mapped_outputs.push(visit(
                *l,
                &self.nodes,
                &mut var_map,
                &mut next_and_var,
                &mut ordered_ands,
            ));
        }

        let num_a = ordered_ands.len() as u32;
        let max_var = num_i + num_l + num_a;
        let num_o = mapped_outputs.len() as u32;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            format!("aig {max_var} {num_i} {num_l} {num_o} {num_a}\n").as_bytes(),
        );
        for nl in mapped_latches {
            bytes.extend_from_slice(format!("{nl}\n").as_bytes());
        }
        for ol in mapped_outputs {
            bytes.extend_from_slice(format!("{ol}\n").as_bytes());
        }

        for (i, (rhs0, rhs1)) in ordered_ands.into_iter().enumerate() {
            let lhs = (1 + num_i + num_l + i as u32) << 1;
            debug_assert!(lhs > rhs0 && rhs0 >= rhs1);
            encode_uleb128(lhs - rhs0, &mut bytes);
            encode_uleb128(rhs0 - rhs1, &mut bytes);
        }

        bytes
    }
}

fn encode_uleb128(mut x: u32, out: &mut Vec<u8>) {
    while x >= 0x80 {
        out.push(((x & 0x7f) as u8) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// Bit-blast a word-level `vf_ir::Module` into a zero-initialized, constraint-folded AIGER circuit.
pub fn bitblast_to_aig(m: &Module) -> Result<AigCircuit> {
    let mut bb = BitBlaster::new(m);
    bb.build()?;
    Ok(bb.aig)
}

struct BitBlaster<'a> {
    m: &'a Module,
    aig: AigCircuit,
    init_done_lit: Lit,
    sig_bits: HashMap<SignalId, Vec<Lit>>,
    /// (latch_idx, invert_on_write) for each bit of each FF.
    ff_latches: HashMap<SignalId, Vec<(usize, bool)>>,
    /// Bit-blasted memory cells: `mem_id -> [word_0_bits, word_1_bits, ...]`.
    mem_bits: HashMap<MemId, Vec<Vec<Lit>>>,
    mem_latches: HashMap<MemId, Vec<Vec<(usize, bool)>>>,
    expr_cache: HashMap<ExprId, Vec<Lit>>,
}

impl<'a> BitBlaster<'a> {
    fn new(m: &'a Module) -> Self {
        let mut aig = AigCircuit::new();
        let (init_done_idx, init_done_lit) = aig.new_latch();
        aig.set_latch_next(init_done_idx, TRUE);
        Self {
            m,
            aig,
            init_done_lit,
            sig_bits: HashMap::new(),
            ff_latches: HashMap::new(),
            mem_bits: HashMap::new(),
            mem_latches: HashMap::new(),
            expr_cache: HashMap::new(),
        }
    }

    fn build(&mut self) -> Result<()> {
        let ff_map: HashMap<SignalId, &vf_ir::Ff> =
            self.m.ffs.iter().map(|f| (f.signal, f)).collect();
        let comb_sigs: HashSet<SignalId> = self.m.comb.iter().map(|c| c.signal).collect();

        // 1. Allocate primary inputs and state latches (LSB-first vectors of `Lit`).
        for s in &self.m.signals {
            if let Some(ff) = ff_map.get(&s.id) {
                let mut eff_bits = Vec::with_capacity(s.width as usize);
                let mut latches = Vec::with_capacity(s.width as usize);
                for bit in 0..s.width {
                    let (lidx, llit) = self.aig.new_latch();
                    if let Some(init) = &ff.init {
                        // MSB-first string -> bit `bit` from LSB
                        let ch = init.bits.as_bytes()[(init.width - 1 - bit) as usize];
                        let inv = ch == b'1';
                        eff_bits.push(if inv { neg(llit) } else { llit });
                        latches.push((lidx, inv));
                    } else {
                        // Unconstrained initial state: choose via PI at cycle 0, latch thereafter.
                        let init_pi = self.aig.new_input();
                        let eff = self.aig.mux(self.init_done_lit, llit, init_pi);
                        eff_bits.push(eff);
                        latches.push((lidx, false));
                    }
                }
                self.sig_bits.insert(s.id, eff_bits);
                self.ff_latches.insert(s.id, latches);
            } else if s.kind == SignalKind::AnyConst {
                let mut eff_bits = Vec::with_capacity(s.width as usize);
                for _ in 0..s.width {
                    let pi = self.aig.new_input();
                    let (lidx, llit) = self.aig.new_latch();
                    let eff = self.aig.mux(self.init_done_lit, llit, pi);
                    self.aig.set_latch_next(lidx, eff);
                    eff_bits.push(eff);
                }
                self.sig_bits.insert(s.id, eff_bits);
            } else if !comb_sigs.contains(&s.id) {
                let bits = (0..s.width).map(|_| self.aig.new_input()).collect();
                self.sig_bits.insert(s.id, bits);
            }
        }

        // 2. Bit-blast bounded memories (up to depth 256) into latch arrays.
        for mem in &self.m.memories {
            if mem.depth > 256 {
                bail!(
                    "memory '{}' has depth {} > 256 (too large for AIGER bit-blasting; use smtbmc)",
                    mem.name,
                    mem.depth
                );
            }
            let init_map: HashMap<u32, &vf_ir::ConstBits> =
                mem.init.iter().map(|(a, v)| (*a, v)).collect();
            let mut words = Vec::with_capacity(mem.depth as usize);
            let mut word_latches = Vec::with_capacity(mem.depth as usize);
            for addr in 0..mem.depth {
                let mut wbits = Vec::with_capacity(mem.data_width as usize);
                let mut wlatches = Vec::with_capacity(mem.data_width as usize);
                for bit in 0..mem.data_width {
                    let (lidx, llit) = self.aig.new_latch();
                    if let Some(init) = init_map.get(&addr) {
                        let ch = init.bits.as_bytes()[(init.width - 1 - bit) as usize];
                        let inv = ch == b'1';
                        wbits.push(if inv { neg(llit) } else { llit });
                        wlatches.push((lidx, inv));
                    } else {
                        let init_pi = self.aig.new_input();
                        let eff = self.aig.mux(self.init_done_lit, llit, init_pi);
                        wbits.push(eff);
                        wlatches.push((lidx, false));
                    }
                }
                words.push(wbits);
                word_latches.push(wlatches);
            }
            self.mem_bits.insert(mem.id, words);
            self.mem_latches.insert(mem.id, word_latches);
        }

        // 3. Bit-blast combinational assigns in topological order.
        for ca in &self.m.comb {
            let bits = self.blast_expr(ca.value)?;
            self.sig_bits.insert(ca.signal, bits);
        }

        // 4. Connect flip-flop next-state logic.
        for ff in &self.m.ffs {
            let next_bits = self.blast_expr(ff.next)?;
            let latches = self.ff_latches[&ff.signal].clone();
            for ((lidx, inv), &nlit) in latches.into_iter().zip(next_bits.iter()) {
                self.aig
                    .set_latch_next(lidx, if inv { neg(nlit) } else { nlit });
            }
        }

        // 5. Connect bit-blasted memory write ports.
        for mem in &self.m.memories {
            let mut cur_words = self.mem_bits[&mem.id].clone();
            for mw in self.m.mem_writes.iter().filter(|w| w.mem == mem.id) {
                let addr_bits = self.blast_expr(mw.addr)?;
                let data_bits = self.blast_expr(mw.data)?;
                let en = self.blast_expr(mw.enable)?[0];
                for addr in 0..mem.depth {
                    let addr_c = self.const_bits_u64(mem.addr_width, addr as u64);
                    let eq = self.blast_eq(&addr_bits, &addr_c);
                    let wr_en = self.aig.and(en, eq);
                    for bit in 0..mem.data_width as usize {
                        cur_words[addr as usize][bit] =
                            self.aig.mux(wr_en, data_bits[bit], cur_words[addr as usize][bit]);
                    }
                }
            }
            let word_latches = self.mem_latches[&mem.id].clone();
            for (addr, latches) in word_latches.into_iter().enumerate() {
                for (bit, (lidx, inv)) in latches.into_iter().enumerate() {
                    let nlit = cur_words[addr][bit];
                    self.aig
                        .set_latch_next(lidx, if inv { neg(nlit) } else { nlit });
                }
            }
        }

        // 6. Fold all assumptions into a sticky `$assumes_violated` latch (`init = 0`).
        let mut assumes_now = TRUE;
        for p in &self.m.properties {
            if p.kind == PropertyKind::Assume {
                let en = self.blast_expr(p.en)?[0];
                let cond = self.blast_expr(p.cond)?[0];
                // en => cond  ===  !en | cond
                let ok = self.aig.or(neg(en), cond);
                assumes_now = self.aig.and(assumes_now, ok);
            }
        }

        let (viol_idx, viol_q) = self.aig.new_latch();
        let viol_now = self.aig.or(viol_q, neg(assumes_now));
        self.aig.set_latch_next(viol_idx, viol_now);
        let assumes_valid = neg(viol_now);

        // 7. Emit folded bad outputs for every Assert property.
        for p in &self.m.properties {
            if p.kind == PropertyKind::Assert {
                let en = self.blast_expr(p.en)?[0];
                let cond = self.blast_expr(p.cond)?[0];
                let raw_fail = self.aig.and(en, neg(cond));
                let bad = self.aig.and(assumes_valid, raw_fail);
                self.aig.bad_outputs.push((p.name.clone(), bad));
            }
        }

        Ok(())
    }

    fn const_bits_u64(&self, width: u32, val: u64) -> Vec<Lit> {
        (0..width)
            .map(|i| if i < 64 && ((val >> i) & 1) != 0 { TRUE } else { FALSE })
            .collect()
    }

    fn blast_expr(&mut self, id: ExprId) -> Result<Vec<Lit>> {
        if let Some(v) = self.expr_cache.get(&id) {
            return Ok(v.clone());
        }
        let node = self.m.expr(id).clone();
        let bits = match node.kind {
            ExprKind::Const(c) => (0..c.width)
                .map(|i| {
                    if c.bits.as_bytes()[(c.width - 1 - i) as usize] == b'1' {
                        TRUE
                    } else {
                        FALSE
                    }
                })
                .collect(),
            ExprKind::Signal(s) => self.sig_bits[&s].clone(),
            ExprKind::Not(a) => self.blast_expr(a)?.into_iter().map(neg).collect(),
            ExprKind::Neg(a) => {
                let inv: Vec<Lit> = self.blast_expr(a)?.into_iter().map(neg).collect();
                let one = self.const_bits_u64(inv.len() as u32, 1);
                self.blast_add(&inv, &one)
            }
            ExprKind::RedAnd(a) => {
                let mut acc = TRUE;
                for l in self.blast_expr(a)? {
                    acc = self.aig.and(acc, l);
                }
                vec![acc]
            }
            ExprKind::RedOr(a) => {
                let mut acc = FALSE;
                for l in self.blast_expr(a)? {
                    acc = self.aig.or(acc, l);
                }
                vec![acc]
            }
            ExprKind::RedXor(a) => {
                let mut acc = FALSE;
                for l in self.blast_expr(a)? {
                    acc = self.aig.xor(acc, l);
                }
                vec![acc]
            }
            ExprKind::Add(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                self.blast_add(&a, &b)
            }
            ExprKind::Sub(a, b) => {
                let a = self.blast_expr(a)?;
                let b_inv: Vec<Lit> = self.blast_expr(b)?.into_iter().map(neg).collect();
                self.blast_add_cin(&a, &b_inv, TRUE)
            }
            ExprKind::Mul(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                let w = a.len();
                let mut acc = vec![FALSE; w];
                for (i, &bi) in b.iter().enumerate() {
                    let mut pp = vec![FALSE; w];
                    for j in 0..(w - i) {
                        pp[i + j] = self.aig.and(a[j], bi);
                    }
                    acc = self.blast_add(&acc, &pp);
                }
                acc
            }
            ExprKind::UDiv(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                self.blast_udiv_rem(&a, &b).0
            }
            ExprKind::UMod(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                self.blast_udiv_rem(&a, &b).1
            }
            ExprKind::SDiv(a, b) | ExprKind::SMod(a, b) => {
                let is_div = matches!(node.kind, ExprKind::SDiv(_, _));
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                let sa = *a.last().unwrap();
                let sb = *b.last().unwrap();
                let abs_a = self.blast_abs(&a, sa);
                let abs_b = self.blast_abs(&b, sb);
                let (q_u, r_u) = self.blast_udiv_rem(&abs_a, &abs_b);
                if is_div {
                    let q_sign = self.aig.xor(sa, sb);
                    self.blast_cond_neg(&q_u, q_sign)
                } else {
                    self.blast_cond_neg(&r_u, sa)
                }
            }
            ExprKind::And(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                a.into_iter().zip(b).map(|(x, y)| self.aig.and(x, y)).collect()
            }
            ExprKind::Or(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                a.into_iter().zip(b).map(|(x, y)| self.aig.or(x, y)).collect()
            }
            ExprKind::Xor(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                a.into_iter().zip(b).map(|(x, y)| self.aig.xor(x, y)).collect()
            }
            ExprKind::Shl(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                self.blast_shift(&a, &b, true, FALSE)
            }
            ExprKind::Lshr(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                self.blast_shift(&a, &b, false, FALSE)
            }
            ExprKind::Ashr(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                let sign = *a.last().unwrap();
                self.blast_shift(&a, &b, false, sign)
            }
            ExprKind::Eq(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![self.blast_eq(&a, &b)]
            }
            ExprKind::Ne(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![neg(self.blast_eq(&a, &b))]
            }
            ExprKind::Ult(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![self.blast_ult(&a, &b)]
            }
            ExprKind::Ule(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![neg(self.blast_ult(&b, &a))]
            }
            ExprKind::Slt(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![self.blast_slt(&a, &b)]
            }
            ExprKind::Sle(a, b) => {
                let a = self.blast_expr(a)?;
                let b = self.blast_expr(b)?;
                vec![neg(self.blast_slt(&b, &a))]
            }
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                let c = self.blast_expr(cond)?[0];
                let t = self.blast_expr(then_val)?;
                let e = self.blast_expr(else_val)?;
                t.into_iter()
                    .zip(e)
                    .map(|(tv, ev)| self.aig.mux(c, tv, ev))
                    .collect()
            }
            ExprKind::Concat(parts) => {
                // `parts` is MSB-first, internal vectors are LSB-first!
                let mut out = Vec::new();
                for &p in parts.iter().rev() {
                    out.extend(self.blast_expr(p)?);
                }
                out
            }
            ExprKind::Slice { expr, hi, lo } => {
                let base = self.blast_expr(expr)?;
                base[lo as usize..=hi as usize].to_vec()
            }
            ExprKind::Zext { expr, width } => {
                let mut v = self.blast_expr(expr)?;
                v.resize(width as usize, FALSE);
                v
            }
            ExprKind::Sext { expr, width } => {
                let mut v = self.blast_expr(expr)?;
                let sign = *v.last().unwrap_or(&FALSE);
                v.resize(width as usize, sign);
                v
            }
            ExprKind::MemRead { mem, addr } => {
                let addr_bits = self.blast_expr(addr)?;
                let words = self.mem_bits[&mem].clone();
                let dw = self.m.memory(mem).data_width as usize;
                let aw = self.m.memory(mem).addr_width;
                let mut acc = vec![FALSE; dw];
                for (idx, word) in words.iter().enumerate() {
                    let c = self.const_bits_u64(aw, idx as u64);
                    let eq = self.blast_eq(&addr_bits, &c);
                    for bit in 0..dw {
                        acc[bit] = self.aig.mux(eq, word[bit], acc[bit]);
                    }
                }
                acc
            }
        };
        self.expr_cache.insert(id, bits.clone());
        Ok(bits)
    }

    fn blast_add(&mut self, a: &[Lit], b: &[Lit]) -> Vec<Lit> {
        self.blast_add_cin(a, b, FALSE)
    }

    fn blast_add_cin(&mut self, a: &[Lit], b: &[Lit], mut carry: Lit) -> Vec<Lit> {
        let mut out = Vec::with_capacity(a.len());
        for (&ai, &bi) in a.iter().zip(b.iter()) {
            let axb = self.aig.xor(ai, bi);
            let sum = self.aig.xor(axb, carry);
            let c1 = self.aig.and(ai, bi);
            let c2 = self.aig.and(axb, carry);
            carry = self.aig.or(c1, c2);
            out.push(sum);
        }
        out
    }

    fn blast_eq(&mut self, a: &[Lit], b: &[Lit]) -> Lit {
        let mut acc = TRUE;
        for (&ai, &bi) in a.iter().zip(b.iter()) {
            let xnor = neg(self.aig.xor(ai, bi));
            acc = self.aig.and(acc, xnor);
        }
        acc
    }

    fn blast_ult(&mut self, a: &[Lit], b: &[Lit]) -> Lit {
        // Unsigned a < b: compute borrow-out of a - b (carry-out of a + ~b + 1 == 0)
        let mut carry = TRUE;
        for (&ai, &bi) in a.iter().zip(b.iter()) {
            let nbi = neg(bi);
            let axb = self.aig.xor(ai, nbi);
            let c1 = self.aig.and(ai, nbi);
            let c2 = self.aig.and(axb, carry);
            carry = self.aig.or(c1, c2);
        }
        neg(carry)
    }

    fn blast_slt(&mut self, a: &[Lit], b: &[Lit]) -> Lit {
        // Flip MSB of both operands and do unsigned `<`!
        let mut af = a.to_vec();
        let mut bf = b.to_vec();
        if let Some(last) = af.last_mut() {
            *last = neg(*last);
        }
        if let Some(last) = bf.last_mut() {
            *last = neg(*last);
        }
        self.blast_ult(&af, &bf)
    }

    fn blast_shift(&mut self, a: &[Lit], sh: &[Lit], left: bool, fill: Lit) -> Vec<Lit> {
        let w = a.len();
        let mut cur = a.to_vec();
        let stages = (w.next_power_of_two().trailing_zeros() as usize).max(1);
        for s in 0..stages.min(sh.len()) {
            let step = 1usize << s;
            let bit = sh[s];
            let mut next = Vec::with_capacity(w);
            for i in 0..w {
                let shifted = if left {
                    if i >= step { cur[i - step] } else { fill }
                } else if i + step < w {
                    cur[i + step]
                } else {
                    fill
                };
                next.push(self.aig.mux(bit, shifted, cur[i]));
            }
            cur = next;
        }
        // If shift amount >= w (any upper bit of `sh` is 1), fill all bits.
        if sh.len() > stages {
            let mut overflow = FALSE;
            for &b in &sh[stages..] {
                overflow = self.aig.or(overflow, b);
            }
            for x in &mut cur {
                *x = self.aig.mux(overflow, fill, *x);
            }
        }
        cur
    }

    fn blast_cond_neg(&mut self, a: &[Lit], do_neg: Lit) -> Vec<Lit> {
        let inv: Vec<Lit> = a.iter().map(|&l| neg(l)).collect();
        let one = self.const_bits_u64(a.len() as u32, 1);
        let negated = self.blast_add(&inv, &one);
        a.iter()
            .zip(negated)
            .map(|(&orig, neg_v)| self.aig.mux(do_neg, neg_v, orig))
            .collect()
    }

    fn blast_abs(&mut self, a: &[Lit], sign: Lit) -> Vec<Lit> {
        self.blast_cond_neg(a, sign)
    }

    fn blast_udiv_rem(&mut self, a: &[Lit], b: &[Lit]) -> (Vec<Lit>, Vec<Lit>) {
        let w = a.len();
        let mut rem = vec![FALSE; w];
        let mut quot = vec![FALSE; w];
        for i in (0..w).rev() {
            // shift rem left by 1, bring in a[i]
            let mut next_rem = Vec::with_capacity(w);
            next_rem.push(a[i]);
            next_rem.extend_from_slice(&rem[..w - 1]);
            // check next_rem >= b
            let ge = neg(self.blast_ult(&next_rem, b));
            let b_inv: Vec<Lit> = b.iter().map(|&l| neg(l)).collect();
            let sub = self.blast_add_cin(&next_rem, &b_inv, TRUE);
            rem = next_rem
                .into_iter()
                .zip(sub)
                .map(|(nr, s)| self.aig.mux(ge, s, nr))
                .collect();
            quot[i] = ge;
        }
        (quot, rem)
    }
}
