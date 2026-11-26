# Chapter 12: Turnkey Protocol VIP Library & Traceable Formal Verification Plans

Every SoC design speaks standard on-chip bus protocols — AXI4, APB, AHB-Lite, TileLink, Wishbone, or RISC-V RVFI — and every tape-out requires a **Formal Verification Plan (VPlan)** proving to architects, safety managers, and auditors that every design requirement is backed by a verified formal artifact.

VeloFormal ships both out of the box: `veloformal vip` and `veloformal vplan`.

---

## 1. Turnkey Protocol Verification IP Library (`veloformal vip`)

Writing protocol compliance assertions from scratch is tedious and error-prone (forgetting that `AWVALID` must remain high until `AWREADY`, or that `PADDR` must remain stable during APB `ACCESS` phase, lets silent deadlocks ship to silicon).

VeloFormal bundles **8 turnkey industry-standard Protocol VIPs** authored in clean, readable `.vfc` v1.0 contracts — and automatically convertible into portable IEEE 1800 SVA `bind` modules for hybrid commercial flows:

| Protocol ID | Standard | Formal Checks Included |
|---|---|---|
| `axi4_lite` | AMBA AXI4-Lite | AW/W/B/AR/R valid-stability under backpressure, payload hold, response channel discipline |
| `axi4_full` | AMBA AXI4-Full | Burst AW/W/B/AR/R stability, `WLAST` framing, `AWLEN`/`ARLEN` burst-length bounds |
| `axi4_stream` | AMBA AXI4-Stream | `TVALID`/`TREADY` hold, `TDATA`/`TLAST` stability under stall, frame reachability cover |
| `apb` | AMBA APB3 / APB4 | `PSEL`/`PENABLE` SETUP → ACCESS phase transitions, `PADDR`/`PWRITE`/`PWDATA` hold |
| `ahb_lite` | AMBA AHB-Lite | `HTRANS` legality (`IDLE`/`BUSY`/`NONSEQ`/`SEQ`), `HADDR`/`HWRITE` hold when `!HREADY` |
| `tilelink_ul` | TileLink-UL (OpenTitan / SiFive) | Channel A/D valid-stability under stall, legal TileLink-UL opcode invariants |
| `wishbone_b4` | Wishbone B4 Pipelined | `STB => CYC`, mutually exclusive `ACK`/`ERR`, address & write-enable hold under `STALL` |
| `rvfi` | RISC-V Formal Interface (RVFI) | `x0` hardwired-zero invariant, PC alignment, instruction retirement reachability cover |

### CLI Usage

```bash
# List all 8 built-in VIP protocols
veloformal vip list

# Generate a ready-to-prove .vfc contract customized for your RTL module
veloformal vip init axi4_lite --module my_csr_slave -o formal/my_csr_slave_axi.vfc

# Or emit a portable IEEE 1800 SVA `bind` checker for Questa / Jasper / Verilator
veloformal vip init tilelink_ul --module tlul_xbar --sva -o formal/tlul_xbar_sva.sv
```

### MCP Usage for AI Agents

Agents connected to `vf-mcp` call `get_vip_template`:
- Omit `protocol` to discover all 8 available VIPs.
- Pass `protocol`, `module_name`, and `format` (`"vfc"` or `"sva"`) to instantly generate a spec-compliant baseline contract without hallucinating protocol rules.

---

## 2. Traceable Formal Verification Plans (`veloformal vplan`)

Ad-hoc scripts running isolated formal checks do not scale to tape-out sign-off. A **Formal Verification Plan (`vplan.json`)** declares every formal requirement in version-controlled JSON, ties each requirement to an owner and verification method, executes them reproducibly, and emits a **traceable Sign-Off Matrix** in JSON, Markdown, and HTML.

### Supported Requirement Kinds

1. **`autocheck`**: Runs zero-SVA 18-check structural & formal bug hunting (with optional `strict: true` and `auto_fsm: true`).
2. **`contract`**: Proves a `.vfc` contract via the multi-engine portfolio.
3. **`audit`**: Runs the full **4-Gate Anti-Cheating Trust Audit** (Safety + Dead-End + Vacuity + Mutation Score `>= min_mutation_score`).
4. **`compose`**: Runs **Automated Hierarchical Assume-Guarantee Composition** (`compose_config`) across leaf submodules and top-level SoC.

### Example `soc_vplan.json`

From `examples/24_hierarchical_compose/soc_vplan.json`:

```json
{
  "project": "VeloFormal Compositional SoC Sign-Off",
  "version": "1.0",
  "requirements": [
    {
      "id": "REQ-ALU-01",
      "title": "Leaf ALU zero-SVA structural and formal AutoCheck clean",
      "kind": "autocheck",
      "owner": "rtl-team",
      "rtl_files": ["heavy_alu_leaf.sv"],
      "depth": 10
    },
    {
      "id": "REQ-ALU-02",
      "title": "Leaf ALU bounded output contract proved",
      "kind": "contract",
      "owner": "formal-team",
      "rtl_files": ["heavy_alu_leaf.sv"],
      "spec_file": "heavy_alu_leaf.vfc",
      "depth": 12
    },
    {
      "id": "REQ-SOC-01",
      "title": "Top-level SoC bounded output proved via hierarchical assume-guarantee composition",
      "kind": "compose",
      "owner": "soc-lead",
      "compose_config": {
        "top_rtl_files": ["heavy_alu_leaf.sv", "soc_top.sv"],
        "top_module": "soc_top",
        "top_spec_file": "soc_top.vfc",
        "submodules": [
          {
            "instance": "u_alu",
            "module_name": "heavy_alu_leaf",
            "rtl_files": ["heavy_alu_leaf.sv"],
            "spec_file": "heavy_alu_leaf.vfc"
          }
        ],
        "depth": 12
      }
    }
  ]
}
```

### Executing & Generating Sign-Off Deliverables

```bash
veloformal vplan examples/24_hierarchical_compose/soc_vplan.json \
  --format markdown \
  --html signoff/soc_vplan.html \
  --md signoff/soc_vplan.md
```

Sample Markdown output (also posted directly to GitHub Actions Step Summaries):

| Req ID | Title | Kind | Status | Trust Audit | Mutation | Detail |
|---|---|---|---|---|---|---|
| `REQ-ALU-01` | Leaf ALU zero-SVA structural and formal AutoCheck clean | `autocheck` | **PASS** | - | - | 0 checks (0 bugs) |
| `REQ-ALU-02` | Leaf ALU bounded output contract proved | `contract` | **PASS** | - | - | PROVED (portfolio) |
| `REQ-SOC-01` | Top-level SoC bounded output proved via hierarchical assume-guarantee composition | `compose` | **PASS** | - | - | composed 1 leaves (FFs 9 -> 2) |

If **any** requirement fails — or if an `audit` requirement fails its mutation kill-rate threshold — `veloformal vplan` exits non-zero, blocking CI merges until traceability is 100% green.
