# Chapter 4: Mastering the `.vfc` v1.0 Contract DSL

This chapter teaches `.vfc` by escalating through three real, CI-verified examples in [`examples/22_dsl_vs_sva_showcase/`](../../examples/22_dsl_vs_sva_showcase/README.md). For the complete formal grammar and type rules, see [Appendix A: `.vfc` v1.0 Language Specification](../../docs/VFC_SPEC.md).

## 4.1 Tier 1 — Ready/Valid Pipeline (`18 lines` vs `59 lines` SVA)

Consider a 1-stage pipelined incrementer (`handshake_pipe.sv`). In traditional SVA, you must write a 59-line `bind` wrapper with `f_past_valid`, repeated `@(posedge clk) disable iff (rst)`, and hand-coded backpressure stability.

In `.vfc`, the entire contract is 18 lines:

```vfc
contract HandshakePipeSpec for handshake_pipe {
    clock posedge clk;
    reset when rst;

    // Built-in ready/valid stability under backpressure + transfer cover
    use handshake(out_valid, out_ready, out_data);

    invariant ready_when_empty: !out_valid => in_ready;

    operation accept when in_valid && in_ready {
        ensure produces_incremented after 1 cycles:
            out_valid && out_data == at_trigger(in_data) + 8'd1;
    }

    cover transfer_42: out_valid && out_ready && out_data == 8'h42;
}
```

Key idioms introduced:
- **`use handshake(valid, ready, data);`** replaces 15 lines of fragile `$past` / `$stable` SVA.
- **`operation ... when ... { ensure ... after 1 cycles: ... }`** automatically aborts in-flight obligations on reset and snapshots `at_trigger(in_data)` into pipeline capture registers.
- **Strict typing**: `out_data` (8-bit) and `8'd1` match widths explicitly — accidental truncation or signed/unsigned confusion is rejected at compile time.

## 4.2 Tier 2 — Synchronous Packet FIFO (`18 lines` vs `96 lines` SVA)

Proving end-to-end FIFO data integrity in traditional SVA requires ~60 lines of `(* anyconst *)` symbolic tracking address + shadow pointers + tracking FSM. In `.vfc`:

```vfc
contract PacketFifoSpec for packet_fifo {
    clock posedge clk;
    reset when rst;

    invariant not_full_and_empty: !(full && empty);
    invariant level_bounded: unsigned(level) <= 4;
    invariant empty_matches_level: empty == (unsigned(level) == 0);
    invariant full_matches_level: full == (unsigned(level) == 4);

    use fifo_scoreboard(wr_en, wr_data, rd_en, rd_data, full, empty, 4);

    cover reaches_full: full;
    cover simultaneous_rw: wr_en && rd_en && !full && !empty;
}
```

One line — `use fifo_scoreboard(...)` — synthesizes the exact `$anyconst` symbolic data-integrity proof, proved unboundedly via ABC IC3/PDR.

## 4.3 Tier 3 — Multi-Cycle Secure Crypto Accelerator (`40 lines` vs `108+ lines` SVA + 2 Tcl scripts)

Real SoC IP combines protocol handshakes, ghost accounting, multi-cycle pipelines, reset safety, and security isolation. `.vfc` unifies **all five domains** in one contract:

```vfc
contract CryptoEngineSpec for crypto_engine {
    clock posedge clk;
    reset when rst;

    use handshake(resp_valid, resp_ready, resp_data);

    ghost jobs_started: logic[4] = 0;
    update {
        if (cmd_valid && cmd_ready && jobs_started < 15) {
            jobs_started <= jobs_started + 1;
        }
    }

    invariant no_accept_while_busy: busy => !cmd_ready;
    invariant done_implies_busy: done => busy;
    invariant completions_le_started: telemetry_out <= jobs_started;

    operation start_job when cmd_valid && cmd_ready {
        ensure enters_busy after 1 cycles: busy;
        ensure completes within 1..3 cycles: done;
        ensure stays_busy until done: busy;
    }

    // 4-State X-Check & 2-Copy Information Flow Confidentiality (impossible in SVA!)
    xfree resp_valid, busy, done, telemetry_out, resp_is_auth;
    secure secret_key -> telemetry_out;
    secure secret_key -> resp_data unless resp_is_auth;

    cover auth_response: resp_valid && resp_is_auth;
}
```

## 4.4 Exporting Portable IEEE 1800 SVA (`emit-sva`)

Need to run your spec inside Questa, JasperGold, or Verilator simulation? Export a clean SVA `bind` file at any time:

```bash
veloformal emit-sva crypto_engine.vfc --rtl crypto_engine.sv -o crypto_engine_sva.sv
```

And format `.vfc` files canonically in CI with `veloformal fmt --check`.
