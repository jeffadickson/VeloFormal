#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Elaborate one or more Verilog/SystemVerilog files using embedded slang.
///
/// `argv` is an array of `argc` null-terminated UTF-8 strings representing
/// slang CLI flags and source files (e.g. `["veloformal", "-DFORMAL", "foo.sv"]`).
///
/// Returns 0 on success (with `*out_json` populated), or non-zero on error
/// (with `*out_diag` populated). Both non-null output pointers must be freed
/// with `vf_slang_free_string`.
int32_t vf_slang_elaborate(
    int32_t argc,
    const char* const* argv,
    char** out_json,
    char** out_diag);

/// Free a string allocated by `vf_slang_elaborate`.
void vf_slang_free_string(char* ptr);

#ifdef __cplusplus
}
#endif
