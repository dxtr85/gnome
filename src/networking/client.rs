use super::serve_socket;
use super::Token;
use crate::crypto::Decrypter;
use crate::crypto::Encrypter;
use crate::crypto::SessionKey;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::receive_remote_swarm_names;
use crate::networking::common::send_subscribed_swarm_names;
use crate::networking::common::time_out;
use crate::networking::subscription::Subscription;
use crate::networking::tcp_client::run_tcp_client;
use async_std::net::UdpSocket;
use async_std::task::spawn;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::GnomeId;
use swarm_consensus::NetworkSettings;
use swarm_consensus::SwarmName;

pub async fn run_client(
    swarm_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    target_host: Option<(UdpSocket, NetworkSettings)>,
) {
    // ) -> Receiver<Subscription> {
    // First pull out broadcast sending pubkey
    // Second receive symmetric key
    // Third receive redirect for new socket
    // Fourth make socket connect to new address
    // Then move previous points out from this function
    //     so that in operates on dedicated socket it receives as argument
    let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let my_id = GnomeId(encr.hash());
    eprintln!("SKT CLIENT {:?}", target_host);
    let mut tcp_addr = None;
    let (socket, send_addr) = if let Some((sock, net_set)) = target_host {
        tcp_addr = Some(net_set.get_predicted_addr(0));
        (sock, net_set.get_predicted_addr(0))
    } else {
        // let result = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await;
        let client_addr: SocketAddr = SocketAddr::new("0.0.0.0".parse().unwrap(), 0);
        let result = UdpSocket::bind(client_addr).await;
        if result.is_err() {
            eprintln!("SKT couldn't bind to address");
            return;
            // return receiver;
        }
        let socket = result.unwrap();
        let result = socket.set_broadcast(true);
        if result.is_err() {
            eprintln!("SKT couldn't enable broadcast");
            return;
            // return receiver;
        }
        (socket, ("255.255.255.255".parse().unwrap(), 1026))
    };
    // eprintln!("Predicted addr: {:?}", send_addr);
    // eprintln!("Pubkey PEM: \n{}", pub_key_pem);

    let send_result = socket.send_to(pub_key_pem.as_bytes(), send_addr).await;
    if send_result.is_err() {
        eprintln!("Unable te send broadcast message: {:?}", send_result);
        return;
        // return receiver;
    } else {
        eprintln!("UDP Send result: {:?}", send_result);
    }

    let timeout_sec = Duration::from_secs(5);
    let (t_send, timeout) = channel();
    spawn(time_out(timeout_sec, Some(t_send)));
    let mut success = false;
    while timeout.try_recv().is_err() && !success {
        if swarm_names.is_empty() {
            eprintln!("User is not subscribed to any Swarms");
            return;
            // return receiver;
        }
        success = establish_secure_connection(
            my_id,
            &socket,
            sender.clone(),
            // req_sender.clone(),
            // resp_receiver,
            decrypter.clone(),
            pipes_sender.clone(),
            swarm_names.clone(),
        )
        .await;
    }
    eprintln!("UDP connection success: {}", success);
    if success {
        tcp_addr = None;
    } else {
        eprintln!(" TCP ADDR: {:?} {:?}", tcp_addr, swarm_names);
    }
    if let Some(addr) = tcp_addr {
        run_tcp_client(
            my_id,
            swarm_names,
            sender,
            decrypter,
            pipes_sender,
            pub_key_pem,
            addr,
        )
        .await
    }
    // eprintln!("Client is done");
    // receiver
}

async fn wait_for_bytes(socket: &UdpSocket) {
    let mut recv_buf = [0u8; 1024];
    loop {
        let recv_result = socket.peek_from(&mut recv_buf).await;
        if recv_result.is_ok() {
            let (count, _remote_addr) = recv_result.unwrap();
            if count > 1 {
                break;
            }
        }
    }
}
async fn establish_secure_connection(
    my_id: GnomeId,
    socket: &UdpSocket,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_names: Vec<SwarmName>,
) -> bool {
    eprintln!("UDP Client trying to establish secure connection");
    let mut remote_gnome_id: GnomeId = GnomeId(0);
    let session_key: SessionKey; // = SessionKey::from_key(&[0; 32]);
    let remote_addr: SocketAddr; // = "0.0.0.0:0".parse().unwrap();
    let count;
    let mut recv_buf = [0u8; 1100];

    // let mut socket_found = false;
    let wait_time = Duration::from_secs(1);
    let t1 = wait_for_bytes(socket).fuse();
    let t2 = time_out(wait_time, None).fuse();

    pin_mut!(t1, t2);

    let timed_out = select! {
        _result1 = t1 =>  false,
        _result2 = t2 => true,
    };
    if timed_out {
        return false;
    }
    let recv_result = socket.recv_from(&mut recv_buf).await;
    if recv_result.is_ok() {
        (count, remote_addr) = recv_result.unwrap();
        // eprintln!("UDP Got {} bytes back from: {:?}", count, remote_addr);
    } else {
        eprintln!("UDP Failed to retrieve bytes from remote");
        return false;
    }

    eprintln!("UDP Received {} bytes", count);
    let decoded_key = decrypter.decrypt(&recv_buf[..count]);
    if let Ok(sym_key) = decoded_key {
        // println!("Got session key: {:?}", sym_key);
        session_key = SessionKey::from_key(&sym_key.try_into().unwrap());
    } else {
        eprintln!("UDP Unable to decode key");
        // return resp_receiver;
        return false;
    }

    let dedicated_socket = UdpSocket::bind(SocketAddr::new(socket.local_addr().unwrap().ip(), 0))
        .await
        .unwrap();
    dedicated_socket.connect(remote_addr).await.unwrap();
    let _ = dedicated_socket.send(&[0u8]).await;

    let mut recv_buf = [0u8; 1100];
    let recv_result = dedicated_socket.recv(&mut recv_buf).await;
    if let Ok(count) = recv_result {
        eprintln!("UDP Received {} bytes", count);
        let decr_res = session_key.decrypt(&recv_buf[..count]);
        if let Ok(remote_pubkey_pem) = decr_res {
            let remote_id_pub_key_pem =
                std::str::from_utf8(&remote_pubkey_pem).unwrap().to_string();
            spawn(prepare_and_serve(
                my_id,
                dedicated_socket,
                // remote_gnome_id,
                session_key,
                swarm_names,
                sender.clone(),
                pipes_sender.clone(),
                // encr,
                remote_id_pub_key_pem,
            ));
            return true;
        } else {
            eprintln!("UDP Failed to decrypt message");
            return false;
        }
    } else {
        eprintln!("UDP Failed to receive data from remote");
        return false;
    }

    // return;
    // }
    // resp_receiver
}

// async fn socket_connect(
//     socket: &UdpSocket,
//     remote_port: u16,
//     remote_addr: SocketAddr,
//     session_key: &SessionKey,
// ) -> Option<UdpSocket> {
//     // let mut recv_buf = [0; 128];
//     // let recv_result = socket.recv_from(&mut recv_buf).await;

//     // if let Ok((count, remote_addr)) = recv_result {
//     // let recv_str = String::from_utf8(Vec::from(&recv_buf[..count])).unwrap();

//     // println!("SKT Received {} bytes: {:?}", count, recv_str);

//     let dedicated_socket = UdpSocket::bind(SocketAddr::new(socket.local_addr().unwrap().ip(), 0))
//         .await
//         .unwrap();
//     let new_sock_addr = dedicated_socket.local_addr().unwrap().port();
//     let new_sock_str = new_sock_addr.to_string();
//     // println!("new sock str: {}", new_sock_str);
//     let bytes_to_send = session_key.encrypt(new_sock_str.as_bytes());
//     // println!("encrypted len:  {}", bytes_to_send.len());

//     let send_result = socket.send_to(&bytes_to_send, remote_addr).await;
//     // let send_result = socket.send_to(new_sock_str.as_bytes(), remote_addr).await;
//     if send_result.is_err() {
//         println!("Unable to send new socket addr: {:?}", send_result);
//         return None; //sub_receiver;
//                      // continue;
//     }
//     println!(
//         "Sent my new socket addr: {:?} to {}",
//         send_result, remote_addr
//     );

//     let new_remote_addr = SocketAddr::new(remote_addr.ip(), remote_port);
//     let conn_result = dedicated_socket.connect(new_remote_addr).await;
//     if conn_result.is_ok() {
//         println!("SKT Connected to server");
//         Some(dedicated_socket)
//     } else {
//         println!("SKT Failed to connect");
//         None
//     }
//     // } else {
//     //     println!("SKT recv result: {:?}", recv_result);
//     //     None
//     // }
// }

pub async fn prepare_and_serve(
    my_id: GnomeId,
    dedicated_socket: UdpSocket,
    // remote_gnome_id: GnomeId,
    session_key: SessionKey,
    swarm_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // encrypter: Encrypter,
    pub_key_pem: String,
) {
    let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let remote_gnome_id = GnomeId(encr.hash());
    eprintln!("UDP client Remote GnomeId: {}", remote_gnome_id);
    // println!("Decrypted PEM using session key:\n {:?}", pub_key_pem);
    send_subscribed_swarm_names(&dedicated_socket, &swarm_names).await;

    let mut remote_names: Vec<SwarmName> = vec![];
    receive_remote_swarm_names(&dedicated_socket, &mut remote_names).await;
    if remote_names.is_empty() {
        eprintln!(
            "UDP Neighbor {} did not provide swarm list",
            remote_gnome_id
        );
        return;
    }

    let mut common_names = vec![];
    distil_common_names(
        my_id,
        remote_gnome_id,
        &mut common_names,
        swarm_names,
        &mut remote_names,
    );
    if common_names.is_empty() {
        eprintln!("UDP No common interests with {}", remote_gnome_id);
        return;
    }
    // eprintln!("Common swarm names: {:?}", common_names);
    let (shared_sender, swarm_extend_receiver) = channel();

    let mut ch_pairs = vec![];
    create_a_neighbor_for_each_swarm(
        common_names,
        remote_names,
        sender.clone(),
        remote_gnome_id,
        &mut ch_pairs,
        shared_sender.clone(),
        pub_key_pem,
    );

    // spawn a task to serve socket
    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = pipes_sender.send((token_send, token_recv_two));
    serve_socket(
        session_key,
        dedicated_socket,
        ch_pairs,
        token_send_two,
        token_recv,
        sender,
        shared_sender,
        swarm_extend_receiver,
    )
    .await;
}
