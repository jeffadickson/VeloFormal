use crate::solver::TraceFrame;
use std::fmt::Write;
use vf_ir::Module;

/// Write a standard IEEE 1364 VCD waveform file from a formal counterexample or cover trace.
pub fn write_vcd(m: &Module, frames: &[TraceFrame]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "$version VeloFormal $end");
    let _ = writeln!(out, "$timescale 1ns $end");
    let _ = writeln!(out, "$scope module {} $end", m.name);

    let mut ids: Vec<(String, u32, String)> = Vec::new();
    for (i, s) in m.signals.iter().enumerate() {
        if s.name.starts_with('$') {
            continue;
        }
        let id = vcd_id(i);
        let _ = writeln!(out, "$var wire {} {} {} $end", s.width, id, s.name);
        ids.push((s.name.clone(), s.width, id));
    }

    let _ = writeln!(out, "$upscope $end");
    let _ = writeln!(out, "$enddefinitions $end");

    for frame in frames {
        let t_lo = frame.step * 10;
        let t_hi = t_lo + 5;
        let _ = writeln!(out, "#{t_lo}");
        for (name, width, id) in &ids {
            if let Some((_, val)) = frame.values.iter().find(|(n, _)| n == name) {
                let is_clk = m
                    .find_signal(name)
                    .map(|sid| m.signal(sid).is_clock)
                    .unwrap_or(false);
                if is_clk {
                    let _ = writeln!(out, "0{id}");
                } else if *width == 1 {
                    let _ = writeln!(out, "{}{id}", val.bits);
                } else {
                    let _ = writeln!(out, "b{} {id}", val.bits);
                }
            }
        }
        // Toggle clock mid-cycle so waveforms read naturally in GTKWave / Surfer.
        let mut any_clk = false;
        for (name, _, id) in &ids {
            if m.find_signal(name)
                .map(|sid| m.signal(sid).is_clock)
                .unwrap_or(false)
            {
                if !any_clk {
                    let _ = writeln!(out, "#{t_hi}");
                    any_clk = true;
                }
                let _ = writeln!(out, "1{id}");
            }
        }
    }

    out
}

fn vcd_id(mut n: usize) -> String {
    // Printable ASCII 33..=126 (`!`..=`~`)
    let mut s = String::new();
    loop {
        s.push((33 + (n % 94)) as u8 as char);
        n /= 94;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    s
}
