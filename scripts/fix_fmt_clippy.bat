cargo fmt --all && ^
cargo clippy --fix --allow-dirty --target riscv32imac-unknown-none-elf --features default -- -D warnings && ^
cargo clippy --fix --allow-dirty --target riscv32imac-unknown-none-elf --features ota -- -D warnings
