use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Sender};
use embassy_time::{Duration, Instant, Timer};
// use esp_wifi::wifi::WifiState;
#[allow(unused_imports)]
use log::{error, info, warn};
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionEvent {
    WiFiConnected,
    WiFiDisconnected,
    LteConnected,
    LteDisconnected,
    LteRegistered,
    LteUnregistered,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveConnection {
    None,
    WiFi,
    Lte,
    Switching,
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectionStatus {
    pub active: ActiveConnection,
    pub wifi_available: bool,
    pub lte_available: bool,
    pub last_switch: Option<Instant>,
    pub switch_count: u32,
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self {
            active: ActiveConnection::None,
            wifi_available: false,
            lte_available: false,
            last_switch: None,
            switch_count: 0,
        }
    }
}

pub static CONN_EVENT_CHAN: Channel<CriticalSectionRawMutex, ConnectionEvent, 16> = Channel::new();
pub static CONN_STATUS_CHAN: Channel<CriticalSectionRawMutex, ConnectionStatus, 4> = Channel::new();
pub static SWITCH_REQUEST_CHAN: Channel<CriticalSectionRawMutex, ActiveConnection, 4> =
    Channel::new();
pub static ACTIVE_CONNECTION_CHAN: Channel<CriticalSectionRawMutex, ActiveConnection, 4> =
    Channel::new();

const SWITCH_DEBOUNCE_TIME: Duration = Duration::from_secs(10);
//const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[embassy_executor::task]
pub async fn net_manager_task(spawner: Spawner) -> ! {
    info!("[NetMgr] Connection manager started");

    let mut status = ConnectionStatus::default();
    let event_receiver = CONN_EVENT_CHAN.receiver();
    let status_sender = CONN_STATUS_CHAN.sender();
    let switch_receiver = SWITCH_REQUEST_CHAN.receiver();
    let active_net_sender = ACTIVE_CONNECTION_CHAN.sender();

    // Start health monitoring tasks
    let health_sender = CONN_EVENT_CHAN.sender();
    let health_receiver = CONN_EVENT_CHAN.receiver();
    spawner.spawn(net_health_monitor(health_sender)).ok();
    spawner.spawn(lte_health_monitor(health_sender)).ok();

    let status_event = health_receiver.try_receive();
    info!("[NetMgr] Current connection status: {:?}", status_event);

    loop {
        // Wait for either a connection event or a periodic health check timeout
        let event_fut = event_receiver.receive();
        let timer_fut = Timer::after(HEALTH_CHECK_INTERVAL);

        embassy_futures::select::select(event_fut, timer_fut).await;

        info!(
            "[NetMgr] Current connection status: {:?}",
            health_receiver.try_receive()
        );
        // Handle connection events
        if let Ok(event) = event_receiver.try_receive() {
            info!("[NetMgr] Received event: {event:?}");

            match event {
                ConnectionEvent::WiFiConnected => {
                    status.wifi_available = true;
                    if should_prefer_wifi(&status) {
                        perform_net_switch(&mut status, ActiveConnection::WiFi).await;
                    }
                }
                ConnectionEvent::WiFiDisconnected => {
                    status.wifi_available = false;
                    if status.active == ActiveConnection::WiFi {
                        if status.lte_available {
                            perform_net_switch(&mut status, ActiveConnection::Lte).await;
                        } else {
                            status.active = ActiveConnection::None;
                        }
                    }
                }
                ConnectionEvent::LteConnected | ConnectionEvent::LteRegistered => {
                    status.lte_available = true;
                    if status.active == ActiveConnection::None && !status.wifi_available {
                        perform_net_switch(&mut status, ActiveConnection::Lte).await;
                    }
                }
                ConnectionEvent::LteDisconnected | ConnectionEvent::LteUnregistered => {
                    status.lte_available = false;
                    if status.active == ActiveConnection::Lte {
                        if status.wifi_available {
                            perform_net_switch(&mut status, ActiveConnection::WiFi).await;
                        } else {
                            status.active = ActiveConnection::None;
                        }
                    }
                }
            }

            // Notify others of status change
            let _ = status_sender.try_send(status);
            let _ = active_net_sender.try_send(status.active);
        }

        // Handle manual switch requests
        if let Ok(requested_connection) = switch_receiver.try_receive() {
            if can_switch_to(&status, requested_connection) {
                perform_net_switch(&mut status, requested_connection).await;
                let _ = status_sender.try_send(status);
                let _ = active_net_sender.try_send(status.active);
            } else {
                warn!("[NetMgr] Cannot switch to {requested_connection:?} - not available");
            }
        }
    }
}

#[embassy_executor::task]
async fn net_health_monitor(
    event_sender: Sender<'static, CriticalSectionRawMutex, ConnectionEvent, 16>,
) -> ! {
    info!("[NetMgr] Health monitor started");

    loop {
        Timer::after(HEALTH_CHECK_INTERVAL).await;
        if esp_wifi::wifi::wifi_state() != esp_wifi::wifi::WifiState::StaConnected {
            info!("[NetMgr] WiFi disconnected");
            let _ = event_sender.try_send(ConnectionEvent::WiFiDisconnected);
        } else {
            info!("[NetMgr] WiFi is connected");
            let _ = event_sender.try_send(ConnectionEvent::WiFiConnected);
        }
    }
}

#[embassy_executor::task]
async fn lte_health_monitor(
    event_sender: Sender<'static, CriticalSectionRawMutex, ConnectionEvent, 16>,
) -> ! {
    info!("[NetMgr] LTE health monitor started");

    loop {
        Timer::after(HEALTH_CHECK_INTERVAL).await;
        // if esp_wifi::wifi::wifi_state() != esp_wifi::wifi::WifiState::StaConnected {
        //     info!("[NetMgr] WiFi disconnected");
        //     let _ = event_sender.try_send(ConnectionEvent::WiFiDisconnected);
        // }
        // Add logic to check LTE status and send events
        // For example:
        if !lte_is_connected() {
            let _ = event_sender.try_send(ConnectionEvent::LteDisconnected);
        }
    }
}

fn lte_is_connected() -> bool {
    // Placeholder for actual LTE connection check logic
    true
}

fn should_prefer_wifi(status: &ConnectionStatus) -> bool {
    status.wifi_available
        && (status.active != ActiveConnection::WiFi)
        && !is_recently_switched(status)
}

fn can_switch_to(status: &ConnectionStatus, target: ActiveConnection) -> bool {
    match target {
        ActiveConnection::WiFi => status.wifi_available,
        ActiveConnection::Lte => status.lte_available,
        ActiveConnection::None => true,
        ActiveConnection::Switching => false,
    }
}

fn is_recently_switched(status: &ConnectionStatus) -> bool {
    if let Some(last_switch) = status.last_switch {
        last_switch.elapsed() < SWITCH_DEBOUNCE_TIME
    } else {
        false
    }
}

async fn perform_net_switch(status: &mut ConnectionStatus, target: ActiveConnection) {
    if status.active == target {
        return;
    }

    info!(
        "[NetMgr] Switching from {:?} to {:?}",
        status.active, target
    );

    let previous = status.active;
    status.active = ActiveConnection::Switching;
    Timer::after(Duration::from_millis(100)).await;
    status.active = target;
    status.last_switch = Some(Instant::now());
    status.switch_count += 1;

    info!(
        "[NetMgr] Successfully switched to {:?} (switch #{}) from {:?}",
        target, status.switch_count, previous
    );
}
