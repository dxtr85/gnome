mod client;
mod common;
mod direct_punch;
mod holepunch;
mod server;
mod sock;
mod stun;
mod subscription;
mod token;
use self::client::run_client;
use self::common::are_we_behind_a_nat;
use self::common::discover_port_allocation_rule;
use self::server::run_server;
use self::sock::serve_socket;
use self::subscription::subscriber;
use self::token::{token_dispenser, Token};
use async_std::net::UdpSocket;
use async_std::task::spawn;
use holepunch::holepunch;
use std::sync::mpsc::{channel, Receiver, Sender};
use stun::{build_request, stun_decode, stun_send};
use swarm_consensus::{NetworkSettings, Request};
// use swarm_consensus::Message;

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

use std::net::{IpAddr, SocketAddr};

use crate::crypto::Decrypter;
use crate::networking::common::identify_nat;

pub async fn run_networking_tasks(
    host_ip: IpAddr,
    server_port: u16,
    buffer_size_bytes: u32,
    uplink_bandwith_bytes_sec: u32,
    notification_receiver: Receiver<(
        String,
        Sender<Request>,
        Sender<u32>,
        Receiver<(NetworkSettings, Option<NetworkSettings>)>,
    )>,
    decrypter: Decrypter,
    pub_key_pem: String,
) {
    let server_addr: SocketAddr = SocketAddr::new("0.0.0.0".parse().unwrap(), server_port);
    // let server_addr: SocketAddr = SocketAddr::new(host_ip, server_port);
    let bind_result = UdpSocket::bind(server_addr).await;
    let (sub_send_one, sub_recv_one) = channel();
    let (sub_send_two, sub_recv_two) = channel();
    let (token_dispenser_send, token_dispenser_recv) = channel();
    let (holepunch_sender, holepunch_receiver) = channel();
    let (token_pipes_sender, token_pipes_receiver) = channel();
    spawn(subscriber(
        host_ip,
        sub_send_one,
        decrypter.clone(),
        token_pipes_sender.clone(),
        pub_key_pem.clone(),
        sub_recv_two,
        sub_send_two.clone(),
        notification_receiver,
        token_dispenser_send,
        holepunch_sender,
    ));

    spawn(token_dispenser(
        buffer_size_bytes,
        uplink_bandwith_bytes_sec,
        // token_msg_sender,
        token_pipes_receiver,
        token_dispenser_recv,
    ));

    // let (decode_req_send, decode_req_recv) = channel();
    // let (decode_resp_send, decode_resp_recv) = channel();
    // println!("bifor");
    // spawn(decrypter_service(
    //     decrypter,
    //     decode_resp_send,
    //     decode_req_recv,
    // ));
    // println!("after");
    if let Ok(socket) = bind_result {
        let puncher = "tudbut.de:4277";
        spawn(holepunch(
            puncher,
            host_ip,
            sub_send_two.clone(),
            // decode_req_send,
            // decode_resp_recv,
            decrypter.clone(),
            token_pipes_sender.clone(),
            holepunch_receiver,
            pub_key_pem.clone(),
        ));

        // Now we know all we need to establish a connection between two hosts behind
        // any type of NAT. (We might not connect if NAT always assigns ports randomly.)
        // From now on if we want to connect to another gnome, we create a new socket,
        // if necessary send just one request to STUN server for port identification
        // and we can send out our expected socket address for other gnome to connect to.

        // The connection procedure should be as follows:
        // Once we receive other socket address we send nine one byte datagrams
        // in 100ms intervals counting down from 9 to 1
        // then we listen for dgrams from remote gnome and receive until we get
        // one with a single byte 1.
        // Now we can pass that socket for Neighbor creation.
        // If we do not receive any dgrams after specified period of time,
        // we can start over from creation of a new socket,
        // but current procedure is not successful.

        // In case of using external proxy like tudbut.de for neighbor discovery
        // we can simply send a drgram to that server and wait for a response.
        // When we receive that response we have a remote socket address.
        // Now we send another request to a proxy from a new socket, and we
        // receive a reply - we note remote_port_1.
        // We send another request to a proxy from yet another socket, and we
        // note remote_port_2 from second reply.
        // We calculate delta_p = remote_port_2 - remote_port_1.
        // Now we calculate remote_port = remote_port_2 + delta_p.
        // We start exchanging messages as described above to calculated remote_port.
        // If no luck, we can turn back to proxy
        // or some other mean to receive a neighbor's socket address.
        let behind_a_nat = are_we_behind_a_nat(&socket).await;
        if let Ok(is_there_nat) = behind_a_nat {
            if is_there_nat {
                println!("NAT detected, identifying...");
                identify_nat(&socket).await;
                discover_port_allocation_rule(&socket).await;
            } else {
                println!("We have a public IP!");
            }
        } else {
            println!("Unable to tell if there is NAT: {:?}", behind_a_nat);
        }
        run_server(
            host_ip,
            socket,
            sub_send_two,
            sub_recv_one,
            token_pipes_sender,
            pub_key_pem,
        )
        .await;
    } else {
        run_client(
            // server_addr,
            // socket,
            host_ip,
            sub_recv_one,
            sub_send_two,
            // token_send_two,
            // token_recv,
            decrypter.clone(),
            // decode_req_send,
            // decode_resp_recv,
            token_pipes_sender,
            pub_key_pem,
        )
        .await;
    };
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
