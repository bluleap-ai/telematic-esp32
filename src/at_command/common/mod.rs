pub mod general;

use crate::at_command::common::general::responses::*;
use atat::atat_derive::AtatUrc;
use atat::atat_derive::{AtatCmd, AtatResp};

#[derive(Clone, AtatResp)]
pub struct NoResponse;

#[derive(Clone, AtatCmd)]
#[at_cmd("", NoResponse, timeout_ms = 1000)]
pub struct AT;

#[derive(Clone, AtatUrc, Debug)]
pub enum Urc {
    #[at_urc("APP RDY")]
    Ready,
    #[at_urc("RDY")]
    AppReady,
    #[at_urc("+UMWI")]
    MessageWaitingIndication(MessageWaitingIndication),

    #[at_urc("+QNTP")]
    NtpTime(NtpTimeResponse),

    /// MQTT open URC
    /// +QMTOPEN: <link_id>,<result> where <link_id> is the link identifier and <result> is the result of the MQTT Open operation.
    #[at_urc("+QMTOPEN")]
    MqttOpen(MqttOpenResponse),

    /// MQTT status URC
    /// +QMTSTAT: <link_id>,<status> where <link_id> is the link identifier and <status> is the status of the MQTT connection.
    #[at_urc("+QMTSTAT")]
    MqttStatus(MqttStatusResponse),

    /// MQTT connection URC
    /// +QMTCONN: <tcpconnectID>,<result>[,<ret_code>]
    #[at_urc("+QMTCONN")]
    MqttConnect(MqttConnectResponse),

    /// MQTT publish URC
    /// +QMTPUB: <tcpconnectID>,<messageID>,<result>[,<value>]
    #[at_urc("+QMTPUB")]
    MqttPublish(MqttPublishResponse),

    /// MQTT Disconnection URC
    /// +QMTDISC: <tcpconnectID>,<result>
    #[at_urc("+QMTDISC")]
    MqttDisconnect(MqttDisconnectResponse),

    /// MQTT Close URC
    /// +QMTCLOSE: <tcpconnectID>,<result>
    #[at_urc("+QMTCLOSE")]
    MqttClose(MqttCloseResponse),

    /// Power Down URC
    /// +QPOWD: POWERED DOWN
    #[at_urc("POWERED DOWN")]
    PowerDown,

    /// Final result code URC
    /// indicates an error related to mobile equipment or network.
    /// +CME ERROR: <err>
    ///
    /// Between other uses, the "no SIM URC" message is returned as a CME error when
    /// the user sends a AT+CPIN? and no SIM is inserted
    #[at_urc("+CME ERROR")]
    CmeError(CmeError),
}
