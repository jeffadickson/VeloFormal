# Chapter 14: CI/CD Pipelines, VS Code IDE Integration & Interactive Visual Debugging

Formal verification succeeds when it fits seamlessly into how hardware engineers and AI agents actually work: inside the IDE while writing RTL, inside pull requests before merging, and with instant visual root-cause debugging when a proof fails.

---

## 1. Interactive Visual Waveform & COI Root-Cause Debugger

Whenever you pass `--report signoff.html` to `contract`, `prove`, `autocheck`, `compose`, or `equiv`, VeloFormal generates a **zero-dependency, self-contained HTML5 Sign-Off Dashboard** that opens in any browser or VS Code Webview.

### Interactive Debugging Features

Every failing property embeds its full trace JSON alongside an interactive SVG waveform engine and Cone-of-Influence (COI) Root-Cause Backtrace:

- **Live Radix Switcher**: Toggle every multi-bit bus instantaneously between `Hex`, `Unsigned Dec`, `Signed Dec`, and `Binary` (`setRadix`).
- **Real-Time Signal Filter**: Type any substring (`valid`, `u_alu`, `cnt`) to filter waveforms live (`setSigFilter`).
- **Zoom Controls**: Expand or compress cycle widths (`-` / `+`) to inspect dense multi-bit hex values (`zoomWave`).
- **Clickable Vertical Cycle Cursor + Signal Snapshot Bar**: Click any cycle header `t=k` or waveform cell to snap the golden cursor line to cycle `k` and display a live snapshot of signal values.
- **Synchronized Clickable COI Root-Cause Backtrace**: Every failing assertion computes its backward COI causal transition chain (`t=0 -> t=1 -> ... -> t=k`). Clicking any backtrace transition row (`bt-line`) jumps the waveform cursor straight to that exact cycle and highlights the active transition!

```bash
# Generate an interactive HTML waveform & COI backtrace report
veloformal contract rtl/fifo.sv --spec formal/fifo.vfc --report signoff/fifo.html
```

---

## 2. VS Code Extension (`editors/vscode/`)

The VeloFormal VS Code extension pairs **live `.vfc` LSP editing** with **one-click formal verification and embedded waveform debugging**:

- **Live `.vfc` Language Server (`vf-lsp`)**: Syntax highlighting, real-time parse/type diagnostics, keyword/template completion, and hover docs.
- **`VeloFormal: Verify Current .vfc Contract`**: Runs the active `.vfc` spec directly from the command palette or editor title bar.
- **`VeloFormal: Run AutoCheck on Active SystemVerilog File`**: Runs 18-check zero-SVA AutoCheck on the open `.sv` buffer and renders results in a dedicated Webview panel.
- **`VeloFormal: Open Interactive Waveform Debugger`**: Opens any generated VeloFormal HTML sign-off report right inside a VS Code editor tab (`enableScripts: true`) — no external GTKWave install needed.

---

## 3. GitHub Actions CI Integration (`action.yml`)

VeloFormal ships a first-class reusable GitHub Action (`jeffadickson/VeloFormal@v1`) at the repository root (`action.yml`) supporting four CI modes (`autocheck`, `contract`, `audit`, `vplan`), native **SARIF 2.1.0 GitHub Code Scanning** inline PR annotations, and automatic **GitHub Step Summary** tables:

```yaml
name: Formal Sign-Off
on: [pull_request]

permissions:
  contents: read
  security-events: write

jobs:
  formal:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: 18-Check Zero-SVA AutoCheck
        uses: jeffadickson/VeloFormal@v1
        with:
          mode: autocheck
          rtl_files: 'rtl/heavy_alu_leaf.sv rtl/soc_top.sv'
          strict: 'true'
          auto_fsm: 'true'
          sarif_file: veloformal.sarif
          html_report: autocheck.html

      - name: Upload SARIF to GitHub Code Scanning
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: veloformal.sarif

      - name: Execute SoC Formal Verification Plan
        uses: jeffadickson/VeloFormal@v1
        with:
          mode: vplan
          vplan_file: 'examples/24_hierarchical_compose/soc_vplan.json'
          html_report: soc-vplan.html
```

See `docs/GITHUB_ACTION.md` for full input/output documentation.

---

## 4. Packaging & One-Click MCP Agent Setup

### Homebrew (`packaging/homebrew/veloformal.rb`)

Install `veloformal`, `vf-mcp`, and `vf-lsp` along with `z3` in one command:

```bash
brew install --formula packaging/homebrew/veloformal.rb
```

### One-Click MCP Configuration for Claude Desktop & Cursor

Drop the ready-made config from `packaging/mcp/` into your AI client:

- **Claude Desktop** (`packaging/mcp/claude_desktop_config.json`):
  ```json
  {
    "mcpServers": {
      "veloformal": {
        "command": "vf-mcp",
        "args": []
      }
    }
  }
  ```
- **Cursor / Claude Code** (`packaging/mcp/cursor_mcp.json`): Copy to `.cursor/mcp.json` in your RTL repository to immediately grant your AI coding agent all 12 typed formal tools (`inspect_rtl`, `scaffold_contract`, `verify_contract`, `prove_sv`, `run_autocheck`, `check_equiv`, `check_deadend`, `check_fault`, `check_power`, `audit_trust`, `compose_verify`, `get_vip_template`).
