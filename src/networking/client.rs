use super::serve_socket;
use super::Token;
use crate::networking::subscription::Subscription;
use crate::prelude::Encrypter;
// use crate::prelude::Encrypter;
use crate::crypto::SessionKey;
use std::net::{IpAddr, SocketAddr};
// use aes_gcm::aead::Buffer;
use async_std::net::UdpSocket;
// use async_std::stream::StreamExt;
use async_std::task::{spawn, yield_now};
use bytes::{BufMut, BytesMut};
use core::panic;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{GnomeId, Neighbor, SwarmTime};

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
    let socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
        .await
        .expect("SKT couldn't bind to address");
    let _ = socket.set_broadcast(true);
    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = pipes_sender.send((token_send, token_recv_two));

    println!("SKT CLIENT");
    let mut buf = BytesMut::with_capacity(128);

    let mut remote_gnome_id: GnomeId = GnomeId(0);
    let mut session_key: SessionKey = SessionKey::from_key(&[0; 32]);

    let send_result = socket
        .send_to(pub_key_pem.as_bytes(), "255.255.255.255:1026")
        .await;
    // println!("PEM send: {:?}", send_result);
    let mut server_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes", count);

        let mut names = vec![];
        println!("Names: {:?}", names);
        let mut recv_buf = BytesMut::zeroed(1024);
        let recv_result = socket.recv_from(&mut recv_buf).await;
        let mut decoded_key: Option<[u8; 32]> = None;
        println!("Dec key: {:?}", decoded_key);
        if let Ok((count, remote_addr)) = recv_result {
            server_addr = remote_addr;
            println!("Received {}bytes", count);

            let _res = sender.send(Subscription::Decode(Box::new(Vec::from(
                &recv_buf[..count],
            ))));
            println!("Sent decode request: {:?}", _res);
            loop {
                let response = receiver.try_recv();
                // print!("l");
                if let Ok(subs_resp) = response {
                    match subs_resp {
                        Subscription::KeyDecoded(symmetric_key) => {
                            decoded_key = Some(*symmetric_key);
                            break;
                        }
                        Subscription::DecodeFailure => panic!("Failed decoding symmetric key!"),
                        Subscription::Added(ref name) => {
                            names = vec![name.to_owned()];
                            println!("{:?}", names);
                        }
                        _ => println!("Unexpected message: {:?}", subs_resp),
                    }
                    // } else {
                    //     println!("Did not receive decoded symmetric key from local decoder");
                }
                yield_now().await
            }
            // println!("SKT Received {} bytes: {:?}", count, remote_pub_key_pem);
            if let Some(sym_key) = decoded_key {
                session_key = SessionKey::from_key(&sym_key);
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
                        remote_gnome_id = GnomeId(encr.hash());
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

    // let local_addr = socket.local_addr().unwrap().to_string();
    // TODO: send a list of swarms we are subscribed to
    println!("Collecting swarm names...");
    let mut names = vec![];
    loop {
        let _ = sender.send(Subscription::ProvideList);
        println!("sent req");
        // TODO: need rework
        yield_now().await;
        let mut recv_result = receiver.recv();
        println!("res: {:?}", recv_result);
        while let Ok(recv_rslt) = receiver.try_recv() {
            recv_result = Ok(recv_rslt);
        }
        // let recv_result = receiver.recv();
        // println!("res2: {:?}", recv_result);
        match recv_result {
            Ok(Subscription::Added(ref name)) => names = vec![name.to_owned()],
            Ok(Subscription::List(ref nnames)) => names = nnames.to_owned(),
            Ok(_) => names = vec![],
            Err(e) => {
                println!("Error: {:?}", e);
                // vec![]
            }
        };
        if names.len() > 0 {
            break;
        }
        yield_now().await;
    }
    // let names = if let Ok(Subscription::List(swarms)) = recv_result {
    //     println!("Sub list: {:?}", swarms);
    //     swarms
    // } else if let Ok(Subscription::Added(swarm_name)) = recv_result {
    //     // println!("Nothing received from sub service! {:?}", recv_result);
    //     vec![swarm_name]
    // };
    for name in &names {
        buf.put(name.as_bytes());
        buf.put_u8(255);
    }
    // buf.put(local_addr.as_bytes());
    let bytes = buf.split();
    println!("After split: {:?}", bytes);
    let send_result = socket.send_to(&bytes, server_addr).await;
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes", count);
    }
    let mut recv_buf = BytesMut::zeroed(1024);
    let recv_result = socket.recv_from(&mut recv_buf).await;
    //TODO: compare received list of swarms with swarm_name and if matches continue
    if let Ok((count, _remote_addr)) = recv_result {
        // TODO: in future compare with list of swarms we are subscribed to
        let recv_str = String::from_utf8(Vec::from(&recv_buf[..count])).unwrap();

        println!("SKT Received {} bytes: {:?}", count, recv_str);
        let conn_result = socket.connect(recv_str).await;
        if conn_result.is_ok() {
            println!("SKT Connected to server");
            let remote_names: Vec<String> =
                if let Ok((count, _from)) = socket.recv_from(&mut recv_buf).await {
                    recv_buf[..count]
                        .split(|n| n == &255u8)
                        .map(|bts| String::from_utf8(bts.to_vec()).unwrap())
                        .collect()
                } else {
                    Vec::new()
                };
            let mut common_names = vec![];
            for name in remote_names {
                // let name = String::from_utf8(bts).unwrap();
                if names.contains(&name) {
                    common_names.push(name.to_owned());
                }
            }
            // println!("Neighbor: {}", neighbor_id);
            let mut ch_pairs = vec![];
            // println!("komon names: {:?}", common_names);
            for name in common_names {
                let (s1, r1) = channel();
                let (s2, r2) = channel();
                let neighbor = Neighbor::from_id_channel_time(
                    remote_gnome_id,
                    r2,
                    s1,
                    SwarmTime(0),
                    SwarmTime(7),
                );
                println!("Request include neighbor");
                let _ = sender.send(Subscription::IncludeNeighbor(name, neighbor));
                ch_pairs.push((s2, r1));
            }
            spawn(serve_socket(
                session_key,
                socket,
                ch_pairs,
                token_send_two,
                token_recv,
            ))
            .await;
        } else {
            println!("SKT Failed to connect");
        }
    } else {
        println!("SKT recv result: {:?}", recv_result);
    }
    println!("SKT run_client complete");
}
