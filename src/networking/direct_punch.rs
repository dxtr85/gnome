use super::common::are_we_behind_a_nat;
use super::common::discover_network_settings;
use super::token::Token;
use crate::networking::{holepunch::punch_it, subscription::Subscription};
use crate::prelude::{Decrypter, Encrypter};
use crate::GnomeId;
use async_std::net::UdpSocket;
use async_std::task::{spawn, yield_now};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{Nat, NetworkSettings, PortAllocationRule, Request};

pub async fn direct_punching_service(
    host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pipes_receiver: Receiver<(String, Sender<Request>, Receiver<NetworkSettings>)>,
    pub_key_pem: String,
) {
    println!("Waiting for direct connect requests.");
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    drop(loc_encr);

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
    spawn(socket_maintainer(
        host_ip,
        pub_key_pem.clone(),
        gnome_id,
        sub_sender.clone(),
        decrypter.clone(),
        pipes_sender.clone(),
        send_my,
        recv_other,
    ));

    let mut waiting_for_my_settings = false;
    let mut request_sender: Option<Sender<Request>> = None;
    loop {
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
    host_ip: IpAddr,
    pub_key_pem: String,
    gnome_id: GnomeId,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    send_my: Sender<NetworkSettings>,
    recv_other: Receiver<(String, NetworkSettings)>,
) {
    let bind_port = 0;
    let bind_addr = (host_ip, bind_port);
    let mut socket = UdpSocket::bind(bind_addr).await.unwrap();
    let mut my_settings = discover_network_settings(&mut socket).await;

    // TODO: race two tasks: either timeout or receive NetworkSettings from pipe
    loop {
        let recv_result = recv_other.try_recv();
        if let Ok((swarm_name, other_settings)) = recv_result {
            // TODO: discover with stun server
            spawn(punch_it(
                socket,
                swarm_name.clone(),
                pub_key_pem.clone(),
                gnome_id,
                decrypter.clone(),
                // my_settings.port_range,
                // other_settings.port_range,
                (my_settings, other_settings),
                // req_sender.clone(),
                pipes_sender.clone(),
                sub_sender.clone(),
            ));
            match my_settings.port_allocation {
                (PortAllocationRule::AddressSensitive, value) => {
                    if value > 0 {
                        my_settings.pub_port += value as u16
                    } else {
                        my_settings.pub_port -= value.abs() as u16
                    }
                }
                (PortAllocationRule::PortSensitive, value) => {
                    if value > 0 {
                        my_settings.pub_port += value as u16
                    } else {
                        my_settings.pub_port -= value.abs() as u16
                    }
                }
                _ => {}
            }
            let _ = send_my.send(my_settings);
            socket = UdpSocket::bind(bind_addr).await.unwrap();
            if my_settings.nat_type != Nat::None {
                let behind_a_nat = are_we_behind_a_nat(&socket).await;
                if let Ok((_is_there_nat, public_addr)) = behind_a_nat {
                    let ip = public_addr.ip();
                    my_settings.pub_ip = ip;
                    let port = public_addr.port();
                    my_settings.pub_port = port;
                }
            } else {
                my_settings.pub_port = socket.local_addr().unwrap().port();
            }
        }
        yield_now().await;
    }
}
