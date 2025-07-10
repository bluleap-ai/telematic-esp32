use embassy_net::{
    tcp::{ConnectError, TcpSocket},
    Ipv4Address, Stack,
};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::{RSA, SHA};
use esp_mbedtls::{asynch::Session, Certificates, Mode, Tls, TlsVersion, X509};
// use esp_println::println;
use log::{error, info, warn};
// use esp_println::println;
use log::{error, info, warn};

// use crate::task::lte::TripData;
use embassy_sync::channel::Channel;

use crate::modem::*;

use crate::cfg::net_cfg::*;
use crate::net::{dns::DnsBuilder, mqtt::MqttClient};
use crate::net::{dns::DnsBuilder, mqtt::MqttClient};
use crate::task::can::TwaiOutbox;
// use crate::task::netmgr::CONN_EVENT_CHAN;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_NET};
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

static IS_WIFI: AtomicBool = AtomicBool::new(false);

#[allow(clippy::uninlined_format_args)]
// use crate::task::netmgr::CONN_EVENT_CHAN;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_NET};
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

static IS_WIFI: AtomicBool = AtomicBool::new(false);

#[allow(clippy::uninlined_format_args)]
#[embassy_executor::task]
pub async fn wifi_mqtt_handler(
    stack: &'static Stack<'static>,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    mut sha: SHA,
    mut rsa: RSA,
) -> ! {
    loop {
        if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_NET.receiver().try_receive() {
            IS_WIFI.store(
                active_connection == ActiveConnection::WiFi,
                Ordering::SeqCst,
            );
            info!("[MQTT] Updated IS_WIFI: {}", IS_WIFI.load(Ordering::SeqCst));
        if let Ok(active_connection) = ACTIVE_CONNECTION_CHAN_NET.receiver().try_receive() {
            IS_WIFI.store(
                active_connection == ActiveConnection::WiFi,
                Ordering::SeqCst,
            );
            info!("[MQTT] Updated IS_WIFI: {}", IS_WIFI.load(Ordering::SeqCst));
        }

        // Check if WiFi is active, wait if not
        if !IS_WIFI.load(Ordering::SeqCst) {
            Timer::after(Duration::from_millis(500)).await;
            continue;
        }

        info!("[WIFI] Using WIFI connection");
        // Ensure the stack is connected
        if !stack.is_link_up() {
            Timer::after(Duration::from_millis(500)).await;
            continue;
        } else {
            Timer::after(Duration::from_millis(3000)).await;
        }

        // Perform MQTT operations using the Wi-Fi stack
        let remote_endpoint = loop {
            if let Ok(endpoint) = dns_query(stack).await {
                break endpoint;
            } else {
                warn!("DNS query failed. Retrying...");
                Timer::after_secs(1).await;
            }
        };

        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        } else {
            Timer::after(Duration::from_millis(3000)).await;
        }

        // Perform MQTT operations using the Wi-Fi stack
        let remote_endpoint = loop {
            if let Ok(endpoint) = dns_query(stack).await {
                break endpoint;
            } else {
                warn!("DNS query failed. Retrying...");
                Timer::after_secs(1).await;
            }
        };

        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
        if let Err(e) = socket.connect(remote_endpoint).await {
            error!("[MQTT] Failed to connect: {e:?}");
            continue;
        }

        if let Err(e) = socket.connect(remote_endpoint).await {
            error!("[MQTT] Failed to connect: {e:?}");
            continue;
        }

        let certificates = Certificates {
            ca_chain: X509::pem(concat!(include_str!("../../cert/crt.pem"), "\0").as_bytes()).ok(),
            certificate: X509::pem(concat!(include_str!("../../cert/dvt.crt"), "\0").as_bytes())
                .ok(),
            private_key: X509::pem(concat!(include_str!("../../cert/dvt.key"), "\0").as_bytes())
                .ok(),
            password: None,
        };

        let tls = Tls::new(&mut sha).unwrap().with_hardware_rsa(&mut rsa);
        let session = match Session::new(
        let tls = Tls::new(&mut sha).unwrap().with_hardware_rsa(&mut rsa);
        let session = match Session::new(
            socket,
            Mode::Client {
                servername: MQTT_CSTR_SERVER_NAME,
            },
            TlsVersion::Tls1_3,
            certificates,
            tls.reference(),
        ) {
            Ok(session) => session,
            Err(e) => {
                error!("[MQTT] Failed to establish TLS session: {e:?}");
                continue;
            }
        };

        ) {
            Ok(session) => session,
            Err(e) => {
                error!("[MQTT] Failed to establish TLS session: {e:?}");
                continue;
            }
        };

        let mut mqtt_client = MqttClient::new(MQTT_CLIENT_ID, session);
        if let Err(e) = mqtt_client
        if let Err(e) = mqtt_client
            .connect(
                remote_endpoint,
                60,
                Some(MQTT_USR_NAME),
                Some(&MQTT_USR_PASS),
            )
            .await
        {
            error!("[MQTT] Failed to connect to broker: {e:?}");
            continue;
        }

        info!("[MQTT] Connected to broker");
        {
            error!("[MQTT] Failed to connect to broker: {e:?}");
            continue;
        }

        info!("[MQTT] Connected to broker");
        loop {
            if let Ok(frame) = can_channel.try_receive() {
            if let Ok(frame) = can_channel.try_receive() {
                let mut frame_str: heapless::String<80> = heapless::String::new();
                let mut can_topic: heapless::String<80> = heapless::String::new();

                let mut can_topic: heapless::String<80> = heapless::String::new();

                writeln!(
                    &mut frame_str,
                    "{{\"id\": \"{:08X}\", \"len\": {}, \"data\": \"{:02X?}\"}}",
                    frame.id, frame.len, frame.data
                )
                .unwrap();


                writeln!(
                    &mut can_topic,
                    &mut can_topic,
                    "channels/{}/messages/client/can",
                    MQTT_CLIENT_ID
                )
                .unwrap();

                if let Err(e) = mqtt_client
                    .publish(&can_topic, frame_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .await
                {
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    break;
                }
                info!("[WIFI] MQTT CAN sent OK {frame_str}");
            }

            if let Ok(trip_data) = gps_channel.try_receive() {
                info!("[WIFI] GPS data received from channel: {trip_data:?}");
                let mut trip_payload: heapless::String<1024> = heapless::String::new();
                let mut buf: [u8; 1024] = [0u8; 1024];
                let mut trip_topic: heapless::String<80> = heapless::String::new();
                let mut trip_str: heapless::String<1024> = heapless::String::new();

                // Serialize to JSON
                if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                    let json = core::str::from_utf8(&buf[..len])
                        .unwrap_or_default()
                        .replace('\"', "'");

                    if trip_payload.push_str(&json).is_err() {
                        error!("[WIFI] Payload buffer overflow");
                    }
                } else {
                    error!("[WIFI] Failed to serialize trip data");
                }
                writeln!(
                    &mut trip_topic,
                    "channels/{}/messages/client/trip",
                    MQTT_CLIENT_ID
                )
                .unwrap();

                writeln!(&mut trip_str, "{trip_payload}").unwrap();

                info!("[WIFI] MQTT payload (trip): {trip_str}");


                if let Err(e) = mqtt_client
                    .publish(&can_topic, frame_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .await
                {
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    break;
                }
                info!("[WIFI] MQTT CAN sent OK {frame_str}");
            }

            if let Ok(trip_data) = gps_channel.try_receive() {
                info!("[WIFI] GPS data received from channel: {trip_data:?}");
                let mut trip_payload: heapless::String<1024> = heapless::String::new();
                let mut buf: [u8; 1024] = [0u8; 1024];
                let mut trip_topic: heapless::String<80> = heapless::String::new();
                let mut trip_str: heapless::String<1024> = heapless::String::new();

                // Serialize to JSON
                if let Ok(len) = serde_json_core::to_slice(&trip_data, &mut buf) {
                    let json = core::str::from_utf8(&buf[..len])
                        .unwrap_or_default()
                        .replace('\"', "'");

                    if trip_payload.push_str(&json).is_err() {
                        error!("[WIFI] Payload buffer overflow");
                    }
                } else {
                    error!("[WIFI] Failed to serialize trip data");
                }
                writeln!(
                    &mut trip_topic,
                    "channels/{}/messages/client/trip",
                    MQTT_CLIENT_ID
                )
                .unwrap();

                writeln!(&mut trip_str, "{trip_payload}").unwrap();

                info!("[WIFI] MQTT payload (trip): {trip_str}");

                if let Err(e) = mqtt_client
                    .publish(&trip_topic, trip_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .publish(&trip_topic, trip_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .await
                {
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    break;
                }
                info!("[WIFI] MQTT GPS sent OK");
            }


            mqtt_client.poll().await;
            Timer::after_secs(1).await;
        }
    }
}

pub async fn dns_query(
    stack: &'static Stack<'static>,
) -> Result<embassy_net::IpEndpoint, ConnectError> {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));
    let mut buffer = [0; 512];
    let dns_ip = Ipv4Address::new(8, 8, 8, 8);
    let remote_endpoint = embassy_net::IpEndpoint {
        addr: embassy_net::IpAddress::Ipv4(dns_ip),
        port: 53,
    };
    socket.connect(remote_endpoint).await?;
    let dns_builder = DnsBuilder::build(MQTT_SERVER_NAME);
    socket.write(&dns_builder.query_data()).await.unwrap();

    let size = socket.read(&mut buffer).await.unwrap();
    let broker_ip = if size > 2 {
        if let Ok(ips) = DnsBuilder::parse_dns_response(&buffer[2..size]) {
            info!("broker IP: {}.{}.{}.{}", ips[0], ips[1], ips[2], ips[3]);
            ips
        } else {
            return Err(ConnectError::NoRoute);
        }
    } else {
        return Err(ConnectError::NoRoute);
    };

    let broker_ipv4 = Ipv4Address::new(broker_ip[0], broker_ip[1], broker_ip[2], broker_ip[3]);

    let remote_endpoint = embassy_net::IpEndpoint {
        addr: embassy_net::IpAddress::Ipv4(broker_ipv4),
        port: MQTT_SERVER_PORT,
    };
    Ok(remote_endpoint)
}
