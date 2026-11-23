use crate::{ExprId, ExprKind, Module, PropertyKind, SignalKind};
use std::fmt::Write;

/// Render a `Module` in a compact RTLIL-inspired human-readable format.
pub fn dump_module(m: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module \\{}", m.name);

    for s in &m.signals {
        let kind = match s.kind {
            SignalKind::Input => "input",
            SignalKind::Output => "output",
            SignalKind::Wire => "wire",
            SignalKind::Reg => "reg",
            SignalKind::AnySeq => "anyseq",
            SignalKind::AnyConst => "anyconst",
        };
        let sign = if s.signed { " signed" } else { "" };
        let init = match &s.init {
            Some(c) => format!(" = {}'b{}", c.width, c.bits),
            None => String::new(),
        };
        let _ = writeln!(out, "  {kind} width {}{sign} \\{}{init}", s.width, s.name);
    }

    for mem in &m.memories {
        let _ = writeln!(
            out,
            "  memory width {} size {} addr_width {} \\{}",
            mem.data_width, mem.depth, mem.addr_width, mem.name
        );
    }

    for c in &m.comb {
        let _ = writeln!(
            out,
            "  connect \\{} = {}",
            m.signal(c.signal).name,
            fmt_expr(m, c.value)
        );
    }

    for ff in &m.ffs {
        let init = match &ff.init {
            Some(c) => format!(" init {}'b{}", c.width, c.bits),
            None => String::new(),
        };
        let arst = match &ff.async_reset {
            Some(r) => format!(
                " arst({}, {}'b{})",
                fmt_expr(m, r.cond),
                r.reset_val.width,
                r.reset_val.bits
            ),
            None => String::new(),
        };
        let _ = writeln!(
            out,
            "  dff \\{} <= {}{init}{arst}",
            m.signal(ff.signal).name,
            fmt_expr(m, ff.next)
        );
    }

    for mw in &m.mem_writes {
        let _ = writeln!(
            out,
            "  memwr \\{}[{}] <= {} if {}",
            m.memory(mw.mem).name,
            fmt_expr(m, mw.addr),
            fmt_expr(m, mw.data),
            fmt_expr(m, mw.enable)
        );
    }

    for p in &m.properties {
        let kw = match p.kind {
            PropertyKind::Assert => "assert",
            PropertyKind::Assume => "assume",
            PropertyKind::Cover => "cover",
        };
        let _ = writeln!(
            out,
            "  {kw} \"{}\" ({}) en ({})",
            p.name,
            fmt_expr(m, p.cond),
            fmt_expr(m, p.en)
        );
    }

    let _ = writeln!(out, "end");
    out
}

fn fmt_expr(m: &Module, id: ExprId) -> String {
    match &m.expr(id).kind {
        ExprKind::Const(c) => format!("{}'b{}", c.width, c.bits),
        ExprKind::Signal(s) => format!("\\{}", m.signal(*s).name),
        ExprKind::Not(a) => format!("(~{})", fmt_expr(m, *a)),
        ExprKind::Neg(a) => format!("(-{})", fmt_expr(m, *a)),
        ExprKind::RedAnd(a) => format!("(&{})", fmt_expr(m, *a)),
        ExprKind::RedOr(a) => format!("(|{})", fmt_expr(m, *a)),
        ExprKind::RedXor(a) => format!("(^{})", fmt_expr(m, *a)),
        ExprKind::Add(a, b) => format!("({} + {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Sub(a, b) => format!("({} - {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Mul(a, b) => format!("({} * {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::UDiv(a, b) => format!("({} /u {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::SDiv(a, b) => format!("({} /s {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::UMod(a, b) => format!("({} %u {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::SMod(a, b) => format!("({} %s {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::And(a, b) => format!("({} & {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Or(a, b) => format!("({} | {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Xor(a, b) => format!("({} ^ {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Shl(a, b) => format!("({} << {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Lshr(a, b) => format!("({} >> {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Ashr(a, b) => format!("({} >>> {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Eq(a, b) => format!("({} == {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Ne(a, b) => format!("({} != {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Ult(a, b) => format!("({} <u {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Ule(a, b) => format!("({} <=u {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Slt(a, b) => format!("({} <s {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Sle(a, b) => format!("({} <=s {})", fmt_expr(m, *a), fmt_expr(m, *b)),
        ExprKind::Mux {
            cond,
            then_val,
            else_val,
        } => format!(
            "({} ? {} : {})",
            fmt_expr(m, *cond),
            fmt_expr(m, *then_val),
            fmt_expr(m, *else_val)
        ),
        ExprKind::Concat(parts) => {
            let s: Vec<_> = parts.iter().map(|&p| fmt_expr(m, p)).collect();
            format!("{{{}}}", s.join(", "))
        }
        ExprKind::Slice { expr, hi, lo } => format!("{}[{hi}:{lo}]", fmt_expr(m, *expr)),
        ExprKind::Zext { expr, width } => format!("zext({}, {width})", fmt_expr(m, *expr)),
        ExprKind::Sext { expr, width } => format!("sext({}, {width})", fmt_expr(m, *expr)),
        ExprKind::MemRead { mem, addr } => {
            format!("\\{}[{}]", m.memory(*mem).name, fmt_expr(m, *addr))
        }
    }
}
