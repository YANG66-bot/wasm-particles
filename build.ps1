# 构建: Rust -> wasm32 + wasm-bindgen 生成 JS 胶水层到 web/pkg
# 首次运行前请确保已安装:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --locked   (版本须与 Cargo.toml 中 wasm-bindgen 一致: 0.2.127)
cargo build --release --target wasm32-unknown-unknown
if ($LASTEXITCODE -ne 0) { exit 1 }
wasm-bindgen --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/wasm_particles.wasm
