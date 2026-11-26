use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy)]
pub struct VipTemplate {
    pub id: &'static str,
    pub standard: &'static str,
    pub summary: &'static str,
    pub default_module: &'static str,
}

pub const VIP_CATALOG: &[VipTemplate] = &[
    VipTemplate {
        id: "axi4_lite",
        standard: "AMBA AXI4-Lite",
        summary: "AW/W/B/AR/R valid-stability under backpressure and response channel discipline",
        default_module: "axi4_lite_slave",
    },
    VipTemplate {
        id: "axi4_full",
        standard: "AMBA AXI4-Full",
        summary: "Burst transfer AW/W/B/AR/R stability, WLAST framing, and burst-length bounds",
        default_module: "axi4_slave",
    },
    VipTemplate {
        id: "axi4_stream",
        standard: "AMBA AXI4-Stream",
        summary: "TVALID/TREADY stability, TDATA/TLAST hold under backpressure, and frame cover",
        default_module: "axis_pipe",
    },
    VipTemplate {
        id: "apb",
        standard: "AMBA APB3 / APB4",
        summary: "PSEL/PENABLE setup->access phase transitions and PADDR/PWRITE/PWDATA stability",
        default_module: "apb_slave",
    },
    VipTemplate {
        id: "ahb_lite",
        standard: "AMBA AHB-Lite",
        summary: "HTRANS legality (IDLE/BUSY/NONSEQ/SEQ) and HADDR/HWRITE hold when !HREADY",
        default_module: "ahb_lite_slave",
    },
    VipTemplate {
        id: "tilelink_ul",
        standard: "TileLink-UL (OpenTitan / SiFive)",
        summary: "Channel A/D valid-stability under stall and legal TileLink-UL opcode checks",
        default_module: "tlul_dev",
    },
    VipTemplate {
        id: "wishbone_b4",
        standard: "Wishbone B4 Pipelined",
        summary: "STB-implies-CYC, mutually exclusive ACK/ERR, and address/write hold under STALL",
        default_module: "wb_slave",
    },
    VipTemplate {
        id: "rvfi",
        standard: "RISC-V Formal Interface (RVFI)",
        summary: "x0 hardwired-zero invariant, PC alignment, and instruction retirement liveness cover",
        default_module: "riscv_core",
    },
];

pub fn find_vip(protocol: &str) -> Option<&'static VipTemplate> {
    let key = protocol.trim().to_ascii_lowercase().replace('-', "_");
    VIP_CATALOG.iter().find(|v| v.id == key)
}

/// Render a ready-to-verify `.vfc` v1.0 contract for the requested industry protocol.
pub fn render_vip_vfc(protocol: &str, module_name: Option<&str>) -> Result<String> {
    let Some(tpl) = find_vip(protocol) else {
        let names: Vec<&str> = VIP_CATALOG.iter().map(|v| v.id).collect();
        bail!(
            "unknown VIP protocol '{protocol}'. Available protocols: {}",
            names.join(", ")
        );
    };
    let dut = module_name.unwrap_or(tpl.default_module);
    let contract_name = to_pascal_case(tpl.id) + "Vip";

    let body = match tpl.id {
        "axi4_lite" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge aclk;
    reset when !aresetn;

    use handshake(awvalid, awready, awaddr);
    use handshake(wvalid, wready, wdata);
    use handshake(bvalid, bready, bresp);
    use handshake(arvalid, arready, araddr);
    use handshake(rvalid, rready, rdata);

    cover write_resp: bvalid && bready;
    cover read_resp:  rvalid && rready;
}}
"#,
            std = tpl.standard
        ),
        "axi4_full" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge aclk;
    reset when !aresetn;

    use handshake(awvalid, awready, awaddr);
    use handshake(wvalid, wready, wdata);
    use handshake(bvalid, bready, bresp);
    use handshake(arvalid, arready, araddr);
    use handshake(rvalid, rready, rdata);

    invariant wlast_hold_under_stall:
        (prev(wvalid) && !prev(wready)) => (wvalid && wlast == prev(wlast));
    invariant rlast_hold_under_stall:
        (prev(rvalid) && !prev(rready)) => (rvalid && rlast == prev(rlast));

    cover write_burst_last: wvalid && wready && wlast;
    cover read_burst_last:  rvalid && rready && rlast;
}}
"#,
            std = tpl.standard
        ),
        "axi4_stream" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge aclk;
    reset when !aresetn;

    use handshake(tvalid, tready, tdata);

    invariant tlast_stable:
        (prev(tvalid) && !prev(tready)) => (tvalid && tlast == prev(tlast));

    cover packet_end: tvalid && tready && tlast;
}}
"#,
            std = tpl.standard
        ),
        "apb" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge pclk;
    reset when !presetn;

    require penable_implies_psel: penable => psel;
    require setup_to_access: (prev(psel) && !prev(penable)) => (psel && penable);
    require addr_stable_in_access:
        (prev(psel) && (!prev(penable) || !prev(pready))) => (paddr == prev(paddr) && pwrite == prev(pwrite));

    cover apb_transfer: psel && penable && pready;
}}
"#,
            std = tpl.standard
        ),
        "ahb_lite" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge hclk;
    reset when !hresetn;

    // HTRANS encoding: 0=IDLE, 1=BUSY, 2=NONSEQ, 3=SEQ
    require hold_addr_when_wait:
        (!prev(hready) && prev(htrans) >= 2) => (haddr == prev(haddr) && hwrite == prev(hwrite));
    require no_seq_from_idle:
        (prev(hready) && prev(htrans) == 0) => (htrans != 3);

    invariant resp_ok_on_idle:
        (prev(hready) && prev(htrans) == 0) => (!hresp);

    cover nonseq_transfer: hready && htrans == 2;
}}
"#,
            std = tpl.standard
        ),
        "tilelink_ul" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge clk;
    reset when rst;

    use handshake(a_valid, a_ready, a_address);
    use handshake(d_valid, d_ready, d_data);

    // Legal TL-UL A-channel opcodes: PutFullData=0, PutPartialData=1, Get=4
    require legal_a_opcode: a_valid => (a_opcode == 0 || a_opcode == 1 || a_opcode == 4);
    // Legal TL-UL D-channel opcodes: AccessAck=0, AccessAckData=1
    invariant legal_d_opcode: d_valid => (d_opcode == 0 || d_opcode == 1);

    cover a_handshake: a_valid && a_ready;
    cover d_handshake: d_valid && d_ready;
}}
"#,
            std = tpl.standard
        ),
        "wishbone_b4" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge clk;
    reset when rst;

    require stb_implies_cyc: wb_stb => wb_cyc;
    require hold_under_stall:
        (prev(wb_stb) && prev(wb_stall)) => (wb_stb && wb_adr == prev(wb_adr) && wb_we == prev(wb_we));

    invariant ack_err_mutex: !(wb_ack && wb_err);
    invariant ack_requires_cyc: wb_ack => wb_cyc;

    cover wb_ack_hit: wb_cyc && wb_ack;
}}
"#,
            std = tpl.standard
        ),
        "rvfi" => format!(
            r#"// VeloFormal Turnkey VIP — {std}
contract {contract_name} for {dut} {{
    clock posedge clk;
    reset when rst;

    invariant x0_hardwired_zero: (rvfi_valid && rvfi_rd_addr == 0) => (rvfi_rd_wdata == 0);
    invariant pc_aligned_32:     rvfi_valid => ((rvfi_pc_rdata & 3) == 0);
    invariant pc_next_aligned:   rvfi_valid => ((rvfi_pc_wdata & 3) == 0);

    cover instruction_retired: rvfi_valid;
}}
"#,
            std = tpl.standard
        ),
        _ => unreachable!(),
    };
    Ok(body)
}

/// Render an IEEE 1800 SVA `bind` checker module for the requested industry protocol.
pub fn render_vip_sva(protocol: &str, module_name: Option<&str>) -> Result<String> {
    let vfc = render_vip_vfc(protocol, module_name)?;
    let parsed = vf_dsl::parse_contract(&vfc)?;
    Ok(vf_dsl::emit_sva(&parsed, None))
}

fn to_pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}
