use crate::cfg::net_cfg::{WIFI_PSWD, WIFI_SSID};
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::WifiState;
use esp_wifi::wifi::{ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent};
use log::{error, info, warn};
#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    info!("[WiFi] Connection task started");
    info!(
        "[WiFi] Device capabilities: {:?}",
        controller.capabilities()
    );
    info!("[WiFi] Disabling power saving mode");
    if let Err(e) = controller.set_power_saving(esp_wifi::config::PowerSaveMode::None) {
        warn!("[WiFi] Failed to disable power saving mode: {e:?}");
        // This is not critical for operation so we can continue
    }
    loop {
        // Check if already connected to avoid unnecessary reconnection attempts
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            info!("[WiFi] Already connected. Waiting for disconnect event...");
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            info!("[WiFi] Disconnected. Reconnecting in 5 seconds...");
            Timer::after(Duration::from_millis(5000)).await
        }
        // Check if the WiFi controller needs to be started or configured
        if !matches!(controller.is_started(), Ok(true)) {
            // String<32> is because in ClientConfiguration need ssid to be String<32>
            let ssid = match WIFI_SSID.try_into() {
                Ok(ssid) => ssid,
                Err(e) => {
                    error!("[WiFi] Invalid SSID format: {e:?}");
                    continue; // Retry the connection loop
                }
            };
            // String<64> is because in ClientConfiguration need password to be String<64>
            let password = match WIFI_PSWD.try_into() {
                Ok(pwd) => pwd,
                Err(e) => {
                    error!("[WiFi] Invalid password format: {e:?}");
                    continue; // Retry the connection loop
                }
            };
            let client_config = Configuration::Client(ClientConfiguration {
                ssid,
                password,
                ..Default::default()
            });

            if let Err(e) = controller.set_configuration(&client_config) {
                warn!("[WiFi] Failed to set WiFi configuration: {e:?}");
                continue; // Retry the connection loop
            }

            // Attempt to connect to the configured WiFi network
            info!("[WiFi] Starting WiFi STA for SSID: {WIFI_SSID}");
            if let Err(e) = controller.start_async().await {
                warn!("[WiFi] Failed to start controller: {e:?}");
                continue;
            }
        }
        info!("[WiFi] Attempting to connect to SSID: {WIFI_SSID}...");

        match controller.connect_async().await {
            Ok(_) => info!("[WiFi] Successfully connected to SSID: {WIFI_SSID}"),
            Err(e) => {
                error!("[WiFi] Failed to connect to SSID: {WIFI_SSID}: {e:?}");
                info!("[WiFi] Retrying in 5 seconds...");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    info!("[WiFi] Network task started");
    runner.run().await
}
