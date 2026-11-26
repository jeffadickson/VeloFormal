# VeloFormal GitHub Action Guide

Gate every RTL pull request with zero-SVA AutoCheck, `.vfc` formal contracts, 4-gate Anti-Cheating Trust Audits, or a full Formal Verification Plan (`vplan.json`) — with native GitHub Code Scanning SARIF annotations and interactive HTML waveform artifacts.

## 1. Zero-SVA PR Gate (18-Check AutoCheck + SARIF Code Scanning)

```yaml
name: RTL Formal AutoCheck

on:
  pull_request:
    paths: ['rtl/**/*.sv']

permissions:
  contents: read
  security-events: write

jobs:
  autocheck:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Run VeloFormal 18-Check AutoCheck
        uses: jeffadickson/VeloFormal@v1
        with:
          mode: autocheck
          rtl_files: 'rtl/alu.sv rtl/fifo.sv'
          strict: 'true'
          auto_fsm: 'true'
          sarif_file: veloformal.sarif
          html_report: autocheck-signoff.html

      - name: Upload SARIF to GitHub Code Scanning
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: veloformal.sarif

      - name: Upload Interactive Waveform Report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: formal-signoff-report
          path: autocheck-signoff.html
```

## 2. 4-Gate Anti-Cheating Trust Audit Gate

Ensure AI-generated or human-authored formal specs are mathematically sound, non-vacuous, dead-end-free, and kill at least 75% of injected RTL mutations:

```yaml
      - name: 4-Gate Formal Trust Audit
        uses: jeffadickson/VeloFormal@v1
        with:
          mode: audit
          rtl_files: 'rtl/sync_fifo.sv'
          spec_file: 'formal/sync_fifo.vfc'
          min_mutation_score: '75'
          html_report: fifo-audit.html
```

## 3. Full Traceable Formal Verification Plan (`vplan.json`)

```yaml
      - name: Execute SoC Formal Verification Plan
        uses: jeffadickson/VeloFormal@v1
        with:
          mode: vplan
          vplan_file: 'formal/soc_vplan.json'
          html_report: soc-vplan.html
```
