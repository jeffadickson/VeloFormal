use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use vf_ir::{ConstBits, ExprId, ExprKind, MemId, Module, PropertyKind, SignalId, SignalKind};

/// Export a `vf_ir::Module` to standard BTOR2 format (compatible with Bitwuzla, Pono, rIC3, AVR).
pub fn export_btor2(m: &Module) -> String {
    let mut b = BtorBuilder::new(m);
    b.build()
}

struct BtorBuilder<'a> {
    m: &'a Module,
    next_id: usize,
    out: String,
    bv_sorts: HashMap<u32, usize>,
    arr_sorts: HashMap<(u32, u32), usize>,
    sig_nodes: HashMap<SignalId, usize>,
    mem_nodes: HashMap<MemId, usize>,
    expr_nodes: HashMap<ExprId, usize>,
}

impl<'a> BtorBuilder<'a> {
    fn new(m: &'a Module) -> Self {
        Self {
            m,
            next_id: 1,
            out: String::new(),
            bv_sorts: HashMap::new(),
            arr_sorts: HashMap::new(),
            sig_nodes: HashMap::new(),
            mem_nodes: HashMap::new(),
            expr_nodes: HashMap::new(),
        }
    }

    fn alloc(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn bv_sort(&mut self, width: u32) -> usize {
        if let Some(&id) = self.bv_sorts.get(&width) {
            return id;
        }
        let id = self.alloc();
        let _ = writeln!(self.out, "{id} sort bitvec {width}");
        self.bv_sorts.insert(width, id);
        id
    }

    fn arr_sort(&mut self, addr_w: u32, data_w: u32) -> usize {
        if let Some(&id) = self.arr_sorts.get(&(addr_w, data_w)) {
            return id;
        }
        let a = self.bv_sort(addr_w);
        let d = self.bv_sort(data_w);
        let id = self.alloc();
        let _ = writeln!(self.out, "{id} sort array {a} {d}");
        self.arr_sorts.insert((addr_w, data_w), id);
        id
    }

    fn emit_const(&mut self, c: &ConstBits) -> usize {
        let sort = self.bv_sort(c.width);
        let id = self.alloc();
        let _ = writeln!(self.out, "{id} const {sort} {}", c.bits);
        id
    }

    fn build(&mut self) -> String {
        let _ = writeln!(self.out, "; BTOR2 exported by VeloFormal for module {}", self.m.name);

        let ff_sigs: HashSet<SignalId> = self.m.ffs.iter().map(|f| f.signal).collect();
        let comb_sigs: HashSet<SignalId> = self.m.comb.iter().map(|c| c.signal).collect();

        // 1. Declare inputs and state registers (including AnyConst as self-holding states).
        for s in &self.m.signals {
            if ff_sigs.contains(&s.id) || s.kind == SignalKind::AnyConst {
                let sort = self.bv_sort(s.width);
                let id = self.alloc();
                let _ = writeln!(self.out, "{id} state {sort} {}", s.name);
                self.sig_nodes.insert(s.id, id);
                if s.kind == SignalKind::AnyConst {
                    let nid = self.alloc();
                    let _ = writeln!(self.out, "{nid} next {sort} {id} {id}");
                }
            } else if !comb_sigs.contains(&s.id) {
                let sort = self.bv_sort(s.width);
                let id = self.alloc();
                let _ = writeln!(self.out, "{id} input {sort} {}", s.name);
                self.sig_nodes.insert(s.id, id);
            }
        }

        // 2. Declare memory states.
        for mem in &self.m.memories {
            let sort = self.arr_sort(mem.addr_width, mem.data_width);
            let id = self.alloc();
            let _ = writeln!(self.out, "{id} state {sort} {}", mem.name);
            self.mem_nodes.insert(mem.id, id);
        }

        // 3. Emit combinational definitions in topological order.
        for ca in &self.m.comb {
            let val = self.emit_expr(ca.value);
            self.sig_nodes.insert(ca.signal, val);
        }

        // 4. Emit FF init and next nodes.
        for ff in &self.m.ffs {
            let state_id = self.sig_nodes[&ff.signal];
            let sort = self.bv_sort(self.m.signal(ff.signal).width);
            if let Some(init) = &ff.init {
                let c = self.emit_const(init);
                let iid = self.alloc();
                let _ = writeln!(self.out, "{iid} init {sort} {state_id} {c}");
            }
            let next_val = self.emit_expr(ff.next);
            let nid = self.alloc();
            let _ = writeln!(self.out, "{nid} next {sort} {state_id} {next_val}");
        }

        // 5. Emit memory next transitions.
        for mem in &self.m.memories {
            let sort = self.arr_sort(mem.addr_width, mem.data_width);
            let state_id = self.mem_nodes[&mem.id];
            let mut cur = state_id;
            for mw in self.m.mem_writes.iter().filter(|w| w.mem == mem.id) {
                let addr = self.emit_expr(mw.addr);
                let data = self.emit_expr(mw.data);
                let en = self.emit_expr(mw.enable);
                let wr = self.alloc();
                let _ = writeln!(self.out, "{wr} write {sort} {cur} {addr} {data}");
                let ite = self.alloc();
                let _ = writeln!(self.out, "{ite} ite {sort} {en} {wr} {cur}");
                cur = ite;
            }
            let nid = self.alloc();
            let _ = writeln!(self.out, "{nid} next {sort} {state_id} {cur}");
        }

        // 6. Emit properties: Assume -> constraint, Assert -> bad (en & !cond).
        let s1 = self.bv_sort(1);
        for p in &self.m.properties {
            let en = self.emit_expr(p.en);
            let cond = self.emit_expr(p.cond);
            match p.kind {
                PropertyKind::Assume => {
                    let imp = self.alloc();
                    let _ = writeln!(self.out, "{imp} implies {s1} {en} {cond}");
                    let cid = self.alloc();
                    let _ = writeln!(self.out, "{cid} constraint {imp} ; {}", p.name);
                }
                PropertyKind::Assert => {
                    let ncond = self.alloc();
                    let _ = writeln!(self.out, "{ncond} not {s1} {cond}");
                    let fail = self.alloc();
                    let _ = writeln!(self.out, "{fail} and {s1} {en} {ncond}");
                    let bid = self.alloc();
                    let _ = writeln!(self.out, "{bid} bad {fail} ; {}", p.name);
                }
                PropertyKind::Cover => {
                    let hit = self.alloc();
                    let _ = writeln!(self.out, "{hit} and {s1} {en} {cond}");
                    let _ = writeln!(self.out, "; cover {} = node {hit}", p.name);
                }
                PropertyKind::Fair => {
                    let imp = self.alloc();
                    let _ = writeln!(self.out, "{imp} implies {s1} {en} {cond}");
                    let fid = self.alloc();
                    let _ = writeln!(self.out, "{fid} fair {imp} ; {}", p.name);
                }
                PropertyKind::Live => {
                    let ncond = self.alloc();
                    let _ = writeln!(self.out, "{ncond} not {s1} {cond}");
                    let not_hit = self.alloc();
                    let _ = writeln!(self.out, "{not_hit} and {s1} {en} {ncond}");
                    let jid = self.alloc();
                    let _ = writeln!(self.out, "{jid} justice 1 {not_hit} ; {}", p.name);
                }
            }
        }

        std::mem::take(&mut self.out)
    }

    fn emit_expr(&mut self, id: ExprId) -> usize {
        if let Some(&n) = self.expr_nodes.get(&id) {
            return n;
        }
        let w = self.m.width(id);
        let sort = self.bv_sort(w);
        let res = match &self.m.expr(id).kind {
            ExprKind::Const(c) => self.emit_const(c),
            ExprKind::Signal(s) => self.sig_nodes[s],
            ExprKind::Not(a) => {
                let a = self.emit_expr(*a);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} not {sort} {a}");
                n
            }
            ExprKind::Neg(a) => {
                let a = self.emit_expr(*a);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} neg {sort} {a}");
                n
            }
            ExprKind::RedAnd(a) => {
                let a = self.emit_expr(*a);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} redand {sort} {a}");
                n
            }
            ExprKind::RedOr(a) => {
                let a = self.emit_expr(*a);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} redor {sort} {a}");
                n
            }
            ExprKind::RedXor(a) => {
                let a = self.emit_expr(*a);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} redxor {sort} {a}");
                n
            }
            ExprKind::Add(a, b) => self.bin("add", sort, *a, *b),
            ExprKind::Sub(a, b) => self.bin("sub", sort, *a, *b),
            ExprKind::Mul(a, b) => self.bin("mul", sort, *a, *b),
            ExprKind::UDiv(a, b) => self.bin("udiv", sort, *a, *b),
            ExprKind::SDiv(a, b) => self.bin("sdiv", sort, *a, *b),
            ExprKind::UMod(a, b) => self.bin("urem", sort, *a, *b),
            ExprKind::SMod(a, b) => self.bin("srem", sort, *a, *b),
            ExprKind::And(a, b) => self.bin("and", sort, *a, *b),
            ExprKind::Or(a, b) => self.bin("or", sort, *a, *b),
            ExprKind::Xor(a, b) => self.bin("xor", sort, *a, *b),
            ExprKind::Shl(a, b) => self.bin("sll", sort, *a, *b),
            ExprKind::Lshr(a, b) => self.bin("srl", sort, *a, *b),
            ExprKind::Ashr(a, b) => self.bin("sra", sort, *a, *b),
            ExprKind::Eq(a, b) => self.bin("eq", sort, *a, *b),
            ExprKind::Ne(a, b) => self.bin("neq", sort, *a, *b),
            ExprKind::Ult(a, b) => self.bin("ult", sort, *a, *b),
            ExprKind::Ule(a, b) => self.bin("ulte", sort, *a, *b),
            ExprKind::Slt(a, b) => self.bin("slt", sort, *a, *b),
            ExprKind::Sle(a, b) => self.bin("slte", sort, *a, *b),
            ExprKind::Mux {
                cond,
                then_val,
                else_val,
            } => {
                let c = self.emit_expr(*cond);
                let t = self.emit_expr(*then_val);
                let e = self.emit_expr(*else_val);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} ite {sort} {c} {t} {e}");
                n
            }
            ExprKind::Concat(parts) => {
                let mut acc = self.emit_expr(parts[0]);
                let mut acc_w = self.m.width(parts[0]);
                for &p in &parts[1..] {
                    let pn = self.emit_expr(p);
                    acc_w += self.m.width(p);
                    let cs = self.bv_sort(acc_w);
                    let n = self.alloc();
                    let _ = writeln!(self.out, "{n} concat {cs} {acc} {pn}");
                    acc = n;
                }
                acc
            }
            ExprKind::Slice { expr, hi, lo } => {
                let e = self.emit_expr(*expr);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} slice {sort} {e} {hi} {lo}");
                n
            }
            ExprKind::Zext { expr, width } => {
                let extra = *width - self.m.width(*expr);
                let e = self.emit_expr(*expr);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} uext {sort} {e} {extra}");
                n
            }
            ExprKind::Sext { expr, width } => {
                let extra = *width - self.m.width(*expr);
                let e = self.emit_expr(*expr);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} sext {sort} {e} {extra}");
                n
            }
            ExprKind::MemRead { mem, addr } => {
                let mnode = self.mem_nodes[mem];
                let a = self.emit_expr(*addr);
                let n = self.alloc();
                let _ = writeln!(self.out, "{n} read {sort} {mnode} {a}");
                n
            }
        };
        self.expr_nodes.insert(id, res);
        res
    }

    fn bin(&mut self, op: &str, sort: usize, a: ExprId, b: ExprId) -> usize {
        let a = self.emit_expr(a);
        let b = self.emit_expr(b);
        let n = self.alloc();
        let _ = writeln!(self.out, "{n} {op} {sort} {a} {b}");
        n
    }
}
