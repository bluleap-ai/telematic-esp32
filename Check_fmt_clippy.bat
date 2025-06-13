REM  cargo clippy --release  >> log.txt 2>&1
if exist log.txt del log.txt
cargo fmt --all -- --check > log.txt 2>&1
cargo clippy --workspace --release --target riscv32imac-unknown-none-elf --exclude uint_test -- -D warnings >> log.txt 2>&1
cargo clippy --workspace --target riscv32imac-unknown-none-elf --features ota --exclude uint_test -- -D warnings >> log.txt 2>&1
REM Path: build.bat
