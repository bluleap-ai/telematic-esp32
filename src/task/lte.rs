use crate::modem::*;
use crate::task::can::*;
use crate::task::netmgr::get_active_connection;
use crate::task::netmgr::ActiveConnection;
use crate::task::netmgr::LTE_IS_CONNECTED;
use core::sync::atomic::Ordering;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use log::{error, info};
/// Task to handle LTE MQTT operations and health checks using a state machine.
///
/// Follows a sequence of modem initialization, GPS setup, LTE connectivity, GPS data
/// retrieval, health checks, and MQTT publishing. Transitions to Error state on failures
/// and attempts recovery. Listens for health check requests on `CHECK_LTE_HEALTH_CHAN`
/// and active connection updates on `ACTIVE_CONNECTION_CHAN_LTE`. Sends connection
/// events to `CONN_EVENT_CHAN`.
#[embassy_executor::task]
pub async fn lte_mqtt_handler(
    mqtt_client_id: &'static str,
    mut modem: Modem,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    ca_chain: &'static [u8],
    certificate: &'static [u8],
    private_key: &'static [u8],
) -> ! {
    // Initialize LTE
    loop {
        info!("Initializing LTE...");
        if let Err(e) = modem
            .lte_init("mqtt_client_id", ca_chain, certificate, private_key)
            .await
        {
            error!("LTE initialization failed: {e:?}");
        }
        Timer::after(Duration::from_secs(1)).await;
        // Initialize GPS
        info!("Initializing GPS...");
        if let Err(e) = modem.gps_init().await {
            error!("GPS initialization failed: {e:?}");
        } else {
            info!("GPS initialized successfully");
        }
        loop {
            // Get GPS data and send to channel
            info!("Retrieving GPS data...");
            let tripdata = match modem.get_gps(mqtt_client_id).await {
                //should be refactor
                Err(e) => {
                    let device_id_1: heapless::String<36> = heapless::String::new();
                    let trip_id_1: heapless::String<36> = heapless::String::new();
                    error!("Failed to retrieve GPS data: {e:?}");
                    TripData {
                        device_id: device_id_1,
                        trip_id: trip_id_1,
                        latitude: 0f64,
                        longitude: 0f64,
                        timestamp: 0,
                    }
                }
                Ok(tripdata) => {
                    info!("GPS data retrieved successfully: {tripdata:?}");
                    tripdata
                }
            };
            // Send to channel
            info!("Sending GPS data to channel...");
            match modem.send_gps_to_channel(tripdata, gps_channel).await {
                Ok(()) => info!("GPS data sent to channel successfully"),
                Err(e) => error!("Failed to send GPS data to channel: {e:?}"),
            }

            Timer::after(Duration::from_secs(1)).await;
            let active_connection = get_active_connection().await;
            if active_connection == ActiveConnection::WiFi {
                info!("[LTE] Wifi is on. Prefer Wifi over LTE");
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
            // Initialize MQTT
            info!("Initializing MQTT...");
            match modem.init_mqtt_over_lte().await {
                Err(e) => {
                    error!("Failed to initialize MQTT: {e:?}");
                    LTE_IS_CONNECTED.store(false, Ordering::SeqCst);
                    break;
                }
                Ok(()) => {
                    info!("MQTT initialized successfully");
                    LTE_IS_CONNECTED.store(true, Ordering::SeqCst);
                }
            }
            // Public GPS to server
            info!("Publishing GPS data to server...");
            match modem
                .push_gps_data_to_server(gps_channel, mqtt_client_id)
                .await
            {
                Ok(()) => info!("GPS data published to server successfully"),
                Err(e) => error!("Failed to publish GPS data to server: {e:?}"),
            }
            Timer::after_secs(1).await;

            // Publish CAN data to server
            info!("Publishing CAN data to server...");
            match modem
                .push_can_data_to_server(can_channel, mqtt_client_id)
                .await
            {
                Ok(()) => info!("CAN data published to server successfully"),
                Err(e) => error!("Failed to publish CAN data to server: {e:?}"),
            }
            Timer::after_secs(1).await;
        }
        Timer::after_secs(1).await;
    }
}
