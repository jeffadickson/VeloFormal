# Chapter 13: The Real-World Silicon Gauntlet — OpenTitan, TileLink-UL & Modern SystemVerilog

Toy formal verifiers work on textbook Verilog-2001 counters and immediately choke on real open-source silicon like **OpenTitan**, **Ibex**, **CVA6**, or **PULP**. Production SystemVerilog makes heavy use of IEEE 1800-2017/2023 constructs:

- Shared `package` `typedef struct packed` and `typedef enum logic` definitions
- Multi-dimensional packed arrays (`logic [DEPTH-1:0][WIDTH-1:0] storage`)
- Compound assignments (`cnt += 1'b1`, `ptr++`) mixed with non-blocking resets (`<=`)
- Assignment patterns (`'{default: '0}`, `'{valid: 1'b1, data: d}`)
- Instance arrays (`prim_cell u_cell [N]`)
- Bit/byte streaming operators (`{<<1{vec}}`, `{>>{a, b}}`)
- `case (expr) inside` with `[lo:hi]` value ranges
- `interface` + `modport` bus bundles passed across module hierarchies

Because VeloFormal embeds **Slang v9.1** directly inside `crates/vf-slang` and lowers elaborated ASTs into word-level `vf-ir`, every one of these constructs compiles natively — with **zero preprocessing hacks** and **accurate bit-level AutoCheck bounds**.

---

## 1. Case Study A: OpenTitan `prim_fifo_sync` Pattern

`examples/23_silicon_gauntlet/ot_prim_fifo_sync.sv` exercises:
- `package ot_fifo_pkg` with `typedef struct packed` (`fifo_entry_t`) and `typedef enum logic`
- 2D packed storage array `logic [DEPTH-1:0][ENTRY_W-1:0] mem`
- Assignment pattern reset `mem <= '{default: '0};`
- Compound arithmetic `cnt += 3'd1;` / `cnt -= 3'd1;` inside the same `always_ff` as `cnt <= 3'd0;`

Notice a subtle IEEE 1800 detail: when `cnt <= 3'd0` appears in the `if (rst_ni == 1'b0)` branch and `cnt += 3'd1` appears in the `else` branch, `vf-slang` properly merges both blocking and non-blocking updates onto the flip-flop's next-state mux, and computes multi-dimensional packed array bounds as `num_elems = total_width / elem_width` so AutoCheck's `oob_index` verifies exact entry bounds rather than raw bit width!

```bash
veloformal autocheck examples/23_silicon_gauntlet/ot_prim_fifo_sync.sv --strict
veloformal contract  examples/23_silicon_gauntlet/ot_prim_fifo_sync.sv \
  --spec examples/23_silicon_gauntlet/ot_prim_fifo_sync.vfc --vacuity
```

---

## 2. Case Study B: OpenTitan `prim_arbiter_ppc` Pattern

`examples/23_silicon_gauntlet/ot_prim_arbiter_ppc.sv` exercises:
- **Instance Arrays**: `arb_lane u_lanes [N]` flattened cleanly into distinct hierarchical IR instances (`u_lanes[0]`, `u_lanes[1]`, ...)
- **Streaming Reversal**: `{<<1{req_i}}` for reverse-priority masking
- **`case (...) inside` Value Ranges**: `4'b0000`, `[4'd1:4'd3]`, `[4'd4:4'd15]` lowered to exact range-comparison predicates

Its contract (`ot_prim_arbiter_ppc.vfc`) proves:
- **One-hot grant mutual exclusion** (`(gnt_o & (gnt_o - 1)) == 0`)
- **No spurious grants** (`(gnt_o & ~req_i) == 0`)
- **Work conservation**: whenever any request is active (`req_i != 0`), at least one grant is issued (`gnt_o != 0`).

```bash
veloformal contract examples/23_silicon_gauntlet/ot_prim_arbiter_ppc.sv \
  --spec examples/23_silicon_gauntlet/ot_prim_arbiter_ppc.vfc --vacuity
```

---

## 3. Case Study C: TileLink-UL `interface` & `modport` Crossbar

`examples/23_silicon_gauntlet/tlul_if_crossbar.sv` models a real SoC interconnect using SystemVerilog `interface tlul_if` with `modport host` and `modport dev` port bundles connected across a 2-port crossbar.

`vf-slang` automatically unwraps Slang's `ModportPortSymbol` bindings back to the canonical underlying interface signals — allowing both zero-SVA AutoCheck and `.vfc` contracts to verify bus handshakes, address decoding, and deadlock-freedom directly across `interface` hierarchies:

```bash
veloformal autocheck examples/23_silicon_gauntlet/tlul_if_crossbar.sv --strict
veloformal contract  examples/23_silicon_gauntlet/tlul_if_crossbar.sv \
  --spec examples/23_silicon_gauntlet/tlul_if_crossbar.vfc --vacuity
```

---

## 4. Running the Full 18-Check Zero-SVA AutoCheck Suite

Before writing a single line of `.vfc` or SVA on an imported IP block, run VeloFormal's expanded 18-check AutoCheck:

```bash
veloformal autocheck rtl/my_ip.sv --auto-fsm --strict --sarif my_ip.sarif --report my_ip.html
```

| Tier | Checks Included |
|---|---|
| **Formal Memory & Arithmetic** | `oob_index`, `div_by_zero`, `shift_overflow`, `overflow`, `truncation` |
| **Formal Control & SV Semantics** | `unique_case`, `priority_case`, `unique_if`, `priority_if`, `parallel_case`, `full_case` |
| **Formal FSM Verification** | `fsm_unreachable`, `fsm_deadlock`, `fsm_livelock` (with `--auto-fsm` discovery) |
| **Structural & Reset Hygiene** | `comb_loop`, `multi_driver`, `const_reg`, `no_reset`, `unread_signal` |

Every formal AutoCheck finding is backed by a mathematical proof or a concrete counterexample trace — never heuristic lint guesswork.
