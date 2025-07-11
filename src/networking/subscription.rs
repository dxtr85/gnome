// use crate::crypto::Decrypter;
use async_std::channel::Sender as ASender;
use async_std::task::sleep;
// use rsa::sha2::digest::HashMarker;
use crate::networking::{Nat, NetworkSettings, PortAllocationRule};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use swarm_consensus::{Neighbor, SwarmName, ToGnome};

use crate::manager::ToGnomeManager;
use crate::networking::status::Transport;
use crate::networking::Notification;
// use crate::networking::NotificationBundle;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Requestor {
    Udp,
    Tcp,
    Udpv6,
    Tcpv6,
}

#[derive(Debug)]
pub enum Subscription {
    Added(SwarmName),
    Removed(SwarmName),
    ProvideList(Requestor),
    List(Vec<SwarmName>),
    IncludeNeighbor(SwarmName, Neighbor),
    Distribute(IpAddr, u16, Nat, (PortAllocationRule, i8), Transport),
    TransportNotAvailable(Requestor),
    TransportAvailable(Requestor),
    // Decode(Box<Vec<u8>>),
    // KeyDecoded(Box<[u8; 32]>),
    // DecodeFailure,
}

pub async fn subscriber(
    to_gmgr: ASender<ToGnomeManager>,
    sub_senders: HashMap<Requestor, Sender<Subscription>>,
    // sub_sender_bis: Sender<Subscription>,
    // decrypter: Decrypter,
    // pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // pub_key_pem: String,
    sub_receiver: Receiver<Subscription>,
    // sub_sender_two: Sender<Subscription>,
    notification_receiver: Receiver<Notification>,
    // token_dispenser_send: Sender<Sender<u64>>,
    holepunch_sender: Sender<SwarmName>,
    // direct_punch_sender: Sender<(SwarmName, Sender<ToGnome>, Receiver<NetworkSettings>)>,
    direct_punch_sender: Sender<(SwarmName, Sender<ToGnome>, Receiver<Vec<u8>>)>,
) {
    let mut swarms: HashMap<SwarmName, Sender<ToGnome>> = HashMap::with_capacity(10);
    let mut names: Vec<SwarmName> = Vec::with_capacity(10);
    eprintln!("Subscriber service started");
    let mut notify_holepunch = true;
    let sleep_time = Duration::from_millis(8);
    // let mut pending_neighbors = HashMap::new();
    loop {
        // print!("sub");
        let recv_result = notification_receiver.try_recv();
        match recv_result {
            Ok(notif) => {
                match notif {
                    Notification::SetFounder(founder) => {
                        eprintln!("NotiFounder: {}", founder);
                        for name in &mut names {
                            if name.founder.is_any() {
                                eprintln!("Subscription setting founder to: {}", founder);
                                name.founder = founder;
                            }
                        }
                    }
                    Notification::RemoveSwarm(swarm_names) => {
                        for swarm_name in swarm_names {
                            eprintln!("Networking about to remove {}", swarm_name);
                            //TODO: should direct punch be notified?
                            let res = swarms.remove(&swarm_name);
                            eprintln!("Removed from swarms list: {}", res.is_some());
                            let mut name_idx = None;
                            for (idx, name) in names.iter().enumerate() {
                                if *name == swarm_name {
                                    name_idx = Some(idx);
                                    break;
                                }
                            }
                            if let Some(idx) = name_idx {
                                eprintln!("Removing from names list");
                                names.remove(idx);
                            }
                            for sender in sub_senders.values() {
                                let _ = sender.send(Subscription::Removed(swarm_name.clone()));
                            }
                            //TODO: should token dispenser be informed?
                        }
                    }
                    Notification::AddSwarm(notif_bundle) => {
                        // TODO: only one punching service for all swarms!
                        // TODO: find a way to update swarm_name once founder is determined
                        eprintln!(
                            "Subscription received Bundle for {}",
                            notif_bundle.swarm_name
                        );
                        let _ = direct_punch_sender.send((
                            notif_bundle.swarm_name.clone(),
                            notif_bundle.request_sender.clone(),
                            notif_bundle.network_settings_receiver,
                        ));
                        // if let Some(neighbors) = pending_neighbors.remove(&notif_bundle.swarm_name)
                        // {
                        //     eprintln!("Sending pending neighbors…");
                        //     for neighbor in neighbors {
                        //         let _ = notif_bundle
                        //             .request_sender
                        //             .send(ToGnome::AddNeighbor(neighbor));
                        //     }
                        // }
                        swarms.insert(notif_bundle.swarm_name.clone(), notif_bundle.request_sender);
                        if !names.contains(&notif_bundle.swarm_name) {
                            names.push(notif_bundle.swarm_name.clone());
                        }
                        // TODO: inform existing sockets about new subscription
                        eprintln!("Added swarm in networking: {}", notif_bundle.swarm_name);
                        // TODO: serve err results
                        for sender in sub_senders.values() {
                            let _ =
                                sender.send(Subscription::Added(notif_bundle.swarm_name.clone()));
                        }
                        // let _ =
                        //     sub_sender.send(Subscription::Added(notif_bundle.swarm_name.clone()));
                        // let _ = sub_sender_bis
                        //     .send(Subscription::Added(notif_bundle.swarm_name.clone()));
                        if notify_holepunch {
                            let h_res = holepunch_sender.send(notif_bundle.swarm_name);
                            if h_res.is_err() {
                                notify_holepunch = false;
                            }
                        }
                        // let _ = token_dispenser_send.send(notif_bundle.token_sender);
                        // TODO: sockets should be able to respond if they want to join
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                eprintln!("Subscriber disconnected");
                break;
            }
            Err(_) => {}
        }
        if let Ok(sub) = sub_receiver.try_recv() {
            // eprintln!("Received: {:?}", sub);
            match sub {
                Subscription::IncludeNeighbor(swarm, neighbor) => {
                    if let Some(sender) = swarms.get(&swarm) {
                        eprintln!("Subscription req add {} to {}", neighbor.id, swarm);
                        let _ = sender.send(ToGnome::AddNeighbor(neighbor));
                    } else if swarms.len() == 1 && swarms.keys().next().unwrap().founder.is_any() {
                        eprintln!("Setting generic swarm founder to: {}", swarm.founder);
                        // let key = swarms.keys().next().unwrap();
                        let existing_sender = swarms.into_values().next().unwrap();
                        let _ = existing_sender.send(ToGnome::SetFounder(swarm.founder));
                        let _ = existing_sender.send(ToGnome::AddNeighbor(neighbor));
                        swarms = HashMap::new();
                        names = vec![swarm.clone()];
                        swarms.insert(swarm, existing_sender.clone());
                    } else {
                        eprintln!("No sender for {} found", swarm);
                        // pending_neighbors
                        //     .entry(swarm)
                        //     .or_insert(vec![])
                        //     .push(neighbor);
                    }
                }
                Subscription::ProvideList(req) => {
                    if let Some(sender) = sub_senders.get(&req) {
                        eprintln!("sub sending:");
                        for name in &names {
                            eprintln!("{}", name);
                        }
                        let _ = sender.send(Subscription::List(names.clone()));
                    }
                    // if matches!(req, Requestor::Udp) {
                    //     let _ = sub_sender.send(Subscription::List(names.clone()));
                    // } else {
                    //     let _ = sub_sender_bis.send(Subscription::List(names.clone()));
                    // }
                }
                Subscription::Distribute(ip, port, nat, port_rule, tr) => {
                    // we send this to Gnome manager instead,
                    // then sending it just once will do
                    eprintln!("Received Distribute, sending PubAddr");
                    let _ = to_gmgr
                        .send(ToGnomeManager::PublicAddress(ip, port, nat, port_rule, tr))
                        .await;
                    // if let Some(sender) = swarms.values().next().take() {
                    // for sender in swarms.values() {
                    //     let request =
                    //         ToGnome::NetworkSettingsUpdate(false, ip, port, nat, port_rule);
                    //     let _ = sender.send(request);
                    // }
                }
                Subscription::TransportNotAvailable(req) => {
                    //TODO
                    let (ip, tr) = match req {
                        Requestor::Udp => (IpAddr::from_str("0.0.0.0").unwrap(), Transport::Udp),
                        Requestor::Tcp => (IpAddr::from_str("0.0.0.0").unwrap(), Transport::Tcp),
                        Requestor::Udpv6 => (IpAddr::from_str("::").unwrap(), Transport::Udp),
                        Requestor::Tcpv6 => (IpAddr::from_str("::").unwrap(), Transport::Tcp),
                    };
                    let _ = to_gmgr
                        .send(ToGnomeManager::PublicAddress(
                            ip,
                            0,
                            Nat::Unknown,
                            (PortAllocationRule::Random, 0),
                            tr,
                        ))
                        .await;
                    // if let Some(sender) = swarms.values().next().take() {
                    //     let request = ToGnome::NetworkSettingsUpdate(
                    //         false,
                    //         ip,
                    //         0,
                    //         Nat::Unknown,
                    //         (PortAllocationRule::Random, 0),
                    //     );
                    //     let _ = sender.send(request);
                    // }
                }
                Subscription::TransportAvailable(req) => {
                    let (ip, tr) = match req {
                        Requestor::Udp => (IpAddr::from_str("0.0.0.0").unwrap(), Transport::Udp),
                        Requestor::Tcp => (IpAddr::from_str("0.0.0.0").unwrap(), Transport::Tcp),
                        Requestor::Udpv6 => (IpAddr::from_str("::").unwrap(), Transport::Udp),
                        Requestor::Tcpv6 => (IpAddr::from_str("::").unwrap(), Transport::Tcp),
                    };
                    let _ = to_gmgr
                        .send(ToGnomeManager::PublicAddress(
                            ip,
                            1,
                            Nat::Unknown,
                            (PortAllocationRule::Random, 0),
                            tr,
                        ))
                        .await;
                    // if let Some(sender) = swarms.values().next().take() {
                    //     let request = ToGnome::NetworkSettingsUpdate(
                    //         false,
                    //         ip,
                    //         1,
                    //         Nat::Unknown,
                    //         (PortAllocationRule::Random, 0),
                    //     );
                    //     let _ = sender.send(request);
                    // }
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
                    eprintln!("Unexpected msg: {:?}", other)
                }
            }
        }
        // print!("Y");
        sleep(sleep_time).await;
    }
}
