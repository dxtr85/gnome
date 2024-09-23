use super::serve_socket;
use super::Token;
use crate::crypto::Encrypter;
use crate::crypto::{generate_symmetric_key, SessionKey};
use crate::networking::common::collect_subscribed_swarm_names;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::receive_remote_swarm_names;
use crate::networking::common::send_subscribed_swarm_names;
use crate::networking::subscription::Subscription;
use async_std::net::UdpSocket;
use async_std::task::spawn;
use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeId;

pub async fn run_server(
    // host_ip: IpAddr,
    socket: UdpSocket,
    sub_sender: Sender<Subscription>,
    mut sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
) {
    eprintln!("--------------------------------------");
    eprintln!("- - - - - - - - SERVER - - - - - - - -");
    eprintln!("- Listens on: {:?}   -", socket.local_addr().unwrap());
    eprintln!("--------------------------------------");
    // println!("My Pubkey PEM:\n {:?}", pub_key_pem);
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    eprintln!("My GnomeId: {}", gnome_id);
    loop {
        let mut remote_gnome_id: GnomeId = GnomeId(0);
        let mut session_key: SessionKey = SessionKey::from_key(&[0; 32]);
        let optional_sock = establish_secure_connection(
            // host_ip,
            &socket,
            &mut remote_gnome_id,
            &mut session_key,
            &pub_key_pem,
        )
        .await;
        if optional_sock.is_none() {
            // println!("Failed to establish secure connection with Neighbor");
            continue;
        }
        let (dedicated_socket, remote_pub_key_pem) = optional_sock.unwrap();

        let mut swarm_names = Vec::new();
        sub_receiver =
            collect_subscribed_swarm_names(&mut swarm_names, sub_sender.clone(), sub_receiver)
                .await;

        swarm_names.sort();
        spawn(prepare_and_serve(
            dedicated_socket,
            remote_gnome_id,
            session_key,
            sub_sender.clone(),
            swarm_names,
            token_pipes_sender.clone(),
            // encrypter,
            remote_pub_key_pem,
        ));
        eprintln!("--------------------------------------");
    }
}

async fn establish_secure_connection(
    // host_ip: IpAddr,
    socket: &UdpSocket,
    remote_gnome_id: &mut GnomeId,
    session_key: &mut SessionKey,
    pub_key_pem: &str,
    // ) -> Option<(UdpSocket, Encrypter)> {
) -> Option<(UdpSocket, String)> {
    let mut bytes = [0u8; 1100];
    let mut count;
    let mut remote_addr;
    loop {
        let result = socket.recv_from(&mut bytes).await;
        if result.is_err() {
            eprintln!("Failed to receive data on socket: {:?}", result);
            return None;
        }
        (count, remote_addr) = result.unwrap();
        if count > 1 {
            break;
        }
    }
    let id_pub_key_pem = std::str::from_utf8(&bytes[..count]).unwrap();
    if id_pub_key_pem == pub_key_pem {
        return None;
    }
    eprintln!("SKT Received {} bytes", count);
    let result = Encrypter::create_from_data(id_pub_key_pem);
    if result.is_err() {
        eprintln!("Failed to build Encripter from received PEM: {:?}", result);
        return None;
    }
    let encr = result.unwrap();

    let dedicated_socket = UdpSocket::bind(SocketAddr::new(socket.local_addr().unwrap().ip(), 0))
        .await
        .unwrap();
    // let dedicated_port = dedicated_socket.local_addr().unwrap().port();
    // let mut bytes_to_send = Vec::from(dedicated_port.to_be_bytes());
    let key = generate_symmetric_key();
    // bytes_to_send.append(&mut Vec::from(key));
    // TODO maybe send remote external IP here?
    let bytes_to_send = Vec::from(&key);
    let encr_res = encr.encrypt(&bytes_to_send);
    if encr_res.is_err() {
        eprintln!("Failed to encrypt symmetric key: {:?}", encr_res);
        return None;
    }
    eprintln!("Encrypted symmetric key");

    let encrypted_data = encr_res.unwrap();
    // let res = socket.send_to(&encrypted_data, remote_addr).await;
    let res = dedicated_socket.send_to(&encrypted_data, remote_addr).await;
    if res.is_err() {
        eprintln!("Failed to send encrypted symmetric key: {:?}", res);
        return None;
    }
    eprintln!("Sent encrypted symmetric key {}", encrypted_data.len());

    *session_key = SessionKey::from_key(&key);

    let mut r_buf = [0u8; 32];
    let r_res = dedicated_socket.recv_from(&mut r_buf).await;
    if r_res.is_err() {
        eprintln!("Failed to receive ping from Neighbor");
        return None;
    }
    let (_count, remote_addr) = r_res.unwrap();
    let conn_result = dedicated_socket.connect(remote_addr).await;
    if conn_result.is_err() {
        eprintln!("Unable to connect dedicated socket: {:?}", conn_result);
        return None;
    }

    let my_encrypted_pubkey = session_key.encrypt(pub_key_pem.as_bytes());
    let res2 = dedicated_socket.send(&my_encrypted_pubkey).await;
    if res2.is_err() {
        eprintln!("Error sending encrypted pubkey response: {:?}", res2);
        return None;
    }
    eprintln!("Sent encrypted public key");

    *remote_gnome_id = GnomeId(encr.hash());
    eprintln!("Remote GnomeId: {}", remote_gnome_id);
    Some((dedicated_socket, id_pub_key_pem.to_string()))
}

// async fn create_dedicated_socket(
//     socket: &UdpSocket,
//     dedicated_socket: UdpSocket,
//     // host_ip: IpAddr,
//     mut remote_addr: SocketAddr,
//     session_key: &SessionKey,
// ) -> Option<UdpSocket> {
//     // let send_result = socket
//     //     .send_to(
//     //         dedicated_socket
//     //             .local_addr()
//     //             .unwrap()
//     //             .to_string()
//     //             .as_bytes(),
//     //         remote_addr,
//     //     )
//     //     .await;
//     // if send_result.is_err() {
//     //     println!("Unable to send new socket addr: {:?}", send_result);
//     //     return None;
//     // }
//     let mut bytes = [0; 128];
//     let recv_result = socket.recv_from(&mut bytes).await;

//     if let Ok((count, rem_addr)) = recv_result {
//         println!("Received {} bytes: {:?}", count, bytes);
//         let decryp_res = session_key.decrypt(&bytes[..count]);
//         if decryp_res.is_err() {
//             println!(
//                 "Unable to decrypt new socket addr with session key:\n{:?}",
//                 decryp_res
//             );
//             return None;
//         }

//         let recv_str_res = String::from_utf8(decryp_res.unwrap());
//         if recv_str_res.is_err() {
//             println!("Unable to parse String: {:?}", recv_str_res);
//             return None;
//         }
//         let port_res = recv_str_res.unwrap().parse::<u16>();
//         if port_res.is_err() {
//             println!("Failed constructing u16 port number: {:?}", port_res);
//             return None;
//         }
//         let port = port_res.unwrap();
//         remote_addr = SocketAddr::new(rem_addr.ip(), port);
//     }
//     let conn_result = dedicated_socket.connect(remote_addr).await;
//     if conn_result.is_err() {
//         println!("Unable to connect dedicated socket: {:?}", conn_result);
//         return None;
//     }

//     println!("SKT Connected to client");
//     Some(dedicated_socket)
// }

async fn prepare_and_serve(
    dedicated_socket: UdpSocket,
    remote_gnome_id: GnomeId,
    session_key: SessionKey,
    sub_sender: Sender<Subscription>,
    swarm_names: Vec<String>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // encrypter: Encrypter,
    pub_key_pem: String,
) {
    eprintln!("Waiting for data from remote...");
    let mut remote_names = vec![];
    receive_remote_swarm_names(&dedicated_socket, &mut remote_names).await;

    let mut common_names = vec![];

    send_subscribed_swarm_names(&dedicated_socket, &swarm_names).await;
    distil_common_names(&mut common_names, swarm_names, &remote_names);

    let (shared_sender, swarm_extend_receiver) = channel();
    let mut ch_pairs = vec![];
    create_a_neighbor_for_each_swarm(
        common_names,
        remote_names,
        sub_sender.clone(),
        remote_gnome_id,
        &mut ch_pairs,
        shared_sender.clone(),
        // encrypter,
        pub_key_pem,
    );

    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = token_pipes_sender.send((token_send, token_recv_two));
    serve_socket(
        session_key,
        dedicated_socket,
        ch_pairs,
        token_send_two,
        token_recv,
        sub_sender,
        shared_sender,
        swarm_extend_receiver,
    )
    .await;
}
