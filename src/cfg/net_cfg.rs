use core::ffi::CStr;
// WIFI configuration constants
pub const WIFI_SSID: &str = "BV Public";
pub const WIFI_PSWD: &str = "banviencorp";
// MQTT configuration constants
pub const MQTT_CSTR_SERVER_NAME: &CStr = c"broker.bluleap.ai";
pub const MQTT_SERVER_NAME: &str = "broker.bluleap.ai";
pub const MQTT_SERVER_PORT: u16 = 8883;
pub const MQTT_CLIENT_ID: &str = "9514a033-a92c-4abc-a46f-c3fe1a986c19";
pub const MQTT_USR_NAME: &str = "16b42082-73e1-4bc4-9c50-bb0666fab2b2";
// MQTT_USR_PASS is a sensitive value, so we store it as a byte array
pub const MQTT_USR_PASS: [u8; 36] = *b"f57f9bf3-07b3-4ba5-ae1f-bf6f579e346d";
