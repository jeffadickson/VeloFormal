use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ElaboratedDesign {
    pub module: String,
    pub signals: Vec<SignalDecl>,
    pub memories: Vec<MemDecl>,
    pub assigns: Vec<ContAssign>,
    pub processes: Vec<ProcessDecl>,
}

#[derive(Debug, Deserialize)]
pub struct SignalDecl {
    pub name: String,
    pub width: u32,
    pub signed: bool,
    pub direction: String,
    pub init: Option<Expr>,
}

#[derive(Debug, Deserialize)]
pub struct MemDecl {
    pub name: String,
    pub data_width: u32,
    pub depth: u32,
    pub lo_index: i32,
}

#[derive(Debug, Deserialize)]
pub struct ContAssign {
    pub lhs: Expr,
    pub rhs: Expr,
    #[serde(default)]
    pub loc: String,
}

#[derive(Debug, Deserialize)]
pub struct ProcessDecl {
    pub kind: String,
    pub body: Stmt,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timing {
    Implicit,
    SignalEvent {
        edge: String,
        expr: Expr,
    },
    EventList {
        events: Vec<Timing>,
    },
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stmt {
    Empty,
    Block {
        label: Option<String>,
        items: Vec<Stmt>,
    },
    Timed {
        timing: Timing,
        body: Box<Stmt>,
    },
    Assign {
        lhs: Expr,
        rhs: Expr,
    },
    Nba {
        lhs: Expr,
        rhs: Expr,
    },
    ExprStmt {
        expr: Expr,
    },
    If {
        cond: Expr,
        then: Box<Stmt>,
        #[serde(rename = "else")]
        else_branch: Option<Box<Stmt>>,
    },
    Case {
        expr: Expr,
        items: Vec<CaseItem>,
        default: Option<Box<Stmt>>,
    },
    ImmediateAssert {
        assert_kind: String,
        cond: Expr,
        #[serde(default)]
        loc: String,
    },
    ConcurrentAssert {
        assert_kind: String,
        prop: AssertionExpr,
        #[serde(default)]
        loc: String,
    },
    ForLoop {
        #[serde(default)]
        loc: String,
    },
    Unsupported {
        #[serde(default)]
        loc: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct CaseItem {
    pub exprs: Vec<Expr>,
    pub body: Stmt,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssertionExpr {
    Simple {
        expr: Expr,
    },
    Clocking {
        clocking: Timing,
        expr: Box<AssertionExpr>,
    },
    DisableIff {
        cond: Expr,
        expr: Box<AssertionExpr>,
    },
    Unary {
        op: String,
        expr: Box<AssertionExpr>,
    },
    Binary {
        op: String,
        left: Box<AssertionExpr>,
        right: Box<AssertionExpr>,
    },
    Unsupported,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    Const {
        width: u32,
        signed: bool,
        bits: String,
    },
    Ref {
        name: String,
        width: u32,
        signed: bool,
    },
    Conv {
        width: u32,
        signed: bool,
        operand: Box<Expr>,
    },
    Unary {
        op: String,
        width: u32,
        signed: bool,
        operand: Box<Expr>,
    },
    Binary {
        op: String,
        width: u32,
        signed: bool,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Ternary {
        width: u32,
        signed: bool,
        cond: Box<Expr>,
        then: Box<Expr>,
        #[serde(rename = "else")]
        else_expr: Box<Expr>,
    },
    Concat {
        width: u32,
        signed: bool,
        operands: Vec<Expr>,
    },
    Replicate {
        width: u32,
        signed: bool,
        count: u64,
        concat: Box<Expr>,
    },
    BitSelect {
        width: u32,
        signed: bool,
        value: Box<Expr>,
        selector: Box<Expr>,
        range_left: Option<i64>,
        range_right: Option<i64>,
    },
    MemSelect {
        width: u32,
        signed: bool,
        value: Box<Expr>,
        selector: Box<Expr>,
    },
    RangeSelect {
        width: u32,
        signed: bool,
        value: Box<Expr>,
        sel_kind: String,
        left_const: Option<i64>,
        right_const: Option<i64>,
        left: Option<Box<Expr>>,
        right: Option<Box<Expr>>,
        range_left: Option<i64>,
        range_right: Option<i64>,
    },
    Call {
        name: String,
        width: u32,
        signed: bool,
        args: Vec<Expr>,
    },
    Unsupported {
        width: u32,
        signed: bool,
    },
}

impl Expr {
    pub fn width(&self) -> u32 {
        match self {
            Expr::Const { width, .. }
            | Expr::Ref { width, .. }
            | Expr::Conv { width, .. }
            | Expr::Unary { width, .. }
            | Expr::Binary { width, .. }
            | Expr::Ternary { width, .. }
            | Expr::Concat { width, .. }
            | Expr::Replicate { width, .. }
            | Expr::BitSelect { width, .. }
            | Expr::MemSelect { width, .. }
            | Expr::RangeSelect { width, .. }
            | Expr::Call { width, .. }
            | Expr::Unsupported { width, .. } => *width,
        }
    }

    pub fn is_signed(&self) -> bool {
        match self {
            Expr::Const { signed, .. }
            | Expr::Ref { signed, .. }
            | Expr::Conv { signed, .. }
            | Expr::Unary { signed, .. }
            | Expr::Binary { signed, .. }
            | Expr::Ternary { signed, .. }
            | Expr::Concat { signed, .. }
            | Expr::Replicate { signed, .. }
            | Expr::BitSelect { signed, .. }
            | Expr::MemSelect { signed, .. }
            | Expr::RangeSelect { signed, .. }
            | Expr::Call { signed, .. }
            | Expr::Unsupported { signed, .. } => *signed,
        }
    }
}
