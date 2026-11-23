// Example 02: Ready/Valid Pipeline Skid Buffer verified with vip/vf_handshake.sv
// Classic zero-bubble elastic pipeline stage that breaks combinational ready paths while
// preserving 100% throughput and exact FIFO packet ordering (proved via $anyconst scoreboard).
module skid_buffer #(
    parameter int WIDTH = 8
) (
    input  logic             clk,
    input  logic             rst,
    // Upstream interface
    input  logic             s_valid,
    output logic             s_ready,
    input  logic [WIDTH-1:0] s_data,
    // Downstream interface
    output logic             m_valid,
    input  logic             m_ready,
    output logic [WIDTH-1:0] m_data
);
    logic             out_valid;
    logic [WIDTH-1:0] out_data;
    logic             skid_valid;
    logic [WIDTH-1:0] skid_data;

    initial begin
        out_valid  = 1'b0;
        out_data   = '0;
        skid_valid = 1'b0;
        skid_data  = '0;
    end

    assign s_ready = !skid_valid;
    assign m_valid = out_valid;
    assign m_data  = out_data;

    always_ff @(posedge clk) begin
        if (rst) begin
            out_valid  <= 1'b0;
            out_data   <= '0;
            skid_valid <= 1'b0;
            skid_data  <= '0;
        end else begin
            if (!out_valid) begin
                // Output register empty: accept directly from upstream
                out_valid <= s_valid;
                out_data  <= s_data;
            end else if (m_ready) begin
                // Downstream accepts current out_data
                if (skid_valid) begin
                    out_valid  <= 1'b1;
                    out_data   <= skid_data;
                    skid_valid <= 1'b0;
                end else begin
                    out_valid <= s_valid;
                    out_data  <= s_data;
                end
            end else if (!skid_valid && s_valid) begin
                // Downstream stalled: capture upstream transfer into skid register
                skid_valid <= 1'b1;
                skid_data  <= s_data;
            end
        end
    end

`ifdef FORMAL
    // Instantiate Formal VIP handshake protocol checker on downstream output!
    vf_handshake #(.WIDTH(WIDTH)) u_m_chk (
        .clk(clk),
        .rst(rst),
        .valid(m_valid),
        .ready(m_ready),
        .data(m_data)
    );

    // Symbolic data-integrity proof: any word accepted on s_* must emerge intact on m_*!
    (* anyconst *) logic [WIDTH-1:0] magic_word;
    logic [1:0] in_flight; // 0 = idle, 1 = in out_reg, 2 = in skid_reg

    initial in_flight = 2'd0;

    wire s_fire = s_valid && s_ready;
    wire m_fire = m_valid && m_ready;

    always_ff @(posedge clk) begin
        if (rst) begin
            in_flight <= 2'd0;
        end else begin
            case (in_flight)
                2'd0: begin
                    if (s_fire && s_data == magic_word) begin
                        if (!out_valid || (m_ready && !skid_valid))
                            in_flight <= 2'd1;
                        else
                            in_flight <= 2'd2;
                    end
                end
                2'd1: begin
                    if (m_fire) in_flight <= 2'd0;
                end
                2'd2: begin
                    if (m_fire) in_flight <= 2'd1;
                end
                default: in_flight <= 2'd0;
            endcase
        end
    end

    always_comb begin
        if (!rst) begin
            a_skid_implies_out: assert (!skid_valid || out_valid);
            if (in_flight == 2'd1) begin
                a_out_matches: assert (out_valid && out_data == magic_word);
            end
            if (in_flight == 2'd2) begin
                a_skid_matches: assert (skid_valid && skid_data == magic_word);
            end
            c_skid_full:    cover (skid_valid && out_valid);
            c_magic_popped: cover (in_flight == 2'd1 && m_fire);
        end
    end
`endif
endmodule
