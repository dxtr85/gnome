use crate::manager::ToGnomeManager;
use crate::networking::common::collect_subscribed_swarm_names;
use crate::networking::direct_punch::direct_punching_service;
mod client;
mod common;
mod direct_punch;
mod holepunch;
mod server;
mod sock;
pub mod status;
mod stun;
mod subscription;
mod tcp_client;
mod tcp_common;
mod tcp_server;
mod token;
use self::client::run_client;
use self::server::run_server;
use self::sock::serve_socket;
// pub use self::status::NetworkSummary;
use self::subscription::subscriber;
use self::tcp_server::run_tcp_server;
use self::token::Token;
use crate::crypto::Decrypter;
// use async_std::channel::Sender;
// use async_std::net::TcpListener;
// use async_std::net::UdpSocket;
use a_swarm_consensus::{GnomeId, SwarmName, ToGnome};
use smol::channel::Sender;
use smol::net::TcpListener;
use smol::net::UdpSocket;
use smol::Executor;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{channel, Receiver, Sender as SSender};
use std::sync::Arc;
use subscription::Requestor;
// use swarm_consensus::{GnomeToManager, Notification, NotificationBundle};

pub enum Notification {
    AddSwarm(NotificationBundle),
    RemoveSwarm(Vec<SwarmName>),
    SetFounder(GnomeId),
}
pub struct NotificationBundle {
    pub swarm_name: SwarmName,
    pub request_sender: SSender<ToGnome>,
    // pub token_sender: Sender<u64>,
    // pub network_settings_receiver: Receiver<NetworkSettings>,
    pub network_settings_receiver: Receiver<Vec<u8>>,
}
// #[derive(Debug)]
// enum ConnError {
//     Disconnected,
//     // LocalStreamClosed,
// }
// impl fmt::Display for ConnError {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         match self {
//             ConnError::Disconnected => write!(f, "ConnError: Disconnected"),
//             // ConnError::LocalStreamClosed => write!(f, "ConnError: LocalStreamClosed"),
//         }
//     }
// }
// impl Error for ConnError {}

pub async fn run_networking_tasks(
    executor: Arc<Executor<'_>>,
    to_gmgr: Sender<ToGnomeManager>,
    server_port: u16,
    // buffer_size_bytes: u64,
    // uplink_bandwith_bytes_sec: u64,
    notification_receiver: Receiver<Notification>,
    decrypter: Decrypter,
    pub_key_pem: String,
) {
    eprintln!("In run_networking_tasks");
    let server_addr: SocketAddr = SocketAddr::new("0.0.0.0".parse().unwrap(), server_port);
    let ipv6_server_addr: SocketAddr =
        SocketAddr::new("0:0:0:0:0:0:0:0".parse().unwrap(), server_port + 1);
    let bind_result = UdpSocket::bind(server_addr).await;
    let ipv6_bind_result = UdpSocket::bind(ipv6_server_addr).await;
    let tcp_bind_result = TcpListener::bind(server_addr).await;
    let ipv6_tcp_bind_result = TcpListener::bind(ipv6_server_addr).await;
    let (sub_send_one, mut sub_recv_one) = channel();
    let (sub_send_one_bis, sub_recv_one_bis) = channel();
    let (sub_send_one_cis, sub_recv_one_cis) = channel();
    let (sub_send_one_dis, sub_recv_one_dis) = channel();
    let (sub_send_two, sub_recv_two) = channel();
    // let (token_dispenser_send, token_dispenser_recv) = channel();
    let (holepunch_sender, _holepunch_receiver) = channel();
    // let (token_pipes_sender, token_pipes_receiver) = channel();
    let (send_pair, recv_pair) = channel();
    // eprintln!("spawn token_dispenser");
    // executor.spawn(token_dispenser(
    //     buffer_size_bytes,
    //     uplink_bandwith_bytes_sec,
    //     // token_msg_sender,
    //     token_pipes_receiver,
    //     token_dispenser_recv,
    // ));
    let mut sub_sends = HashMap::new();
    // If all ports are already in use, we have to add one sender
    // in order for this entire thing to work
    // if bind_result.is_ok() {
    sub_sends.insert(Requestor::Udp, sub_send_one.clone());
    // }
    if tcp_bind_result.is_ok() {
        sub_sends.insert(Requestor::Tcp, sub_send_one_bis);
    }
    if ipv6_bind_result.is_ok() {
        sub_sends.insert(Requestor::Udpv6, sub_send_one_dis);
    }
    if ipv6_tcp_bind_result.is_ok() {
        sub_sends.insert(Requestor::Tcpv6, sub_send_one_cis);
    }
    // if sub_sends.is_empty() {
    //     sub_sends.insert(Requestor::Udp, sub_send_one);
    // }
    executor
        .spawn(subscriber(
            to_gmgr.clone(),
            sub_sends,
            // decrypter.clone(),
            // token_pipes_sender.clone(),
            // pub_key_pem.clone(),
            sub_recv_two,
            // sub_send_two.clone(),
            notification_receiver,
            // token_dispenser_send,
            holepunch_sender,
            send_pair,
        ))
        .detach();
    let mut swarm_names = vec![];
    sub_recv_one = collect_subscribed_swarm_names(
        &mut swarm_names,
        Requestor::Udp,
        sub_send_two.clone(),
        sub_recv_one,
    )
    .await;
    eprintln!("received swarm names len: {}", swarm_names.len());
    let c_ex = executor.clone();
    executor
        .spawn(run_client(
            c_ex,
            swarm_names,
            sub_send_two.clone(),
            decrypter.clone(),
            // token_pipes_sender.clone(),
            pub_key_pem.clone(),
            None,
        ))
        .detach();
    // .await;
    // IPv4
    if let Ok(listener) = tcp_bind_result {
        // let mut my_names = vec![];
        // sub_recv_one =
        //     collect_subscribed_swarm_names(&mut my_names, sub_send_two.clone(), sub_recv_one).await;
        eprintln!("Run TCP server with swarm names: ");
        let c_ex = executor.clone();
        executor
            .spawn(run_tcp_server(
                c_ex,
                listener,
                sub_send_two.clone(),
                sub_recv_one_bis,
                // token_pipes_sender.clone(),
                pub_key_pem.clone(),
                // my_names,
            ))
            .detach();
        eprintln!("Run TCP server with swarm names AFTER");
        let _ = sub_send_two.send(subscription::Subscription::TransportAvailable(
            Requestor::Tcp,
        ));
    } else {
        eprintln!(
            "Failed to bind socket: {:?}",
            tcp_bind_result.err().unwrap()
        );
        let _ = sub_send_two.send(subscription::Subscription::TransportNotAvailable(
            Requestor::Tcp,
        ));
    }
    if let Ok(socket) = bind_result {
        eprintln!("HAVE bind result OK");
        let c_ex = executor.clone();
        executor
            .spawn(run_server(
                c_ex,
                socket,
                sub_send_two.clone(),
                sub_recv_one,
                // token_pipes_sender.clone(),
                pub_key_pem.clone(),
            ))
            .detach();
        let _ = sub_send_two.send(subscription::Subscription::TransportAvailable(
            Requestor::Udp,
        ));
    } else {
        eprintln!("Failed to bind socket: {:?}", bind_result.err().unwrap());
        let _ = sub_send_two.send(subscription::Subscription::TransportNotAvailable(
            Requestor::Udp,
        ));
    };
    // eprintln!("one");
    // IPv6
    if let Ok(listener) = ipv6_tcp_bind_result {
        // eprintln!("one in");
        // let mut my_names = vec![];
        // sub_recv_one =
        //     collect_subscribed_swarm_names(&mut my_names, sub_send_two.clone(), sub_recv_one).await;
        eprintln!("Run TCP server with swarm names:2");
        let c_ex = executor.clone();
        executor
            .spawn(run_tcp_server(
                c_ex,
                listener,
                sub_send_two.clone(),
                sub_recv_one_cis,
                // token_pipes_sender.clone(),
                pub_key_pem.clone(),
                // my_names,
            ))
            .detach();
        eprintln!("Run TCP server with swarm names:2AFTER");
        let _ = sub_send_two.send(subscription::Subscription::TransportAvailable(
            Requestor::Tcpv6,
        ));
    } else {
        eprintln!(
            "Failed to bind IPv6 TCP socket: {:?}",
            ipv6_tcp_bind_result.err().unwrap()
        );
        let _ = sub_send_two.send(subscription::Subscription::TransportNotAvailable(
            Requestor::Tcpv6,
        ));
    }
    // eprintln!("two");
    if let Ok(socket) = ipv6_bind_result {
        eprintln!("HEVE bind resuls6: OK");
        let c_ex = executor.clone();
        let _r = executor
            .spawn(run_server(
                c_ex,
                socket,
                sub_send_two.clone(),
                sub_recv_one_dis,
                // token_pipes_sender.clone(),
                pub_key_pem.clone(),
            ))
            .detach();
        let _ = sub_send_two.send(subscription::Subscription::TransportAvailable(
            Requestor::Udpv6,
        ));
    } else {
        eprintln!(
            "Failed to bind IPv6 UDP socket: {:?}",
            ipv6_bind_result.err().unwrap()
        );
        let _ = sub_send_two.send(subscription::Subscription::TransportNotAvailable(
            Requestor::Udpv6,
        ));
    };
    //TODO: We need to organize how and which networking services get started.
    // 1. We always need to run basic services like token_dispenser and subscriber.
    // 2. We also always need to run_client and try to run_server.
    // 2. We need te establish if we have a public_ip.

    // TODO: if we are NOT behind a NAT what do we do with Neighbor's NetworkSettings?
    // let behind_nat_result = are_we_behind_a_nat(
    //     &UdpSocket::bind(SocketAddr::new("0.0.0.0".parse().unwrap(), 0))
    //         .await
    //         .unwrap(),
    // )
    // .await;
    // let we_are_behind_nat;
    // // let mut our_public_ip = None;
    // if let Ok((behind_a_nat, _public_addr)) = behind_nat_result {
    //     we_are_behind_nat = behind_a_nat;
    //     // our_public_ip = Some(public_addr.ip());
    // } else {
    //     we_are_behind_nat = true;
    // }
    // 3. a) If we are not behind a NAT then we are done - no, we are not!!!
    //    b) Other way we are behind a NAT.
    // if we_are_behind_nat {
    // In case we are behind a NAT we need to run direct_punch and holepunch
    // Both of those services need a sophisticated procedure for connection establishment.
    // eprintln!("spwaning dps");
    let c_ex = executor.clone();
    executor
        .spawn(direct_punching_service(
            c_ex,
            to_gmgr.clone(),
            server_port,
            sub_send_two.clone(),
            // req_sender.clone(),
            // resp_receiver,
            decrypter.clone(),
            // token_pipes_sender.clone(),
            recv_pair,
            // receiver,
            pub_key_pem.clone(),
            // swarm_name.clone(),
            // net_set_recv,
            // None,
        ))
        .detach();
    //     // let puncher = "tudbut.de:4277";
    //     let puncher = SocketAddr::new("217.160.249.125".parse().unwrap(), 4277);
    //     executor.spawn(holepunch(
    //         puncher,
    //         // host_ip,
    //         sub_send_two,
    //         // decode_req_send,
    //         // decode_resp_recv,
    //         decrypter,
    //         token_pipes_sender,
    //         holepunch_receiver,
    //         pub_key_pem,
    //     ));
    // }

    // let (decode_req_send, decode_req_recv) = channel();
    // let (decode_resp_send, decode_resp_recv) = channel();
    // println!("bifor");
    // executor.spawn(decrypter_service(
    //     decrypter,
    //     decode_resp_send,
    //     decode_req_recv,
    // ));
    // println!("after");
}

// async fn decrypter_service(
//     decrypter: Decrypter,
//     sender: Sender<[u8; 32]>,
//     receiver: Receiver<Vec<u8>>,
// ) {
//     println!("Starting decrypter service");
//     loop {
//         // for (sender, receiver) in &channel_pairs {
//         if let Ok(msg) = receiver.try_recv() {
//             println!("decoding: {:?}", msg);
//             let decode_res = decrypter.decrypt(&msg);
//             if let Ok(decoded) = decode_res {
//                 if decoded.len() == 32 {
//                     // let sym_port_key: [u8; 34] = decoded.try_into().unwrap();
//                     // let sym_key: [u8; 32] = sym_port_key[2..].try_into().unwrap();
//                     let sym_key: [u8; 32] = decoded.try_into().unwrap();
//                     // let port: u16 =
//                     //     u16::from_be_bytes(sym_port_key[0..2].try_into().unwrap());
//                     println!("succesfully decoded");
//                     let _ = sender.send(sym_key);
//                 } else {
//                     println!("Decoded symmetric key has wrong size: {}", decoded.len());
//                     // let _ = sub_sender.send(Subscription::DecodeFailure);
//                 }
//             } else {
//                 println!("Failed decoding message: {:?}", decode_res);
//                 // let _ = sub_sender.send(Subscription::DecodeFailure);
//             }
//         }
//         yield_now().await;
//         // }
//     }
// }

// Decode(Box<Vec<u8>>),
// KeyDecoded(Box<[u8; 32]>),
// async fn connect_with_lan_gnomes(
//     swarm_name: String,
//     host_ip: IpAddr,
//     broadcast_ip: IpAddr,
//     server_port: u16,
//     _sender: Sender<(Sender<Message>, Receiver<Message>)>,
//     _request_sender: Sender<Request>,
// ) {
//     //TODO: extend to send a broadcast dgram to local network
//     // and create a dedicated connection for each response
//     // sleep(Duration::from_millis(10)).await;
//     let socket = UdpSocket::bind(SocketAddr::new(host_ip, 1026))
//         .await
//         .expect("Unable to bind socket");
//     println!("Socket bind");
//     socket
//         .set_broadcast(true)
//         .expect("Unable to set broadcast on socket");
//     // TODO: we need to send id_rsa_pub_pem first
//     // we send to 255.255.255.255:port
//     // we recv on 0.0.0.0:port
//     // on both(?) sides we need to set broadcast to true
//     let mut buf = BytesMut::from(swarm_name.clone().as_bytes());
//     let local_addr = socket.local_addr().unwrap().to_string();
//     println!("SKT sending broadcast {} to {:?}", local_addr, broadcast_ip);
//     buf.put(local_addr.as_bytes());
//     socket
//         .send_to(&buf, SocketAddr::new(broadcast_ip, server_port))
//         .await
//         .expect("Unable to send broadcast packet");

//     println!("SKT Listening for responses from LAN servers");
//     while let Ok((count, _server_addr)) = socket.recv_from(&mut buf).await {
//         let _dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
//             .await
//             .expect("SKT couldn't bind to address");
//         let recv_str = String::from_utf8(Vec::from(&buf[..count])).unwrap();
//         let _server_addr: SocketAddr = recv_str.parse().expect("Received incorrect socket addr");
//         // TODO: figure this out later
//         // run_client(
//         //     // swarm_name.clone(),
//         //     server_addr,
//         //     dedicated_socket,
//         //     // sender.clone(),
//         //     request_sender.clone(),
//         // )
//         // .await;
//     }
// }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nat {
    Unknown = 0,
    None = 1,
    FullCone = 2,
    AddressRestrictedCone = 4,
    PortRestrictedCone = 8,
    SymmetricWithPortControl = 16,
    Symmetric = 32,
}
impl Nat {
    pub fn from(byte: u8) -> Self {
        match byte {
            1 => Self::None,
            2 => Self::FullCone,
            4 => Self::AddressRestrictedCone,
            8 => Self::PortRestrictedCone,
            16 => Self::SymmetricWithPortControl,
            32 => Self::Symmetric,
            _o => Self::Unknown,
        }
    }
    pub fn update(&mut self, new_value: Nat) {
        // we always keep more restrictive option
        match new_value {
            Self::None => {
                let myself = std::mem::replace(self, Self::None);
                match myself {
                    Self::None | Self::Unknown => {
                        //do nothing, already updated
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::FullCone => {
                let myself = std::mem::replace(self, Self::FullCone);
                match myself {
                    Self::None => {
                        *self = Self::None;
                    }
                    _other => {
                        //do nothing
                    }
                }
            }
            Self::AddressRestrictedCone => {
                let myself = std::mem::replace(self, Self::AddressRestrictedCone);
                match myself {
                    Self::None | Self::Unknown | Self::FullCone | Self::AddressRestrictedCone => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::PortRestrictedCone => {
                let myself = std::mem::replace(self, Self::PortRestrictedCone);
                match myself {
                    Self::None
                    | Self::Unknown
                    | Self::FullCone
                    | Self::AddressRestrictedCone
                    | Self::PortRestrictedCone => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::SymmetricWithPortControl => {
                let myself = std::mem::replace(self, Self::SymmetricWithPortControl);
                match myself {
                    Self::None
                    | Self::Unknown
                    | Self::FullCone
                    | Self::AddressRestrictedCone
                    | Self::PortRestrictedCone
                    | Self::SymmetricWithPortControl => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::Symmetric => {
                *self = Self::Symmetric;
            }
            Self::Unknown => {
                //do nothing
            }
        }
    }
    pub fn no_nat(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortAllocationRule {
    Random = 0,
    FullCone = 1,
    AddressSensitive = 2,
    PortSensitive = 4,
}
impl PortAllocationRule {
    pub fn from(byte: u8) -> Self {
        match byte {
            1 => Self::FullCone,
            2 => Self::AddressSensitive,
            4 => Self::PortSensitive,
            _o => Self::Random,
        }
    }
    pub fn update(&mut self, new_value: Self) {
        match new_value {
            Self::Random => {
                //do nothing
            }
            Self::FullCone => {
                let myself = std::mem::replace(self, Self::AddressSensitive);
                match myself {
                    Self::Random | Self::FullCone => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::AddressSensitive => {
                let myself = std::mem::replace(self, Self::AddressSensitive);
                match myself {
                    Self::Random | Self::AddressSensitive => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
            Self::PortSensitive => {
                let myself = std::mem::replace(self, Self::PortSensitive);
                match myself {
                    Self::Random | Self::AddressSensitive | Self::PortSensitive => {
                        //do nothing
                    }
                    other => {
                        *self = other;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    UDPoverIP4 = 0,
    TCPoverIP4 = 1,
    UDPoverIP6 = 2,
    TCPoverIP6 = 3,
}

impl Transport {
    pub fn from(byte: u8) -> Result<Self, u8> {
        match byte {
            0 => Ok(Self::UDPoverIP4),
            1 => Ok(Self::TCPoverIP4),
            2 => Ok(Self::UDPoverIP6),
            3 => Ok(Self::TCPoverIP6),
            other => Err(other),
        }
    }
    pub fn byte(&self) -> u8 {
        match self {
            Self::UDPoverIP4 => 0,
            Self::TCPoverIP4 => 1,
            Self::UDPoverIP6 => 2,
            Self::TCPoverIP6 => 3,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkSettings {
    pub pub_ip: IpAddr,
    pub pub_port: u16,
    pub nat_type: Nat,
    pub port_allocation: (PortAllocationRule, i8),
    pub transport: Transport,
}

impl NetworkSettings {
    pub fn from(bytes: &[u8]) -> Vec<NetworkSettings> {
        let mut bytes_iter = bytes.iter();
        let mut ns_vec = vec![];
        //TODO: support multiple NSs
        while let Some(ip_ver) = bytes_iter.next() {
            let mut ip_bytes: Vec<u8> = vec![];
            let pub_ip = match ip_ver {
                4 => {
                    for _i in 0..4 {
                        ip_bytes.push(*bytes_iter.next().unwrap());
                    }
                    let array: [u8; 4] = ip_bytes.try_into().unwrap();
                    IpAddr::from(array)
                }
                6 => {
                    for _i in 0..16 {
                        ip_bytes.push(*bytes_iter.next().unwrap());
                    }
                    let array: [u8; 16] = ip_bytes.try_into().unwrap();
                    IpAddr::from(array)
                }
                other => {
                    panic!("Uncecognized IP version: {}", other);
                }
            };

            let nat_type = match bytes_iter.next().unwrap() {
                0 => Nat::Unknown,
                1 => Nat::None,
                2 => Nat::FullCone,
                4 => Nat::AddressRestrictedCone,
                8 => Nat::PortRestrictedCone,
                16 => Nat::SymmetricWithPortControl,
                32 => Nat::Symmetric,
                other => {
                    eprintln!("Unrecognized NatType while parsing: {}", other);
                    Nat::Unknown
                }
            };

            let mut port_bytes: [u8; 2] = [0, 0];
            port_bytes[0] = *bytes_iter.next().unwrap();
            port_bytes[1] = *bytes_iter.next().unwrap();
            let pub_port: u16 = ((port_bytes[0]) as u16) << 8 | port_bytes[1] as u16;

            let port_allocation_rule = match bytes_iter.next().unwrap() {
                0 => PortAllocationRule::Random,
                1 => PortAllocationRule::FullCone,
                2 => PortAllocationRule::AddressSensitive,
                4 => PortAllocationRule::PortSensitive,
                _ => PortAllocationRule::Random,
            };
            let delta_port = *bytes_iter.next().unwrap() as i8;
            let mut transport = Transport::from(*bytes_iter.next().unwrap()).unwrap();
            if transport == Transport::UDPoverIP4 && *ip_ver == 6 {
                transport = Transport::UDPoverIP6;
            } else if transport == Transport::TCPoverIP4 && *ip_ver == 6 {
                transport = Transport::TCPoverIP6;
            } else if transport == Transport::UDPoverIP6 && *ip_ver == 4 {
                transport = Transport::UDPoverIP4;
            } else if transport == Transport::TCPoverIP6 && *ip_ver == 4 {
                transport = Transport::TCPoverIP4;
            }
            // for a_byte in bytes_iter {
            //     ip_bytes.push(*a_byte);
            // }
            // let bytes_len = ip_bytes.len();
            // let pub_ip = if bytes_len == 4 {
            //     let array: [u8; 4] = ip_bytes.try_into().unwrap();
            //     IpAddr::from(array)
            // } else if bytes_len == 16 {
            //     let array: [u8; 16] = ip_bytes.try_into().unwrap();
            //     IpAddr::from(array)
            // } else {
            //     eprintln!("Unable to parse IP addr from: {:?}", ip_bytes);
            //     IpAddr::from([0, 0, 0, 0])
            // };
            ns_vec.push(NetworkSettings {
                pub_ip,
                pub_port,
                nat_type,
                port_allocation: (port_allocation_rule, delta_port),
                transport,
            })
        }
        ns_vec
    }

    pub fn bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32);
        let pub_ip = self.pub_ip;
        match pub_ip {
            std::net::IpAddr::V4(ip4) => {
                bytes.push(4);
                for b in ip4.octets() {
                    bytes.push(b);
                }
            }
            std::net::IpAddr::V6(ip4) => {
                bytes.push(6);
                for b in ip4.octets() {
                    bytes.push(b);
                }
            }
        }
        bytes.push(self.nat_type as u8);
        for b in self.pub_port.to_be_bytes() {
            bytes.push(b);
        }
        // put_u16(bytes, self.pub_port);
        // bytes.put_u16(self.pub_port);
        bytes.push(self.port_allocation.0 as u8);
        // TODO: fix this!
        // bytes.put_i8(self.port_allocation.1);
        bytes.push(self.port_allocation.1 as u8);
        bytes.push(self.transport.byte());
        bytes
    }
    pub fn new_not_natted(pub_ip: IpAddr, pub_port: u16, transport: Transport) -> Self {
        Self {
            pub_ip,
            pub_port,
            nat_type: Nat::None,
            port_allocation: (PortAllocationRule::FullCone, 0),
            transport,
        }
    }
    pub fn len(&self) -> usize {
        if self.pub_ip.is_ipv4() {
            9
        } else {
            21
        }
    }
    pub fn update(&mut self, other: Self) {
        self.pub_ip = other.pub_ip;
        self.pub_port = other.pub_port;
        self.nat_type = other.nat_type;
        self.port_allocation = other.port_allocation;
    }

    pub fn set_port(&mut self, port: u16) {
        self.pub_port = port;
    }

    pub fn get_predicted_addr(&self, mut iter: u8) -> (IpAddr, u16) {
        let mut port = self.port_increment(self.pub_port);
        while iter > 0 {
            iter -= 1;
            port = self.port_increment(port);
        }
        (self.pub_ip, port)
    }

    pub fn refresh_required(&self) -> bool {
        self.port_allocation.0 != PortAllocationRule::FullCone
    }
    pub fn nat_at_most_address_sensitive(&self) -> bool {
        self.nat_type == Nat::None
            || self.nat_type == Nat::FullCone
            || self.nat_type == Nat::AddressRestrictedCone
    }
    pub fn no_nat(&self) -> bool {
        self.nat_type == Nat::None
    }
    pub fn nat_port_restricted(&self) -> bool {
        self.nat_type == Nat::PortRestrictedCone
    }
    pub fn nat_symmetric(&self) -> bool {
        self.nat_type == Nat::Symmetric
    }
    pub fn nat_symmetric_with_port_control(&self) -> bool {
        self.nat_type == Nat::SymmetricWithPortControl
    }
    pub fn nat_unknown(&self) -> bool {
        self.nat_type == Nat::Unknown
    }
    pub fn port_allocation_predictable(&self) -> bool {
        self.port_allocation.0 == PortAllocationRule::FullCone
            || self.port_allocation.0 == PortAllocationRule::AddressSensitive
            || self.port_allocation.0 == PortAllocationRule::PortSensitive
    }
    pub fn port_sensitive_allocation(&self) -> bool {
        self.port_allocation.0 == PortAllocationRule::PortSensitive
    }
    pub fn port_increment(&self, port: u16) -> u16 {
        match self.port_allocation {
            (PortAllocationRule::AddressSensitive | PortAllocationRule::PortSensitive, value) => {
                if value > 0 {
                    port + (value as u16)
                } else {
                    port - (value.unsigned_abs() as u16)
                }
            }
            _ => port,
        }
    }
}

impl Default for NetworkSettings {
    fn default() -> Self {
        NetworkSettings {
            pub_ip: IpAddr::from([0, 0, 0, 0]),
            pub_port: 1026,
            nat_type: Nat::Unknown,
            port_allocation: (PortAllocationRule::Random, 0),
            transport: Transport::UDPoverIP4,
        }
    }
}
