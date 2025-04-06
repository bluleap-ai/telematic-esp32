//! Responses for General Commands
use atat::atat_derive::{AtatEnum, AtatResp};
use atat::heapless::String;
use atat::heapless_bytes::Bytes;

/// Common response
/// Text string that just return "OK".
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct CommonResponse {
    pub res: String<64>,
}

/// 4.1 Manufacturer identification
/// Text string identifying the manufacturer.
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct ManufacturerId {
    pub id: String<64>,
}

/// Model identification
/// Text string identifying the manufacturer.
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct ModelId {
    pub id: String<64>,
}

/// Software version identification
/// Read a text string that identifies the software version of the module.
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct SoftwareVersion {
    pub id: String<64>,
}

/// Get SIM card status
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct SimCardStatus {
    pub status: String<64>,
}

/// Network Registration Status
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct NetworkRegisStatus {
    pub status: String<64>,
}

/// Network Signal Quality
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct NetworkSignalQuality {
    #[at_arg(position = 0)]
    pub rssi: u8,
    #[at_arg(position = 1)]
    pub ber: u8,
}

/// Network Operator Name
#[allow(dead_code)]
#[derive(Clone, Debug, AtatResp)]
pub struct NetworkOperatorName {
    #[at_arg(position = 0)]
    pub mode: u8,
    #[at_arg(position = 1)]
    pub format: u8,
    #[at_arg(position = 2)]
    pub oper: String<64>,
    #[at_arg(position = 3)]
    pub act: u8,
}

/// GPS data
#[allow(dead_code)]
#[derive(Default, Clone, Debug, AtatResp)]
pub struct GpsData {
    #[at_arg(position = 0)]
    pub gsm_type: atat::heapless_bytes::Bytes<6>,
    #[at_arg(position = 1)]
    pub utc: atat::heapless_bytes::Bytes<10>,
    #[at_arg(position = 2)]
    pub status: char,
    #[at_arg(position = 3)]
    pub latitude: f64,
    #[at_arg(position = 4)]
    pub latitude_direction: char,
    #[at_arg(position = 5)]
    pub longitude: f64,
    #[at_arg(position = 6)]
    pub longtitude_direction: char,
    #[at_arg(position = 7)]
    pub spkm: f64,
    #[at_arg(position = 8)]
    pub heading: f64,
    #[at_arg(position = 9)]
    pub date: atat::heapless_bytes::Bytes<6>,
    #[at_arg(position = 10)]
    pub magnetic: atat::heapless_bytes::Bytes<5>,
    #[at_arg(position = 11)]
    pub magnetic_direction: char,
    #[at_arg(position = 12)]
    pub checksum: atat::heapless_bytes::Bytes<4>,
}

/// Echo on
#[derive(Debug, Clone, PartialEq, AtatEnum)]
#[repr(u8)]
pub enum EchoOn {
    ///  Unit does not echo the characters in command mode
    Off = 0,
    /// Unit echoes the characters in command mode. (default)
    On = 1,
}

/// Functionality level of UE
#[derive(Debug, Clone, PartialEq, AtatEnum)]
#[repr(u8)]
pub enum FunctionalityLevelOfUE {
    /// Minimum functionality
    Minimum = 0,
    /// Full functionality (default)
    Full = 1,
    /// Disable modem both transmit and receive RF circuits
    DisableRF = 4,
}

/// Configure RATs Searching Sequence effect
#[derive(Debug, Clone, PartialEq, AtatEnum)]
#[repr(u8)]
pub enum ConfigurationEffect {
    /// After reboot
    AfterReboot = 0,
    /// immediately
    Immediately = 1,
}

/// Echo on
#[derive(Debug, Clone, PartialEq, AtatEnum)]
#[repr(u8)]
pub enum PowerDownMode {
    ///  Immediately power down
    Immediate = 0,
    /// Normal power down (default)
    Normal = 1,
}

#[derive(Debug, Clone, AtatResp)]
pub struct NoResponse;

#[derive(Clone, AtatResp)]
#[allow(dead_code)]
pub struct OkResponse {
    pub code: String<2>,
}

#[derive(Clone, AtatResp)]
pub struct Ready;

#[derive(Clone, AtatResp)]
pub struct AppReady;

#[derive(Clone, AtatResp, Debug)]
pub struct MessageWaitingIndication;

/// Imei
///
/// International Mobile Equipment Identity (IMEI) number of the module.
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct Imei {
    pub imei: Bytes<15>,
}

/// ICCID
///
/// Integrated Circuit Card Identifier number of the (U)SIM card.
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct Iccid {
    pub iccid: Bytes<20>,
}

/// Firmware version identification
///
/// Returns the firmware version of the module.
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct VersionInfo {
    #[at_arg(position = 0)]
    pub code: Bytes<64>,
}

/// Network Information
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct NetworkInfo {
    /// Access technology
    /// String type. Access technology selected.
    /// "No Service"
    /// "GSM"
    /// "GPRS"
    /// "EDGE"
    /// "eMTC"
    /// "NBIoT"
    #[at_arg(position = 1)]
    pub act: String<32>,
    /// Operator
    /// String type. Operator name in numeric format.
    #[at_arg(position = 2)]
    pub oper: Option<String<32>>,
    /// Band
    /// String type. Band selected.
    /// "GSM 850"
    /// "GSM 900"
    /// "GSM 1800"
    /// "GSM 1900"
    /// "LTE BAND 1" – "LTE BAND 85"
    #[at_arg(position = 3)]
    pub band: Option<String<32>>,
    /// Channel
    /// Integer type. Channel selected.
    #[at_arg(position = 4)]
    pub channel: Option<u32>,
}

/// Network Registration Status (LTE-M)
/// When <n>=0, 1, or 2 and the command is executed successfully:
/// +CEREG: <n>,<stat>[,[<tac>],[<ci>],[<AcT>[,<cause_type>,<reject_cause>]]]
///
/// When <n>=4 and the command is executed successfully :
/// +CEREG: <n>,<stat>[,[<tac>],[<ci>],[<AcT>][,[<cause_type>],[<reject_cause>][,[<Active-Time>],[<Periodic-TAU>]]]]
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct EPSNetworkRegistrationStatusResponse {
    /// <n>
    /// Integer type. The type of unsolicited result code presentation.
    /// 0 Disable network registration unsolicited result code
    /// 1 Enable network registration unsolicited result code: +CEREG: <stat>
    /// 2 Enable network registration and location information unsolicited result code:
    ///   +CEREG: <stat>[,[<tac>],[<ci>],[<AcT>]]
    /// 4 For a UE that has applied PSM, and network assigns T3324 to UE, enable
    /// network registration and location information unsolicited result code:
    ///   +CEREG: <stat>[,[<tac>],[<ci>],[<AcT>][,,[,[<Active-Time>],[<Periodic-TAU>]]]]
    #[at_arg(position = 1)]
    pub n: u8,
    /// <stat>
    /// Integer type. The EPS network registration status.
    /// 0: Not registered, ME is not currently searching a new operator to register to
    /// 1: Registered, home network
    /// 2: Not registered, but ME is currently searching a new operator to register to
    /// 3: Registration denied
    /// 4: Unknown (e.g. out of E-UTRAN coverage)
    /// 5: Registered, roaming
    #[at_arg(position = 2)]
    pub stat: u8,
    /// <tac>
    /// String type. Two-byte tracking area code in hexadecimal format.
    #[at_arg(position = 3)]
    pub tac: Option<String<4>>,
    /// <ci>
    /// String type. Four-byte E-UTRAN cell ID in hexadecimal format.
    #[at_arg(position = 4)]
    pub ci: Option<String<8>>,
    /// <AcT>
    /// Integer type. The access technology.
    /// 0: GSM (Not applicable)
    /// 8: eMTC
    /// 9: NB-IoT
    #[at_arg(position = 5)]
    pub act: Option<u8>,
    /// <cause_type>
    /// Integer type. The type of <reject_cause>.
    /// 0: Indicates that <reject_cause> contains an EMM cause value.
    /// 1: Indicates that <reject_cause> contains a manufacturer-specific cause.
    #[at_arg(position = 6)]
    pub cause_type: Option<u8>,
    /// <reject_cause>
    /// Integer type. Contains the cause of the failed registration. The value is of type as
    /// defined by <cause_type>.
    #[at_arg(position = 7)]
    pub reject_cause: Option<u8>,
    /// <Active-Time>
    /// String type. One byte in an 8-bit format. Active Time value (T3324) to be allocated to
    /// the UE. (e.g. "00001111" equals to 1 minute)
    /// Bits 5 to 1 represent the binary coded timer value.
    /// Bits 6 to 8 define the timer value unit as follows:
    /// Bits
    /// 8 7 6
    /// 0 0 0 value is incremented in multiples of 2 seconds
    /// 0 0 1 value is incremented in multiples of 1 minute
    /// 0 1 0 value is incremented in multiples of decihours
    /// 1 1 1 value indicates that the timer is deactivated.
    #[at_arg(position = 8)]
    pub active_time: Option<String<8>>,
    /// <Periodic-TAU>
    /// String type. One byte in an 8-bit format. Extend periodic TAU value (T3412_ext) to
    /// be allocated to the UE in E-UTRAN.
    /// (e.g. "00001010" equals to 100 minutes)
    /// Bits 5 to 1 represent the binary coded timer value.
    /// Bits 6 to 8 define the timer value unit as follows:
    /// Bits
    /// 8 7 6
    /// 0 0 0 value is incremented in multiples of 10 minutes
    /// 0 0 1 value is incremented in multiples of 1 hour
    /// 0 1 0 value is incremented in multiples of 10 hours
    /// 0 1 1 value is incremented in multiples of 2 seconds
    /// 1 0 0 value is incremented in multiples of 30 seconds
    /// 1 0 1 value is incremented in multiples of 1 minute
    #[at_arg(position = 9)]
    pub periodic_tau: Option<String<8>>,
}

/// Network Registration Status (GPRS)
///
/// When <n>=0, 1, or 2 and the command is executed successfully:
///   +CGREG: <n>,<stat>[,[<lac>],[<ci>],[<AcT>],[<rac>][,<cause_type>,<reject_cause>]]
///
/// When <n>=4 and the command is executed successfully :
///   +CGREG: <n>,<stat>[,[<lac>],[<ci>],[<AcT>],[<rac>][,[<cause_type>],[<reject_cause>][,[<Active-Time>],[<Periodic-RAU>],[<GPRS-READY-timer>]]]]
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct EGPRSNetworkRegistrationStatusResponse {
    /// <n>
    /// Integer type. The type of unsolicited result code presentation.
    /// 0 Disable network registration unsolicited result code
    /// 1 Enable network registration unsolicited result code: +CGREG: <stat>
    /// 2 Enable network registration and location information unsolicited result code:
    ///   +CGREG: <stat>[,[<lac>],[<ci>],[<AcT>],[<rac>]]
    /// 4 For a UE that has applied PSM, and network assigns T3324 to UE, enable
    /// network registration and location information unsolicited result code:
    ///   +CGREG: <stat>[,[<lac>],[<ci>],[<AcT>],[<rac>][,,[,[<Active-Time>],[<Periodic-RAU>],[<GPRS-READY-timer>]]]]
    #[at_arg(position = 1)]
    pub n: u8,
    /// <stat>
    /// Integer type. The EGPRS network registration status.
    /// 0: Not registered, MT is not currently searching a new operator to register to
    /// 1: Registered, home network
    /// 2: Not registered, but MT is currently searching a new operator to register to
    /// 3: Registration denied
    /// 4: Unknown (e.g. out of GERAN coverage)
    /// 5: Registered, roaming
    #[at_arg(position = 2)]
    pub stat: u8,
    /// <lac>
    /// String type. Two-byte location area code in hexadecimal format.
    #[at_arg(position = 3)]
    pub lac: Option<String<4>>,
    /// <ci>
    /// String type. Four-byte cell ID in hexadecimal format.
    #[at_arg(position = 4)]
    pub ci: Option<String<8>>,
    /// <AcT>
    /// Integer type. The access technology.
    /// 0: GSM
    /// 8: eMTC (Not applicable)
    /// 9: NB-IoT (Not applicable)
    #[at_arg(position = 5)]
    pub act: Option<u8>,
    /// <rac>
    /// Integer type. Routing Area Code.
    #[at_arg(position = 6)]
    pub rac: Option<u8>,
    /// <cause_type>
    /// Integer type. The type of <reject_cause>.
    /// 0: Indicates that <reject_cause> contains an EMM cause value.
    /// 1: Indicates that <reject_cause> contains a manufacturer-specific cause.
    #[at_arg(position = 6)]
    pub cause_type: Option<u8>,
    /// <reject_cause>
    /// Integer type. Contains the cause of the failed registration. The value is of type as
    /// defined by <cause_type>.
    #[at_arg(position = 7)]
    pub reject_cause: Option<u8>,
    /// <Active-Time>
    /// String type. One byte in an 8-bit format. Active Time value (T3312) to be allocated to
    /// the UE. (e.g. "00001111" equals to 1 minute)
    /// Bits 5 to 1 represent the binary coded timer value.
    /// Bits 6 to 8 define the timer value unit as follows:
    /// Bits
    /// 8 7 6
    /// 0 0 0 value is incremented in multiples of 2 seconds
    /// 0 0 1 value is incremented in multiples of 1 minute
    /// 0 1 0 value is incremented in multiples of decihours
    /// 1 1 1 value indicates that the timer is deactivated.
    #[at_arg(position = 8)]
    pub active_time: Option<String<8>>,
    /// <Periodic-RAU>
    /// String type(?) Not documented in the Quectel BG95 AT Commands Manual, should be the same as <Periodic-TAU>
    #[at_arg(position = 9)]
    pub periodic_rau: Option<String<8>>,
    /// <GPRS-READY-timer>
    /// String type(?) Not documented in the Quectel BG95 AT Commands Manual
    #[at_arg(position = 10)]
    pub gprs_ready_timer: Option<String<8>>,
}

/// Signal Information
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct GetSignalStrengthResponse {
    /// String type. Service mode in which the MT will unsolicitedly report the signal strength.
    pub mode: String<32>,
    /// Integer type. Received signal strength, available in GSM and LTE modes.
    pub rssi: Option<i16>,
    /// Integer type. Reference signal received power, available in LTE mode.
    pub lte_rsrp: Option<i16>,
    /// Integer type. Signal to interference plus noise ratio in in 1/5th of a dB, available in LTE mode.
    pub lte_sinr: Option<i16>,
    /// Integer type. Reference signal received quality in dB, available in LTE mode.
    pub lte_rsrq: Option<i16>,
}

/// Information of the current Packet Data Protocol Context
///
/// List of the currently activated contexts and their IP addresses:
/// +QIACT: 1,<context_state>,<context_type>[,<IP_address>]
/// [.....]
/// +QIACT: 16,<context_state>,<context_type>[,<IP_address>]]
///
/// NOTE: we will only parse the first context
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct PDPContextInfo {
    /// <contextID>
    /// Integer type. The PDP context identifier.
    #[at_arg(position = 1)]
    pub context_id: u8,
    /// <context_state>
    /// Integer type. The PDP context state.
    /// 0: Deactivated
    /// 1: Activated
    #[at_arg(position = 2)]
    pub context_state: u8,
    /// <context_type>
    /// Integer type. The PDP context type.
    /// 1: IPV4
    /// 2: IPV6
    #[at_arg(position = 3)]
    pub context_type: u8,
    /// <IP_address>
    /// String type. The IP address.
    #[at_arg(position = 4)]
    pub ip_address: Option<String<64>>,
}

/// Latest Time Synchronized Through NITZ Network
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct NitzTimeResponse {
    /// String type: "<time>,<dst>""
    /// Time format: String type "yy/MM/dd,hh:mm:ss±zz", where characters indicate year (two last
    /// digits), month, day, hour, minutes, seconds and time zone (indicates the difference,
    /// expressed in quarters of an hour, between the local time and GMT; range -48...+48). E.g. 6th
    /// of May 2004, 22:10:00 GMT+2 hours equals “04/05/06,22:10:00+08”.
    /// DST format: Integer type with the daylight saving time.
    #[at_arg(position = 1)]
    pub time_and_dst: String<32>,
}

/// Latest Time Synchronized Through NTP Network
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct NtpTimeResponse {
    /// Error code of operation.
    pub err: u8,
    /// <time>
    /// String type. Format: "yy/MM/dd,hh:mm:ss±zz", where characters indicate year (two last
    /// digits), month, day, hour, minutes, seconds and time zone (indicates the difference,
    /// expressed in quarters of an hour, between the local time and GMT; range -48...+48). E.g. 6th
    /// of May 2004, 22:10:00 GMT+2 hours equals “04/05/06,22:10:00+08”.
    #[at_arg(position = 2)]
    pub time: String<32>,
}

/// MQTT Open Response
///
///
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttOpenResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <result>
    /// Integer type. The result of the operation.
    /// -1 Failed to open network
    /// 0 Opened network successfully
    /// 1 Wrong parameter
    /// 2 MQTT identifier is occupied
    /// 3 Failed to activate PDP
    /// 4 Failed to parse domain name
    /// 5 Network disconnection error
    #[at_arg(position = 2)]
    pub result: i8,
}

/// URC +QMTSTAT response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttStatusResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <err>
    /// Integer type. The status of the MQTT connection.
    /// 1: Connection is closed or reset by the peer.
    /// 2: Sending PINGREQ packet timed or failed.
    /// 3: Sending CONNECT packet timed out or failed.
    /// 4: Received CONNACK packet timed out or failed.
    /// 5: Client sends DISCONNECT packet but server is initiative to close MQTT.
    /// 6: Client is initiative to close MQTT connection due to packet sending failure all the time.
    /// 7: The link is not alive or the server is unavailable.
    #[at_arg(position = 2)]
    pub err: u8,
}

/// URC +QMTCONN response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttConnectResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <result>
    /// Integer type. The result of the operation.
    /// 0: Sent CONNECT packet successfully.
    /// 1: Packet retrasnmission.
    /// 2: Failed to send CONNECT packet.
    #[at_arg(position = 2)]
    pub result: u8,
    /// <ret_code>
    /// Integer type. The return code of the CONNACK packet.
    /// 0: Connection accepted
    /// 1: Connection refused, unacceptable protocol version
    /// 2: Connection refused, identifier rejected
    /// 3: Connection refused, server unavailable
    /// 4: Connection refused, bad user name or password
    /// 5: Connection refused, not authorized
    #[at_arg(position = 3)]
    pub ret_code: u8,
}

/// URC +QMTPUB response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttPublishResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <messageID>
    /// Integer type. The message identifier.
    #[at_arg(position = 2)]
    pub message_id: u16,
    /// <result>
    /// Integer type. The result of the operation.
    /// 0: Sent PUBLISH packet successfully.
    /// 1: Packet retrasnmission.
    /// 2: Failed to send PUBLISH packet.
    #[at_arg(position = 3)]
    pub result: u8,
    /// <value>
    /// Integer type.
    /// If result is 1, the value is the number of retransmissions.
    /// If 0 or 2, the value is not present.
    #[at_arg(position = 4)]
    pub value: Option<u8>,
}

/// URC +QMTDISC response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttDisconnectResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <result>
    /// Integer type. The result of the operation.
    /// -1: Failed to close network
    /// 0: Closed network successfully
    #[at_arg(position = 2)]
    pub result: i8,
}

/// URC +QMTCLOSE response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct MqttCloseResponse {
    /// <tcpconnectID>
    /// Integer type. The MQTT socket identifier from 0 to 5.
    #[at_arg(position = 1)]
    pub tcpconnect_id: u8,
    /// <result>
    /// Integer type. The result of the operation.
    /// -1: Failed to close network
    /// 0: Closed network successfully
    #[at_arg(position = 2)]
    pub result: i8,
}

/// URC +CME ERROR response
///
/// Indicates an error related to mobile equipment or network.
/// +CME ERROR: <err>
/// There are many possible errors, the most common are:
/// 3: Operation not allowed
/// 10: SIM not inserted
/// 11: SIM PIN required
/// 12: SIM PUK required
/// 13: SIM failure
/// 14: SIM busy
/// 15: SIM wrong
/// 16: Incorrect password
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct CmeError {
    /// <err>
    /// Integer type. The error code.
    #[at_arg(position = 1)]
    pub err: u8,
}

/// URC +QFLST response
#[derive(Clone, Debug, AtatResp)]
#[allow(dead_code)]
pub struct FileListResponse {
    #[at_arg(position = 1)]
    pub file_name: String<64>,
    #[at_arg(position = 2)]
    pub size: u32,
}
