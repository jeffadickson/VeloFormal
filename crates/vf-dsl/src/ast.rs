use crate::lexer::Span;

#[derive(Debug, Clone)]
pub struct Contract {
    pub name: String,
    pub target_module: String,
    pub span: Span,
    pub params: Vec<ParamDecl>,
    pub clock: Option<ClockDecl>,
    pub reset: Option<ResetDecl>,
    pub symbolics: Vec<SymbolicDecl>,
    pub ghosts: Vec<GhostDecl>,
    pub updates: Vec<UpdateBlock>,
    pub requires: Vec<LabeledExpr>,
    pub invariants: Vec<LabeledExpr>,
    pub operations: Vec<Operation>,
    pub fairs: Vec<LabeledExpr>,
    pub eventuallys: Vec<LabeledExpr>,
    pub covers: Vec<LabeledExpr>,
    pub uses: Vec<UseClause>,
    pub xfrees: Vec<XFreeClause>,
    pub secures: Vec<SecureClause>,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub name: String,
    pub constraint: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    PosEdge,
    NegEdge,
}

#[derive(Debug, Clone)]
pub struct ClockDecl {
    pub edge: EdgeKind,
    pub signal: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ResetDecl {
    pub cond: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolicKind {
    AnyConst,
    AnySeq,
}

#[derive(Debug, Clone)]
pub struct SymbolicDecl {
    pub name: String,
    pub width: u32,
    pub signed: bool,
    pub kind: SymbolicKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct GhostDecl {
    pub name: String,
    pub width: u32,
    pub signed: bool,
    pub init: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct UpdateBlock {
    pub stmts: Vec<UpdateStmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum UpdateStmt {
    Assign {
        target: String,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_body: Vec<UpdateStmt>,
        else_body: Vec<UpdateStmt>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct LabeledExpr {
    pub label: String,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub name: String,
    pub trigger: Expr,
    pub ensures: Vec<EnsureClause>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum EnsureTiming {
    After(u32),
    Within(u32, u32),
    Until(Expr),
    Eventually,
}

#[derive(Debug, Clone)]
pub struct EnsureClause {
    pub label: String,
    pub timing: EnsureTiming,
    pub response: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct UseClause {
    pub protocol: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct XFreeClause {
    pub targets: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SecureClause {
    pub sources: Vec<String>,
    pub targets: Vec<String>,
    pub unless: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    LogNot,
    BitNot,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Implies,
    LogOr,
    LogAnd,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Bool(bool),
    Int(u64),
    SizedLit {
        width: u32,
        signed: bool,
        bits: String,
    },
    Ident(String),
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Slice {
        expr: Box<Expr>,
        hi: u32,
        lo: u32,
    },
    Concat(Vec<Expr>),
}
