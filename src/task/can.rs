use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use embedded_can::{Frame, Id};
use esp_hal::twai::TwaiRx;
use log::{error, info};

#[derive(Debug)]
#[allow(dead_code)]
pub struct CanFrame {
    pub id: u32,
    pub len: u8,
    pub data: [u8; 8],
}

pub type TwaiOutbox = Channel<NoopRawMutex, CanFrame, 16>;

#[embassy_executor::task]
pub async fn can_receiver(
    mut rx: TwaiRx<'static, esp_hal::Async>,
    channel: &'static TwaiOutbox,
) -> ! {
    info!("[CAN] Can rx task started\r");
    loop {
        let frame = rx.receive_async().await;

        match frame {
            Ok(frame) => {
                // repeat the frame back
                match frame.id() {
                    // ION doesn't work with StandardId
                    Id::Standard(_id) => {}
                    Id::Extended(id) => {
                        let mut data = [0u8; 8];
                        data[0..frame.data().len()].copy_from_slice(frame.data());

                        // if id.as_raw() == MQTT_CAN_PACKET {
                        info!("Receive MQTT CAN packet");
                        // Try to send can frame to wifi task without blocking
                        let _ = channel.try_send(CanFrame {
                            id: id.as_raw(),
                            len: frame.dlc() as u8,
                            data,
                        });
                        // }
                    }
                }
            }
            Err(e) => {
                error!("TWAIT Received error: {e:?}\r");
            }
        }
    }
}
