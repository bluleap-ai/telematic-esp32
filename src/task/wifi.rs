use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
    WifiState,
};
use log::{error, info, warn};

use crate::cfg::net_cfg::{WIFI_PSWD, WIFI_SSID};

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    info!("[WiFi] Connection task started");
    info!(
        "[WiFi] Device capabilities: {:?}",
        controller.capabilities()
    );
    info!("[WiFi] Disabling power saving mode");
    controller
        .set_power_saving(esp_wifi::config::PowerSaveMode::None)
        .unwrap();
    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            info!("[WiFi] Already connected. Waiting for disconnect event...");
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            info!("[WiFi] Disconnected. Reconnecting in 5 seconds...");
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: WIFI_SSID.try_into().unwrap(),
                password: WIFI_PSWD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
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
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static, WifiStaDevice>>) {
    info!("[WiFi] Network task started");
    runner.run().await
}
