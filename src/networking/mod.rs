mod client;
mod server;
mod sock;
mod subscription;
mod token;
use self::client::run_client;
use self::server::run_server;
use self::sock::serve_socket;
use self::subscription::subscriber;
use self::token::{token_dispenser, Token};
use async_std::net::UdpSocket;
use async_std::task::spawn;
// use bytes::{BufMut, BytesMut};
// use std::error::Error;
// use std::fmt;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::Request;
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

pub async fn run_networking_tasks(
    host_ip: IpAddr,
    server_port: u16,
    buffer_size_bytes: u32,
    uplink_bandwith_bytes_sec: u32,
    notification_receiver: Receiver<(String, Sender<Request>, Sender<u32>)>,
    decrypter: Decrypter,
    pub_key_pem: String,
) {
    let server_addr: SocketAddr = SocketAddr::new("0.0.0.0".parse().unwrap(), server_port);
    // let server_addr: SocketAddr = SocketAddr::new(host_ip, server_port);
    let bind_result = UdpSocket::bind(server_addr).await;
    let (sub_send_one, sub_recv_one) = channel();
    let (sub_send_two, sub_recv_two) = channel();
    let (token_dispenser_send, token_dispenser_recv) = channel();
    spawn(subscriber(
        sub_send_one,
        sub_recv_two,
        notification_receiver,
        token_dispenser_send,
        decrypter,
    ));
    let (token_pipes_sender, token_pipes_receiver) = channel();
    // let (token_msg_sender_two, token_msg_receiver_two) = channel();
    spawn(token_dispenser(
        buffer_size_bytes,
        uplink_bandwith_bytes_sec,
        // token_msg_sender,
        token_pipes_receiver,
        token_dispenser_recv,
    ));

    //TODO: make use of token_msg_sender_two, token_msg_receiver

    if let Ok(socket) = bind_result {
        run_server(
            // gnome_id,
            host_ip,
            socket,
            sub_send_two,
            sub_recv_one,
            token_pipes_sender,
            pub_key_pem,
        )
        .await;
    } else {
        // if let Ok((swarm_name, req_sender)) = subscription_receiver.try_recv() {
        //     let futu_one = connect_with_lan_gnomes(
        //         swarm_name.clone(),
        //         host_ip,
        //         broadcast_ip,
        //         server_port,
        //         sender.clone(),
        //         req_sender.clone(),
        //     );

        //     let futu_two = async {
        run_client(
            // server_addr,
            // socket,
            host_ip,
            sub_recv_one,
            sub_send_two,
            // token_send_two,
            // token_recv,
            token_pipes_sender,
            pub_key_pem,
        )
        .await;
        //     };
        //     join!(futu_one, futu_two);
        // }
    };
}

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
