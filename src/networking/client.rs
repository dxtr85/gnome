use super::serve_socket;
use super::Token;
use crate::crypto::SessionKey;
use crate::networking::common::collect_subscribed_swarm_names;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::receive_remote_swarm_names;
use crate::networking::common::send_subscribed_swarm_names;
use crate::networking::subscription::Subscription;
use crate::prelude::Encrypter;
use async_std::net::UdpSocket;
use async_std::task::{spawn, yield_now};
use bytes::BytesMut;
use core::panic;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeId;

pub async fn run_client(
    // server_addr: SocketAddr,
    // socket: UdpSocket,
    host_ip: IpAddr,
    receiver: Receiver<Subscription>,
    sender: Sender<Subscription>,
    // token_send: Sender<Token>,
    // token_recv: Receiver<Token>,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
) {
    println!("SKT CLIENT");
    let result = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await;
    if result.is_err() {
        println!("SKT couldn't bind to address");
        return;
    }
    let socket = result.unwrap();

    let result = socket.set_broadcast(true);
    if result.is_err() {
        println!("SKT couldn't enable broadcast");
        return;
    }

    let mut names = vec![];
    let receiver = collect_subscribed_swarm_names(&mut names, sender.clone(), receiver).await;
    if names.is_empty() {
        println!("User is not subscribed to any Swarms");
        return;
    }

    let mut remote_gnome_id: GnomeId = GnomeId(0);
    let mut session_key: SessionKey = SessionKey::from_key(&[0; 32]);
    let mut remote_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    establish_secure_connection(
        &socket,
        // &mut names,
        pub_key_pem.as_bytes(),
        &mut remote_addr,
        &mut remote_gnome_id,
        &mut session_key,
        sender.clone(),
        receiver,
    )
    .await;
    if remote_gnome_id.0 == 0 {
        println!("Failed to establish secure connection with Neighbor");
        return;
    }

    send_subscribed_swarm_names(&socket, &names, remote_addr).await;

    let mut recv_buf = BytesMut::zeroed(1024);
    let mut remote_names: Vec<String> = vec![];
    receive_remote_swarm_names(&socket, &mut recv_buf, &mut remote_names).await;
    if remote_names.is_empty() {
        println!("Neighbor {} did not provide swarm list", remote_gnome_id);
        return;
    }

    socket_connect(&socket, &mut recv_buf).await;

    let mut common_names = vec![];
    distil_common_names(&mut common_names, names, remote_names);
    if common_names.is_empty() {
        println!("No common interests with {}", remote_gnome_id);
        return;
    }

    let mut ch_pairs = vec![];
    create_a_neighbor_for_each_swarm(common_names, sender, remote_gnome_id, &mut ch_pairs);

    // spawn a task to serve socket
    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = pipes_sender.send((token_send, token_recv_two));
    spawn(serve_socket(
        session_key,
        socket,
        ch_pairs,
        token_send_two,
        token_recv,
    ))
    .await;
    println!("SKT run_client complete");
}

async fn establish_secure_connection(
    socket: &UdpSocket,
    pub_key_pem: &[u8],
    remote_addr: &mut SocketAddr,
    remote_gnome_id: &mut GnomeId,
    session_key: &mut SessionKey,
    sender: Sender<Subscription>,
    receiver: Receiver<Subscription>,
) {
    let send_result = socket.send_to(pub_key_pem, "255.255.255.255:1026").await;
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes", count);

        let mut recv_buf = BytesMut::zeroed(1024);
        let recv_result = socket.recv_from(&mut recv_buf).await;
        let mut decoded_key: Option<[u8; 32]> = None;
        println!("Dec key: {:?}", decoded_key);
        if let Ok((count, remote_adr)) = recv_result {
            *remote_addr = remote_adr;
            println!("Received {}bytes", count);

            let _res = sender.send(Subscription::Decode(Box::new(Vec::from(
                &recv_buf[..count],
            ))));
            println!("Sent decode request: {:?}", _res);
            loop {
                let response = receiver.try_recv();
                if let Ok(subs_resp) = response {
                    match subs_resp {
                        Subscription::KeyDecoded(symmetric_key) => {
                            decoded_key = Some(*symmetric_key);
                            break;
                        }
                        Subscription::DecodeFailure => panic!("Failed decoding symmetric key!"),
                        _ => println!("Unexpected message: {:?}", subs_resp),
                    }
                }
                yield_now().await
            }
            if let Some(sym_key) = decoded_key {
                *session_key = SessionKey::from_key(&sym_key);
                println!("Got session key: {:?}", sym_key);

                let mut recv_buf = BytesMut::zeroed(1024);
                let recv_result = socket.recv_from(&mut recv_buf).await;
                if let Ok((count, _remote_addr)) = recv_result {
                    println!("Received {}bytes", count);
                    let decr_res = session_key.decrypt(&recv_buf[..count]);
                    if let Ok(remote_pubkey_pem) = decr_res {
                        let remote_id_pub_key_pem =
                            std::str::from_utf8(&remote_pubkey_pem).unwrap();
                        let encr = Encrypter::create_from_data(remote_id_pub_key_pem).unwrap();
                        *remote_gnome_id = GnomeId(encr.hash());
                        println!("Remote GnomeId: {}", remote_gnome_id);
                        println!(
                            "Decrypted PEM using session key:\n {:?}",
                            remote_id_pub_key_pem
                        );
                    }
                }
            }
        }
    }
}

async fn socket_connect(socket: &UdpSocket, recv_buf: &mut BytesMut) {
    let recv_result = socket.recv_from(recv_buf).await;
    //TODO: compare received list of swarms with swarm_name and if matches continue
    if let Ok((count, _remote_addr)) = recv_result {
        // TODO: in future compare with list of swarms we are subscribed to
        let recv_str = String::from_utf8(Vec::from(&recv_buf[..count])).unwrap();

        println!("SKT Received {} bytes: {:?}", count, recv_str);
        let conn_result = socket.connect(recv_str).await;
        if conn_result.is_ok() {
            println!("SKT Connected to server");
        } else {
            println!("SKT Failed to connect");
        }
    } else {
        println!("SKT recv result: {:?}", recv_result);
    }
}
