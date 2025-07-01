//! ### 4 - General Commands
use atat::{atat_derive::AtatCmd, heapless_bytes::Bytes, AtatCmd};
use heapless::String;

use super::response::*;

/// Disable echo mode E0
///
/// Disable echo mode
#[derive(Clone, AtatCmd)]
#[at_cmd("E0", NoResponse)]
pub struct DisableEchoMode;

/// 4.1 Manufacturer identification +CGMI
///
/// Text string identifying the manufacturer.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMI", ManufacturerId)]
pub struct GetManufacturerId;

/// Model identification +CGMM
///
/// Read a text string that identifies the device model.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMM", ModelId)]
pub struct GetModelId;

/// Software version identification +CGMR
///
/// Read a text string that identifies the software version of the module
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMR", SoftwareVersion)]
pub struct GetSoftwareVersion;

/// Full-Functionality Mode +CFUN=1
///
/// Set to Full-Functionality Mode
#[derive(Clone, AtatCmd)]
#[at_cmd("+CFUN=1", CommonResponse)]
pub struct SetFullFuncMode;

/// SIM card status +CPIN?
///
/// Check SIM card status
#[derive(Clone, AtatCmd)]
#[at_cmd("+CPIN?", SimCardStatus)]
pub struct GetSimCardStatus;

/// Software reset for Quectel
///
/// Reset quectel module by software
#[derive(Clone, AtatCmd)]
#[at_cmd("+QRST=1", NoResponse)]
pub struct SoftwareReset;

/// Network Registration Status +CREG?
///
/// Check Network Registration Status
#[derive(Clone, AtatCmd)]
#[at_cmd("+CREG?", NetworkRegisStatus)]
pub struct GetNetworkRegisStatus;

/// Network Signal Quality +CSQ
///
/// Check Network Signal Quality
#[derive(Clone, AtatCmd)]
#[at_cmd("+CSQ", NetworkSignalQuality)]
pub struct GetNetworkSignalQuality;

/// Network Operator Name +COPS?
///
/// Check Network Operator Name
#[derive(Clone, AtatCmd)]
#[at_cmd("+COPS?", NetworkOperatorName)]
pub struct GetNetworkOperatorName;

/// Packet-Switched Network +CGATT=1
///
/// Attach to the Packet-Switched Network
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGATT=1", NoResponse)]
pub struct AttachPacketSwitch;

/// Enable GPS Functionality +QGPS=1
///
/// Enable GPS Functionality
#[derive(Clone, AtatCmd)]
#[at_cmd("+QGPS=1", NoResponse, timeout_ms = 1000, response_code = false)]
pub struct EnableGpsFunc;

/// Enable Assisted GPS Functionality +QAGPS=1
///
/// Enable Assisted GPS
#[derive(Clone, AtatCmd)]
#[at_cmd("+QAGPS=1", NoResponse, response_code = false)]
pub struct EnableAssistGpsFunc;

/// Enable GNSS Functionality +QGPSCFG="gnssconfig",1
///
/// Enable GNSS Functionality
#[derive(Clone, AtatCmd)]
#[at_cmd("+QGPSCFG=\"gnssconfig\",1", NoResponse)]
pub struct EnableGnssFunc;

/// GPS Position Data +QGPSGNMEA="RMC"
///
/// Retrieve GPS Position Data
#[derive(Clone, AtatCmd)]
#[at_cmd("+QGPSGNMEA=\"RMC\"", GpsData)]
pub struct RetrieveGpsRmc;

/// Echo On/Off E
///
/// This command configures whether or not the unit echoes the characters received
/// from the DTE in Command Mode. If <echo_on> is omitted, it turns off the echoing.
#[derive(Debug, PartialEq, Clone, AtatCmd)]
#[at_cmd("E", NoResponse, timeout_ms = 1000, value_sep = false)]
pub struct SetEcho {
    #[at_arg(position = 0)]
    pub on: EchoOn,
}

/// Reset to Factory Default
/// AT&F
/// This command resets all parameters to their factory default values.
///
/// The command responds with OK.
#[derive(Clone, AtatCmd)]
#[at_cmd("&F", NoResponse, timeout_ms = 300)]
pub struct ResetToFactoryDefault;

/// AT+CFUN Set UE Functionality
///
/// This command sets the UE functionality.
#[derive(Debug, PartialEq, Clone, AtatCmd)]
#[at_cmd("+CFUN", NoResponse, timeout_ms = 1000)]
pub struct SetUeFunctionality {
    #[at_arg(position = 0)]
    pub fun: FunctionalityLevelOfUE,
}

/// AT+QGMR Query Firmware Version
///
/// This command is used to query the firmware version of the module.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QGMR", VersionInfo, timeout_ms = 1000)]
pub struct GetVersionInfo;

/// AT+CGMR Query Firmware Version
///
/// This command is used to query the firmware version of the module.
/// It is a new version of QGMR command but only returns the first part
/// of the firmware version ("BG95M3LAR02A03" from "BG95M3LAR02A03_01.012.01.012").
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGMR", VersionInfo, timeout_ms = 1000)]
pub struct GetVersionInfoCGMR;

/// AT+QCFG="band" Band Configuration
///
/// The command is used to configure the modem to narrow down the searchs to the main
/// bands used in Europe:
/// * GSM: 900MHz and 1800MHz
/// * CAT-M: Bands 3, 8, 20
/// * NB-IoT: Bands 3, 8, 20
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG=\"band\",3,80084,80084,1", NoResponse, timeout_ms = 300)]
pub struct ConfigureBandsEurope {}

/// AT+QCFG="nwscanseq" Configure RATs Searching Sequence
///
/// This Write Command configures the searching sequence of RATs or queries the current setting.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG", NoResponse, timeout_ms = 300)]
pub struct ConfigureRatSearchingSequence {
    /// "nwscanseq" fixed string
    #[at_arg(position = 1)]
    pub param: String<16>,
    /// <scanseq>
    /// Numeric String without quotes representing RATs searching sequence, e.g.: 020301 stands for eMTC → NB-IoT → GSM.
    /// 00 Automatic (eMTC → NB-IoT → GSM)
    /// 01 GSM
    /// 02 eMTC
    /// 03 NB-IoT
    #[at_arg(position = 2)]
    pub rat_searching_sequence: Bytes<8>,
    /// <effect>
    /// determines when the command will take effect.
    /// The configurations will be saved automatically (1) or after a reboot (0).
    #[at_arg(position = 3)]
    pub effect: ConfigurationEffect,
}

/// AT+QCFG="nvrestore",0 Restore Factory Configuration
/// This command restores the factory configuration.
/// The command responds with OK.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG=\"nvrestore\",0", NoResponse, timeout_ms = 300)]
pub struct RestoreFactoryConfiguration;

/// AT+QCFG="nwscanmode" Configure RATs Searching Mode
///
/// This Write Command configures the searching mode of RATs or queries the current setting.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG", NoResponse, timeout_ms = 300)]
pub struct ConfigureRatSearchingMode {
    /// "nwscanmode" fixed string
    #[at_arg(position = 1)]
    pub param: String<16>,
    /// <scanmode>
    /// Numeric String without quotes representing RATs searching mode, e.g.: 0 stands for Automatic.
    /// 0 Automatic (GSM and LTE)
    /// 1 GSM only
    /// 3 LTE only
    #[at_arg(position = 2)]
    pub rat_searching_mode: u8,
    /// <effect>
    /// determines when the command will take effect.
    /// The configurations will be saved automatically (1) or after a reboot (0).
    #[at_arg(position = 3)]
    pub effect: ConfigurationEffect,
}

/// AT+QCFG="servicedomain" Configure Service Domain
///
/// This Write Command configures the service domain to be registered or queries the current setting.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG", NoResponse, timeout_ms = 300)]
pub struct ConfigureServiceDomain {
    /// "servicedomain" fixed string
    #[at_arg(position = 1)]
    pub param: String<16>,
    /// <service>
    /// Integer type. Service domain to be registered.
    /// 1 PS only
    /// 2 CS & PS
    #[at_arg(position = 2)]
    pub service_domain: u8,
    /// <effect>
    /// determines when the command will take effect.
    /// The configurations will be saved automatically (1) or after a reboot (0).
    #[at_arg(position = 3)]
    pub effect: ConfigurationEffect,
}

/// AT+QCFG="iotopmode" Configure Network Category to be Searched for under LTE RAT
///
/// This Write Command configures the network category to be searched for under LTE RAT or queries the
/// current setting.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCFG", NoResponse, timeout_ms = 300)]
pub struct ConfigureIotOpMode {
    /// "iotopmode" fixed string
    #[at_arg(position = 1)]
    pub param: String<16>,
    /// <iotopmode>
    /// Integer type. Network category to be searched for under LTE RAT.
    /// 0 eMTC
    /// 1 NB-IoT
    /// 2 eMTC and NB-IoT
    #[at_arg(position = 2)]
    pub mode: u8,
    /// <effect>
    /// determines when the command will take effect.
    /// The configurations will be saved automatically (1) or after a reboot (0).
    #[at_arg(position = 3)]
    pub effect: ConfigurationEffect,
}

/// AT+QICSGP Configure Parameters of a TCP/IP Context
///
/// This command configures the <APN>, <username>, <password> and other parameters of a TCP/IP
/// context.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QICSGP", NoResponse, timeout_ms = 300)]
pub struct ConfigureContext {
    /// <contextid>
    /// Integer type. The context ID. The range is from 1 to 16.
    #[at_arg(position = 0)]
    pub context_id: u8,
    /// <contexttype>
    /// Integer type. The context type.
    /// 1: IPV4
    /// 2: IPV6
    /// 3: IPV4V6
    #[at_arg(position = 1)]
    pub context_type: u8,
    /// <apn>
    /// String type. The APN.
    #[at_arg(position = 2)]
    pub apn: String<64>,
    /// <username>
    /// String type. The username.
    #[at_arg(position = 3)]
    pub username: String<64>,
    /// <password>
    /// String type. The password.
    #[at_arg(position = 4)]
    pub password: String<64>,
    /// <authentication>
    /// Integer type. Authentication methods.
    /// 0 None
    /// 1 PAP
    /// 2 CHAP
    /// 3 PAP or CHAP
    #[at_arg(position = 5)]
    pub authentication: u8,
}

/// AT+QNWINFO Query Network Information
///
/// This command indicates network information such as the access technology selected, the operator, and
/// the band selected.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QNWINFO", NetworkInfo, timeout_ms = 300)]
pub struct GetNetworkInfo;

/// AT+CEREG EPS Network Registration Status
///
/// This command queries the LTE network registration status and controls the presentation of an unsolicited
/// result code +CEREG: <stat> when <n>=1 and there is a change in the MT’s EPS network registration
/// status in E-UTRAN, or unsolicited result code +CEREG: <stat>[,[<tac>],[<ci>],[<AcT>]] when <n>=2
/// and there is a change of the network cell in E-UTRAN.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CEREG?", EPSNetworkRegistrationStatusResponse, timeout_ms = 300)]
pub struct GetEPSNetworkRegistrationStatus;

/// AT+CGREG EGPRS Network Registration Status
///
/// This command queries the EGPRS network registration status and controls the presentation of an
/// unsolicited result code +CGREG: <stat> when <n>=1 and there is a change in the MT’s EGPRS network
/// registration status in GERAN, or unsolicited result code +CGREG: <stat>[,[<lac>],[<ci>],[<AcT>],[<rac>]]
/// when <n>=2 and there is a change of the network cell in GERAN.
#[derive(Clone, AtatCmd)]
#[at_cmd("+CGREG?", EGPRSNetworkRegistrationStatusResponse, timeout_ms = 300)]
pub struct GetEGPRSNetworkRegistrationStatus;

/// AT+QCSQ Query and Report Signal Strength
///
/// The command is used to query and report the signal strength of the current service network. If the MT is
/// registered on multiple networks in different service modes, customers can query the signal strength of
/// networks in each mode. No matter whether the MT is registered on a network or not, the command can be
/// run to query the signal strength or allow the MT to unsolicitedly report the detected signal strength if the
/// MT camps on the network. If the MT is not using any service network or the service mode is uncertain,
/// "NOSERVICE" will be returned as the query result.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCSQ", GetSignalStrengthResponse, timeout_ms = 300)]
pub struct GetSignalStrength;

/// AT+QIACT Activate a PDP Context and query
///
/// Before activating a PDP context with AT+QIACT, the context should be configured by AT+QICSGP. After
/// activation, the IP address can be queried with AT+QIACT?. Although the range of <contextID> is 1–16,
/// the module supports maximum three PDP contexts activated simultaneously under LTE Cat M/EGPRS and
/// maximum two under LTE Cat NB2. Depending on the network, it may take at most 150 seconds to return
/// OK or ERROR after executing AT+QIACT. Before the response is returned, other AT commands cannot be executed.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QIACT", NoResponse, timeout_ms = 150000)]
pub struct ActivatePDPContext {
    /// <contextid>
    /// Integer type. The context ID. The range is from 1 to 16.
    #[at_arg(position = 1)]
    pub context_id: u8,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("+QIACT?", PDPContextInfo, timeout_ms = 300)]
pub struct GetPDPContextInfo;

/// AT+QIACT Deactivate a PDP Context
///
/// This command deactivates a specific context and close all TCP/IP connections set up in this context.
/// Depending on the network, it may take at most 40 seconds to return OK or ERROR after executing
/// AT+QIDEACT. Before the response is returned, other AT commands cannot be executed.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QIDEACT", NoResponse, timeout_ms = 40000)]
pub struct DeactivatePDPContext {
    /// <contextid>
    /// Integer type. The context ID. The range is from 1 to 16.
    #[at_arg(position = 1)]
    pub context_id: u8,
}

/// AT+QLTS Obtain the Latest Time Synchronized Through Network
///
/// The Execution Command returns the latest time synchronized through network.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QLTS", NitzTimeResponse, timeout_ms = 300)]
pub struct GetNetworkNitzTime {
    /// <mode>
    /// Integer type. Query network time mode
    /// 0: Query the latest time that has been synchronized through network
    /// 1: Query the current GMT time calculated from the latest time that has been synchronized through network
    /// 2: Query the current LOCAL time calculated from the latest time that has been synchronized through network
    #[at_arg(position = 1)]
    pub mode: u8,
}

/// AT+QNTP Synchronize Local Time with NTP Server
///
/// The Write Command synchronizes UTC with the NTP server. Before using NTP, the host should activate the context.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QNTP", NoResponse, timeout_ms = 300)]
pub struct GetNetworkNtpTime {
    /// <contextid>
    /// Integer type. The context ID. The range is from 1 to 16.
    #[at_arg(position = 1)]
    pub context_id: u8,
    /// <server>
    /// String type. The NTP server address. The maximum length is 100 bytes.
    #[at_arg(position = 2)]
    pub server: String<100>,
}

/// AT+QMTOPEN Open a Network for MQTT Client and query
///
/// The command is used to open a network for MQTT client.
///
/// The command responds with OK. We need to get the response from the URC +QMTOPEN,
/// that can last up to 75 seconds and returns a MqttOpenResponse.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTOPEN", NoResponse, timeout_ms = 300)]
pub struct MqttOpen {
    /// <linkid>
    /// Integer type. The link ID. The range is from 0 to 5.
    #[at_arg(position = 1)]
    pub link_id: u8,
    /// <server>
    /// String type. The server address. The maximum length is 100 bytes.
    #[at_arg(position = 2)]
    pub server: String<100>,
    /// <port>
    /// Integer type. The server port. The range is 1-65535.
    #[at_arg(position = 3)]
    pub port: u16,
}

/// AT+QMTCONN Establish an MQTT Connection
///
/// The command is used to establish an MQTT connection. To be used after the TCP connection is established.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTCONN", NoResponse, timeout_ms = 5000)]
pub struct MqttConnect {
    /// <tcpconnectID>
    /// String type. MQTT socket identifier. The range is from 0 to 5.
    #[at_arg(position = 1)]
    pub tcp_connect_id: u8,
    /// <clientID>
    /// String type. The client identifier. The maximum length is 23 bytes.
    #[at_arg(position = 2)]
    pub client_id: String<23>,
    /// <username>
    /// String type. The username. The maximum length is 64 bytes.
    #[at_arg(position = 3)]
    pub username: Option<String<64>>,
    /// <password>
    /// String type. The password. The maximum length is 64 bytes.
    #[at_arg(position = 4)]
    pub password: Option<String<64>>,
}

/// AT+QMTPUBEX Publish an MQTT Message with Extended Parameters
///
/// The command responds with OK. We need to get the response from the URC +QMTPUB.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTPUB", NoResponse, timeout_ms = 300)]
pub struct MqttPublishExtended {
    /// <tcpconnectID>
    /// Integer type. MQTT socket identifier. The range is from 0 to 5.
    #[at_arg(position = 1)]
    pub tcp_connect_id: u8,
    /// <msg_id>
    /// Integer type. The message identifier. The range is from 0 to 65535.
    #[at_arg(position = 2)]
    pub msg_id: u16,
    /// <qos>
    /// Integer type. The QoS level. The range is from 0 to 2.
    /// 0: At most once
    /// 1: At least once
    /// 2: Exactly once
    #[at_arg(position = 3)]
    pub qos: u8,
    /// <retain>
    /// Integer type. Retain flag. The range is from 0 to 1.
    /// 0: The server must publish the message as if the message was not retained.
    /// 1: The server must publish the message as if the message was retained.
    #[at_arg(position = 4)]
    pub retain: u8,
    /// <topic>
    /// String type. The topic. The maximum length is 128 bytes.
    /// The topic name must be a UTF-8 encoded string.
    /// The topic name must not include the wildcard characters + and #.
    #[at_arg(position = 5)]
    pub topic: String<128>,
    /// <payload>
    /// String type. The payload. The maximum length is 1024 bytes.
    /// The payload must be a UTF-8 encoded string.
    /// The payload must not include the null character.
    #[at_arg(position = 6)]
    pub payload: String<1024>,
}

/// AT+QMTDISC Disconnect a MQTT Connection
///
/// The command is used when a client requests a disconnection from MQTT server. A DISCONNECT
/// message is sent from the client to the server to indicate that it is about to close its TCP/IP connection.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTDISC", NoResponse, timeout_ms = 300)]
pub struct MqttDisconnect {
    /// <tcpconnectID>
    /// Integer type. MQTT socket identifier. The range is from 0 to 5.
    #[at_arg(position = 1)]
    pub tcp_connect_id: u8,
}

/// AT+QMTCLOSE Close an MQTT Network
///
/// The command is used to close a network for MQTT client.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTCLOSE", NoResponse, timeout_ms = 300)]
pub struct MqttClose {
    /// <tcpconnectID>
    /// Integer type. MQTT socket identifier. The range is from 0 to 5.
    #[at_arg(position = 1)]
    pub tcp_connect_id: u8,
}

/// AT+QPOWD Power Down the Module
///
/// This command powers down the module.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QPOWD", NoResponse, timeout_ms = 300)]
pub struct PowerDown {
    /// Integer type.
    #[at_arg(position = 1)]
    pub mode: PowerDownMode,
}

/// AT+GSN Request International Mobile Equipment Identity (IMEI)
///
/// This command returns the International Mobile Equipment Identity (IMEI) number of the product in
/// information text which permits the user to identify the individual ME device. It is identical with AT+CGSN.
#[derive(Clone, AtatCmd)]
#[at_cmd("+GSN", Imei, timeout_ms = 300)]
pub struct GetImei;

/// AT+QCCID Show Integrated Circuit Card Identifier (ICCID)
///
/// The command returns the ICCID (Integrated Circuit Card Identifier) number of the (U)SIM card.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QCCID", Iccid, timeout_ms = 300)]
pub struct GetIccid;

/// AT+QMTCFG set the MQTT configurations
///
/// The command is used to set MQTT configurations.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QMTCFG", NoResponse)]
pub struct MqttConfig {
    /// config name
    /// String type. Name of configuration: SSL, version, ...
    #[at_arg(position = 1)]
    pub name: String<12>,
    /// First parameter
    #[at_arg(position = 2)]
    pub param_1: Option<u8>,
    /// Second parameter
    #[at_arg(position = 3)]
    pub param_2: Option<u8>,
    /// Third parameter
    #[at_arg(position = 4)]
    pub param_3: Option<u8>,
}

/// AT+QFDEL delete file path
///
/// The command is used to delete the specified file <filename> in UFS.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QFDEL", NoResponse)]
pub struct FileDel {
    /// Name of the file to be deleted
    /// The max length is 80 bytes
    #[at_arg(position = 1)]
    pub name: String<80>,
}

/// AT+QFUPL upload a File to the Storage
///
/// The command is used to uploads a file to storage.
#[derive(Clone, AtatCmd)]
#[at_cmd("+QFUPL", NoResponse)]
pub struct FileUpl {
    /// Name of the file to be uploaded
    /// The max length is 80 bytes
    #[at_arg(position = 1)]
    pub name: String<80>,
    /// The file size expected to be uploaded.
    /// The default value is 10240. Unit: byte
    #[at_arg(position = 2)]
    pub size: u32,
}

/// AT+QSSLCFG Configure Parameters of an SSL Cert
///
/// The command is used to configure parameters of an SSL Cert
#[derive(Clone, AtatCmd)]
#[at_cmd("+QSSLCFG", NoResponse)]
pub struct SslConfigCert {
    /// Name of SSL context
    /// eg: sslversion, cacert, ...
    #[at_arg(position = 1)]
    pub name: String<80>,
    /// The context ID. Range 0-5
    #[at_arg(position = 2)]
    pub context_id: u8,
    /// The cert file path
    #[at_arg(position = 3)]
    pub cert_path: Option<String<80>>,
}

/// AT+QSSLCFG Configure Parameters of an SSL Cert
///
/// The command is used to configure parameters of an SSL Cert
#[derive(Clone, AtatCmd)]
#[at_cmd("+QSSLCFG", NoResponse)]
pub struct SslConfigOther {
    /// Name of SSL context
    /// eg: sslversion, cacert, ...
    #[at_arg(position = 1)]
    pub name: String<80>,
    /// The context ID. Range 0-5
    #[at_arg(position = 2)]
    pub context_id: u8,
    /// The cert file path
    #[at_arg(position = 3)]
    pub level: u8,
}

#[derive(Clone, AtatCmd)]
#[at_cmd("+QSSLCFG=\"ciphersuite\",2,0xFFFF", NoResponse)]
pub struct SslSetCipherSuite;

#[derive(Clone, AtatCmd)]
#[at_cmd("+QFLST", NoResponse)]
pub struct FileList;

/// Send raw data to UART with out any AT command format
///
/// Send raw data to UART
#[derive(Clone)]
pub struct SendRawData {
    pub raw_data: heapless::Vec<u8, 4096>,
    pub len: usize,
}

impl AtatCmd for SendRawData {
    type Response = NoResponse;

    const MAX_LEN: usize = 4096;
    const EXPECTS_RESPONSE_CODE: bool = false;

    fn write(&self, buf: &mut [u8]) -> usize {
        buf[..self.len].copy_from_slice(&self.raw_data);
        self.len
    }

    fn parse(
        &self,
        _resp: Result<&[u8], atat::InternalError>,
    ) -> Result<Self::Response, atat::Error> {
        Ok(NoResponse)
    }
}
