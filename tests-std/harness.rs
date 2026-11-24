// Compiles the std-only core modules on their own, so their tests run with
// nothing but rustc (no crates.io access needed).
//   rustc --edition 2021 --test harness.rs -o harness && ./harness
#[path = "../crates/keepsake-core/src/jsonc.rs"]
pub mod jsonc;
#[path = "../crates/keepsake-core/src/fmt.rs"]
pub mod fmt;
#[path = "../crates/keepsake-core/src/parse.rs"]
pub mod parse;
#[path = "../crates/keepsake-core/src/fsmeasure.rs"]
pub mod fsmeasure;
