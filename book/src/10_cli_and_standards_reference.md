# Chapter 10: CLI, SymbiYosys, Questa Tcl & CI Reference

## 10.1 Complete `veloformal` CLI Reference

| Command | Purpose | Key Flags |
|---|---|---|
| `bmc` | Bounded Model Checking | `--depth`, `--skip`, `--append`, `--solver`, `--vcd` |
| `prove` | Unbounded safety proof | `--engine auto\|portfolio\|kind\|pdr`, `--coi`, `--case-split`, `--blackbox`, `--cutpoint`, `--cache-dir` |
| `live` | Liveness & fairness lasso detection | `--depth`, `--vcd` |
| `cover` | Reachability witness generation | `--depth`, `--append`, `--vcd-dir` |
| `vacuity` | Assertion antecedent reachability | `--depth` |
| `deadend` | Over-constraint assumption dead-end detection | `--depth`, `--vcd` |
| `equiv` | Questa-class SLEC equivalence | `--gold`, `--gate`, `--latency-gold`, `--latency-gate`, `--skip-cycles`, `--reset`, `--reset-cycles`, `--valid-cond`, `--output`, `--ignore-output` |
| `contract` | Verify `.vfc` v1.0 contract | `--spec`, `--mode`, `--role provider\|caller`, `--vacuity`, `--report`, `--format`, `--emit-cert`, `--verify-cert` |
| `emit-sva` | Export `.vfc` to portable IEEE 1800 SVA `bind` | `--rtl`, `--top`, `-o` |
| `fmt` | Canonical 4-space LF `.vfc` formatter | `--check`, `-i` |
| `inspect` | Elaborated RTL introspection | `--format text\|json` |
| `scaffold` | Auto-scaffold starter `.vfc` from RTL | `-o` |
| `audit` | 4-Gate Anti-Cheating Trust Audit | `--spec`, `--min-mutation-score`, `--max-mutants`, `--format text\|json` |
| `proof-core` | COI coverage + minimal assumption core | `--depth` |
| `mutate` | Formal mutation testing completeness (%) | `--spec`, `--max-mutants`, `--report`, `--format` |
| `autocheck` | Push-button RTL bug hunting | `--fsm`, `--check-overflow`, `--check-const-regs`, `--vcd-dir` |
| `covercheck` | Branch witnesses & dead-code proofs | `--depth`, `--vcd-dir` |
| `connectivity` | CSV pin-to-pin routing verification | `--spec` |
| `secure` | 2-copy Information Flow Tracking | `--source`, `--target`, `--declassify`, `--vcd` |
| `xcheck` | 4-state dual-rail X-propagation check | `--target`, `--x-source`, `--skip-cycles`, `--vcd-dir` |
| `regcheck` | CSR verification (CSV/RDL/IP-XACT) | `--spec`, `--protocol raw\|apb`, `--read-latency 0\|1` |
| `cdc` | Structural CDC/RDC + SDC + formal Gray check | `--sdc`, `--gray` |
| `fault` | ISO 26262 SA0/SA1/SEU fault analysis | `--target`, `--output`, `--alarm`, `--detect-latency`, `--models`, `--report`, `--format` |
| `powercheck` | IEEE 1801 UPF isolation & retention verifier | `--upf`, `--pd-off`, `--vcd-dir` |
| `sby` | SymbiYosys `.sby` multi-task project runner | `-d <workdir>` |
| `tcl` | Questa-Formal `.do` Tcl script runner | `--report`, `--format text\|json\|junit\|sarif` |
| `dump-ir` / `dump-btor2` / `dump-smt2` / `dump-aiger` | Standard formal interchange exports | `-o` (for `dump-aiger`) |

## 10.2 SymbiYosys (`.sby`) Compatibility

Run existing `.sby` multi-task projects directly:

```bash
veloformal sby examples/01_traffic_light_fsm/traffic_light.sby -d workdir/
```

Supports `[tasks]`, `[options]` (`mode bmc|prove|live|cover`, `depth`, `skip`, `append`, `multiclock`, `expect`), `[engines]` (`smtbmc z3|bitwuzla|cvc5`, `abc pdr`), `[script]` (`read`, `prep -top`, `chformal`, `async2sync`), and `[files]` / inline `[file name]`.

## 10.3 Questa-Formal Tcl (`.do`) & CI Integration (SARIF / JUnit / HTML)

Execute `.do` verification suites in GitHub Actions with native SARIF 2.1 code-scanning upload, JUnit XML test reporting, and self-contained Interactive HTML Sign-Off Dashboards:

```bash
veloformal tcl examples/21_questa_parity/run_questa_parity.do \
    --format sarif --report signoff.html > results.sarif
```

Supported `.do` directives include `formal compile`, `vlog`, `netlist clock`, `netlist reset`, `formal verify`, `formal contract`, `autocheck verify`, `covercheck verify`, `cdc run`, `xcheck verify`, `regcheck verify` (`-protocol apb`, `-read_latency`), `slec verify` (`-reset`, `-valid_cond`, `-ignore_output`), `fault verify`, `powercheck verify`, and `deadend verify`.
