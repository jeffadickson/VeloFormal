use anyhow::{bail, Result};

use crate::ast::*;
use crate::lexer::{tokenize, Span, Token, TokenKind};

pub fn parse_contract(src: &str) -> Result<Contract> {
    let tokens = tokenize(src)?;
    let mut p = Parser { tokens, pos: 0 };
    let c = p.parse_contract()?;
    if !p.at(&TokenKind::Eof) {
        bail!(
            "unexpected trailing token {:?} at {}",
            p.peek().kind,
            p.peek().span
        );
    }
    Ok(c)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn at(&self, kind: &TokenKind) -> bool {
        &self.peek().kind == kind
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Ident(s) if s == kw)
    }

    fn next(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: TokenKind) -> Result<Span> {
        let tok = self.next();
        if tok.kind == expected {
            Ok(tok.span)
        } else {
            bail!(
                "expected {:?}, found {:?} at {}",
                expected,
                tok.kind,
                tok.span
            )
        }
    }

    fn expect_kw(&mut self, kw: &str) -> Result<Span> {
        let tok = self.next();
        if matches!(&tok.kind, TokenKind::Ident(s) if s == kw) {
            Ok(tok.span)
        } else {
            bail!(
                "expected keyword '{kw}', found {:?} at {}",
                tok.kind,
                tok.span
            )
        }
    }

    fn expect_ident(&mut self) -> Result<(String, Span)> {
        let tok = self.next();
        if let TokenKind::Ident(s) = tok.kind {
            Ok((s, tok.span))
        } else {
            bail!("expected identifier, found {:?} at {}", tok.kind, tok.span)
        }
    }

    fn expect_u32(&mut self) -> Result<(u32, Span)> {
        let tok = self.next();
        if let TokenKind::Int(v) = tok.kind {
            let v32 = u32::try_from(v)
                .map_err(|_| anyhow::anyhow!("integer {v} exceeds u32 at {}", tok.span))?;
            Ok((v32, tok.span))
        } else {
            bail!("expected integer, found {:?} at {}", tok.kind, tok.span)
        }
    }

    fn parse_contract(&mut self) -> Result<Contract> {
        let span = self.expect_kw("contract")?;
        let (name, _) = self.expect_ident()?;
        self.expect_kw("for")?;
        let (target_module, _) = self.expect_ident()?;
        self.expect(TokenKind::LBrace)?;

        let mut contract = Contract {
            name,
            target_module,
            span,
            params: Vec::new(),
            clock: None,
            reset: None,
            symbolics: Vec::new(),
            ghosts: Vec::new(),
            updates: Vec::new(),
            requires: Vec::new(),
            invariants: Vec::new(),
            operations: Vec::new(),
            fairs: Vec::new(),
            eventuallys: Vec::new(),
            covers: Vec::new(),
            uses: Vec::new(),
            xfrees: Vec::new(),
            secures: Vec::new(),
        };

        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            if self.at_kw("parameter") {
                contract.params.push(self.parse_param()?);
            } else if self.at_kw("clock") {
                if contract.clock.is_some() {
                    bail!("duplicate `clock` declaration at {}", self.peek().span);
                }
                contract.clock = Some(self.parse_clock()?);
            } else if self.at_kw("reset") {
                if contract.reset.is_some() {
                    bail!("duplicate `reset` declaration at {}", self.peek().span);
                }
                contract.reset = Some(self.parse_reset()?);
            } else if self.at_kw("symbolic") {
                contract.symbolics.push(self.parse_symbolic()?);
            } else if self.at_kw("ghost") {
                contract.ghosts.push(self.parse_ghost()?);
            } else if self.at_kw("update") {
                contract.updates.push(self.parse_update()?);
            } else if self.at_kw("require") {
                contract.requires.push(self.parse_labeled_expr("require")?);
            } else if self.at_kw("invariant") {
                contract
                    .invariants
                    .push(self.parse_labeled_expr("invariant")?);
            } else if self.at_kw("operation") {
                contract.operations.push(self.parse_operation()?);
            } else if self.at_kw("fair") {
                contract.fairs.push(self.parse_labeled_expr("fair")?);
            } else if self.at_kw("eventually") {
                contract
                    .eventuallys
                    .push(self.parse_labeled_expr("eventually")?);
            } else if self.at_kw("cover") {
                contract.covers.push(self.parse_labeled_expr("cover")?);
            } else if self.at_kw("use") {
                contract.uses.push(self.parse_use()?);
            } else if self.at_kw("xfree") {
                contract.xfrees.push(self.parse_xfree()?);
            } else if self.at_kw("secure") {
                contract.secures.push(self.parse_secure()?);
            } else {
                bail!(
                    "unexpected clause token {:?} at {}",
                    self.peek().kind,
                    self.peek().span
                );
            }
        }

        self.expect(TokenKind::RBrace)?;
        Ok(contract)
    }

    fn parse_param(&mut self) -> Result<ParamDecl> {
        let span = self.expect_kw("parameter")?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        if self.at_kw("int") || self.at_kw("integer") {
            self.next();
        } else {
            bail!(
                "expected parameter type `int` or `integer` at {}",
                self.peek().span
            );
        }
        let constraint = if self.at_kw("where") {
            self.next();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Semi)?;
        Ok(ParamDecl {
            name,
            constraint,
            span,
        })
    }

    fn parse_clock(&mut self) -> Result<ClockDecl> {
        let span = self.expect_kw("clock")?;
        let edge = if self.at_kw("posedge") {
            self.next();
            EdgeKind::PosEdge
        } else if self.at_kw("negedge") {
            self.next();
            EdgeKind::NegEdge
        } else {
            bail!(
                "expected `posedge` or `negedge` after `clock` at {}",
                self.peek().span
            );
        };
        let (signal, _) = self.expect_ident()?;
        self.expect(TokenKind::Semi)?;
        Ok(ClockDecl { edge, signal, span })
    }

    fn parse_reset(&mut self) -> Result<ResetDecl> {
        let span = self.expect_kw("reset")?;
        if self.at_kw("when") {
            self.next();
        }
        let cond = self.parse_expr()?;
        self.expect(TokenKind::Semi)?;
        Ok(ResetDecl { cond, span })
    }

    fn parse_logic_type(&mut self) -> Result<(u32, bool)> {
        if self.at_kw("bool") {
            self.next();
            return Ok((1, false));
        }
        self.expect_kw("logic")?;
        let width = if self.at(&TokenKind::LBracket) {
            self.next();
            let (w, wsp) = self.expect_u32()?;
            if w == 0 || w > 256 {
                bail!("logic width {w} must be in 1..=256 at {wsp}");
            }
            self.expect(TokenKind::RBracket)?;
            w
        } else {
            1
        };
        let signed = if self.at_kw("signed") {
            self.next();
            true
        } else {
            false
        };
        Ok((width, signed))
    }

    fn parse_symbolic(&mut self) -> Result<SymbolicDecl> {
        let span = self.expect_kw("symbolic")?;
        let kind = if self.at_kw("anyseq") {
            self.next();
            SymbolicKind::AnySeq
        } else if self.at_kw("anyconst") {
            self.next();
            SymbolicKind::AnyConst
        } else {
            SymbolicKind::AnyConst
        };
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let (width, signed) = self.parse_logic_type()?;
        self.expect(TokenKind::Semi)?;
        Ok(SymbolicDecl {
            name,
            width,
            signed,
            kind,
            span,
        })
    }

    fn parse_ghost(&mut self) -> Result<GhostDecl> {
        let span = self.expect_kw("ghost")?;
        let (name, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let (width, signed) = self.parse_logic_type()?;
        let init = if self.at(&TokenKind::Eq) {
            self.next();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Semi)?;
        Ok(GhostDecl {
            name,
            width,
            signed,
            init,
            span,
        })
    }

    fn parse_update(&mut self) -> Result<UpdateBlock> {
        let span = self.expect_kw("update")?;
        self.expect(TokenKind::LBrace)?;
        let stmts = self.parse_update_stmts()?;
        self.expect(TokenKind::RBrace)?;
        Ok(UpdateBlock { stmts, span })
    }

    fn parse_update_stmts(&mut self) -> Result<Vec<UpdateStmt>> {
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            if self.at_kw("if") {
                stmts.push(self.parse_update_if()?);
            } else {
                let (target, span) = self.expect_ident()?;
                self.expect(TokenKind::Le)?;
                let value = self.parse_expr()?;
                self.expect(TokenKind::Semi)?;
                stmts.push(UpdateStmt::Assign {
                    target,
                    value,
                    span,
                });
            }
        }
        Ok(stmts)
    }

    fn parse_update_if(&mut self) -> Result<UpdateStmt> {
        let span = self.expect_kw("if")?;
        self.expect(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::LBrace)?;
        let then_body = self.parse_update_stmts()?;
        self.expect(TokenKind::RBrace)?;
        let else_body = if self.at_kw("else") {
            self.next();
            if self.at_kw("if") {
                vec![self.parse_update_if()?]
            } else {
                self.expect(TokenKind::LBrace)?;
                let b = self.parse_update_stmts()?;
                self.expect(TokenKind::RBrace)?;
                b
            }
        } else {
            Vec::new()
        };
        Ok(UpdateStmt::If {
            cond,
            then_body,
            else_body,
            span,
        })
    }

    fn parse_labeled_expr(&mut self, kw: &str) -> Result<LabeledExpr> {
        let span = self.expect_kw(kw)?;
        let (label, _) = self.expect_ident()?;
        self.expect(TokenKind::Colon)?;
        let expr = self.parse_expr()?;
        self.expect(TokenKind::Semi)?;
        Ok(LabeledExpr { label, expr, span })
    }

    fn parse_operation(&mut self) -> Result<Operation> {
        let span = self.expect_kw("operation")?;
        let (name, _) = self.expect_ident()?;
        self.expect_kw("when")?;
        let trigger = self.parse_expr()?;
        self.expect(TokenKind::LBrace)?;
        let mut ensures = Vec::new();
        while !self.at(&TokenKind::RBrace) && !self.at(&TokenKind::Eof) {
            ensures.push(self.parse_ensure()?);
        }
        self.expect(TokenKind::RBrace)?;
        if ensures.is_empty() {
            bail!("operation '{name}' at {span} must contain at least one `ensure` clause");
        }
        Ok(Operation {
            name,
            trigger,
            ensures,
            span,
        })
    }

    fn parse_ensure(&mut self) -> Result<EnsureClause> {
        let span = self.expect_kw("ensure")?;
        let (label, _) = self.expect_ident()?;
        let timing = if self.at_kw("after") {
            self.next();
            let (n, nsp) = self.expect_u32()?;
            if n == 0 {
                bail!("`after 0 cycles` is invalid at {nsp}; delay must be >= 1");
            }
            if self.at_kw("cycles") || self.at_kw("cycle") {
                self.next();
            } else {
                bail!("expected `cycles` after delay at {}", self.peek().span);
            }
            EnsureTiming::After(n)
        } else if self.at_kw("within") {
            self.next();
            let (lo, lsp) = self.expect_u32()?;
            self.expect(TokenKind::DotDot)?;
            let (hi, hsp) = self.expect_u32()?;
            if lo == 0 {
                bail!("`within 0..{hi}` is invalid at {lsp}; lower bound must be >= 1");
            }
            if lo > hi {
                bail!("`within {lo}..{hi}` has inverted bounds at {hsp}");
            }
            if self.at_kw("cycles") || self.at_kw("cycle") {
                self.next();
            } else {
                bail!("expected `cycles` after window at {}", self.peek().span);
            }
            EnsureTiming::Within(lo, hi)
        } else if self.at_kw("until") {
            self.next();
            let event = self.parse_expr()?;
            EnsureTiming::Until(event)
        } else if self.at_kw("eventually") {
            self.next();
            EnsureTiming::Eventually
        } else {
            bail!(
                "expected `after`, `within`, `until`, or `eventually` in `ensure {label}` at {}",
                self.peek().span
            );
        };
        self.expect(TokenKind::Colon)?;
        let response = self.parse_expr()?;
        self.expect(TokenKind::Semi)?;
        Ok(EnsureClause {
            label,
            timing,
            response,
            span,
        })
    }

    fn parse_use(&mut self) -> Result<UseClause> {
        let span = self.expect_kw("use")?;
        let (protocol, _) = self.expect_ident()?;
        self.expect(TokenKind::LParen)?;
        let mut args = Vec::new();
        if !self.at(&TokenKind::RParen) {
            loop {
                args.push(self.parse_expr()?);
                if self.at(&TokenKind::Comma) {
                    self.next();
                } else {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Semi)?;
        Ok(UseClause {
            protocol,
            args,
            span,
        })
    }

    fn parse_xfree(&mut self) -> Result<XFreeClause> {
        let span = self.expect_kw("xfree")?;
        let mut targets = Vec::new();
        loop {
            let (name, _) = self.expect_ident()?;
            targets.push(name);
            if self.at(&TokenKind::Comma) {
                self.next();
            } else {
                break;
            }
        }
        self.expect(TokenKind::Semi)?;
        Ok(XFreeClause { targets, span })
    }

    fn parse_secure(&mut self) -> Result<SecureClause> {
        let span = self.expect_kw("secure")?;
        let mut sources = Vec::new();
        loop {
            let (name, _) = self.expect_ident()?;
            sources.push(name);
            if self.at(&TokenKind::Comma) {
                self.next();
            } else {
                break;
            }
        }
        self.expect(TokenKind::ThinArrow)?;
        let mut targets = Vec::new();
        loop {
            let (name, _) = self.expect_ident()?;
            targets.push(name);
            if self.at(&TokenKind::Comma) {
                self.next();
            } else {
                break;
            }
        }
        let unless = if self.at_kw("unless") {
            self.next();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(TokenKind::Semi)?;
        Ok(SecureClause {
            sources,
            targets,
            unless,
            span,
        })
    }

    // ---- Expression parser (precedence climbing) ----

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_implies()
    }

    fn parse_implies(&mut self) -> Result<Expr> {
        let lhs = self.parse_ternary()?;
        if self.at(&TokenKind::FatArrow) {
            let span = self.next().span;
            let rhs = self.parse_implies()?;
            Ok(Expr {
                kind: ExprKind::Binary {
                    op: BinOp::Implies,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            })
        } else {
            Ok(lhs)
        }
    }

    fn parse_ternary(&mut self) -> Result<Expr> {
        let cond = self.parse_log_or()?;
        if self.at(&TokenKind::Question) {
            let span = self.next().span;
            let then_expr = self.parse_expr()?;
            self.expect(TokenKind::Colon)?;
            let else_expr = self.parse_ternary()?;
            Ok(Expr {
                kind: ExprKind::Ternary {
                    cond: Box::new(cond),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                },
                span,
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_log_or(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_log_and()?;
        while self.at(&TokenKind::OrOr) {
            let span = self.next().span;
            let rhs = self.parse_log_and()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::LogOr,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_log_and(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_bit_or()?;
        while self.at(&TokenKind::AndAnd) {
            let span = self.next().span;
            let rhs = self.parse_bit_or()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::LogAnd,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bit_or(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_bit_xor()?;
        while self.at(&TokenKind::Pipe) {
            let span = self.next().span;
            let rhs = self.parse_bit_xor()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitOr,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bit_xor(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_bit_and()?;
        while self.at(&TokenKind::Caret) {
            let span = self.next().span;
            let rhs = self.parse_bit_and()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitXor,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bit_and(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_eq()?;
        while self.at(&TokenKind::Amp) {
            let span = self.next().span;
            let rhs = self.parse_eq()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitAnd,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_eq(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_rel()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::NotEq => BinOp::Ne,
                _ => break,
            };
            let span = self.next().span;
            let rhs = self.parse_rel()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_rel(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_shift()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Le => BinOp::Le,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::Ge => BinOp::Ge,
                _ => break,
            };
            let span = self.next().span;
            let rhs = self.parse_shift()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Shl => BinOp::Shl,
                TokenKind::Shr => BinOp::Shr,
                _ => break,
            };
            let span = self.next().span;
            let rhs = self.parse_add()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            let span = self.next().span;
            let rhs = self.parse_mul()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            let span = self.next().span;
            let rhs = self.parse_unary()?;
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        let op = match self.peek().kind {
            TokenKind::Bang => Some(UnOp::LogNot),
            TokenKind::Tilde => Some(UnOp::BitNot),
            TokenKind::Minus => Some(UnOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            let span = self.next().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op,
                    expr: Box::new(expr),
                },
                span,
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        while self.at(&TokenKind::LBracket) {
            let span = self.next().span;
            let (hi, _) = self.expect_u32()?;
            let lo = if self.at(&TokenKind::Colon) {
                self.next();
                self.expect_u32()?.0
            } else {
                hi
            };
            self.expect(TokenKind::RBracket)?;
            if lo > hi {
                bail!("invalid bit slice [{hi}:{lo}] at {span}");
            }
            expr = Expr {
                kind: ExprKind::Slice {
                    expr: Box::new(expr),
                    hi,
                    lo,
                },
                span,
            };
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let tok = self.next();
        match tok.kind {
            TokenKind::Int(v) => Ok(Expr {
                kind: ExprKind::Int(v),
                span: tok.span,
            }),
            TokenKind::SizedLit {
                width,
                signed,
                bits,
            } => Ok(Expr {
                kind: ExprKind::SizedLit {
                    width,
                    signed,
                    bits,
                },
                span: tok.span,
            }),
            TokenKind::Ident(name) => {
                if name == "true" {
                    return Ok(Expr {
                        kind: ExprKind::Bool(true),
                        span: tok.span,
                    });
                }
                if name == "false" {
                    return Ok(Expr {
                        kind: ExprKind::Bool(false),
                        span: tok.span,
                    });
                }
                if self.at(&TokenKind::LParen) {
                    self.next();
                    let mut args = Vec::new();
                    if !self.at(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.at(&TokenKind::Comma) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    Ok(Expr {
                        kind: ExprKind::Call { name, args },
                        span: tok.span,
                    })
                } else {
                    Ok(Expr {
                        kind: ExprKind::Ident(name),
                        span: tok.span,
                    })
                }
            }
            TokenKind::LParen => {
                let inner = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(inner)
            }
            TokenKind::LBrace => {
                let mut parts = Vec::new();
                if !self.at(&TokenKind::RBrace) {
                    loop {
                        parts.push(self.parse_expr()?);
                        if self.at(&TokenKind::Comma) {
                            self.next();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBrace)?;
                if parts.is_empty() {
                    bail!("empty concatenation `{{}}` at {}", tok.span);
                }
                Ok(Expr {
                    kind: ExprKind::Concat(parts),
                    span: tok.span,
                })
            }
            other => bail!("unexpected token {:?} in expression at {}", other, tok.span),
        }
    }
}
