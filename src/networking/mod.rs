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
use self::common::are_we_behind_a_nat;
use self::holepunch::holepunch;
use self::server::run_server;
use self::sock::serve_socket;
// pub use self::status::NetworkSummary;
use self::subscription::subscriber;
use self::tcp_server::run_tcp_server;
use self::token::{token_dispenser, Token};
use crate::crypto::Decrypter;
use async_std::channel::Sender;
use async_std::net::TcpListener;
use async_std::net::UdpSocket;
use async_std::task::spawn;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender as SSender};
use subscription::Requestor;
use swarm_consensus::{GnomeToManager, Notification, NotificationBundle};

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
    let (holepunch_sender, holepunch_receiver) = channel();
    // let (token_pipes_sender, token_pipes_receiver) = channel();
    let (send_pair, recv_pair) = channel();
    eprintln!("spawn token_dispenser");
    // spawn(token_dispenser(
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
    spawn(subscriber(
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
    ));
    let mut swarm_names = vec![];
    sub_recv_one = collect_subscribed_swarm_names(
        &mut swarm_names,
        Requestor::Udp,
        sub_send_two.clone(),
        sub_recv_one,
    )
    .await;
    eprintln!("received swarm names len: {}", swarm_names.len());
    // swarm_names.sort();
    spawn(run_client(
        swarm_names,
        sub_send_two.clone(),
        decrypter.clone(),
        // token_pipes_sender.clone(),
        pub_key_pem.clone(),
        None,
    ));
    // .await;
    // IPv4
    if let Ok(listener) = tcp_bind_result {
        // let mut my_names = vec![];
        // sub_recv_one =
        //     collect_subscribed_swarm_names(&mut my_names, sub_send_two.clone(), sub_recv_one).await;
        // eprintln!("Run TCP server with swarm names: {:?}", my_names);
        spawn(run_tcp_server(
            listener,
            sub_send_two.clone(),
            sub_recv_one_bis,
            // token_pipes_sender.clone(),
            pub_key_pem.clone(),
            // my_names,
        ));
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
        spawn(run_server(
            socket,
            sub_send_two.clone(),
            sub_recv_one,
            // token_pipes_sender.clone(),
            pub_key_pem.clone(),
        ));
        let _ = sub_send_two.send(subscription::Subscription::TransportAvailable(
            Requestor::Udp,
        ));
    } else {
        eprintln!("Failed to bind socket: {:?}", bind_result.err().unwrap());
        let _ = sub_send_two.send(subscription::Subscription::TransportNotAvailable(
            Requestor::Udp,
        ));
    };
    // IPv6
    if let Ok(listener) = ipv6_tcp_bind_result {
        // let mut my_names = vec![];
        // sub_recv_one =
        //     collect_subscribed_swarm_names(&mut my_names, sub_send_two.clone(), sub_recv_one).await;
        // eprintln!("Run TCP server with swarm names: {:?}", my_names);
        spawn(run_tcp_server(
            listener,
            sub_send_two.clone(),
            sub_recv_one_cis,
            // token_pipes_sender.clone(),
            pub_key_pem.clone(),
            // my_names,
        ));
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
    if let Ok(socket) = ipv6_bind_result {
        spawn(run_server(
            socket,
            sub_send_two.clone(),
            sub_recv_one_dis,
            // token_pipes_sender.clone(),
            pub_key_pem.clone(),
        ));
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
    spawn(direct_punching_service(
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
    ));
    //     // let puncher = "tudbut.de:4277";
    //     let puncher = SocketAddr::new("217.160.249.125".parse().unwrap(), 4277);
    //     spawn(holepunch(
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
    // spawn(decrypter_service(
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
