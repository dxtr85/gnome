// use crate::crypto::Decrypter;
use crate::prelude::Decrypter;
use async_std::task::{spawn, yield_now};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc::{Receiver, Sender};
use swarm_consensus::{Nat, Neighbor, NetworkSettings, Request};

use super::token::Token;

#[derive(Debug)]
pub enum Subscription {
    Added(String),
    // Removed(String),
    ProvideList,
    List(Vec<String>),
    IncludeNeighbor(String, Neighbor),
    Distribute(IpAddr, u16, Nat),
    // Decode(Box<Vec<u8>>),
    // KeyDecoded(Box<[u8; 32]>),
    // DecodeFailure,
}

pub async fn subscriber(
    host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    sub_receiver: Receiver<Subscription>,
    sub_sender_two: Sender<Subscription>,
    notification_receiver: Receiver<(
        String,
        Sender<Request>,
        Sender<u32>,
        Receiver<NetworkSettings>,
    )>,
    token_dispenser_send: Sender<Sender<u32>>,
    holepunch_sender: Sender<String>,
    direct_punch_sender: Sender<(String, Sender<Request>, Receiver<NetworkSettings>)>,
) {
    let mut swarms: HashMap<String, Sender<Request>> = HashMap::with_capacity(10);
    let mut names: Vec<String> = Vec::with_capacity(10);
    println!("Subscriber service started");
    let mut notify_holepunch = true;
    loop {
        // print!("sub");
        let recv_result = notification_receiver.try_recv();
        match recv_result {
            Ok((swarm_name, req_sender, band_sender, net_set_recv)) => {
                // TODO: only one punching service for all swarms!
                let _ = direct_punch_sender.send((
                    swarm_name.clone(),
                    req_sender.clone(),
                    net_set_recv,
                ));
                swarms.insert(swarm_name.clone(), req_sender);
                names.push(swarm_name.clone());
                // TODO: inform existing sockets about new subscription
                println!("Added swarm: {}", swarm_name);
                // TODO: serve err results
                let _ = sub_sender.send(Subscription::Added(swarm_name.clone()));
                if notify_holepunch {
                    let h_res = holepunch_sender.send(swarm_name);
                    if h_res.is_err() {
                        notify_holepunch = false;
                    }
                }
                let _ = token_dispenser_send.send(band_sender);
                // TODO: sockets should be able to respond if they want to join
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                println!("subscriber disconnected from Manager");
                break;
            }
            Err(_) => {}
        }
        if let Ok(sub) = sub_receiver.try_recv() {
            // println!("Received: {:?}", sub);
            match sub {
                Subscription::IncludeNeighbor(swarm, neighbor) => {
                    if let Some(sender) = swarms.get(&swarm) {
                        let _ = sender.send(Request::AddNeighbor(neighbor));
                    } else {
                        println!("No sender for {} found", swarm);
                    }
                }
                Subscription::ProvideList => {
                    println!("sub sending: {:?}", names);
                    let _ = sub_sender.send(Subscription::List(names.clone()));
                }
                Subscription::Distribute(ip, port, nat) => {
                    for sender in swarms.values() {
                        let request = Request::NetworkSettingsUpdate(false, ip, port, nat);
                        let _ = sender.send(request);
                    }
                }
                // Subscription::Decode(msg) => {
                // println!("decoding: {:?}", msg);
                // let decode_res = decrypter.decrypt(msg.deref());
                // if let Ok(decoded) = decode_res {
                //     if decoded.len() == 32 {
                //         // let sym_port_key: [u8; 34] = decoded.try_into().unwrap();
                //         // let sym_key: [u8; 32] = sym_port_key[2..].try_into().unwrap();
                //         let sym_key: [u8; 32] = decoded.try_into().unwrap();
                //         // let port: u16 =
                //         //     u16::from_be_bytes(sym_port_key[0..2].try_into().unwrap());
                //         println!("succesfully decoded");
                //         let _ = sub_sender.send(Subscription::KeyDecoded(Box::new(sym_key)));
                //     } else {
                //         println!("Decoded symmetric key has wrong size: {}", decoded.len());
                //         let _ = sub_sender.send(Subscription::DecodeFailure);
                //     }
                // } else {
                //     println!("Failed decoding message: {:?}", decode_res);
                //     let _ = sub_sender.send(Subscription::DecodeFailure);
                // }
                // }
                other => {
                    println!("Unexpected msg: {:?}", other)
                }
            }
        }
        // print!("Y");
        yield_now().await;
    }
}
