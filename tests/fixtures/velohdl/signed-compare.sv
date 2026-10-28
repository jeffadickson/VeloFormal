module signed_compare (
    input logic clk,
    input logic rst_n,
    output logic signed [7:0] count
);
    always_ff @(posedge clk) begin
        if (!rst_n)
            count <= -8'sd1;
        else if (count < 8'sd0)
            count <= -count;
        else
            count <= count - 8'sd1;
    end
endmodule
