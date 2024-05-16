use async_std::net::UdpSocket;
use rand::random;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
// TODO:
// - retrieve a list of stun servers (first use a fixed address or two)
// - implement request message creation
//    1. without change request;
//    2. with full change request;
//    3. with port change request.
// - implement response message parsing
// - implement procedure to obtain "Delta p", port allocation rule, and port filtering
//   https://datatracker.ietf.org/doc/id/draft-takeda-symmetric-nat-traversal-00.txt

#[derive(Debug)]
pub enum StunMessageType {
    BindingRequest = 0x0001,
    BindingResponse = 0x0101,
    BindingErrorResponse = 0x0111,
    // SharedSecretRequest = 0x0002,
    // SharedSecretResponse = 0x0102,
    // SharedSecretErrorResponse = 0x0112,
    Unexpected,
}
impl StunMessageType {
    pub fn bytes(&self) -> [u8; 2] {
        match self {
            StunMessageType::BindingRequest => [0, 1],
            StunMessageType::BindingResponse => [1, 1],
            StunMessageType::BindingErrorResponse => [1, 0x11],
            StunMessageType::Unexpected => [0xff, 0xff],
        }
    }
}
pub fn stun_decode(bytes: &[u8]) -> StunMessage {
    let typ_raw = u16::from_be_bytes([bytes[0], bytes[1]]);
    let typ = match typ_raw {
        0x0001 => StunMessageType::BindingRequest,
        0x0101 => StunMessageType::BindingResponse,
        0x0111 => StunMessageType::BindingErrorResponse,
        _ => StunMessageType::Unexpected,
    };
    let length = u16::from_be_bytes([bytes[2], bytes[3]]);
    let id0 = u64::from_be_bytes([
        bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
    ]);
    let id1 = u64::from_be_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]);
    let id = (id0, id1) as TransactionId;
    let mut idx = 20;
    let mut attributes = vec![];
    while idx < 20 + length {
        let typ_raw = u16::from_be_bytes([bytes[idx as usize], bytes[idx as usize + 1]]);
        let typ = parse_attribute_type(typ_raw);
        let len = u16::from_be_bytes([bytes[(idx as usize) + 2], bytes[(idx as usize) + 3]]);
        idx += 4;
        let mut value_raw = Vec::new();
        let mut len_iter = len;
        while len_iter > 0 {
            value_raw.push(bytes[idx as usize]);
            idx += 1;
            len_iter -= 1;
        }
        let value = match typ {
            StunAttributeType::ErrorCode => {
                let code = StunErrorCode::parse(value_raw);
                StunAttributeValue::Error(code)
            }
            StunAttributeType::ChangeRequest => {
                let cr = match value_raw.get(3).unwrap() {
                    2 => StunChangeRequest::ChangePort,
                    4 => StunChangeRequest::ChangeIp,
                    6 => StunChangeRequest::ChangeIpPort,
                    _ => StunChangeRequest::Unexpected,
                };
                StunAttributeValue::ChangeRequest(cr)
            }
            StunAttributeType::UnknownAttributes => {
                //TODO: this should probably cover cases where there are more attrs
                let typ1_raw = u16::from_be_bytes([value_raw[0], value_raw[1]]);
                let typ2_raw = u16::from_be_bytes([value_raw[2], value_raw[3]]);
                let typ1 = parse_attribute_type(typ1_raw);
                let typ2 = parse_attribute_type(typ2_raw);
                StunAttributeValue::Unknown((typ1, typ2))
            }
            StunAttributeType::MappedAddress
            | StunAttributeType::ResponseAddress
            | StunAttributeType::SourceAddress
            | StunAttributeType::ChangedAddress => {
                let addr = StunAddress::parse(&value_raw);
                StunAttributeValue::Address(addr)
            }
            StunAttributeType::Unexpected
            | StunAttributeType::Username
            | StunAttributeType::Password
            | StunAttributeType::MessageIntegrity
            | StunAttributeType::ReflectedFrom => StunAttributeValue::Unexpected(value_raw),
        };
        attributes.push(StunAttribute {
            typ,
            length: len,
            value,
        });
    }
    StunMessage {
        typ,
        length,
        id,
        attributes,
    }
}

fn parse_attribute_type(typ_raw: u16) -> StunAttributeType {
    match typ_raw {
        0x0001 => StunAttributeType::MappedAddress,
        0x0002 => StunAttributeType::ResponseAddress,
        0x0003 => StunAttributeType::ChangeRequest,
        0x0004 => StunAttributeType::SourceAddress,
        0x0005 => StunAttributeType::ChangedAddress,
        0x0006 => StunAttributeType::Username,
        0x0007 => StunAttributeType::Password,
        0x0008 => StunAttributeType::MessageIntegrity,
        0x0009 => StunAttributeType::ErrorCode,
        0x000a => StunAttributeType::UnknownAttributes,
        0x000b => StunAttributeType::ReflectedFrom,
        other => {
            // println!("Unexpected attr type: {}", other);
            StunAttributeType::Unexpected
        }
    }
}

#[derive(Debug)]
pub struct StunMessage {
    typ: StunMessageType,
    length: Length,
    id: TransactionId,
    attributes: Vec<StunAttribute>,
}
#[derive(Debug)]
pub struct StunAttribute {
    typ: StunAttributeType,
    length: Length,
    value: StunAttributeValue,
}
impl StunAttribute {
    pub fn bytes(&self) -> Vec<u8> {
        let mut vector = Vec::with_capacity(8);
        vector.append(&mut self.typ.bytes());
        vector.extend_from_slice(&self.length.to_be_bytes());
        vector
    }

    pub fn is_mapped_address(&self) -> bool {
        matches!(self.typ, StunAttributeType::MappedAddress)
    }
    pub fn is_changed_address(&self) -> bool {
        matches!(self.typ, StunAttributeType::ChangedAddress)
    }
    pub fn get_address(&self) -> Option<SocketAddr> {
        if let StunAttributeValue::Address(stun_addr) = &self.value {
            Some(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(
                    stun_addr.address[0],
                    stun_addr.address[1],
                    stun_addr.address[2],
                    stun_addr.address[3],
                )),
                stun_addr.port,
            ))
        } else {
            None
        }
    }
}

pub fn build_request(change_request: Option<StunChangeRequest>) -> StunMessage {
    let mut msg_len = 0;
    let id: TransactionId = (random(), random());
    let attributes = if let Some(value) = change_request {
        msg_len = 8;
        let attribute = StunAttribute {
            typ: StunAttributeType::ChangeRequest,
            length: 4 as Length,
            value: StunAttributeValue::ChangeRequest(value),
        };
        vec![attribute]
    } else {
        vec![]
    };

    StunMessage {
        typ: StunMessageType::BindingRequest,
        length: msg_len as Length,
        id,
        attributes,
    }
}

//TODO: needs rework
pub async fn stun_send(sock: &UdpSocket, msg: StunMessage, ip: Option<IpAddr>, port: Option<u16>) {
    // let stun_server: ([u8,4],u16) = ([193,43,148,37], 3478);
    // if let Ok(sock) = UdpSocket::bind("0.0.0.0:3456").await {
    let ip = if ip.is_some() {
        ip.unwrap()
    } else {
        IpAddr::V4(Ipv4Addr::new(193, 43, 148, 37))
    };
    let port = if port.is_some() { port.unwrap() } else { 3478 };
    let to: SocketAddr = SocketAddr::new(ip, port);
    let buf = msg.bytes();
    let _result = sock.send_to(&buf, to).await;
    println!("Send to stun: {:?}", to);
}

pub type Length = u16;
pub type TransactionId = (u64, u64);
impl StunMessage {
    pub fn id(&self) -> TransactionId {
        self.id
    }
    pub fn bytes(&self) -> Vec<u8> {
        let mut bts = Vec::with_capacity(28);
        bts.extend_from_slice(&self.typ.bytes());
        bts.extend_from_slice(&self.length.to_be_bytes());
        bts.extend_from_slice(&self.id.0.to_be_bytes());
        bts.extend_from_slice(&self.id.1.to_be_bytes());
        for attr in &self.attributes {
            bts.extend_from_slice(&attr.bytes())
        }
        bts
    }
    pub fn mapped_address(&self) -> Option<SocketAddr> {
        for attr in &self.attributes {
            if attr.is_mapped_address() {
                return attr.get_address();
            }
        }
        None
    }
    pub fn changed_address(&self) -> Option<SocketAddr> {
        for attr in &self.attributes {
            if attr.is_changed_address() {
                return attr.get_address();
            }
        }
        None
    }
}
// 0                   1                   2                   3
//    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//   |      STUN Message Type        |         Message Length        |
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//   |
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//                            Transaction ID
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//                                                                   |
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

#[derive(Debug)]
pub enum StunAttributeType {
    MappedAddress = 0x0001,
    ResponseAddress = 0x0002,
    ChangeRequest = 0x0003,
    SourceAddress = 0x0004,
    ChangedAddress = 0x0005,
    Username = 0x0006,
    Password = 0x0007,
    MessageIntegrity = 0x0008,
    ErrorCode = 0x0009,
    UnknownAttributes = 0x000a,
    ReflectedFrom = 0x000b,
    Unexpected,
}
impl StunAttributeType {
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            StunAttributeType::MappedAddress => vec![0, 1],
            StunAttributeType::ResponseAddress => vec![0, 2],
            StunAttributeType::ChangeRequest => vec![0, 3],
            StunAttributeType::SourceAddress => vec![0, 4],
            StunAttributeType::ChangedAddress => vec![0, 5],
            StunAttributeType::Username => vec![0, 6],
            StunAttributeType::Password => vec![0, 7],
            StunAttributeType::MessageIntegrity => vec![0, 8],
            StunAttributeType::ErrorCode => vec![0, 0x09],
            StunAttributeType::UnknownAttributes => vec![0, 0x0a],
            StunAttributeType::ReflectedFrom => vec![0, 0x0b],
            StunAttributeType::Unexpected => vec![255, 255],
        }
    }
}

// 0                   1                   2                   3
//    0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//   |         Type                  |            Length             |
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//   |                             Value                             ....
//   +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

//   The following attribute types are defined:
//   0x0001: MAPPED-ADDRESS
//   0x0002: RESPONSE-ADDRESS
//   0x0003: CHANGE-REQUEST
//   0x0004: SOURCE-ADDRESS
//   0x0005: CHANGED-ADDRESS
//   0x0006: USERNAME
//   0x0007: PASSWORD
//   0x0008: MESSAGE-INTEGRITY
//   0x0009: ERROR-CODE
//   0x000a: UNKNOWN-ATTRIBUTES
//   0x000b: REFLECTED-FROM

//                                       Binding  Shared  Shared  Shared
//                     Binding  Binding  Error    Secret  Secret  Secret
// Att.                Req.     Resp.    Resp.    Req.    Resp.   Error
//                                                                Resp.
// _____________________________________________________________________
// MAPPED-ADDRESS      N/A      M        N/A      N/A     N/A     N/A
// RESPONSE-ADDRESS    O        N/A      N/A      N/A     N/A     N/A
// CHANGE-REQUEST      O        N/A      N/A      N/A     N/A     N/A
// SOURCE-ADDRESS      N/A      M        N/A      N/A     N/A     N/A
// CHANGED-ADDRESS     N/A      M        N/A      N/A     N/A     N/A
// USERNAME            O        N/A      N/A      N/A     M       N/A
// PASSWORD            N/A      N/A      N/A      N/A     M       N/A
// MESSAGE-INTEGRITY   O        O        N/A      N/A     N/A     N/A
// ERROR-CODE          N/A      N/A      M        N/A     N/A     M
// UNKNOWN-ATTRIBUTES  N/A      N/A      C        N/A     N/A     C
// REFLECTED-FROM      N/A      C        N/A      N/A     N/A     N/A

// Table 2: Summary of Attributes

// 0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |x x x x x x x x|    Family     |           Port                |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                             Address                           |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
// The address family is always 0x01, corresponding to IPv4.
#[derive(Debug)]
pub struct StunAddress {
    port: u16,
    address: [u8; 4],
}
impl StunAddress {
    pub fn bytes(&self) -> Vec<u8> {
        let mut vector = Vec::with_capacity(8);
        vector.push(0);
        vector.push(1);
        vector.extend_from_slice(&self.port.to_be_bytes());
        vector.extend_from_slice(&self.address);
        vector
    }
    pub fn parse(bytes: &[u8]) -> Self {
        let port = u16::from_be_bytes([bytes[2], bytes[3]]);
        let address = [bytes[4], bytes[5], bytes[6], bytes[7]];
        StunAddress { port, address }
    }
}

#[derive(Debug)]
pub enum StunChangeRequest {
    ChangePort = 2,
    ChangeIp = 4,
    ChangeIpPort = 6,
    Unexpected,
}
impl StunChangeRequest {
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            StunChangeRequest::ChangePort => vec![0, 0, 0, 2],
            StunChangeRequest::ChangeIp => vec![0, 0, 0, 4],
            StunChangeRequest::ChangeIpPort => vec![0, 0, 0, 6],
            StunChangeRequest::Unexpected => vec![255, 255, 255, 255],
        }
    }
}

// CHANGE-REQUEST
// 0                   1                   2                   3
//     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//    |0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 A B 0|
//    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

//    The meaning of the flags is:

//    A: This is the "change IP" flag.  If true, it requests the server
//       to send the Binding Response with a different IP address than the
//       one the Binding Request was received on.

//    B: This is the "change port" flag.  If true, it requests the
//       server to send the Binding Response with a different port than the
// one the Binding Request was received on.
#[derive(Copy, Clone, Debug)]
pub enum StunErrorClass {
    Four = 4,
    Five = 5,
    Six = 6,
    Unexpected = 255,
}
#[derive(Copy, Clone, Debug)]
pub enum StunErrorNumber {
    Zero = 0,
    One = 1,
    Twenty = 20,
    Thirty = 30,
    ThirtyOne = 31,
    ThirtyTwo = 32,
    ThirtyThree = 33,
    Unexpected = 255,
}
#[derive(Debug)]
pub struct StunErrorCode {
    class: StunErrorClass,
    number: StunErrorNumber,
    reason_phrase: Vec<u8>,
}
impl StunErrorCode {
    pub fn bytes(&self) -> Vec<u8> {
        let mut vector = Vec::with_capacity(self.reason_phrase.len() + 4);
        vector.push(0);
        vector.push(0);
        vector.push(self.class as u8);
        vector.push(self.number as u8);
        vector.append(&mut self.reason_phrase.clone());
        vector
    }
    pub fn parse(mut bytes: Vec<u8>) -> Self {
        let class = match bytes[2] {
            4 => StunErrorClass::Four,
            5 => StunErrorClass::Five,
            6 => StunErrorClass::Six,
            _ => StunErrorClass::Unexpected,
        };
        let number = match bytes[3] {
            0 => StunErrorNumber::Zero,
            1 => StunErrorNumber::One,
            20 => StunErrorNumber::Twenty,
            30 => StunErrorNumber::Thirty,
            31 => StunErrorNumber::ThirtyOne,
            32 => StunErrorNumber::ThirtyTwo,
            33 => StunErrorNumber::ThirtyThree,
            _ => StunErrorNumber::Unexpected,
        };
        bytes.pop();
        bytes.pop();
        bytes.pop();
        bytes.pop();
        StunErrorCode {
            class,
            number,
            reason_phrase: bytes,
        }
    }
}

pub type StunUnknownAttributes = (StunAttributeType, StunAttributeType);

#[derive(Debug)]
pub enum StunAttributeValue {
    Address(StunAddress),
    ChangeRequest(StunChangeRequest),
    Error(StunErrorCode),
    Unknown(StunUnknownAttributes),
    Unexpected(Vec<u8>),
}
impl StunAttributeValue {
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            StunAttributeValue::Address(addr) => addr.bytes(),
            StunAttributeValue::ChangeRequest(chr) => chr.bytes(),
            StunAttributeValue::Error(err) => err.bytes(),
            StunAttributeValue::Unknown(unknown) => {
                let mut vector = unknown.0.bytes();
                vector.append(&mut unknown.1.bytes());
                vector
            }
            StunAttributeValue::Unexpected(unex) => unex.clone(),
        }
    }
}
