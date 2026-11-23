module struct_and_func (
    input  logic       clk,
    input  logic       in_valid,
    input  logic [7:0] in_data,
    output logic [7:0] out_data
);
    typedef struct packed {
        logic       valid;
        logic [7:0] data;
    } pkt_t;

    function automatic logic [7:0] rotl1(input logic [7:0] v);
        return {v[6:0], v[7]};
    endfunction

    pkt_t p;
    assign p.valid = in_valid;
    assign p.data  = rotl1(in_data);

    assign out_data = p.valid ? p.data : 8'h00;

`ifdef FORMAL
    (* anyconst *) logic [7:0] k_val;
    always_comb begin
        if (in_valid && in_data == k_val)
            assert (out_data == {k_val[6:0], k_val[7]});
        if (!in_valid)
            assert (out_data == 8'h00);
    end
`endif
endmodule
