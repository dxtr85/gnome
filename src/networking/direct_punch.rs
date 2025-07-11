// use super::common::are_we_behind_a_nat;
use super::common::{are_we_behind_a_nat, discover_network_settings};
// use super::token::Token;
use crate::crypto::{Decrypter, Encrypter};
use crate::manager::ToGnomeManager;
use crate::networking::holepunch::cluster_punch_it;
use crate::networking::status::Transport;
use crate::networking::tcp_client::run_tcp_client;
use crate::networking::{
    client::run_client,
    holepunch::{punch_it, start_communication},
    subscription::Subscription,
};
use async_std::channel::Sender as ASender;
// use crate::GnomeId;
use crate::networking::{Nat, NetworkSettings, PortAllocationRule, Transport as GTransport};
use async_std::net::UdpSocket;
use async_std::task::{sleep, spawn};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::{GnomeId, SwarmName, ToGnome};

pub async fn direct_punching_service(
    to_gmgr: ASender<ToGnomeManager>,
    _server_port: u16,
    subscription_sender: Sender<Subscription>,
    decrypter: Decrypter,
    // token_endpoints_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_endpoints_receiver: Receiver<(SwarmName, Sender<ToGnome>, Receiver<Vec<u8>>)>,
    pub_key_pem: String,
) {
    eprintln!("Waiting for direct connect requests.");

    // TODO: here we need to write a new procedure.
    // It all depends on NAT and port assignment rule,
    // but in case of Full Cone port assignment and any NAT type,
    // All the time we should have one port ready for receiving external
    // connection.
    // This port should be periodically refreshed so that NAT does not
    // conclude it being no longer used and perhaps assign it to someone else.
    // Once we receive remote ip and port we spawn punch_it service
    // and pass it our existing socket.
    // Then we create another socket and send notification
    // to inform gnome about our new socket in case other neighbors
    // want to communicate with us.
    //
    // In other cases procedure is to be determined...
    // Actually this procedure should also apply to above mentioned NAT config.
    // We simply omit parts of it in case it is not necessary to refresh our
    // port, or simply ask STUN server for confirmation about what port
    // we currently have externally.
    // We need a way for gnome to inform networking about the need for new
    // socket.
    // Once networking receives such a request it performs a STUN query in
    // order to learn current external ip and port, and given discovered
    // NAT type, port allocation rule and delta port
    // send back a notification to gnome informing him about our current
    // external socket configuration.
    // Then we receive trough settings_result external ip and port
    // of joining party and use our predicted ip and port to connect to it.

    let mut swarms: HashMap<SwarmName, (Sender<ToGnome>, Receiver<Vec<u8>>)> =
        HashMap::with_capacity(10);
    let (send_other_network_settings, recv_other_network_settings) = channel();
    let (send_my_network_settings, recv_my_network_settings) = channel();
    // TODO: maybe only run it when it makes sense?
    spawn(socket_maintainer(
        to_gmgr.clone(),
        pub_key_pem.clone(),
        // gnome_id,
        subscription_sender.clone(),
        decrypter.clone(),
        // token_endpoints_sender.clone(),
        send_my_network_settings,
        recv_other_network_settings,
    ));
    // println!("after sm spawn");

    let mut waiting_for_my_settings = false;
    let mut send_to_gnome: Option<Sender<ToGnome>> = None;
    let sleep_time = Duration::from_millis(16);
    let mut loop_counter: u8 = 0;
    loop {
        loop_counter = loop_counter.wrapping_add(1);
        // print!("dps");
        if let Ok((swarm_name, to_gnome_sender, net_set_recv)) = swarm_endpoints_receiver.try_recv()
        {
            // eprintln!("DPunch received channels for {}", swarm_name);
            swarms.insert(swarm_name, (to_gnome_sender, net_set_recv));
        }
        if !waiting_for_my_settings || loop_counter % 16 == 0 {
            for (swarm_name, (to_gnome_sender, net_set_recv)) in &swarms {
                let settings_result = net_set_recv.try_recv();
                // match settings_result {
                if let Ok(other_settings) = settings_result {
                    eprintln!("DP Got {:?}", other_settings);
                    // eprintln!("Maybe should also send those to TCP?");
                    // // if other_settings.no_nat()
                    // ns_to_gmgr.push(other_settings.clone());
                    let _ = send_other_network_settings.send((swarm_name.clone(), other_settings));
                    // eprintln!("DPunch waiting for my settings: TRUE");
                    waiting_for_my_settings = true;
                    send_to_gnome = Some(to_gnome_sender.clone());
                }
            }
        } else {
            let recv_result = recv_my_network_settings.try_recv();
            if let Ok(my_settings) = recv_result {
                if my_settings.pub_port == 0 {
                    let _ = send_to_gnome.take();
                } else {
                    // let request = ToGnome::NetworkSettingsUpdate(
                    //     true,
                    //     my_settings.pub_ip,
                    //     my_settings.pub_port,
                    //     my_settings.nat_type,
                    //     my_settings.port_allocation,
                    // );
                    if let Some(_gnome) = send_to_gnome.take() {
                        // let _ = to_gnome.send(request);
                        let _ = to_gmgr
                            .send(ToGnomeManager::PublicAddress(
                                my_settings.pub_ip,
                                my_settings.pub_port,
                                my_settings.nat_type,
                                my_settings.port_allocation,
                                Transport::Udp,
                            ))
                            .await;
                    }
                }
                // eprintln!("DPunch waiting for my settings: FALSE");
                waiting_for_my_settings = false;
            }
        }
        // while let Some(ns) = ns_to_gmgr.pop() {
        //     let _ = to_gmgr.send(ToGnomeManager::TryTcpConnect(ns)).await;
        // }
        // yield_now().await;
        sleep(sleep_time).await;
    }
}

/// This function runs in background awaiting for any Neighbor's NetworkSettings
/// Once it receives a NetworkSettings struct, it tries to open a communication
/// channel with that Neighbor and spawns a new socket for any new incoming NetworkSettings
async fn socket_maintainer(
    _to_gmgr: ASender<ToGnomeManager>,
    pub_key_pem: String,
    // gnome_id: GnomeId,
    subscription_sender: Sender<Subscription>,
    decrypter: Decrypter,
    // token_endpoints_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    my_network_settings_sender: Sender<NetworkSettings>,
    // other_network_settings_reciever: Receiver<(SwarmName, NetworkSettings)>,
    other_network_settings_reciever: Receiver<(SwarmName, Vec<u8>)>,
) {
    //TODO: Right now we only support communication over IPv4
    // since bind_addr is an IPv4 address.
    // In order to support an IPv6 address we just need to try creating
    // a new IPv6 socket. If that succeeds we can continue
    let mut swarm_names = vec![];
    // println!("SM start");
    // let bind_port = 0;
    // let bind_addr = (host_ip, bind_port);
    // let bind_addr = (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0u16);
    let bind_addr = (IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0u16);
    let mut socket = UdpSocket::bind(bind_addr).await.unwrap();
    let mut my_settings = discover_network_settings(&mut socket).await;

    // TODO: race two tasks: either timeout or receive NetworkSettings from pipe
    //       if timeout - we need to refresh the socket by sending to STUN server
    // let timeout_sec = Duration::from_secs(14);
    let trigger_update_at_iter = 14;
    let mut current_iter = 0;
    // let (t_send, timeout) = channel();
    // spawn(time_out(timeout_sec, Some(t_send.clone())));
    let sleep_time = Duration::from_secs(1);
    loop {
        current_iter += 1;
        sleep(sleep_time).await;
        if current_iter > trigger_update_at_iter {
            current_iter = 0;
            my_settings = update_my_pub_addr(&socket, my_settings).await;
            //     spawn(time_out(timeout_sec, Some(t_send.clone())));
        }
        // let recv_result = recv_other.try_recv();
        while let Ok((swarm_name, other_settings)) = other_network_settings_reciever.try_recv() {
            if !swarm_names.contains(&swarm_name) {
                swarm_names.push(swarm_name.clone());
            }
            // TODO: discover with stun server
            let mut other_settings_bundle = NetworkSettings::from(&other_settings);
            eprintln!("DP recvd other: {:?}", other_settings_bundle);
            if other_settings_bundle.is_empty() {
                eprintln!("Unable to construct NetworkSettings from bytes");
                continue;
            }
            while let Some(other_settings) = other_settings_bundle.pop() {
                if other_settings.pub_ip.is_ipv6() {
                    let bind_addr = (IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)), 0u16);
                    if other_settings.transport == GTransport::UDPoverIP6 {
                        if let Ok(socket) = UdpSocket::bind(bind_addr).await {
                            let ping_result = are_we_behind_a_nat(&socket).await;
                            if let Ok((_nat, _port_control, our_addr)) = ping_result {
                                let nat_type = if !_nat {
                                    Nat::None
                                } else if _port_control {
                                    // TODO: maybe add UnknownWithPortControl?
                                    Nat::SymmetricWithPortControl
                                } else {
                                    Nat::Unknown
                                };
                                let my_ipv6_settings = NetworkSettings {
                                    pub_ip: our_addr.ip(),
                                    pub_port: our_addr.port(),
                                    // pub_port: server_port + 1,
                                    nat_type,
                                    port_allocation: (PortAllocationRule::FullCone, 0),
                                    transport: GTransport::UDPoverIP6,
                                };
                                eprintln!("My IPv6 addr: {:?}", our_addr.ip());
                                let _ = my_network_settings_sender.send(my_ipv6_settings);
                            } else {
                                eprintln!(
                                    "Failed to receive back STUN response via IPv6: {:?}",
                                    ping_result.err().unwrap()
                                );
                                let my_ipv6_settings = NetworkSettings {
                                    pub_ip: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 3)),
                                    pub_port: 2,
                                    nat_type: Nat::Unknown,
                                    port_allocation: (PortAllocationRule::FullCone, 0),
                                    transport: GTransport::UDPoverIP6,
                                };
                                let _ = my_network_settings_sender.send(my_ipv6_settings);
                            }
                            // let my_addr = socket.local_addr().unwrap();
                            // let my_ipv6_settings = NetworkSettings {
                            //     pub_ip: my_addr.ip(),
                            //     // pub_port: my_addr.port(),
                            //     pub_port: server_port + 1,
                            //     nat_type: swarm_consensus::Nat::None,
                            //     port_allocation: (PortAllocationRule::FullCone, 0),
                            // };
                            // let _ = my_network_settings_sender.send(my_ipv6_settings);
                            spawn(punch_and_communicate(
                                socket,
                                bind_addr,
                                pub_key_pem.clone(),
                                // _to_gmgr.clone(),
                                subscription_sender.clone(),
                                decrypter.clone(),
                                // token_endpoints_sender.clone(),
                                swarm_names.clone(),
                                (my_settings, other_settings),
                            ));
                        } else {
                            eprintln!("Unable to bind IPv6 socket");
                            let my_ipv6_settings = NetworkSettings {
                                pub_ip: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
                                pub_port: 0,
                                nat_type: Nat::Unknown,
                                port_allocation: (PortAllocationRule::FullCone, 0),
                                transport: GTransport::UDPoverIP6,
                            };
                            let _ = my_network_settings_sender.send(my_ipv6_settings);
                        }
                    } else if other_settings.transport == GTransport::TCPoverIP6 {
                        //TODO: run TCP client here
                        if let Ok(socket) = UdpSocket::bind(bind_addr).await {
                            let ping_result = are_we_behind_a_nat(&socket).await;
                            if let Ok((_nat, _port_control, our_addr)) = ping_result {
                                let nat_type = if !_nat {
                                    Nat::None
                                } else if _port_control {
                                    // TODO: maybe add UnknownWithPortControl?
                                    Nat::SymmetricWithPortControl
                                } else {
                                    Nat::Unknown
                                };
                                let my_ipv6_settings = NetworkSettings {
                                    pub_ip: our_addr.ip(),
                                    pub_port: our_addr.port(),
                                    // pub_port: server_port + 1,
                                    nat_type,
                                    port_allocation: (PortAllocationRule::FullCone, 0),
                                    transport: GTransport::UDPoverIP6,
                                };
                                eprintln!("My IPv6 addr: {:?}", our_addr.ip());
                                let _ = my_network_settings_sender.send(my_ipv6_settings);
                            } else {
                                eprintln!(
                                    "Failed to receive back STUN response via IPv6: {:?}",
                                    ping_result.err().unwrap()
                                );
                                let my_ipv6_settings = NetworkSettings {
                                    pub_ip: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 3)),
                                    pub_port: 2,
                                    nat_type: Nat::Unknown,
                                    port_allocation: (PortAllocationRule::FullCone, 0),
                                    transport: GTransport::UDPoverIP6,
                                };
                                let _ = my_network_settings_sender.send(my_ipv6_settings);
                            }
                            // let my_addr = socket.local_addr().unwrap();
                            // let my_ipv6_settings = NetworkSettings {
                            //     pub_ip: my_addr.ip(),
                            //     // pub_port: my_addr.port(),
                            //     pub_port: server_port + 1,
                            //     nat_type: swarm_consensus::Nat::None,
                            //     port_allocation: (PortAllocationRule::FullCone, 0),
                            // };
                            // let _ = my_network_settings_sender.send(my_ipv6_settings);
                            spawn(punch_and_communicate(
                                socket,
                                bind_addr,
                                pub_key_pem.clone(),
                                // _to_gmgr.clone(),
                                subscription_sender.clone(),
                                decrypter.clone(),
                                // token_endpoints_sender.clone(),
                                swarm_names.clone(),
                                (my_settings, other_settings),
                            ));
                        } else {
                            eprintln!("Unable to bind IPv6 socket");
                            let my_ipv6_settings = NetworkSettings {
                                pub_ip: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
                                pub_port: 0,
                                nat_type: Nat::Unknown,
                                port_allocation: (PortAllocationRule::FullCone, 0),
                                transport: GTransport::UDPoverIP6,
                            };
                            let _ = my_network_settings_sender.send(my_ipv6_settings);
                        }
                        eprintln!("We should run an IPv6 TCP client for {:?}", other_settings);
                        let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
                        let my_id = GnomeId(encr.hash());
                        // for i in 0..11 {
                        eprintln!(
                            "DPTrying to communicate over TCP {}:{}",
                            other_settings.pub_ip,
                            other_settings.pub_port //+ i
                        );
                        run_tcp_client(
                            my_id,
                            swarm_names.clone(),
                            subscription_sender.clone(),
                            decrypter.clone(),
                            // pipes_sender,
                            pub_key_pem.clone(),
                            (other_settings.pub_ip, other_settings.pub_port), //+ i),
                        )
                        .await
                        // }
                    } else {
                        eprintln!("Unexpected transport: {:?}", other_settings.transport);
                    }
                } else if other_settings.transport == GTransport::UDPoverIP4 {
                    let _ = my_network_settings_sender.send(my_settings);
                    // swarm_names.sort();
                    spawn(punch_and_communicate(
                        socket,
                        bind_addr,
                        pub_key_pem.clone(),
                        // _to_gmgr.clone(),
                        subscription_sender.clone(),
                        decrypter.clone(),
                        // token_endpoints_sender.clone(),
                        swarm_names.clone(),
                        (my_settings, other_settings),
                    ));
                    socket = UdpSocket::bind(bind_addr).await.unwrap();
                    my_settings = update_my_pub_addr(&socket, my_settings).await;
                } else if other_settings.transport == GTransport::TCPoverIP4 {
                    //TODO: run TCP client here
                    eprintln!("We should run a TCP client for {:?}", other_settings);
                    let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
                    let my_id = GnomeId(encr.hash());
                    // for i in 0..11{
                    eprintln!(
                        "DPTrying to communicate over TCP {}:{}",
                        other_settings.pub_ip,
                        other_settings.pub_port //+ i
                    );
                    run_tcp_client(
                        my_id,
                        swarm_names.clone(),
                        subscription_sender.clone(),
                        decrypter.clone(),
                        // pipes_sender,
                        pub_key_pem.clone(),
                        (other_settings.pub_ip, other_settings.pub_port), //+ i),
                    )
                    .await
                    // }
                } else {
                    eprintln!("2 Unexpected trasnport: {:?}", other_settings.transport);
                }
            }
        }
        // yield_now().await;
    }
}
async fn update_my_pub_addr(
    socket: &UdpSocket,
    mut my_settings: NetworkSettings,
) -> NetworkSettings {
    let ping_result = are_we_behind_a_nat(socket).await;
    if let Ok((_nat, _port_control, our_addr)) = ping_result {
        let new_ip = our_addr.ip();
        let new_port = our_addr.port();
        if new_ip != my_settings.pub_ip {
            eprintln!(
                "My pub IP has changed from {:?} to {:?}",
                my_settings.pub_ip, new_ip
            );
            my_settings.pub_ip = new_ip;
        }
        if new_port != my_settings.pub_port {
            eprintln!(
                "My pub port has changed from {:?} to {:?}",
                my_settings.pub_port, new_port
            );
            my_settings.pub_port = new_port;
        }
    } else {
        eprintln!("Failed to update my public IP");
    }
    my_settings
}

async fn punch_and_communicate(
    socket: UdpSocket,
    bind_addr: (IpAddr, u16),
    pub_key_pem: String,
    // to_gmgr: ASender<ToGnomeManager>,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    // pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_names: Vec<SwarmName>,
    (my_settings, other_settings): (NetworkSettings, NetworkSettings),
) {
    if other_settings.no_nat() {
        // eprintln!("DP Case 0 - there is no NAT for {:?}", other_settings);
        run_client(
            swarm_names.clone(),
            sub_sender,
            decrypter,
            // pipes_sender,
            pub_key_pem,
            Some((socket, other_settings)),
        )
        .await;
    } else {
        // TODO: here we need to strategize how to proceed
        // depending on our and theirs NAT type and port allocation rule
        // We first need to establish wether our NAT or their NAT is less
        // restrictive. If both are equal use pub_ip to determine which.
        // No we have two paths. We either have a less restrictive NAT or not.
        // None = 1, -- covered
        eprintln!("DPunch {:?} and {:?}", bind_addr, other_settings);
        let punch_it_result = if my_settings.nat_at_most_address_sensitive()
            || other_settings.nat_at_most_address_sensitive()
        {
            eprintln!("DP Case 1");
            // We: FullCone,                them: AddressRestrictedCone = 4,
            // We: FullCone,                them: PortRestrictedCone = 8,
            // We: FullCone,                them: SymmetricWithPortControl = 16,
            // We: FullCone,                them: Symmetric = 32, or Unknown = 0
            // We: AddressRestrictedCone,   them PortRestrictedCone = 8,
            // We: AddressRestrictedCone,   them SymmetricWithPortControl = 16,
            // We: AddressRestrictedCone,   them Symmetric = 32, or Unknown = 0
            //    ^ for all above we run punch_it procedure on one socket
            //      them also run punch_it on one socket
            punch_it(socket, (my_settings, other_settings)).await
        } else if my_settings.nat_port_restricted()
            && other_settings.nat_symmetric_with_port_control()
        {
            eprintln!("DP Case 2");
            // We: PortRestrictedCone,      them: SymmetricWithPortControl = 16,
            // TODO:   ^ we run punch_it on one socket
            //      them run punch_it on one socket
            //      if no success...
            punch_it(socket, (my_settings, other_settings)).await
        } else if my_settings.nat_port_restricted()
            && (other_settings.nat_symmetric() || other_settings.nat_unknown())
        {
            eprintln!("DP Case 3");
            // We: PortRestrictedCone,      them: Symmetric = 32, or Unknown = 0
            // TODO:   ^ we run punch_it on one socket
            //      them run punch_it on one socket
            //      if no success...
            punch_it(socket, (my_settings, other_settings)).await
        } else if my_settings.nat_symmetric_with_port_control()
            && (other_settings.nat_symmetric() || other_settings.nat_unknown())
        {
            eprintln!("DP Case 4");
            // We: SymmetricWithPortControl,them: Symmetric = 32, or Unknown = 0
            // TODO:   ^ we run punch_it on one socket
            //      them run punch_it on one socket
            //      if no success...
            let p_res = punch_it(socket, (my_settings, other_settings)).await;
            if p_res.is_some() {
                p_res
            } else {
                let his_port_min = other_settings.port_increment(other_settings.pub_port);
                let his_port_max = other_settings.get_predicted_addr(50).1;
                cluster_punch_it(
                    my_settings.pub_ip,
                    other_settings.pub_ip,
                    my_settings.pub_port,
                    50,
                    (
                        his_port_min,
                        other_settings.port_allocation.1.unsigned_abs() as u16, //TODO:?
                        his_port_max,
                    ),
                    Duration::from_secs(60),
                )
                .await
            }
        } else {
            eprintln!("DP Case 5 - no luck");
            None
        };
        // let socket_recv_result = socket_receiver.recv();
        if let Some(dedicated_socket) = punch_it_result {
            spawn(start_communication(
                dedicated_socket,
                swarm_names,
                pub_key_pem.clone(),
                // gnome_id,
                decrypter.clone(),
                // pipes_sender.clone(),
                sub_sender.clone(),
            ));
            // return;
        } else {
            eprintln!("Unable to communicate over UDP with {:?}", other_settings);
            //todo: here we should send back to gmgr
            // or simply start a TCP client with given settings
            // let _ = to_gmgr
            //     .send(ToGnomeManager::TryTcpConnect(other_settings))
            //     .await;
            //
            // TODO: do not start TCP client, other_settings are
            // dedicated for UDP communication.
            //
            // If we receive other_settings with transport set to TCP
            // should we try to communicate over TCP with those settings.
        };
    }

    // if let (
    //     PortAllocationRule::AddressSensitive | PortAllocationRule::PortSensitive,
    //     value,
    // ) = my_settings.port_allocation
    // {
    //     if value > 0 {
    //         my_settings.pub_port += value as u16
    //     } else {
    //         my_settings.pub_port -= (value as i16).unsigned_abs()
    //     }
    // };
}
