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
    #[serde(default)]
    pub attrs: Vec<String>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stmt {
    Empty,
    Block {
        label: Option<String>,
        items: Vec<Stmt>,
    },
    Timed {
        timing: Box<TimingJson>,
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
        #[serde(default)]
        check: String,
        #[serde(default)]
        loc: String,
        cond: Expr,
        then: Box<Stmt>,
        #[serde(rename = "else")]
        else_branch: Option<Box<Stmt>>,
    },
    Case {
        #[serde(default)]
        check: String,
        #[serde(default)]
        loc: String,
        #[serde(default = "default_case_kind")]
        case_kind: String,
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
        loop_var: Option<String>,
        init: Option<Expr>,
        stop: Option<Expr>,
        step: Option<Expr>,
        body: Option<Box<Stmt>>,
    },
    RepeatLoop {
        count: u64,
        body: Box<Stmt>,
    },
    Return {
        expr: Option<Expr>,
    },
    Unsupported {
        #[serde(default)]
        loc: String,
    },
}

fn default_case_kind() -> String {
    "normal".to_string()
}

/// Cloneable timing enum for `Stmt::Timed`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimingJson {
    Implicit,
    SignalEvent {
        edge: String,
        expr: Expr,
    },
    EventList {
        events: Vec<TimingJson>,
    },
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CaseItem {
    pub exprs: Vec<Expr>,
    pub body: Stmt,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeqElement {
    pub delay_min: u64,
    pub delay_max: Option<u64>,
    pub expr: AssertionExpr,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeqRepetition {
    pub kind: String,
    pub min: u64,
    pub max: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssertionExpr {
    Simple {
        expr: Expr,
        #[serde(default)]
        repetition: Option<SeqRepetition>,
    },
    SeqWithMatch {
        expr: Box<AssertionExpr>,
        #[serde(default)]
        repetition: Option<SeqRepetition>,
    },
    FirstMatch {
        expr: Box<AssertionExpr>,
    },
    Conditional {
        cond: Expr,
        if_expr: Box<AssertionExpr>,
        #[serde(default)]
        else_expr: Option<Box<AssertionExpr>>,
    },
    Clocking {
        clocking: TimingJson,
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
    SeqConcat {
        elements: Vec<SeqElement>,
    },
    StrongWeak {
        expr: Box<AssertionExpr>,
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
        #[serde(default)]
        mask_z: Option<String>,
        #[serde(default)]
        mask_xz: Option<String>,
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
    Inside {
        width: u32,
        signed: bool,
        left: Box<Expr>,
        ranges: Vec<Expr>,
    },
    ValueRange {
        width: u32,
        signed: bool,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Streaming {
        width: u32,
        signed: bool,
        slice_size: u64,
        operands: Vec<Expr>,
    },
    AssignExpr {
        width: u32,
        signed: bool,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
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
    FieldSelect {
        width: u32,
        signed: bool,
        bit_offset: u64,
        value: Box<Expr>,
    },
    FuncCall {
        name: String,
        width: u32,
        signed: bool,
        params: Vec<String>,
        args: Vec<Expr>,
        body: Box<Stmt>,
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
            | Expr::Inside { width, .. }
            | Expr::ValueRange { width, .. }
            | Expr::Streaming { width, .. }
            | Expr::AssignExpr { width, .. }
            | Expr::Ternary { width, .. }
            | Expr::Concat { width, .. }
            | Expr::Replicate { width, .. }
            | Expr::BitSelect { width, .. }
            | Expr::MemSelect { width, .. }
            | Expr::RangeSelect { width, .. }
            | Expr::FieldSelect { width, .. }
            | Expr::FuncCall { width, .. }
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
            | Expr::Inside { signed, .. }
            | Expr::ValueRange { signed, .. }
            | Expr::Streaming { signed, .. }
            | Expr::AssignExpr { signed, .. }
            | Expr::Ternary { signed, .. }
            | Expr::Concat { signed, .. }
            | Expr::Replicate { signed, .. }
            | Expr::BitSelect { signed, .. }
            | Expr::MemSelect { signed, .. }
            | Expr::RangeSelect { signed, .. }
            | Expr::FieldSelect { signed, .. }
            | Expr::FuncCall { signed, .. }
            | Expr::Call { signed, .. }
            | Expr::Unsupported { signed, .. } => *signed,
        }
    }
}
