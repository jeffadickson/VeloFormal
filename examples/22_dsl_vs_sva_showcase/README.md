# VeloFormal `.vfc` Contract DSL vs Traditional SVA — 3-Tier Side-by-Side Showcase

A progressive, CI-verified comparison of VeloFormal's non-invasive `.vfc` v1.0 Contract DSL against traditional IEEE 1800 SystemVerilog Assertions (`bind` + manual `$past` reset guards + manual `$anyconst` scoreboards + separate vendor Tcl scripts).

**Both** the `.vfc` contract and the traditional SVA `bind` file in every tier are formally proved on the exact same pure-RTL design (`ZERO` `` `ifdef FORMAL `` edits to production RTL).

## Escalation Summary

| Tier | Design | Traditional SVA | VeloFormal `.vfc` | Reduction | What Traditional SVA Forces You To Hand-Code (or Can't Express) |
|---|---|---:|---:|---:|---|
| **Tier 1 (Simple)** | [`01_handshake_pipe`](01_handshake_pipe/) | **59 lines** | **18 lines** | **3.3x** | Manual port re-declarations, `bind`, `f_past_valid` reset guard, repeated `@(posedge clk) disable iff (rst)`, hand-coded ready/valid backpressure stability |
| **Tier 2 (Intermediate)** | [`02_packet_fifo`](02_packet_fifo/) | **96 lines** | **18 lines** | **5.3x** | Infamous 60-line `(* anyconst *)` symbolic tracking address, shadow write/read pointers, and `f_in_fifo` state machine (one off-by-one bug invalidates the proof!) |
| **Tier 3 (Complex SoC IP)** | [`03_secure_crypto_engine`](03_secure_crypto_engine/) | **108 lines** *(incomplete!)* | **40 lines** *(complete!)* | **2.7x+ & 3-in-1** | Hand-coded ghost counter, cryptic `|=>` / `##[1:3]` / `until` SVA — **and SVA STILL cannot express 4-state `xfree` or 2-copy `secure` Information Flow Confidentiality/Declassification!** |

---

## Tier 1: Ready/Valid Pipelined Incrementer (`01_handshake_pipe/`)

```bash
# VeloFormal .vfc (18 lines — proves safety + cover + vacuity in one shot)
veloformal contract examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.sv \
    --spec examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.vfc --vacuity

# Traditional SVA bind (59 lines)
veloformal prove examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe.sv \
    examples/22_dsl_vs_sva_showcase/01_handshake_pipe/handshake_pipe_sva.sv --top handshake_pipe
```

**VeloFormal `.vfc` (18 lines):**
```vfc
contract HandshakePipeSpec for handshake_pipe {
    clock posedge clk;
    reset when rst;

    use handshake(out_valid, out_ready, out_data);

    invariant ready_when_empty: !out_valid => in_ready;

    operation accept when in_valid && in_ready {
        ensure produces_incremented after 1 cycles:
            out_valid && out_data == at_trigger(in_data) + 8'd1;
    }

    cover transfer_42: out_valid && out_ready && out_data == 8'h42;
}
```

---

## Tier 2: Synchronous Packet FIFO with Data Integrity (`02_packet_fifo/`)

```bash
# VeloFormal .vfc (18 lines — 1-line built-in $anyconst scoreboard!)
veloformal contract examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.sv \
    --spec examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.vfc --vacuity

# Traditional SVA bind + manual symbolic scoreboard (96 lines)
veloformal prove examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo.sv \
    examples/22_dsl_vs_sva_showcase/02_packet_fifo/packet_fifo_sva.sv --top packet_fifo
```

**VeloFormal `.vfc` (18 lines):**
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

---

## Tier 3: Multi-Cycle Secure Crypto Accelerator (`03_secure_crypto_engine/`)

One single 40-line `.vfc` contract unifies **five verification domains**: ready/valid protocol compliance, ghost-state job accounting, multi-cycle temporal windows (`after` / `within 1..3` / `until`), 4-state X-propagation (`xfree`), and 2-copy Information Flow Confidentiality & Declassification (`secure ... -> ... unless ...`) — plus passes the 4-gate Anti-Cheating Trust Audit:

```bash
# VeloFormal .vfc (40 lines — proves ALL 5 domains in one command!)
veloformal contract examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv \
    --spec examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.vfc \
    --vacuity --report crypto_signoff.html

# 4-Gate Anti-Cheating Trust Audit
veloformal audit examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv \
    --spec examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.vfc

# Traditional SVA bind (108 lines — still missing X-Check and Secure IFT!)
veloformal prove examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv \
    examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine_sva.sv --top crypto_engine
```

Need portable IEEE 1800 SVA for interop with Questa or JasperGold? Export it automatically from `.vfc`:
```bash
veloformal emit-sva examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.vfc \
    --rtl examples/22_dsl_vs_sva_showcase/03_secure_crypto_engine/crypto_engine.sv
```
