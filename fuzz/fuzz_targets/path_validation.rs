#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    dux_core::__fuzz_path_validation_and_policy(input);
});
