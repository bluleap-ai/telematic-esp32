use core::ffi::CStr;
// WIFI configuration constants
pub const WIFI_SSID: &str = "Kim Ngan";
pub const WIFI_PSWD: &str = "kimngan1501";
// MQTT configuration constants
pub const MQTT_CSTR_SERVER_NAME: &CStr = c"broker.bluleap.ai";
pub const MQTT_SERVER_NAME: &str = "broker.bluleap.ai";
pub const MQTT_SERVER_PORT: u16 = 8883;
pub const MQTT_CLIENT_ID: &str = "5680ff91-2d1c-4d0a-a8f7-f9c2a2066740";
pub const MQTT_USR_NAME: &str = "bike_test";
pub const MQTT_USR_PASS: [u8; 9] = *b"bike_test";
