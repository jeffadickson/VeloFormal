class Veloformal < Formula
  desc "Formal verification for the human and agentic era (Slang SV + .vfc DSL + Formal Apps + MCP)"
  homepage "https://github.com/jeffadickson/VeloFormal"
  url "https://github.com/jeffadickson/VeloFormal/archive/refs/tags/v1.0.0.tar.gz"
  license "Apache-2.0"
  head "https://github.com/jeffadickson/VeloFormal.git", branch: "main"

  depends_on "cmake" => :build
  depends_on "rust" => :build
  depends_on "z3"

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/vf-cli")
    system "cargo", "install", *std_cargo_args(path: "crates/vf-mcp")
    system "cargo", "install", *std_cargo_args(path: "crates/vf-lsp")
  end

  test do
    assert_match "axi4_lite", shell_output("#{bin}/veloformal vip list")
  end
end
