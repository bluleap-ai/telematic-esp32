REM  cargo clippy --release  >> log.txt 2>&1
if exist log.txt del log.txt
cargo fmt --all -- --check > log.txt 2>&1
cargo clippy --target riscv32imac-unknown-none-elf --features default -- -D warnings >> log.txt 2>&1
cargo clippy --target riscv32imac-unknown-none-elf --features ota -- -D warnings >> log.txt 2>&1
REM Path: build.bat
