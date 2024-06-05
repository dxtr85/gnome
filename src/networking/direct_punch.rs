// use super::common::are_we_behind_a_nat;
use super::common::{are_we_behind_a_nat, discover_network_settings, time_out};
use super::token::Token;
use crate::networking::holepunch::cluster_punch_it;
use crate::networking::{
    client::run_client,
    holepunch::{punch_it, start_communication},
    subscription::Subscription,
};
use crate::prelude::Decrypter;
// use crate::GnomeId;
use async_std::net::UdpSocket;
use async_std::task::{spawn, yield_now};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::{NetworkSettings, Request};

pub async fn direct_punching_service(
    // host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pipes_receiver: Receiver<(String, Sender<Request>, Receiver<NetworkSettings>)>,
    pub_key_pem: String,
) {
    println!("Waiting for direct connect requests.");

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

    let mut swarms: HashMap<String, (Sender<Request>, Receiver<NetworkSettings>)> =
        HashMap::with_capacity(10);
    let (send_other, recv_other) = channel();
    let (send_my, recv_my) = channel();
    // TODO: maybe only run it when it makes sense?
    spawn(socket_maintainer(
        // host_ip,
        pub_key_pem.clone(),
        // gnome_id,
        sub_sender.clone(),
        decrypter.clone(),
        pipes_sender.clone(),
        send_my,
        recv_other,
    ));
    // println!("after sm spawn");

    let mut waiting_for_my_settings = false;
    let mut request_sender: Option<Sender<Request>> = None;
    loop {
        // print!("dps");
        if let Ok((swarm_name, req_sender, net_set_recv)) = pipes_receiver.try_recv() {
            swarms.insert(swarm_name, (req_sender, net_set_recv));
        }
        if !waiting_for_my_settings {
            for (swarm_name, (req_sender, net_set_recv)) in &swarms {
                let settings_result = net_set_recv.try_recv();
                // match settings_result {
                if let Ok(other_settings) = settings_result {
                    let _ = send_other.send((swarm_name.clone(), other_settings));
                    println!("His: {:?}", other_settings);
                    waiting_for_my_settings = true;
                    request_sender = Some(req_sender.clone());
                }
            }
        } else {
            let recv_result = recv_my.try_recv();
            if let Ok(my_settings) = recv_result {
                println!("My: {:?}", my_settings);
                let request = Request::NetworkSettingsUpdate(
                    true,
                    my_settings.pub_ip,
                    my_settings.pub_port,
                    my_settings.nat_type,
                );
                if let Some(req_sender) = &request_sender {
                    let _ = req_sender.send(request);
                }
                waiting_for_my_settings = false;
            }
        }
        yield_now().await;
    }
}

async fn socket_maintainer(
    // host_ip: IpAddr,
    pub_key_pem: String,
    // gnome_id: GnomeId,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    send_my: Sender<NetworkSettings>,
    recv_other: Receiver<(String, NetworkSettings)>,
) {
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
    let timeout_sec = Duration::from_secs(14);
    let (t_send, timeout) = channel();
    spawn(time_out(timeout_sec, Some(t_send.clone())));
    loop {
        if timeout.try_recv().is_ok() {
            my_settings = update_my_pub_addr(&socket, my_settings).await;
            spawn(time_out(timeout_sec, Some(t_send.clone())));
        }
        let recv_result = recv_other.try_recv();
        if let Ok((swarm_name, other_settings)) = recv_result {
            if !swarm_names.contains(&swarm_name) {
                swarm_names.push(swarm_name.clone());
            }
            // TODO: discover with stun server
            println!("recvd other!");
            let _ = send_my.send(my_settings);
            spawn(punch_and_communicate(
                socket,
                bind_addr,
                pub_key_pem.clone(),
                // gnome_id.clone(),
                sub_sender.clone(),
                decrypter.clone(),
                pipes_sender.clone(),
                swarm_names.clone(),
                (my_settings, other_settings),
            ));
            socket = UdpSocket::bind(bind_addr).await.unwrap();
            my_settings = update_my_pub_addr(&socket, my_settings).await;
        }
        yield_now().await;
    }
}
async fn update_my_pub_addr(
    socket: &UdpSocket,
    mut my_settings: NetworkSettings,
) -> NetworkSettings {
    let ping_result = are_we_behind_a_nat(socket).await;
    if let Ok((_nat, our_addr)) = ping_result {
        let new_ip = our_addr.ip();
        let new_port = our_addr.port();
        if new_ip != my_settings.pub_ip {
            println!(
                "My pub IP has changed from {:?} to {:?}",
                my_settings.pub_ip, new_ip
            );
            my_settings.pub_ip = new_ip;
        }
        if new_port != my_settings.pub_port {
            println!(
                "My pub port has changed from {:?} to {:?}",
                my_settings.pub_port, new_port
            );
            my_settings.pub_port = new_port;
        }
    }
    my_settings
}

async fn punch_and_communicate(
    socket: UdpSocket,
    bind_addr: (IpAddr, u16),
    pub_key_pem: String,
    // gnome_id: GnomeId,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_names: Vec<String>,
    (my_settings, other_settings): (NetworkSettings, NetworkSettings),
) {
    if other_settings.no_nat() {
        println!("DP Case 0 - there is no NAT");
        run_client(
            swarm_names.clone(),
            sub_sender,
            decrypter,
            pipes_sender,
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
        println!("DPunch {:?} and {:?}", bind_addr, other_settings);
        let punch_it_result = if my_settings.nat_at_most_address_sensitive()
            || other_settings.nat_at_most_address_sensitive()
        {
            println!("DP Case 1");
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
            println!("DP Case 2");
            // We: PortRestrictedCone,      them: SymmetricWithPortControl = 16,
            // TODO:   ^ we run punch_it on one socket
            //      them run punch_it on one socket
            //      if no success...
            punch_it(socket, (my_settings, other_settings)).await
        } else if my_settings.nat_port_restricted()
            && (other_settings.nat_symmetric() || other_settings.nat_unknown())
        {
            println!("DP Case 3");
            // We: PortRestrictedCone,      them: Symmetric = 32, or Unknown = 0
            // TODO:   ^ we run punch_it on one socket
            //      them run punch_it on one socket
            //      if no success...
            punch_it(socket, (my_settings, other_settings)).await
        } else if my_settings.nat_symmetric_with_port_control()
            && (other_settings.nat_symmetric() || other_settings.nat_unknown())
        {
            println!("DP Case 4");
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
            println!("DP Case 5 - no luck");
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
                pipes_sender.clone(),
                sub_sender.clone(),
            ));
            // return;
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
