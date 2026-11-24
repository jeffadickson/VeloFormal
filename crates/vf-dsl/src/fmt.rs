use std::fmt::Write;

use crate::ast::*;

/// Canonical 4-space LF formatter for `.vfc` v1.0 contracts (idempotent).
pub fn format_contract(c: &Contract) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "contract {} for {} {{", c.name, c.target_module);

    for p in &c.params {
        let _ = write!(out, "    parameter {}: int", p.name);
        if let Some(cond) = &p.constraint {
            let _ = write!(out, " where {}", fmt_expr(cond, 0));
        }
        out.push_str(";\n");
    }
    if let Some(clk) = &c.clock {
        let edge = match clk.edge {
            EdgeKind::PosEdge => "posedge",
            EdgeKind::NegEdge => "negedge",
        };
        let _ = writeln!(out, "    clock {edge} {};", clk.signal);
    }
    if let Some(rst) = &c.reset {
        let _ = writeln!(out, "    reset when {};", fmt_expr(&rst.cond, 0));
    }

    for s in &c.symbolics {
        let kind = match s.kind {
            SymbolicKind::AnyConst => "",
            SymbolicKind::AnySeq => "anyseq ",
        };
        let sign = if s.signed { " signed" } else { "" };
        let _ = writeln!(
            out,
            "    symbolic {kind}{}: logic[{}]{sign};",
            s.name, s.width
        );
    }
    for g in &c.ghosts {
        let sign = if g.signed { " signed" } else { "" };
        let _ = write!(out, "    ghost {}: logic[{}]{sign}", g.name, g.width);
        if let Some(init) = &g.init {
            let _ = write!(out, " = {}", fmt_expr(init, 0));
        }
        out.push_str(";\n");
    }
    for u in &c.updates {
        out.push_str("\n    update {\n");
        fmt_update_stmts(&u.stmts, 2, &mut out);
        out.push_str("    }\n");
    }

    if !c.requires.is_empty() {
        out.push('\n');
        for r in &c.requires {
            let _ = writeln!(out, "    require {}: {};", r.label, fmt_expr(&r.expr, 0));
        }
    }
    if !c.invariants.is_empty() {
        out.push('\n');
        for inv in &c.invariants {
            let _ = writeln!(
                out,
                "    invariant {}: {};",
                inv.label,
                fmt_expr(&inv.expr, 0)
            );
        }
    }
    for op in &c.operations {
        let _ = writeln!(
            out,
            "\n    operation {} when {} {{",
            op.name,
            fmt_expr(&op.trigger, 0)
        );
        for ens in &op.ensures {
            let _ = write!(out, "        ensure {} ", ens.label);
            match &ens.timing {
                EnsureTiming::After(n) => {
                    let _ = write!(out, "after {n} cycles");
                }
                EnsureTiming::Within(a, b) => {
                    let _ = write!(out, "within {a}..{b} cycles");
                }
                EnsureTiming::Until(ev) => {
                    let _ = write!(out, "until {}", fmt_expr(ev, 0));
                }
                EnsureTiming::Eventually => {
                    out.push_str("eventually");
                }
            }
            let _ = writeln!(out, ": {};", fmt_expr(&ens.response, 0));
        }
        out.push_str("    }\n");
    }
    if !c.fairs.is_empty() || !c.eventuallys.is_empty() {
        out.push('\n');
        for f in &c.fairs {
            let _ = writeln!(out, "    fair {}: {};", f.label, fmt_expr(&f.expr, 0));
        }
        for ev in &c.eventuallys {
            let _ = writeln!(
                out,
                "    eventually {}: {};",
                ev.label,
                fmt_expr(&ev.expr, 0)
            );
        }
    }
    if !c.covers.is_empty() {
        out.push('\n');
        for cov in &c.covers {
            let _ = writeln!(out, "    cover {}: {};", cov.label, fmt_expr(&cov.expr, 0));
        }
    }
    if !c.uses.is_empty() || !c.xfrees.is_empty() || !c.secures.is_empty() {
        out.push('\n');
        for u in &c.uses {
            let args: Vec<String> = u.args.iter().map(|a| fmt_expr(a, 0)).collect();
            let _ = writeln!(out, "    use {}({});", u.protocol, args.join(", "));
        }
        for xf in &c.xfrees {
            let _ = writeln!(out, "    xfree {};", xf.targets.join(", "));
        }
        for sec in &c.secures {
            let _ = write!(
                out,
                "    secure {} -> {}",
                sec.sources.join(", "),
                sec.targets.join(", ")
            );
            if let Some(u) = &sec.unless {
                let _ = write!(out, " unless {}", fmt_expr(u, 0));
            }
            out.push_str(";\n");
        }
    }

    out.push_str("}\n");
    out
}

fn fmt_update_stmts(stmts: &[UpdateStmt], indent: usize, out: &mut String) {
    let pad = "    ".repeat(indent);
    for s in stmts {
        match s {
            UpdateStmt::Assign { target, value, .. } => {
                let _ = writeln!(out, "{pad}{target} <= {};", fmt_expr(value, 0));
            }
            UpdateStmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let _ = writeln!(out, "{pad}if ({}) {{", fmt_expr(cond, 0));
                fmt_update_stmts(then_body, indent + 1, out);
                if else_body.is_empty() {
                    let _ = writeln!(out, "{pad}}}");
                } else if else_body.len() == 1 && matches!(else_body[0], UpdateStmt::If { .. }) {
                    let _ = write!(out, "{pad}}} else ");
                    fmt_else_if(&else_body[0], indent, out);
                } else {
                    let _ = writeln!(out, "{pad}}} else {{");
                    fmt_update_stmts(else_body, indent + 1, out);
                    let _ = writeln!(out, "{pad}}}");
                }
            }
        }
    }
}

fn fmt_else_if(stmt: &UpdateStmt, indent: usize, out: &mut String) {
    let pad = "    ".repeat(indent);
    if let UpdateStmt::If {
        cond,
        then_body,
        else_body,
        ..
    } = stmt
    {
        let _ = writeln!(out, "if ({}) {{", fmt_expr(cond, 0));
        fmt_update_stmts(then_body, indent + 1, out);
        if else_body.is_empty() {
            let _ = writeln!(out, "{pad}}}");
        } else if else_body.len() == 1 && matches!(else_body[0], UpdateStmt::If { .. }) {
            let _ = write!(out, "{pad}}} else ");
            fmt_else_if(&else_body[0], indent, out);
        } else {
            let _ = writeln!(out, "{pad}}} else {{");
            fmt_update_stmts(else_body, indent + 1, out);
            let _ = writeln!(out, "{pad}}}");
        }
    }
}

fn prec(op: BinOp) -> u8 {
    match op {
        BinOp::Implies => 1,
        BinOp::LogOr => 3,
        BinOp::LogAnd => 4,
        BinOp::BitOr => 5,
        BinOp::BitXor => 6,
        BinOp::BitAnd => 7,
        BinOp::Eq | BinOp::Ne => 8,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 9,
        BinOp::Shl | BinOp::Shr => 10,
        BinOp::Add | BinOp::Sub => 11,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 12,
    }
}

fn op_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Implies => "=>",
        BinOp::LogOr => "||",
        BinOp::LogAnd => "&&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::BitAnd => "&",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
    }
}

pub fn fmt_expr(e: &Expr, min_prec: u8) -> String {
    match &e.kind {
        ExprKind::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        ExprKind::Int(v) => v.to_string(),
        ExprKind::SizedLit {
            width,
            signed,
            bits,
        } => {
            let s = if *signed { "s" } else { "" };
            format!("{width}'{s}b{bits}")
        }
        ExprKind::Ident(s) => s.clone(),
        ExprKind::Unary { op, expr } => {
            let sym = match op {
                UnOp::LogNot => "!",
                UnOp::BitNot => "~",
                UnOp::Neg => "-",
            };
            format!("{sym}{}", fmt_expr(expr, 13))
        }
        ExprKind::Binary { op, left, right } => {
            let p = prec(*op);
            let rp = if *op == BinOp::Implies { p } else { p + 1 };
            let s = format!(
                "{} {} {}",
                fmt_expr(left, p),
                op_str(*op),
                fmt_expr(right, rp)
            );
            if p < min_prec {
                format!("({s})")
            } else {
                s
            }
        }
        ExprKind::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            let s = format!(
                "{} ? {} : {}",
                fmt_expr(cond, 3),
                fmt_expr(then_expr, 0),
                fmt_expr(else_expr, 2)
            );
            if 2 < min_prec {
                format!("({s})")
            } else {
                s
            }
        }
        ExprKind::Call { name, args } => {
            let a: Vec<String> = args.iter().map(|x| fmt_expr(x, 0)).collect();
            format!("{name}({})", a.join(", "))
        }
        ExprKind::Slice { expr, hi, lo } => {
            if hi == lo {
                format!("{}[{hi}]", fmt_expr(expr, 14))
            } else {
                format!("{}[{hi}:{lo}]", fmt_expr(expr, 14))
            }
        }
        ExprKind::Concat(parts) => {
            let ps: Vec<String> = parts.iter().map(|x| fmt_expr(x, 0)).collect();
            format!("{{{}}}", ps.join(", "))
        }
    }
}
