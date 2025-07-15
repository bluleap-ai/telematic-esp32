use embassy_net::{
    tcp::{ConnectError, TcpSocket},
    Ipv4Address, Stack,
};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::{RSA, SHA};
use esp_mbedtls::{asynch::Session, Certificates, Mode, Tls, TlsVersion, X509};
// use esp_println::println;
use log::{error, info, warn};

// use crate::task::lte::TripData;
use embassy_sync::channel::Channel;

use crate::modem::*;

use crate::cfg::net_cfg::*;
use crate::net::{dns::DnsBuilder, mqtt::MqttClient};
use crate::task::can::TwaiOutbox;
// use crate::task::netmgr::CONN_EVENT_CHAN;
use crate::task::netmgr::{ActiveConnection, ACTIVE_CONNECTION_CHAN_NET};
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

static IS_WIFI: AtomicBool = AtomicBool::new(false);

/// Wifi MQTT handler task
/// This task performs:
///
/// 1.Monitors if WiFi or LTE should be used for MQTT
/// 2.Ensures WiFi network stack is connected and stable
/// 3.Resolves the MQTT broker's hostname to an IP address
/// 4.Establishes a TCP connection to the MQTT broker
/// 5.Creates an encrypted TLS 1.3 session with client certificates
/// 6.Authenticates and connects to the MQTT broker
/// 7.Continuously publishes CAN and GPS data as JSON
/// 8.On any failure, restarts the entire connection process
#[allow(clippy::uninlined_format_args)]
#[embassy_executor::task]
pub async fn wifi_mqtt_handler(
    stack: &'static Stack<'static>,
    can_channel: &'static TwaiOutbox,
    gps_channel: &'static Channel<NoopRawMutex, TripData, 8>,
    mut sha: SHA,
    mut rsa: RSA,
) -> ! {
    info!("[WIFI] MQTT task");
    let tls = Tls::new(&mut sha).unwrap().with_hardware_rsa(&mut rsa);
    loop {
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

        // TCP Connection Establishment
        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 4096];
        let mut socket = TcpSocket::new(*stack, &mut rx_buffer, &mut tx_buffer);
        // Attempt to connect to the MQTT broker
        if let Err(e) = socket.connect(remote_endpoint).await {
            error!("[MQTT] Failed to connect: {e:?}");
            continue;
        }

        let certificates = Certificates {
            ca_chain: X509::pem(concat!(include_str!("../../certs/crt.pem"), "\0").as_bytes()).ok(),
            certificate: X509::pem(concat!(include_str!("../../certs/dvt.crt"), "\0").as_bytes())
                .ok(),
            private_key: X509::pem(concat!(include_str!("../../certs/dvt.key"), "\0").as_bytes())
                .ok(),
            password: None,
        };

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

        let mut mqtt_client = MqttClient::new(MQTT_CLIENT_ID, session);
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
        // Inner loop continuously publishes telematic data to the broker
        loop {
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

            if let Ok(frame) = can_channel.try_receive() {
                let mut frame_str: heapless::String<80> = heapless::String::new();
                let mut can_topic: heapless::String<80> = heapless::String::new();

                if writeln!(
                    &mut frame_str,
                    "{{\"id\": \"{:08X}\", \"len\": {}, \"data\": \"{:02X?}\"}}",
                    frame.id, frame.len, frame.data
                )
                .is_err()
                {
                    error!("[WIFI] Failed to format CAN frame JSON");
                    continue;
                }

                if writeln!(
                    &mut can_topic,
                    "channels/{}/messages/client/can",
                    MQTT_CLIENT_ID
                )
                .is_err()
                {
                    error!("[WIFI] Failed to format CAN topic string");
                    continue;
                }

                if let Err(e) = mqtt_client
                    .publish(&can_topic, frame_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .await
                {
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    mqtt_client.disconnect().await;
                    // continue;
                }
                info!("[WIFI] MQTT CAN sent OK {frame_str}");
                info!("[WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI]");
                info!("                                                SUCCESS");
                info!("[WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI]");
            }
            // GPS/Trip data processing
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

                if writeln!(
                    &mut trip_topic,
                    "channels/{}/messages/client/trip",
                    MQTT_CLIENT_ID
                )
                .is_err()
                {
                    error!("[WIFI] Failed to format trip topic string");
                    continue;
                }

                if writeln!(&mut trip_str, "{trip_payload}").is_err() {
                    error!("[WIFI] Failed to format trip payload string");
                    continue;
                };

                info!("[WIFI] MQTT payload (trip): {trip_str}");

                if let Err(e) = mqtt_client
                    .publish(&trip_topic, trip_str.as_bytes(), mqttrust::QoS::AtMostOnce)
                    .await
                {
                    error!("[WIFI] Failed to publish MQTT packet: {e:?}");
                    mqtt_client.disconnect().await;
                    // continue;
                    break;
                }
                info!("[WIFI] MQTT GPS sent OK");
                info!("[WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI]");
                info!("                                                SUCCESS");
                info!("[WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI][WIFI]");
            }
            // Connection maintainance
            mqtt_client.poll().await;
            Timer::after_secs(1).await;
        }
    }
}

/// Performs DNS resolution to get the IP address of the MQTT broker
///
/// This function manually implements DNS resolution by:
/// 1. Connecting to Google's DNS server (8.8.8.8)
/// 2. Sending a DNS query for the broker hostname
/// 3. Parsing the response to extract the IP address
/// 4. Returning an endpoint with the resolved IP and MQTT port
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
    // Connect to DNS server
    socket.connect(remote_endpoint).await?;
    let dns_builder = DnsBuilder::build(MQTT_SERVER_NAME);

    if let Err(e) = socket.write(&dns_builder.query_data()).await {
        error!("[DNS] Failed to write DNS query: {e:?}");
        return Err(ConnectError::NoRoute);
    }

    let size = match socket.read(&mut buffer).await {
        Ok(s) => s,
        Err(e) => {
            error!("[DNS] Failed to read DNS response: {e:?}");
            return Err(ConnectError::NoRoute);
        }
    };

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
