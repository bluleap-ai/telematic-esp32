use embassy_net::{tcp::TcpSocket, IpEndpoint};
use embassy_time::Instant;
use esp_mbedtls::asynch::Session;
use log::{error, info, warn};
use mqttrust::{
    encoding::v4::{encode_slice, Connect, Protocol},
    MqttError, Packet, Publish, QoS,
};

#[allow(dead_code)]
pub struct MqttClient<'a> {
    client_id: &'a str,
    session: Session<'a, TcpSocket<'a>>,
    connection_state: bool,
    recv_buffer: [u8; 512],
    recv_index: usize,
    keep_alive_secs: Option<u16>,
    last_sent_millis: u64,
}
#[allow(dead_code)]
impl<'a> MqttClient<'a> {
    pub fn new(client_id: &'a str, session: Session<'a, TcpSocket<'a>>) -> Self {
        MqttClient {
            client_id,
            session,
            connection_state: false,
            recv_buffer: [0u8; 512],
            recv_index: 0,
            keep_alive_secs: None,
            last_sent_millis: 0,
        }
    }

    pub async fn connect(
        &mut self,
        end_point: IpEndpoint,
        keep_alive_secs: u16,
        username: Option<&'a str>,
        password: Option<&'a [u8]>,
    ) -> Result<(), MqttError> {
        self.keep_alive_secs = Some(keep_alive_secs);

        // if self.session.state() != State::Closed {
        //     self.disconnect();
        // }
        if let Err(e) = self.session.connect().await {
            error!("Failed to connect to {:?}: {:?}", end_point, e);
            return Err(MqttError::Overflow);
        }

        let conn_pkt = Packet::Connect(Connect {
            protocol: Protocol::MQTT311,
            keep_alive: keep_alive_secs,
            client_id: self.client_id,
            clean_session: true,
            last_will: None,
            username,
            password,
        });

        self.last_sent_millis = self.current_millis();
        self.connection_state = true;
        self.send(conn_pkt).await?;
        Ok(())
    }

    pub async fn disconnect(&mut self) {
        let _ = self.session.close().await;
        self.connection_state = false;
    }

    pub async fn publish(
        &mut self,
        topic_name: &str,
        payload: &[u8],
        qos: QoS,
    ) -> Result<(), MqttError> {
        let pub_pkt = Packet::Publish(Publish {
            dup: false,
            qos,
            pid: None,
            retain: false,
            topic_name,
            payload,
        });

        self.send(pub_pkt).await?;
        self.last_sent_millis = self.current_millis();
        Ok(())
    }

    pub async fn poll(&mut self) {
        // if self.session.state() == State::Closed {
        //     self.connection_state = false;
        //     warn!("socket state is closed");
        //     return;
        // }

        if let Some(keep_alive_secs) = self.keep_alive_secs {
            if (self.last_sent_millis + (keep_alive_secs * 1000) as u64) < self.current_millis() {
                let ping_err = self.send(Packet::Pingreq).await;
                match ping_err {
                    Ok(()) => {
                        info!("Ping success");
                        self.last_sent_millis = self.current_millis();
                    }
                    Err(_) => warn!("Ping failed"),
                }
            }
        }
    }

    async fn send(&mut self, packet: Packet<'_>) -> Result<(), MqttError> {
        let mut buffer = [0u8; 4096];
        let len = encode_slice(&packet, &mut buffer).unwrap();
        match self.session.write(&buffer[..len]).await {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to send MQTT: {:?}", e);
                Err(MqttError::Overflow)
            }
        }
    }

    async fn receive(&mut self) -> Result<(), MqttError> {
        match self.session.read(&mut self.recv_buffer).await {
            Ok(len) => {
                self.recv_index = len;
                Ok(())
            }
            Err(_) => Err(MqttError::Overflow),
        }
    }

    fn current_millis(&self) -> u64 {
        Instant::now().as_millis()
    }
}
