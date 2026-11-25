# Chapter 7: Functional Safety (ISO 26262) & Low-Power (IEEE 1801 UPF)

Automotive, aerospace, medical, and mobile SoCs require formal proof beyond functional correctness: **fault tolerance** under hardware defects and **power-domain integrity** across sleep/wake transitions.

## 7.1 ISO 26262 Formal Fault Analysis (`veloformal fault`)

Traditional fault simulation injects random faults under random vectors — leaving huge uncertainty about whether an untested input sequence could turn a silent bit flip into a safety hazard.

`veloformal fault` constructs a **2-copy fault-injection miter** (`gold` vs `fault`) for every target bit and fault model:
- **`SA0`**: Permanent Stuck-At-0
- **`SA1`**: Permanent Stuck-At-1
- **`SEU`**: Transient Single-Event Upset (symbolic single-cycle bit-flip at any unconstrained cycle $t \ge 0$)

Every fault site is mathematically classified into one of three ISO 26262 buckets:
1. **`SAFE/MASKED`**: Proved unboundedly that the fault can **never** alter any safety output under any input sequence (e.g., Triple Modular Redundancy majority voting).
2. **`DETECTED`**: The fault can alter an output, **but** is formally proved to assert the safety mechanism `--alarm` within `--detect-latency` cycles (e.g., parity, ECC, lockstep comparator).
3. **`SPF` (Dangerous Single-Point Fault)**: A concrete counterexample trace where the fault corrupts a safety output **without** triggering the safety alarm!

### Example 1 — Triple Modular Redundancy (100% Masked, SPFM = 100%)
```bash
veloformal fault examples/21_questa_parity/tmr_voter.sv \
    --target r0 --target r1 --target r2 --output safe_out
```

### Example 2 — Parity-Protected Register (100% Detected, DC = 100%, SPFM = 100%)
```bash
veloformal fault examples/21_questa_parity/parity_reg.sv \
    --target data_reg --alarm parity_err --report fmeda_signoff.html
```

VeloFormal computes exact **Diagnostic Coverage (DC %)** and **Single-Point Fault Metric (SPFM %)** and renders an Interactive HTML FMEDA Dashboard with waveforms and Root-Cause Backtraces for any dangerous SPF.

---

## 7.2 IEEE 1801 UPF Low-Power Formal Verification (`veloformal powercheck`)

Power-gated SoC domains fail catastrophically if a powered-off block drives un-clamped signals into an always-on domain or loses state across sleep/wake retention.

`veloformal powercheck` parses IEEE 1801 `.upf` power intent (`create_power_domain`, `set_isolation`, `set_isolation_control`, `set_retention`, `set_retention_control`) and formally proves four classes of low-power safety:

1. **Isolation Clamps (`0`, `1`, `latch`)**: Whenever `isolation_signal` is active, every isolated signal is proved unboundedly to clamp to `0`, all-`1`s, or hold its previous value (`latch`).
2. **Power-Down Isolation Sequencing (`--pd-off DOMAIN=sig`)**: Proves isolation is always asserted before/during power-off (`pd_off => iso_active`).
3. **Un-Isolated Crossing Detection**: Flags any output or crossing signal in a switchable power domain omitted from isolation rules.
4. **Retention Register Save/Restore (`set_retention` + `set_retention_control`)**: Synthesizes a formal shadow retention latch and proves that asserting `save` followed later by `restore` recovers the exact saved register value.

```bash
veloformal powercheck examples/21_questa_parity/upf_iso_dut.sv \
    --upf examples/21_questa_parity/upf_iso.upf \
    --pd-off PD_ACCEL=pd_off
```
