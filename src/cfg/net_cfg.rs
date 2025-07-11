use core::ffi::CStr;
// WIFI configuration constants
pub const WIFI_SSID: &str = "BV Public";
pub const WIFI_PSWD: &str = "banviencorp";
// MQTT configuration constants
pub const MQTT_CSTR_SERVER_NAME: &CStr = c"broker.bluleap.ai";
pub const MQTT_SERVER_NAME: &str = "broker.bluleap.ai";
pub const MQTT_SERVER_PORT: u16 = 8883;
pub const MQTT_CLIENT_ID: &str = "mqttx_xxxxxxxx";
pub const MQTT_USR_NAME: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
// MQTT_USR_PASS is a sensitive value, so we store it as a byte array
pub const MQTT_USR_PASS: [u8; 36] = *b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
