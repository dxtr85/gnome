use super::serve_socket;
use super::Token;
use crate::networking::subscription::Subscription;
use async_std::net::UdpSocket;
use async_std::task::spawn;
use bytes::{BufMut, BytesMut};
use core::panic;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{GnomeId, Neighbor, SwarmTime};

use std::net::{IpAddr, SocketAddr};

use crate::prelude::Encrypter;
// use crate::prelude::Encrypter;
use crate::crypto::{generate_symmetric_key, SessionKey};

pub async fn run_server(
    // gnome_id: GnomeId,
    host_ip: IpAddr,
    socket: UdpSocket,
    sub_sender: Sender<Subscription>,
    sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
) {
    println!("--------------------------------------");
    println!("- - - - - - - - SERVER - - - - - - - -");
    println!("- Listens on: {:16?}   -", socket.local_addr().unwrap());
    println!("--------------------------------------");
    let mut buf = BytesMut::zeroed(1030);
    let mut bytes = buf.split();
    loop {
        // println!("loopa");
        // if let Ok((swarm_name, sender)) = subscription_receiver.try_recv() {
        // swarms.insert(swarm_name, sender);
        // TODO: inform existing sockets about new subscription
        // TODO: sockets should be able to respond if they want to join
        // }

        // First we receive pubkey_pem
        // TODO: use pubkey to establish GnomeId
        // let mut encrypter: Option<Encrypter> = None;
        let result = socket.recv_from(&mut bytes).await;
        let remote_gnome_id: GnomeId;
        let session_key: SessionKey;
        if let Ok((count, remote_addr)) = result {
            println!("SKT Received {} bytes", count);
            let id_pub_key_pem = std::str::from_utf8(&bytes[..count]).unwrap();
            println!("remote PEM:\n{}", id_pub_key_pem);
            if let Ok(encr) = Encrypter::create_from_data(id_pub_key_pem) {
                remote_gnome_id = GnomeId(encr.hash());
                println!("Remote GnomeId: {}", remote_gnome_id);
                // encrypter = Some(encr.clone());
                //TODO: here we need to create AES symmetric key and send it
                let key = generate_symmetric_key();
                let encr_res = encr.encrypt(&key);
                session_key = SessionKey::from_key(&key);
                println!("My Pubkey PEM:\n {:?}", pub_key_pem);
                let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
                let gnome_id = GnomeId(encr.hash());
                println!("My GnomeId:\n {}", gnome_id);
                let encrypted_pubkey = session_key.encrypt(pub_key_pem.as_bytes());
                println!("Encrypting key: {:?}", encr_res);
                if let Ok(encrypted_symmetric_key) = encr_res {
                    let res = socket.send_to(&encrypted_symmetric_key, remote_addr).await;
                    if res.is_ok() {
                        println!("Sent encrypted symmetric key ");
                        let res2 = socket.send_to(&encrypted_pubkey, remote_addr).await;
                        if res2.is_ok() {
                            println!("Sent encrypted public key");
                        } else {
                            println!("Error sending encrypted pubkey response: {:?}", res);
                        }
                    } else {
                        println!("Error sending encrypted symmetric key: {:?}", res);
                    }
                    // panic!("Remove me2!");
                }
            } else {
                panic!("Could not build encrypter from received data!");
            };
        } else {
            panic!("Remove me3!");
        }
        println!("Waiting for data from remote...");
        let result = socket.recv_from(&mut bytes).await;
        if let Ok((count, remote_addr)) = result {
            print!("SKT Received {} bytes: ", count);
            // TODO: recv_str should contain a list of swarms that remote gnome
            //       wants to join
            let names: &Vec<String> = &bytes[..count]
                .split(|n| n == &255u8)
                .map(|bts| String::from_utf8(bts.to_vec()).unwrap())
                .collect();
            // let bytes_to_names = &bytes[..count].split(|n| n == &0u8).collect();
            // let mut names = vec![];
            // for bts in bytes_to_names {
            //     let name = String::from_utf8(bts).unwrap();
            //     names.push(name);
            // }
            let mut common_names = vec![];
            // let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
            // println!("his: {:?}", names);
            // let remote_addr: SocketAddr = recv_str.parse().unwrap();
            let dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await.unwrap();
            // TODO: send swarm names we subscribe and match remote interests
            let _ = sub_sender.send(Subscription::ProvideList);
            let recv_result = sub_receiver.recv();
            let swarm_names = match recv_result {
                Ok(Subscription::Added(name)) => vec![name],
                Ok(Subscription::List(names)) => names,
                Ok(_) => vec![],
                Err(e) => {
                    println!("Error: {:?}", e);
                    vec![]
                }
            };
            // println!("my: {:?}", swarm_names);
            // if let Ok(Subscription::List(swarms)) = sub_receiver.recv() {
            // } else {
            //     println!("did not recv subscription list");
            // }
            buf.put(
                dedicated_socket
                    .local_addr()
                    .unwrap()
                    .to_string()
                    .as_bytes(),
            );
            let bytes_to_send = buf.split();
            // println!("sending: {:?}", bytes_to_send);
            let send_result = socket.send_to(&bytes_to_send, remote_addr).await;
            if let Ok(count) = send_result {
                println!("SKT Sent {} bytes: {:?}", count, bytes_to_send);
                let conn_result = dedicated_socket.connect(remote_addr).await;
                if let Ok(()) = conn_result {
                    println!("SKT Connected to client");
                    for swarm_name in swarm_names {
                        if names.contains(&swarm_name) {
                            common_names.push(swarm_name.clone());
                            buf.put(swarm_name.as_bytes());
                            buf.put_u8(255);
                        }
                    }
                    let to_send = buf.split();
                    let _ = dedicated_socket.send(&to_send).await;
                    // Here we send our GnomeId
                    // TODO: no need to send GnomeId anymore
                    // let _send_result = dedicated_socket.send(&gnome_id.0.to_be_bytes()).await;
                    // println!("Send result: {:?}", _send_result);
                    // let mut rbuf = BytesMut::zeroed(4);
                    // let recv_result = dedicated_socket.recv(&mut rbuf).await;
                    // println!("Recv result: {:?}", recv_result);
                    // println!("{:?}", rbuf);
                    // let mut neighbor_id = GnomeId(0);
                    // if let Ok(size) = recv_result {
                    //     if size == 4 {
                    // TODO: fix this, now we set GnomeId as a hash of his pubkey PEM
                    // let num: u32 = u32::from_be(rbuf.a);
                    // println!("r3: {}", rbuf[3]);
                    // let num: u32 = (rbuf[0] as u32)
                    //     << 24 + (rbuf[1] as u32)
                    //     << 16 + (rbuf[2] as u32)
                    //     << 8 + rbuf[3];
                    //         let num: u64 = rbuf[3] as u64;
                    //         neighbor_id = GnomeId(num);
                    //         println!("NeighborId updated: {}", num);
                    //     }
                    // }
                    // println!("Neighbor: {}", neighbor_id);
                    // TODO: for each element we sent create a Neighbor
                    // and send it down the pipe
                    // let (s1, r1) = channel();
                    // let (s2, r2) = channel();
                    // TODO: put s1 & r2 into neighbor instead of following
                    // let send_result = sender.send((s1, r2));
                    // TODO: read subscribed swarm names from socket
                    // for each matching sname create a neighbor and
                    // send it to service via sender from swarms hashmap
                    // rewrite serve socket and message exchange to
                    // include agreed upon preamble informing of
                    // swarm that is supposed to receive given message

                    // TODO: how to update a socket about changed list of
                    // subscribed swarms? (Probably with channels ;)
                    // TODO: same as above goes to run_client
                    let mut ch_pairs = vec![];
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
                        // println!("Request include neighbor");

                        let _ = sub_sender.send(Subscription::IncludeNeighbor(name, neighbor));
                        ch_pairs.push((s2, r1));
                    }
                    if send_result.is_err() {
                        println!("send result: {:?}", send_result);
                    }
                    if send_result.is_ok() {
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
                        // spawn(serve_socket(dedicated_socket, networking_receiver));
                    }
                    println!("--------------------------------------");
                }
            }
        }
    }
}
