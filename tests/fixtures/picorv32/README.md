# Pinned PicoRV32 fixture

`simpleuart.v` is unmodified from YosysHQ/picorv32 at commit
`ef203c2b0a3fb793280f5114941416c425c5b461`, path `picosoc/simpleuart.v`:
https://github.com/YosysHQ/picorv32/blob/ef203c2b0a3fb793280f5114941416c425c5b461/picosoc/simpleuart.v

Retrieved on 2026-10-20. ISC permission and copyright notices are retained in
the source and COPYING. The locally authored testbench exercises all sixteen
divider byte-write masks with three data patterns, reset, and hold cycles. It
does not validate the UART's serial transmitter or receiver.

The testbench emits a VCD waveform. The C# helper samples settled rising edges
and checks the divider against an independent expected-value register. This
is simulation trace checking, not proof of the entire module.
