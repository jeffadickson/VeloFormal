# Questa Formal Tcl Script exercising VeloFormal's full Questa Parity Suite
# Run: veloformal tcl examples/21_questa_parity/run_questa_parity.do

# 1. ISO 26262 Formal Fault Analysis on TMR voter (100% Safe/Masked)
formal compile -d tmr_voter tmr_voter.sv
fault verify -target r0 -alarm tmr_err -depth 10

# 2. IEEE 1801 UPF Power-Domain Isolation Verification
formal compile -d upf_iso_dut upf_iso_dut.sv
powercheck verify -upf upf_iso.upf

# 3. Native AMBA APB3 Register Check with SystemRDL 2.0
formal compile -d apb_csr apb_csr.sv
regcheck verify -spec apb_csr.rdl -protocol apb

# 4. SLEC Low-Power Clock Gating (LPGC) + Safety-Insertion Output Filtering
slec verify -gold lpgc_gold.sv -gate lpgc_gate.sv -reset rst=1 -valid_cond out_valid -ignore_output cg_active

exit
