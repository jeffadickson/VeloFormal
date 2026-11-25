# Chapter 9: Silicon Case Studies

Every case study below lives in [`examples/`](../../examples/README.md) and runs continuously in CI.

## 9.1 Case Study 1 — 2-Stage Pipelined RV32I RISC-V Core (`examples/20_riscv_rv32i_formal/`)

Verifying a real RISC-V CPU core demonstrates word-level SMT + `.vfc` contracts on realistic 32-bit datapath and control logic:
- **RTL** ([`rv32i_core.sv`](../../examples/20_riscv_rv32i_formal/rv32i_core.sv)): 2-stage Fetch/Execute pipelined RV32I core supporting `ADD`, `SUB`, `AND`, `OR`, `XOR`, `SLT`, `ADDI`, `JAL`, and `BEQ` with 32x32-bit register file.
- **Contract** ([`rv32i_isa.vfc`](../../examples/20_riscv_rv32i_formal/rv32i_isa.vfc)): Proves architectural invariants — `x0` is hardwired zero across all instructions, PC remains 4-byte aligned, ALU results match ISA semantics, register-writeback timing matches pipeline latency — plus 4-state `xfree` on PC and retirement outputs!

```bash
veloformal contract examples/20_riscv_rv32i_formal/rv32i_core.sv \
    --spec examples/20_riscv_rv32i_formal/rv32i_isa.vfc --report rv32i_signoff.html
```

## 9.2 Case Study 2 — AMBA AXI4-Lite & APB Protocol Compliance (`examples/04_*`, `05_*`)

Attach VeloFormal's reusable Formal Assertion Library (`vip/vf_axi4_lite.sv`, `vip/vf_apb.sv`, `vip/vf_handshake.sv`) directly to your RTL without touching production code:

```bash
veloformal prove vip/vf_handshake.sv vip/vf_axi4_lite.sv \
    examples/04_axi4_lite_slave_vip/axi4_lite_slave.sv --top axi4_lite_slave

veloformal prove vip/vf_apb.sv \
    examples/05_apb_bridge_vip/apb_bridge_vip.sv --top apb_bridge_vip
```

## 9.3 Case Study 3 — Non-Inductive Credit Counter (`examples/08_pdr_credit_counter/`)

Some invariants hold for all reachable states yet fail k-induction for every small $k$ because unreachable states violate an unstated auxiliary invariant. Run `veloformal prove --engine pdr` (or `--engine portfolio`) to watch ABC IC3/PDR automatically synthesize the strengthening invariant:

```bash
veloformal prove examples/08_pdr_credit_counter/credit_flow.sv --engine portfolio
```

## 9.4 Case Study 4 — 4-Way Round-Robin Fairness vs Fixed-Priority Starvation (`examples/06_arbiter_liveness/`)

Safety alone cannot catch starvation! VeloFormal's bounded fair lasso engine (`veloformal live`) proves round-robin arbitration serves every persistent requestor while producing a concrete starving lasso trace for a fixed-priority arbiter:

```bash
veloformal live examples/06_arbiter_liveness/rr_arbiter_4.sv --depth 8
veloformal live examples/06_arbiter_liveness/fixed_prio_starve.sv --depth 6 --vcd starve.vcd
```

## 9.5 Case Study 5 — ISO 26262 TMR & Parity Safety Analysis (`examples/21_questa_parity/`)

Compare an unprotected register (`SPFM = 0%`), a parity-checked register (`DC = 100%`, `SPFM = 100%`), and a TMR majority-voted register (`100% SAFE/MASKED`, `SPFM = 100%`):

```bash
veloformal fault examples/21_questa_parity/tmr_voter.sv --target r0 --output safe_out
veloformal fault examples/21_questa_parity/parity_reg.sv --target data_reg --alarm parity_err
```
