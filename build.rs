use std::env;
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::Path;

fn generate_net_cfg() {
    // Tell Cargo to rebuild if any environment variables change
    println!("cargo:rerun-if-env-changed=WIFI_PSWD");
    println!("cargo:rerun-if-env-changed=WIFI_SSID");
    println!("cargo:rerun-if-env-changed=MQTT_SERVER_NAME");
    println!("cargo:rerun-if-env-changed=MQTT_SERVER_PORT");
    println!("cargo:rerun-if-env-changed=MQTT_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=MQTT_USR_NAME");
    println!("cargo:rerun-if-env-changed=MQTT_USR_PASS");

    // Read environment variables
    let ssid = env::var("WIFI_SSID").expect("WIFI_SSID not set");
    let pswd = env::var("WIFI_PSWD").expect("WIFI_PSWD not set");
    let server_name = env::var("MQTT_SERVER_NAME").expect("MQTT_SERVER_NAME not set");
    let server_port = env::var("MQTT_SERVER_PORT")
        .expect("MQTT_SERVER_PORT not set")
        .parse::<u16>()
        .expect("Invalid port number");
    let client_id = env::var("MQTT_CLIENT_ID").expect("MQTT_CLIENT_ID not set");
    let usr_name = env::var("MQTT_USR_NAME").expect("MQTT_USR_NAME not set");
    let usr_pass = env::var("MQTT_USR_PASS").expect("MQTT_USR_PASS not set");

    // Prepare the generated Rust code
    let generated_file = "src/cfg/net_cfg.rs";
    let generated_code = format!(
        r#"use core::ffi::CStr;
// WIFI configuration constants
pub const WIFI_SSID: &str = "{ssid}";
pub const WIFI_PSWD: &str = "{pswd}";
// MQTT configuration constants
pub const MQTT_CSTR_SERVER_NAME: &CStr = c"{server_name}";
pub const MQTT_SERVER_NAME: &str = "{server_name}";
pub const MQTT_SERVER_PORT: u16 = {server_port};
pub const MQTT_CLIENT_ID: &str = "{client_id}";
pub const MQTT_USR_NAME: &str = "{usr_name}";
pub const MQTT_USR_PASS: [u8; 9] = *b"{usr_pass}";
"#,
        server_name = server_name,
        server_port = server_port,
        client_id = client_id,
        usr_name = usr_name,
        usr_pass = usr_pass
    );

    // Ensure the target directory exists before writing the file
    let dir_path = Path::new("src/cfg");
    if !dir_path.exists() {
        create_dir_all(dir_path).expect("Failed to create cfg directory");
    }

    // Write the generated code to the net_cfg.rs file
    let mut file = File::create(generated_file).expect("Failed to create net_cfg.rs file");
    file.write_all(generated_code.as_bytes())
        .expect("Failed to write to net_cfg.rs");

    // Print a message so Cargo knows the file has been generated
    println!("cargo:rerun-if-changed=cfg/net_cfg.rs");
}

fn geneator() {
    // Generate the net_cfg.rs file with the configuration constants
    generate_net_cfg();
}
// This build script generates a Rust source file with constants for MQTT configuration.

fn linker() {
    // Link the C library (if needed)
    println!("cargo:rustc-link-arg-bins=-Tlinkall.x");
}

fn main() {
    // Generate the Rust source file with MQTT configuration constants
    geneator();
    // Linker script for the C library (if needed)
    linker();
}
