# VeloFormal Contract Language (`.vfc`) Specification — Version 1.0

**Status**: Stable (Implemented in [`crates/vf-dsl`](../crates/vf-dsl/src/lib.rs))  
**File Extension**: `.vfc`  
**Encoding**: UTF-8, LF line endings  

---

## 1. Design Philosophy & Scope

The **VeloFormal Contract Language (`.vfc`)** is a statically typed, brace-delimited formal specification language for synchronous Verilog and SystemVerilog RTL. It is designed around five principles:

1. **Non-Invasive by Default**: Production RTL requires zero `` `ifdef FORMAL `` instrumentation. Contracts bind externally to an elaborated module (`contract Name for rtl_module { ... }`).
2. **Strict Width & Signedness Safety**: Silent Verilog bit-truncation and accidental signed/unsigned mixing are compile-time errors.
3. **Reset-Safe Temporal Ergonomics**: 1-cycle history operators (`prev`, `rose`, `fell`, `stable`, `changed`) and multi-cycle operations (`after`, `within`, `until`, `at_trigger`) automatically manage cycle-0 / post-reset validity guards without manual `f_past_valid` boilerplate.
4. **Unified Multi-Domain Verification**: Safety invariants, liveness/fairness, ghost models, symbolic `$anyconst`/`$anyseq` variables, reusable protocol templates (`use`), 4-state X-propagation (`xfree`), and 2-copy Information Flow Security (`secure`) coexist in one readable file.
5. **Zero Lock-In**: Every safety/temporal/cover contract can be exported to portable IEEE 1800 SystemVerilog SVA `bind` files via `veloformal emit-sva`.

---

## 2. Lexical Structure

### 2.1 Whitespace & Comments
- Whitespace (`' '`, `'\t'`, `'\r'`, `'\n'`) separates tokens and is otherwise insignificant.
- Line comments begin with `//` or `#` and extend to `\n`.
- Block comments begin with `/*` and end with `*/` (non-nesting; unterminated block comments are syntax errors).

### 2.2 Identifiers
```ebnf
ident       ::= ident_start ident_cont* ('.' ident_start ident_cont*)* ;
ident_start ::= 'a'..'z' | 'A'..'Z' | '_' ;
ident_cont  ::= ident_start | '0'..'9' ;
```
Dotted paths (e.g., `u_alu.result`) reference hierarchical internal signals flattened by Slang. Note that `..` is tokenized as the range operator `DotDot`, never part of an identifier.

### 2.3 Literals
1. **Boolean literals**: `true`, `false` (type: `bool`, width `1`, unsigned).
2. **Unsized integer literals**:
   - Decimal: `[0-9][0-9_]*` (e.g., `0`, `42`, `1_000`)
   - Hexadecimal: `0x[0-9a-fA-F_]+` / `0X...`
   - Binary: `0b[01_]+` / `0B...`
   - Unsized integer literals adapt to the width and signedness of a paired operand if and only if their numeric value fits without truncation (Section 4.3).
3. **Sized Verilog-style literals**:
   ```ebnf
   sized_lit ::= dec_digits '\'' ('s' | 'S')? ('b'|'B'|'d'|'D'|'h'|'H') lit_digits ;
   ```
   Examples: `8'd1`, `4'hF`, `8'b1010_0101`, `16'sd5`. Width must be in `1..=256`.

### 2.4 Keywords
Contextual/reserved keywords:
`contract`, `for`, `parameter`, `int`, `where`, `clock`, `posedge`, `negedge`, `reset`, `when`, `symbolic`, `anyconst`, `anyseq`, `ghost`, `logic`, `bool`, `signed`, `update`, `if`, `else`, `require`, `invariant`, `operation`, `ensure`, `after`, `within`, `cycles`, `cycle`, `until`, `eventually`, `fair`, `cover`, `use`, `xfree`, `secure`, `unless`, `true`, `false`.

### 2.5 Operators & Precedence (Lowest to Highest)

| Prec | Operator(s) | Associativity | Category |
|---:|---|---|---|
| 1 | `=>` | Right | Logical implication (`!a \|\| b`) |
| 2 | `? :` | Right | Ternary conditional |
| 3 | `\|\|` | Left | Logical OR |
| 4 | `&&` | Left | Logical AND |
| 5 | `\|` | Left | Bitwise OR |
| 6 | `^` | Left | Bitwise XOR |
| 7 | `&` | Left | Bitwise AND |
| 8 | `==`, `!=` | Left | Equality / inequality |
| 9 | `<`, `<=`, `>`, `>=` | Left | Relational comparison |
| 10 | `<<`, `>>` | Left | Left shift / right shift (arithmetic if signed, logical if unsigned) |
| 11 | `+`, `-` | Left | Addition / subtraction |
| 12 | `*`, `/`, `%` | Left | Multiplication / division / remainder |
| 13 | `!`, `~`, `-` | Right (prefix) | Logical NOT, bitwise NOT, arithmetic negation |
| 14 | `e[hi:lo]`, `e[bit]`, `fn(args...)`, `{a, b, ...}`, `(e)` | Postfix / primary | Bit slice, function call, concatenation, grouping |

---

## 3. Complete Formal EBNF Grammar

```ebnf
contract        ::= "contract" ident "for" ident "{" contract_item* "}" ;

contract_item   ::= param_decl
                  | clock_decl
                  | reset_decl
                  | symbolic_decl
                  | ghost_decl
                  | update_block
                  | require_decl
                  | invariant_decl
                  | operation_decl
                  | fair_decl
                  | eventually_decl
                  | cover_decl
                  | use_clause
                  | xfree_clause
                  | secure_clause ;

param_decl      ::= "parameter" ident (":" "int")? ("where" expr)? ";" ;
clock_decl      ::= "clock" ("posedge" | "negedge") ident ";" ;
reset_decl      ::= "reset" "when"? expr ";" ;

logic_type      ::= "bool"
                  | "logic" ("[" u32 "]")? "signed"? ;

symbolic_decl   ::= "symbolic" ("anyconst" | "anyseq")? ident ":" logic_type ";" ;
ghost_decl      ::= "ghost" ident ":" logic_type ("=" expr)? ";" ;

update_block    ::= "update" "{" update_stmt* "}" ;
update_stmt     ::= ident "<=" expr ";"
                  | "if" "(" expr ")" "{" update_stmt* "}"
                    ("else" (update_if | "{" update_stmt* "}"))? ;

require_decl    ::= "require" ident ":" expr ";" ;
invariant_decl  ::= "invariant" ident ":" expr ";" ;
fair_decl       ::= "fair" ident ":" expr ";" ;
eventually_decl ::= "eventually" ident ":" expr ";" ;
cover_decl      ::= "cover" ident ":" expr ";" ;

operation_decl  ::= "operation" ident "when" expr "{" ensure_clause+ "}" ;
ensure_clause   ::= "ensure" ident ensure_timing ":" expr ";" ;
ensure_timing   ::= "after" u32 ("cycles" | "cycle")?
                  | "within" u32 ".." u32 ("cycles" | "cycle")?
                  | "until" expr
                  | "eventually" ;

use_clause      ::= "use" ident "(" (expr ("," expr)*)? ")" ";" ;
xfree_clause    ::= "xfree" ident ("," ident)* ";" ;
secure_clause   ::= "secure" ident ("," ident)* "->" ident ("," ident)* ("unless" expr)? ";" ;
```

---

## 4. Static Type System & Width / Signedness Rules

Every lowered expression carries a static type `(width: u32, signed: bool, unsized_val: Option<u64>)`.

### 4.1 Signal & Declaration Typing
- **RTL Signals**: Resolved directly against the Slang-elaborated target module (`target_module`). Width and signedness are inherited verbatim from IEEE 1800 elaboration. Referencing an unknown signal is a compile-time error.
- **`parameter`**: Evaluated as compile-time signed 64-bit integers supplied via `-G NAME=VAL` (or MCP `params`). Any `where` constraint must evaluate to `true` at elaboration time.
- **`bool`**: Equivalent to unsigned 1-bit `logic[1]`.
- **`logic[W]` / `logic[W] signed`**: Width `W` must satisfy `1 <= W <= 256`.

### 4.2 Strict Operand Matching Rules
For binary arithmetic (`+`, `-`, `*`, `/`, `%`), bitwise (`&`, `|`, `^`), equality (`==`, `!=`), relational (`<`, `<=`, `>`, `>=`), and ternary branches (`c ? a : b`):

1. **No Mixed Signedness**: If both operands are sized and `left.signed != right.signed`, lowering fails with a **mixed signed and unsigned** error. Use explicit `signed(e)` or `unsigned(e)`.
2. **No Silent Width Mismatch**: If both operands are sized and `left.width != right.width`, lowering fails with a **width mismatch** error. Use explicit `resize(e, W)` or slicing `e[hi:lo]`.
3. **Unsized Literal / Parameter Adaptation**: If one operand is an unsized integer literal or parameter with value `v` and the other is sized `(W, signed)`:
   - For unsigned `W`: `v` is accepted iff `v < 2^W` (otherwise an overflow/range error is reported).
   - For signed `W`: `v` is accepted iff `v < 2^(W-1)`.
   - The unsized operand is widened/narrowed cleanly to `(W, signed)`.

### 4.3 Bit Slices & Concatenation
- `e[hi:lo]` requires `hi >= lo` and `hi < width(e)`. Result type: unsigned `logic[hi - lo + 1]`.
- `e[bit]` is shorthand for `e[bit:bit]`. Result type: `bool`.
- `{e_0, e_1, ..., e_k}` concatenates MSB-first; result width is $\sum \text{width}(e_i)$, unsigned.

---

## 5. Clock, Reset & Automatic History Guards

### 5.1 Active Guard (`active`)
If a contract declares `reset when R;`, VeloFormal defines the active (non-reset) enable signal:
$$\text{active}_t \equiv \neg R_t$$
If no `reset` clause is declared, $\text{active}_t \equiv \text{true}$.

### 5.2 Automatic Cycle-0 & Post-Reset History Validity (`hist_valid`)
Whenever any expression calls `prev(e)`, `rose(s)`, `fell(s)`, `stable(s)`, or `changed(s)`, VeloFormal automatically synthesizes a 1-bit register `$vfc_hist_valid`:
- Initial value at $t = 0$: `0`
- Next-state: $\text{active}_t$ (i.e. `0` on the cycle immediately following reset, `1` once at least one active clock cycle has elapsed)

Every assertion whose expression invokes a history operator is automatically conjoined with `hist_valid` in its enable guard — eliminating spurious cycle-0 `$past(...)` failures without manual `f_past_valid` boilerplate.

---

## 6. Formal Operational & Temporal Semantics

Let $s_0, s_1, s_2, \dots$ be an execution trace of the synchronous transition system.

### 6.1 `require` and `invariant`
Under `--role provider` (default):
- `require label: P;` emits an **Assumption** active when $\text{active}_t$ holds:
  $$\forall t \ge 0.\; \text{active}_t \implies P_t$$
- `invariant label: P;` emits a **Safety Assertion** active when $\text{active}_t$ (and $\text{hist\_valid}_t$ if history operators are used) holds:
  $$\forall t \ge 0.\; \text{active}_t \implies P_t$$

### 6.2 `symbolic` Declarations
- `symbolic anyconst x: T;` creates an unconstrained symbolic constant (`SignalKind::AnyConst`) satisfying $x_{t+1} = x_t$ for all $t \ge 0$.
- `symbolic anyseq x: T;` creates a fresh unconstrained symbolic value (`SignalKind::AnySeq`) every cycle.

### 6.3 `ghost` Registers & `update` Blocks
- `ghost g: T = init_expr;` introduces a verification-only register `g` (invisible to RTL) initialized to `init_expr` at $t = 0$ and re-initialized to `init_expr` whenever reset $R_t$ is asserted.
- `update { ... }` blocks define deterministic next-state mux trees for ghost registers using non-blocking assignments (`g <= expr;`) and nested `if (...) { ... } else { ... }` conditionals. If no assignment fires in a cycle, `g` holds its current value.
- Assigning to an RTL signal (non-ghost) inside `update` is a compile-time error.

### 6.4 `operation` & Temporal `ensure` Clauses
Given `operation Op when T { ensure L <timing>: Q; }`, let the effective trigger be $\tau_t \equiv \text{active}_t \land T_t$.
Any `at_trigger(e)` sub-expression inside `Q` is captured into pipeline registers at cycle $t$ when $\tau_t$ fires and shifted forward alongside the obligation so `Q` compares current state against the exact trigger-cycle snapshot.

1. **`after N cycles`** ($N \ge 0$):
   - Delays $\tau$ (and `at_trigger` captures) by $N$ resettable register stages (reset aborts in-flight obligations!).
   - Asserts at cycle $t + N$ (provided $\text{active}_{t+k}$ held for all $k \in \{0,\dots,N\}$): $Q_{t+N}$.
2. **`within A..B cycles`** ($1 \le A \le B$):
   - Maintains two $B$-stage resettable chains: delayed trigger $\tau^{(B)}$ and cumulative satisfaction $\text{met}^{(slot)} = \text{met}^{(slot-1)} \lor Q$ for $slot \in [A, B]$.
   - At deadline cycle $t + B$ (if no reset occurred): asserts $\text{active}_{t+B} \land \tau^{(B)} \implies \text{met}^{(B)}$.
   - Keeping $\tau^{(B)}$ separate from $\text{met}$ guarantees that early completion (e.g., finishing at cycle $A$ when $A < B$) is never misclassified as vacuous.
3. **`until E`**:
   - Tracks an active-until latch `act` set when $\tau_t \land \neg E_t$ and cleared upon reset or $E_t$.
   - Asserts on every cycle while active (before $E$ holds): $Q_t$.
4. **`eventually`**:
   - Tracks a pending latch `pend` set when $\tau_t \land \neg Q_t$ and cleared upon reset or $Q_t$.
   - Emits a liveness obligation (`PropertyKind::Live`) requiring $\neg \text{pend}$ infinitely often on all fair paths.

### 6.5 Liveness, Fairness & Cover
- `fair label: F;` emits a fairness constraint (`PropertyKind::Fair`, $\mathbf{GF}\; F$).
- `eventually label: P;` emits a liveness assertion (`PropertyKind::Live`, $\mathbf{GF}\; (\text{active} \implies P)$).
- `cover label: C;` emits a reachability witness goal (`PropertyKind::Cover`, $\exists t.\; \text{active}_t \land C_t$).

---

## 7. Assume-Guarantee Roles & Compositional Proof Certificates

VeloFormal supports dual-role contract verification (`veloformal contract --role <provider|caller>`):

| Clause | `--role provider` (default — verify DUT) | `--role caller` (verify environment/client) |
|---|---|---|
| `require` | **Assume** | **Assert** (proves caller obeys preconditions) |
| `invariant`, `ensure`, `use` | **Assert** | **Assume** (caller may rely on proved guarantees) |

When `--emit-cert <file.vfcert.json>` is passed on an unbounded proof, VeloFormal writes a SHA-256 integrity-hashed `ProofCertificate` binding the RTL IR hash, contract hash, engine winner, and timestamp; `--verify-cert` validates the certificate before compositional caller proofs.

---

## 8. Built-In Protocol Templates (`use`)

| Template | Arguments | Properties Synthesized |
|---|---|---|
| `handshake` | `(valid, ready, data)` | Stability under backpressure: `prev(valid && !ready) => (valid && stable(data))`; plus cover `valid && ready`. |
| `apb` | `(psel, penable, pwrite, paddr, pwdata, pready, prdata)` | `penable => psel`; SETUP→ACCESS progression; control/address/write-data stability across wait states (`!pready`); plus read & write covers. |
| `fifo_scoreboard` | `(wr_en, wr_data, rd_en, rd_data, full, empty, DEPTH)` | Synthesizes a shadow `$anyconst` symbolic slot/data tracker + shadow pointers verifying `empty`/`full` flags and exact FIFO ordering & data integrity on pop. |

---

## 9. Multi-Domain Verification Clauses

### 9.1 4-State X-Propagation (`xfree`)
```vfc
xfree sig_1, sig_2, ...;
```
Automatically invokes VeloFormal's dual-rail 4-state X-Check engine (`(val, is_x)`), proving unboundedly that no uninitialized flip-flop or X-source can propagate an unknown `X` value to any listed target signal (respecting controlling values such as `0 & X == 0`, `1 | X == 1`, and mux selection).

### 9.2 2-Copy Information Flow Security (`secure`)
```vfc
secure src_1, ... -> dst_1, ... [unless declassify_cond];
```
Automatically constructs a 2-copy self-composition miter (`copy_a` vs `copy_b`) tying all inputs except `src_*` equal across both executions, and proves unboundedly that `dst_a == dst_b` on every cycle where `declassify_cond` is false.

---

## 10. Built-In Function Reference

| Function | Signature | Semantics |
|---|---|---|
| `unsigned(e)` | `(T) -> logic[W]` | Reinterprets `e` as unsigned without changing bits |
| `signed(e)` | `(T) -> logic[W] signed` | Reinterprets `e` as two's-complement signed without changing bits |
| `resize(e, W)` | `(T, const u32) -> logic[W] [signed]` | Zero-extends (unsigned) or sign-extends (signed) or truncates to width `W` |
| `ite(c, a, b)` | `(bool, T, T) -> T` | Functional conditional multiplexer |
| `prev(e)` | `(T) -> T` | Value of `e` on the previous clock cycle (auto-adds `hist_valid`) |
| `rose(s)` | `(bool) -> bool` | `!prev(s) && s` (rising edge; auto-adds `hist_valid`) |
| `fell(s)` | `(bool) -> bool` | `prev(s) && !s` (falling edge; auto-adds `hist_valid`) |
| `stable(e)` | `(T) -> bool` | `e == prev(e)` (auto-adds `hist_valid`) |
| `changed(e)` | `(T) -> bool` | `e != prev(e)` (auto-adds `hist_valid`) |
| `at_trigger(e)` | `(T) -> T` | Snapshot of `e` at the triggering cycle of an `operation` (only valid inside `ensure`) |
| `onehot(e)` | `(logic[W]) -> bool` | True iff exactly 1 bit of `e` is `1` |
| `onehot0(e)` | `(logic[W]) -> bool` | True iff at most 1 bit of `e` is `1` |
| `countones(e)` | `(logic[W]) -> logic[ceil(log2(W+1))]` | Population count (Hamming weight) of `e` |

---

## 11. Portable IEEE 1800 SVA Translation (`veloformal emit-sva`)

`veloformal emit-sva <spec.vfc> [--rtl <dut.sv>] [-o out.sv]` translates a `.vfc` contract into a standalone SystemVerilog checker module plus `bind <target_module> <ContractName>_sva ...`:
- Synthesizes `$vfc_hist_valid` (`f_past_valid`) when history operators are used.
- Maps `require` → `assume property`, `invariant` → `assert property`, `cover` → `cover property`.
- Maps `operation` timings: `after N` → `|-> ##N`, `within A..B` → `|-> ##[A:B]`, `until E` → `|=> (Q until E)`, `eventually` → `|-> s_eventually Q`.
- Translates `ghost` + `update` into reset-guarded `always_ff` registers inside the bound checker module.
- Emits structured comments noting any `xfree`/`secure` clauses (which are non-expressible in IEEE 1800 SVA and require VeloFormal or vendor Formal Apps).

---

## 12. Canonical Formatting & Conformance

- **Formatter**: `veloformal fmt <file.vfc> [--check | -i]` produces canonical 4-space indentation, LF line endings, normalized spacing, and deterministic item formatting (idempotent: `fmt(fmt(x)) == fmt(x)`).
- **Conformance Suite**: Verified continuously in [`crates/vf-cli/tests/dsl_and_standards.rs`](../crates/vf-cli/tests/dsl_and_standards.rs) (covering unknown signals, mixed signed/unsigned rejection, width-mismatch rejection, `where` parameter constraints, ghost-only mutation, `at_trigger` scoping, formatter idempotence, and `emit-sva` Slang round-trip proofs).
