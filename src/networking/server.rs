use super::serve_socket;
use super::Token;
use crate::crypto::{generate_symmetric_key, SessionKey};
use crate::networking::common::collect_subscribed_swarm_names;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::receive_remote_swarm_names;
use crate::networking::common::send_subscribed_swarm_names;
use crate::networking::subscription::Subscription;
use crate::prelude::Encrypter;
use async_std::net::UdpSocket;
use async_std::task::spawn;
use bytes::{BufMut, BytesMut};
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeId;

pub async fn run_server(
    host_ip: IpAddr,
    socket: UdpSocket,
    sub_sender: Sender<Subscription>,
    mut sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
) {
    println!("--------------------------------------");
    println!("- - - - - - - - SERVER - - - - - - - -");
    println!("- Listens on: {:?}   -", socket.local_addr().unwrap());
    println!("--------------------------------------");
    println!("My Pubkey PEM:\n {:?}", pub_key_pem);
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    println!("My GnomeId: {}", gnome_id);
    loop {
        let mut remote_gnome_id: GnomeId = GnomeId(0);
        let mut session_key: SessionKey = SessionKey::from_key(&[0; 32]);
        let optional_addr = establish_secure_connection(
            &socket,
            &mut remote_gnome_id,
            &mut session_key,
            &pub_key_pem,
        )
        .await;
        if optional_addr.is_none() {
            println!("Failed to establish secure connection with Neighbor");
            continue;
        }
        let remote_addr = optional_addr.unwrap();

        println!("Waiting for data from remote...");
        let mut buf = BytesMut::zeroed(1030);
        let mut bytes = buf.split();

        let mut remote_names = vec![];
        receive_remote_swarm_names(&socket, &mut bytes, &mut remote_names).await;

        let mut common_names = vec![];
        let dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await.unwrap();

        let mut swarm_names = vec![];
        sub_receiver =
            collect_subscribed_swarm_names(&mut swarm_names, sub_sender.clone(), sub_receiver)
                .await;
        distil_common_names(&mut common_names, swarm_names, remote_names);
        send_subscribed_swarm_names(&socket, &common_names, remote_addr).await;

        buf.put(
            dedicated_socket
                .local_addr()
                .unwrap()
                .to_string()
                .as_bytes(),
        );
        let bytes_to_send = buf.split();

        let send_result = socket.send_to(&bytes_to_send, remote_addr).await;
        if send_result.is_err() {
            println!("Unable to send new socket addr: {:?}", send_result);
            // return sub_receiver;
            continue;
        }
        let count = send_result.unwrap();
        println!("SKT Sent {} bytes: {:?}", count, bytes_to_send);
        let conn_result = dedicated_socket.connect(remote_addr).await;
        if conn_result.is_err() {
            println!("Unable to connect dedicated socket: {:?}", conn_result);
            // return sub_receiver;
            continue;
        }

        println!("SKT Connected to client");

        let mut ch_pairs = vec![];
        create_a_neighbor_for_each_swarm(
            common_names,
            sub_sender.clone(),
            remote_gnome_id,
            &mut ch_pairs,
        );

        let (token_send, token_recv) = channel();
        let (token_send_two, token_recv_two) = channel();
        let _ = token_pipes_sender.send((token_send, token_recv_two));
        spawn(serve_socket(
            session_key,
            dedicated_socket,
            ch_pairs,
            token_send_two,
            token_recv,
        ));

        println!("--------------------------------------");
    }
}

async fn establish_secure_connection(
    socket: &UdpSocket,
    remote_gnome_id: &mut GnomeId,
    session_key: &mut SessionKey,
    pub_key_pem: &str,
) -> Option<SocketAddr> {
    let mut buf = BytesMut::zeroed(1030);
    let mut bytes = buf.split();
    let result = socket.recv_from(&mut bytes).await;
    if result.is_err() {
        println!("Failed to receive data on socket: {:?}", result);
        return None;
    }
    let (count, remote_addr) = result.unwrap();
    println!("SKT Received {} bytes", count);
    let id_pub_key_pem = std::str::from_utf8(&bytes[..count]).unwrap();
    println!("remote PEM:\n{}", id_pub_key_pem);
    let result = Encrypter::create_from_data(id_pub_key_pem);
    if result.is_err() {
        println!("Failed to build Encripter from received PEM: {:?}", result);
        return None;
    }
    let encr = result.unwrap();

    let key = generate_symmetric_key();
    let encr_res = encr.encrypt(&key);
    if encr_res.is_err() {
        println!("Failed to encrypt symmetric key: {:?}", encr_res);
        return None;
    }
    println!("Encrypted symmetric key");

    let encrypted_symmetric_key = encr_res.unwrap();
    let res = socket.send_to(&encrypted_symmetric_key, remote_addr).await;
    if res.is_err() {
        println!("Failed to send encrypted symmetric key: {:?}", res);
        return None;
    }
    println!("Sent encrypted symmetric key ");

    *session_key = SessionKey::from_key(&key);
    let my_encrypted_pubkey = session_key.encrypt(pub_key_pem.as_bytes());
    let res2 = socket.send_to(&my_encrypted_pubkey, remote_addr).await;
    if res2.is_err() {
        println!("Error sending encrypted pubkey response: {:?}", res2);
        return None;
    }
    println!("Sent encrypted public key");

    *remote_gnome_id = GnomeId(encr.hash());
    println!("Remote GnomeId: {}", remote_gnome_id);
    Some(remote_addr)
}
