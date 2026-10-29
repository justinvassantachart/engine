# Receipt consumer conformance

Run `cargo test -p engine-cpp-receipt-tests` with the repository's Rust toolchain.
This native test crate compiles the actual fail-closed parser and input-closure
implementation. It does not grant a compiler capability or prove Clang observer
completeness. The production profile remains legacy-only until the rebuilt
compiler, private import adapter, and actual-browser acceptance gates pass.
