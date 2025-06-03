use crate::data_conversion::message_to_bytes;
use crate::data_conversion::neighbor_request_to_bytes;
use crate::data_conversion::neighbor_response_to_bytes;
use crate::networking::stun::{
    build_request, stun_decode, stun_send, StunChangeRequest, StunMessage,
};
use crate::networking::subscription::Subscription;
use aes_gcm::aead::Buffer;
use async_std::net::{IpAddr, Ipv4Addr, UdpSocket};
use async_std::task;
use async_std::task::{sleep, yield_now};
use futures::SinkExt;
use std::collections::HashMap;
use std::net::Ipv6Addr;
use swarm_consensus::SwarmName;
use swarm_consensus::{CastContent, CastMessage, Message, Neighbor, SwarmTime, WrappedMessage};
// use bytes::{BufMut, BytesMut};
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::cmp::min;
use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::{GnomeId, Nat, NetworkSettings, PortAllocationRule};

use super::subscription::Requestor;

pub async fn collect_subscribed_swarm_names(
    names: &mut Vec<SwarmName>,
    requestor: Requestor,
    sender: Sender<Subscription>,
    receiver: Receiver<Subscription>,
) -> Receiver<Subscription> {
    // eprintln!("Collecting swarm names...");
    let _ = sender.send(Subscription::ProvideList(requestor));
    let sleep_time = Duration::from_millis(128);
    loop {
        if let Ok(subs_msg) = receiver.try_recv() {
            // recv_result = Ok(recv_rslt);
            match subs_msg {
                Subscription::Added(ref _name) => {
                    // names.push(name.to_owned());
                    continue;
                }
                Subscription::Removed(ref _name) => {
                    // names.push(name.to_owned());
                    continue;
                }
                Subscription::List(ref nnames) => {
                    *names = nnames.to_owned();
                    break;
                }
                _ => eprintln!("Unexpected message: {:?}", subs_msg),
            };
        }
        // yield_now().await;
        sleep(sleep_time).await;
    }
    // eprintln!("Collected swarm names: {:?}", names);
    receiver
}

pub fn swarm_names_as_bytes(names: &Vec<SwarmName>, limit: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(limit);
    let mut buf_size = 0;
    for name in names {
        let name_bytes = name.as_bytes();
        let new_size = buf_size + name_bytes.len();
        if new_size < limit {
            for a_byte in name_bytes {
                buf.push(a_byte);
            }
            buf_size = new_size;
        } else {
            break;
        }
    }
    buf
}
pub async fn send_subscribed_swarm_names(socket: &UdpSocket, names: &Vec<SwarmName>) {
    let buf = swarm_names_as_bytes(names, 1450);
    let send_result = socket.send(&buf).await;
    if let Ok(count) = send_result {
        eprintln!("UDP Sent {} bytes", count);
    }
}

pub fn distil_common_names(
    my_id: GnomeId,
    nb_id: GnomeId,
    common_names: &mut Vec<SwarmName>,
    mut my_names: Vec<SwarmName>,
    remote_names: &mut Vec<SwarmName>,
) {
    if my_names.is_empty() {
        my_names.push(SwarmName {
            founder: GnomeId::any(),
            name: "/".to_string(),
        });
    }
    // eprintln!("My: {},(>? {}) nb: {}, ", my_id, my_id > nb_id, nb_id);
    // eprintln!(
    //     "Finding common from my:\n{:?}\n\nand his:\n{:?}",
    //     my_names, remote_names
    // );
    if remote_names.len() == 1 && remote_names[0].founder.is_any() {
        if my_names.len() == 1 && my_names[0].founder.is_any() {
            //TODO: most restrictive, only when initializing network
            if my_id > nb_id {
                common_names.push(SwarmName {
                    founder: my_id,
                    name: my_names[0].name.clone(),
                });
                remote_names[0].founder = my_id;
            } else {
                common_names.push(SwarmName {
                    founder: nb_id,
                    name: my_names[0].name.clone(),
                });
                remote_names[0].founder = nb_id;
            }
        } else {
            //TODO: common, a neighbor is joining existing network
            common_names.push(SwarmName {
                founder: my_id,
                name: my_names[0].name.clone(),
            });
            remote_names[0].founder = my_id;
        }
    } else if my_names.len() == 1 && my_names[0].founder.is_any() {
        //TODO: common, when I am joining existing network
        common_names.push(SwarmName {
            founder: nb_id,
            name: my_names[0].name.clone(),
        });
    } else {
        //TODO: common, when joining an old neighbor while already in network
        for name in remote_names {
            // let name = String::from_utf8(bts).unwrap();
            if my_names.contains(name) {
                common_names.push(name.to_owned());
            }
        }
    }
}
pub fn swarm_names_from_bytes(recv_buf: &[u8]) -> Vec<SwarmName> {
    let mut recvd_names = vec![];
    let count = recv_buf.len();
    let mut i = 0;
    // eprintln!("SNBytes#:{}", count);
    while i < count {
        let name_len = recv_buf[i];
        if name_len < 128 {
            let sn_res = SwarmName::from(&recv_buf[i..i + name_len as usize + 1]);
            if let Ok(sn) = sn_res {
                recvd_names.push(sn);
            } else {
                eprintln!(
                    "Error building generic SwarmName: {:?}",
                    sn_res.err().unwrap()
                );
            }
            i += name_len as usize + 1;
            // eprintln!("i:{}", i)
        } else {
            // eprintln!("Founder not any {}-{}", i, i + name_len as usize - 127);
            let sn_res = SwarmName::from(&recv_buf[i..i + name_len as usize - 127]);
            if let Ok(sn) = sn_res {
                // eprintln!("SN: {}", sn);
                recvd_names.push(sn);
            } else {
                eprintln!("Error building SwarmName: {:?}", sn_res.err().unwrap());
            }
            i += name_len as usize - 127;
            // eprintln!("i2:{}", i)
        }
    }
    recvd_names
}

pub async fn receive_remote_swarm_names(
    socket: &UdpSocket,
    // recv_buf: &mut BytesMut,
    remote_names: &mut Vec<SwarmName>,
) {
    let mut recv_buf = [0u8; 1024];
    *remote_names = if let Ok(count) = socket.recv(&mut recv_buf).await {
        // eprintln!(
        //     "Recv buf (count: {}): {:?}",
        //     count,
        //     &recv_buf[..count] // String::from_utf8(recv_buf[..count].try_into().unwrap()).unwrap()
        // );
        // eprintln!("Reading SwarmNames gnome/networking/common");
        // recv_buf[..count]
        //     // TODO split by some reasonable delimiter
        //     .split(|n| n == &255u8)
        //     .map(|bts| SwarmName::from(bts.to_vec()).unwrap())
        //     .collect()
        swarm_names_from_bytes(&recv_buf[0..count])
    } else {
        Vec::new()
    };
}

// TODO: We need a way to dynamically extend/shrink ch_pairs once created!!!
//       This is in order for client to join new swarms.
// We can do it like this:
// Every Gnome has a Sender channel via which he can request networking task
// to extend supported Swarms by one.
// Once networking task receives such a requests it installs provided Receivers and
// Senders into hashmaps.
// Once installed networking task sends a prepared message to it's remote.
// That message should contain means to identify for which Swarm this UDP channel
// has been created and also some fixed value to indicate we want remote to
// create a corresponding pair.
// Fixed value serves the case when for some reason remote decided to close existing
// channel and did not inform us. If a Datagram without fixed value is received
// and there is no corresponding local channel to send it through, we can ignore it.
// If we receive a Datagram with fixed value and there is no local channel to support,
// we create all necessary Send/Receive pairs and notify any Gnome we are already
// connected through. A Gnome should notify a Manager or User about a new Neighbor
// that has been connected.
//
pub fn create_a_neighbor_for_each_swarm(
    common_names: Vec<SwarmName>,
    remote_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    remote_gnome_id: GnomeId,
    ch_pairs: &mut Vec<(
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    shared_sender: Sender<(
        SwarmName,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    // encrypter: Encrypter,
    pub_key_pem: String,
) {
    eprintln!("Neighbor member of swarms:");
    for r_name in &remote_names {
        eprintln!("{}", r_name);
    }
    // println!("komon names: {:?}", common_names);
    for name in common_names {
        let (s1, r1) = channel();
        let (s2, r2) = channel();
        let (s3, r3) = channel();
        let neighbor = Neighbor::from_id_channel_time(
            remote_gnome_id,
            r1,
            r2,
            s3,
            shared_sender.clone(),
            SwarmTime(0),
            SwarmTime(7),
            remote_names.clone(),
            // pub_key_pem.clone(),
        );
        eprintln!("Request include {} to {}", neighbor.id, name);
        let _ = sender.send(Subscription::IncludeNeighbor(name, neighbor));
        ch_pairs.push((s1, s2, r3));
    }
}
// We need to write a procedure that establishes
// whether or not we are behind a NAT
// To do that we have to ask a STUN server for our public address
// If we do not receive a response within reasonable period of time that
// means firewall cuts off UDP communication
// or a STUN server is unresponsive
// We can try two or three STUN servers.
// In case we receive a response we compare our port and address
// with MAPPED_ADDRESS received from STUN server
// If both IP and PORT are the same then we have a public IP
// and we can connect with anyone we want on any port we want
// If IP is different but port is the same we do not have a public IP
// but we do have control over public port assignment, which means
// we can function as if we had a public IP.
// In case both IP and PORT are different from what we have set up
// we need to run an additional procedure in order to identify
// what type of NAT we are behind.
pub async fn are_we_behind_a_nat(socket: &UdpSocket) -> Result<(bool, bool, SocketAddr), String> {
    let request = build_request(None);
    let local_port = socket.local_addr().unwrap().port();
    let (ip, port) = if socket.local_addr().unwrap().is_ipv4() {
        (IpAddr::V4(Ipv4Addr::new(108, 177, 15, 127)), 3478)
    } else {
        (
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4864, 5, 0x8000, 0, 0, 1)),
            19302,
        )
    };
    // let port = 3478;
    let _send_result = stun_send(socket, request, Some(ip), Some(port)).await;
    // let mut bytes: [u8; 128] = [0; 128];

    let t1 = time_out(Duration::from_secs(1), None).fuse();
    let t2 = wait_for_response(socket, SocketAddr::new(ip, port)).fuse();

    pin_mut!(t1, t2);

    let (received, bytes) = select! {
        _result1 = t1 =>  (false,[0;128]),
        result2 = t2 => (true,result2),
    };
    //new above
    // let recv_result = socket.recv_from(&mut bytes).await;
    // if let Ok((_count, _from)) = recv_result {
    if received {
        let msg = stun_decode(&bytes);
        // println!("Received {} bytes from {:?}:\n{:?}", count, from, msg);
        let mapped_address = msg.mapped_address().unwrap();
        if UdpSocket::bind((mapped_address.ip(), 0)).await.is_ok() {
            // if mapped_address == socket.local_addr().unwrap() {
            Ok((false, local_port == mapped_address.port(), mapped_address))
        } else {
            Ok((true, local_port == mapped_address.port(), mapped_address))
        }
    } else {
        Err("Timed out while waiting for STUN response".to_string())
    }
}

pub async fn wait_for_bytes(socket: &UdpSocket) {
    // println!("waiting for bytes");
    let mut bytes: [u8; 10] = [0; 10];
    loop {
        let recv_res = socket.recv_from(&mut bytes).await;
        // println!("Recv: {:?}", recv_res);
        if let Ok((count, _from)) = recv_res {
            // println!("Recv: {} from {:?}", bytes[0], from);
            if count == 1 && bytes[0] == 1 {
                return;
            }
        }
    }
}

pub async fn wait_for_response(socket: &UdpSocket, addr: SocketAddr) -> [u8; 128] {
    // println!("waiting for bytes");
    let mut bytes: [u8; 128] = [0; 128];
    loop {
        let _recv_res = socket.recv_from(&mut bytes).await;

        match _recv_res {
            Ok((_count, from)) => {
                if addr == from {
                    // println!("Socket recv: {:?} {:?}", _recv_res, bytes);
                    return bytes;
                }
            }
            _ => continue,
        }

        // println!("Recv: {:?}", recv_res);
        //     if let Ok((count, from)) = recv_res {
        //         // println!("Recv: {} from {:?}", bytes[0], from);
        //         if count == 1 && bytes[0] == 1 {
        //             return;
        //         }
        //     }
    }
}
// This procedure for NAT identification has two stages:
// 1 - we ask STUN server to reply from a different IP & port
// 2 - we ask STUN server to reply from same IP but different port
// If we get response for request 1, then we are behind a Full Cone NAT,
// which means we can receive messages from anyone
// If we do not get a response we need to verify request nr 2
// If we get a response for request 2, then we are behind an Address
// Restricted Cone NAT, and if we do not receive any response
// then we are behind a symmetric NAT.
// In case we are behind a symmetric NAT we can only receive a response from
// an IP and PORT that we have previously sent a message to, no other pair
// of IP and PORT will get through our NAT.
// We still need to find out how are external PORTS assigned.
pub async fn identify_nat(socket: &UdpSocket, have_port_control: bool) -> Nat {
    let request = build_request(Some(StunChangeRequest::ChangeIpPort));
    let first_id = request.id();
    let _send_result = stun_send(socket, request, None, None).await;
    let request = build_request(Some(StunChangeRequest::ChangePort));
    let second_id = request.id();
    let _send_result = stun_send(socket, request, None, None).await;

    let mut millis_remaining = 1000;
    let mut received_responses = Vec::new();
    // eprintln!("Waiting for STUN to respond...");
    while millis_remaining > 0 && received_responses.len() < 2 {
        let t1 = sleep(Duration::from_millis(1)).fuse();
        let t2 = receive_response(socket).fuse();

        pin_mut!(t1, t2);
        select! {
            _result1 = t1 =>{
                 millis_remaining-=1;
             },
            result2 = t2 => {
                // eprintln!("Received a response: {:?}", result2);
                received_responses.push(result2)}
        };
        yield_now().await;
    }
    if received_responses.is_empty() {
        // eprintln!("NAT type: Symmetric ");
        if have_port_control {
            Nat::SymmetricWithPortControl
        } else {
            Nat::Symmetric
        }
    } else {
        let mut first_response_received = false;
        let mut second_response_received = false;
        for response in received_responses {
            if response.id() == first_id {
                first_response_received = true;
            } else if response.id() == second_id {
                second_response_received = true;
            }
        }
        if first_response_received {
            eprintln!("NAT type: FullCone ");
            Nat::FullCone
        } else if second_response_received {
            eprintln!("NAT type: AddressRestrictedCone ");
            Nat::AddressRestrictedCone
        } else {
            eprintln!("NAT type: Unknown ");
            Nat::Unknown
        }
    }
    // let _recv_result = socket.recv_from(&mut bytes).await;
    // let second_response = stun_decode(&bytes);
}
async fn receive_response(socket: &UdpSocket) -> StunMessage {
    let mut bytes: [u8; 128] = [0; 128];
    let _res = socket.recv_from(&mut bytes).await;
    let response = stun_decode(&bytes.clone());
    eprintln!("Received a response: {:?}", response);
    response
}

pub async fn time_out(mut time: Duration, sender: Option<Sender<()>>) {
    let time_step = Duration::from_millis(100);
    while time > Duration::ZERO {
        // print!(".");
        time -= time_step;
        sleep(time_step).await;
    }
    // println!("timed out");
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
}

// In order to find how ports are assigned we send four messages to STUN server:
// First one is identical to the very first request we have sent to STUN server
// and can be skipped
// Second request we send to the same STUN IP address but different PORT that we
// have received as as response to first request in CHANGED_ADDRESS
// Third request is sent to same port as first request, but IP is taken from
// CHANGED_ADDRESS stored in first response
// Fourth request is sent to both IP and PORT taken from CHANGED_ADDRESS stored
// in first response
// This way we obtain four MAPPED_ADDRESSes and we use them to
// determine the port allocation rule, find out delta p value
// and evaluate port allocation consistency.
// To further evaluate consistency we can use another socket or two and repeat
// the four step procedure.
// If ports in MAPPED_ADDRESS from reply 1 and 2 are the same, and also reply
// 3 and 4 have same ports, but those two ports are not the same, we have
// address sensitive port allocation, and delta p
// is equal to difference between second and first external port number.
// If both ports are equal we have a Full Cone NAT.
// In case every port in MAPPED_ADDRESS replies is different we are behind
// a Port Resticted Cone NAT or symmetric NAT and we determine delta p
// as a difference between port numbers in two consecutive responses.
// This difference should be constant for every consecutive pair
// (maybe one exception is allowed where increment is larger than usual).
pub async fn discover_port_allocation_rule(socket: &UdpSocket) -> (PortAllocationRule, i8) {
    // let local_port = socket.local_addr().unwrap().port();
    // eprintln!("Local port: {}", local_port);
    let (ip, port) = if socket.local_addr().unwrap().is_ipv4() {
        (IpAddr::V4(Ipv4Addr::new(108, 177, 15, 127)), 3478)
    } else {
        (
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4864, 5, 0x8000, 0, 0, 1)),
            19302,
        )
    };
    stun_send(socket, build_request(None), Some(ip), Some(port)).await;
    // let mut bytes: [u8; 128] = [0; 128];
    let t1 = time_out(Duration::from_secs(1), None).fuse();
    let t2 = wait_for_response(socket, SocketAddr::new(ip, port)).fuse();
    // TODO
    // let recv_result = socket.recv_from(&mut bytes).await;

    pin_mut!(t1, t2);

    let (received, bytes) = select! {
        _result1 = t1 =>  (false,[0;128]),
        result2 = t2 => (true,result2),
    };

    // eprintln!("STUN recv result: {:?}", recv_result);
    // if let Ok((_count, _from)) = recv_result {
    if !received {
        return (PortAllocationRule::Random, 0);
    }
    let msg = stun_decode(&bytes);
    // eprintln!("Decoded: {:?}", msg);
    // if let Some(changed_address) = msg.changed_address() {
    if let Some(changed_address) = msg.mapped_address() {
        let m_addr_1 = msg.mapped_address().unwrap();
        // println!("Mapped address 1: {:?}", m_addr_1);
        let port_1 = m_addr_1.port();
        // eprintln!("Public port: {}", port_1);
        // if port_1 == local_port {
        //     eprintln!("Seems we have control over public port assignment");
        // }
        stun_send(
            socket,
            build_request(None),
            None,
            Some(changed_address.port()),
        )
        .await;
        // bytes = [0; 128];
        // let _recv_result = socket.recv_from(&mut bytes).await;
        let t1 = time_out(Duration::from_secs(1), None).fuse();
        let t2 = wait_for_response(socket, SocketAddr::new(ip, changed_address.port())).fuse();
        // TODO
        // let recv_result = socket.recv_from(&mut bytes).await;

        pin_mut!(t1, t2);

        let (received, bytes) = select! {
            _result1 = t1 =>  (false,[0;128]),
            result2 = t2 => (true,result2),
        };
        if !received {
            //TODO: check this,
            // no response after sending to same address, but different port
            return (PortAllocationRule::PortSensitive, 0);
        }
        let msg = stun_decode(&bytes);
        let m_addr_2 = msg.mapped_address().unwrap();
        // println!("Mapped address 2: {:?}", m_addr_2);
        let port_2 = m_addr_2.port();
        stun_send(
            socket,
            build_request(None),
            Some(changed_address.ip()),
            None,
        )
        .await;
        // bytes = [0; 128];
        // let _recv_result = socket.recv_from(&mut bytes).await;
        let t1 = time_out(Duration::from_secs(1), None).fuse();
        let t2 = wait_for_response(socket, SocketAddr::new(changed_address.ip(), port)).fuse();
        // TODO
        // let recv_result = socket.recv_from(&mut bytes).await;

        pin_mut!(t1, t2);

        let (received, bytes) = select! {
            _result1 = t1 =>  (false,[0;128]),
            result2 = t2 => (true,result2),
        };
        if !received {
            //TODO: check this,
            // no response after sending to different ip address, but same port
            return (PortAllocationRule::AddressSensitive, 0);
        }
        let msg = stun_decode(&bytes);
        let m_addr_3 = msg.mapped_address().unwrap();
        // println!("Mapped address 3: {:?}", m_addr_3);
        let port_3 = m_addr_3.port();
        stun_send(
            socket,
            build_request(None),
            Some(changed_address.ip()),
            Some(changed_address.port()),
        )
        .await;
        // bytes = [0; 128];
        // let _recv_result = socket.recv_from(&mut bytes).await;
        let t1 = time_out(Duration::from_secs(1), None).fuse();
        let t2 = wait_for_response(
            socket,
            SocketAddr::new(changed_address.ip(), changed_address.port()),
        )
        .fuse();
        // TODO
        // let recv_result = socket.recv_from(&mut bytes).await;

        pin_mut!(t1, t2);

        let (received, bytes) = select! {
            _result1 = t1 =>  (false,[0;128]),
            result2 = t2 => (true,result2),
        };
        if !received {
            //TODO: check this,
            // no response after sending to different ip address
            return (PortAllocationRule::AddressSensitive, 0);
        }
        let msg = stun_decode(&bytes);
        let m_addr_4 = msg.mapped_address().unwrap();
        // println!("Mapped address 4: {:?}", m_addr_4);
        let port_4 = m_addr_4.port();
        let p1_and_p2_eq = port_1 == port_2;
        let p3_and_p4_eq = port_3 == port_4;
        if p1_and_p2_eq && p3_and_p4_eq {
            if port_1 == port_3 {
                eprintln!("Port allocation: FullCone, delta p: 0");
                (PortAllocationRule::FullCone, 0)
            } else {
                eprintln!(
                    "Port allocation: AddressSensitive, delta p: {}",
                    port_3 - port_2
                );
                (
                    PortAllocationRule::AddressSensitive,
                    (port_3 - port_2) as i8,
                )
            }
        } else {
            let d1 = port_2 - port_1;
            let d2 = port_3 - port_2;
            let d3 = port_4 - port_3;
            eprintln!(
                "Port allocation: PortSensitive delta p: {} {} {}",
                d1, d2, d3
            );
            let delta_p = min(min(d1, d2), d3) as i8;
            (PortAllocationRule::PortSensitive, delta_p)
        }
    } else {
        (PortAllocationRule::Random, 1)
    }
    // } else {
    //     (PortAllocationRule::Random, 0)
    // }
}
// Now we know all we need to establish a connection between two hosts behind
// any type of NAT. (We might not connect if NAT always assigns ports randomly.)
// From now on if we want to connect to another gnome, we create a new socket,
// send just one request to STUN server for port identification if necessary,
// and we can send out our expected socket address for other gnome to connect to.

pub async fn discover_network_settings(socket: &mut UdpSocket) -> NetworkSettings {
    eprintln!(
        "discover_network_settings {:?}",
        socket.local_addr().unwrap()
    );
    let behind_a_nat = are_we_behind_a_nat(socket).await;
    let mut my_settings = NetworkSettings::default();
    if let Ok((is_there_nat, we_have_port_control, public_addr)) = behind_a_nat {
        let ip = public_addr.ip();
        my_settings.pub_ip = ip;
        let port = public_addr.port();
        my_settings.pub_port = port;
        if is_there_nat {
            let nat = identify_nat(socket, we_have_port_control).await;
            eprintln!("NAT: {:?}", nat);
            my_settings.nat_type = nat;
            let port_allocation = discover_port_allocation_rule(socket).await;
            // eprintln!("Port allocation rule: {:?}", port_allocation);
            my_settings.port_allocation = port_allocation;
        } else {
            eprintln!("We have no NAT");
            my_settings.nat_type = Nat::None;
            my_settings.port_allocation = (PortAllocationRule::FullCone, 0);
        }
    } else {
        eprintln!("Unable to tell if there is NAT: {:?}", behind_a_nat);
    }
    my_settings
}

//TODO: drop receiver on error
pub async fn read_bytes_from_local_stream(
    receivers: &mut HashMap<u8, Receiver<WrappedMessage>>,
) -> Result<Vec<u8>, String> {
    // println!("read_bytes_from_local_stream");
    let sleep_time = Duration::from_micros(100);
    let mut to_drop = vec![];
    loop {
        for (id, receiver) in receivers.iter_mut() {
            let next_option = receiver.try_recv();
            if let Ok(message) = next_option {
                // eprintln!("Got message local: {:?}", message);
                // TODO: here we need to add a Datagram header
                // indicating type of message and id
                let mut dgram_header = *id;
                match message {
                    WrappedMessage::NoOp => return Ok(vec![]),
                    WrappedMessage::Cast(c_msg) => {
                        dgram_header += if c_msg.is_unicast() {
                            64
                        } else if c_msg.is_multicast() {
                            128
                        } else {
                            192
                        };
                        // eprintln!("Sending cast over network: {:?}", c_msg);
                        let c_id = c_msg.id();
                        let mut merged_bytes = Vec::with_capacity(10);
                        merged_bytes.push(dgram_header);
                        merged_bytes.push(c_id.0);
                        match c_msg.content {
                            CastContent::Data(data) => {
                                // let data = c_msg.get_data().unwrap();
                                // for a_byte in data.bytes() {
                                //     merged_bytes.push(a_byte);
                                // }
                                merged_bytes.append(&mut data.bytes());
                            }
                            CastContent::Request(n_req) => {
                                // eprint!("Request");
                                neighbor_request_to_bytes(n_req, &mut merged_bytes);
                            }
                            CastContent::Response(n_resp) => {
                                // eprint!("Response");
                                neighbor_response_to_bytes(n_resp, &mut merged_bytes);
                            }
                        }
                        // eprintln!(" constructed: {:?}", merged_bytes);
                        return Ok(merged_bytes);
                    }
                    WrappedMessage::Regular(message) => {
                        let mut bytes = message_to_bytes(message);
                        let mut merged_bytes = Vec::with_capacity(bytes.len() + 1);
                        merged_bytes.push(dgram_header);
                        merged_bytes.append(&mut bytes);
                        return Ok(merged_bytes);
                    }
                }
            } else {
                let err = next_option.err().unwrap();
                match err {
                    std::sync::mpsc::TryRecvError::Disconnected => {
                        to_drop.push(*id);
                    }
                    _other => {
                        // eprintln!("Next option: {:?}", other);
                    }
                }
            }
        }
        while let Some(id) = to_drop.pop() {
            receivers.remove(&id);
        }
        if receivers.is_empty() {
            eprintln!("End serving socket with no local receivers");
            break;
        }
        task::sleep(sleep_time).await;
    }
    Err("No receivers".to_string())
    // Err(ConnError::LocalStreamClosed)
}
