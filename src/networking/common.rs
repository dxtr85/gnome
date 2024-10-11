use crate::crypto::Encrypter;
use crate::networking::stun::{
    build_request, stun_decode, stun_send, StunChangeRequest, StunMessage,
};
use crate::networking::subscription::Subscription;
use async_std::net::{IpAddr, Ipv4Addr, UdpSocket};
use async_std::task::{sleep, yield_now};
use swarm_consensus::SwarmName;
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
use swarm_consensus::{
    CastMessage, GnomeId, Message, Nat, Neighbor, NetworkSettings, PortAllocationRule, SwarmTime,
    WrappedMessage,
};

pub async fn collect_subscribed_swarm_names(
    names: &mut Vec<SwarmName>,
    sender: Sender<Subscription>,
    receiver: Receiver<Subscription>,
) -> Receiver<Subscription> {
    // println!("Collecting swarm names...");
    let _ = sender.send(Subscription::ProvideList);
    let sleep_time = Duration::from_millis(128);
    loop {
        if let Ok(subs_msg) = receiver.try_recv() {
            // recv_result = Ok(recv_rslt);
            match subs_msg {
                Subscription::Added(ref name) => {
                    names.push(name.to_owned());
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
    receiver
}

pub async fn send_subscribed_swarm_names(
    socket: &UdpSocket,
    names: &Vec<SwarmName>,
    // remote_addr: SocketAddr,
) {
    // let mut buf = BytesMut::with_capacity(1030);
    let mut buf = Vec::new();
    for name in names {
        for a_byte in name.as_bytes() {
            buf.push(a_byte);
        }
        // TODO split with some other value
        buf.push(255);
    }
    buf.pop();
    // println!("After split: {:?}", &buf);
    // let send_result = socket.send_to(&bytes, remote_addr).await;
    let send_result = socket.send(&buf).await;
    if let Ok(count) = send_result {
        eprintln!("SKT Sent {} bytes", count);
    }
}

pub fn distil_common_names(
    common_names: &mut Vec<SwarmName>,
    names: Vec<SwarmName>,
    remote_names: &Vec<SwarmName>,
) {
    for name in remote_names {
        // let name = String::from_utf8(bts).unwrap();
        if names.contains(&name) {
            common_names.push(name.to_owned());
        }
    }
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
        recv_buf[..count]
            // TODO split by some reasonable delimiter
            .split(|n| n == &255u8)
            .map(|bts| SwarmName::from(bts.to_vec()).unwrap())
            .collect()
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
    // println!("Neighbor: {}", neighbor_id);
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
        eprintln!("Request include neighbor");
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
pub async fn are_we_behind_a_nat(socket: &UdpSocket) -> Result<(bool, SocketAddr), String> {
    let request = build_request(None);
    let ip = IpAddr::V4(Ipv4Addr::new(108, 177, 15, 127));
    let port = 3478;
    let _send_result = stun_send(socket, request, Some(ip), Some(port)).await;
    // let mut bytes: [u8; 128] = [0; 128];

    let t1 = time_out(Duration::from_secs(3), None).fuse();
    // TODO: serv pairs of sender-receiver
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
            Ok((false, mapped_address))
        } else {
            Ok((true, mapped_address))
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
pub async fn identify_nat(socket: &UdpSocket) -> Nat {
    let request = build_request(Some(StunChangeRequest::ChangeIpPort));
    let first_id = request.id();
    let _send_result = stun_send(socket, request, None, None).await;
    let request = build_request(Some(StunChangeRequest::ChangePort));
    let second_id = request.id();
    let _send_result = stun_send(socket, request, None, None).await;

    let mut millis_remaining = 1000;
    let mut received_responses = Vec::new();
    eprintln!("Waiting for STUN to respond...");
    while millis_remaining > 0 && received_responses.len() < 2 {
        let t1 = sleep(Duration::from_millis(1)).fuse();
        let t2 = receive_response(socket).fuse();

        pin_mut!(t1, t2);
        select! {
            _result1 = t1 =>{
                 millis_remaining-=1;
             },
            result2 = t2 => {
                eprintln!("Received a response: {:?}", result2);
                received_responses.push(result2)}
        };
        yield_now().await;
    }
    if received_responses.is_empty() {
        eprintln!("NAT type: Symmetric ");
        Nat::Symmetric
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
    stun_send(socket, build_request(None), None, None).await;
    let mut bytes: [u8; 128] = [0; 128];
    let recv_result = socket.recv_from(&mut bytes).await;
    if let Ok((_count, _from)) = recv_result {
        let msg = stun_decode(&bytes);
        if let Some(changed_address) = msg.changed_address() {
            let m_addr_1 = msg.mapped_address().unwrap();
            // println!("Mapped address 1: {:?}", m_addr_1);
            let port_1 = m_addr_1.port();
            stun_send(
                socket,
                build_request(None),
                None,
                Some(changed_address.port()),
            )
            .await;
            bytes = [0; 128];
            let _recv_result = socket.recv_from(&mut bytes).await;
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
            bytes = [0; 128];
            let _recv_result = socket.recv_from(&mut bytes).await;
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
            bytes = [0; 128];
            let _recv_result = socket.recv_from(&mut bytes).await;
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
            (PortAllocationRule::Random, 0)
        }
    } else {
        (PortAllocationRule::Random, 0)
    }
}
// Now we know all we need to establish a connection between two hosts behind
// any type of NAT. (We might not connect if NAT always assigns ports randomly.)
// From now on if we want to connect to another gnome, we create a new socket,
// send just one request to STUN server for port identification if necessary,
// and we can send out our expected socket address for other gnome to connect to.

pub async fn discover_network_settings(socket: &mut UdpSocket) -> NetworkSettings {
    let behind_a_nat = are_we_behind_a_nat(socket).await;
    let mut my_settings = NetworkSettings::default();
    if let Ok((is_there_nat, public_addr)) = behind_a_nat {
        let ip = public_addr.ip();
        my_settings.pub_ip = ip;
        let port = public_addr.port();
        my_settings.pub_port = port;
        if is_there_nat {
            eprintln!("NAT detected, identifying...");
            let nat = identify_nat(socket).await;
            my_settings.nat_type = nat;
            let port_allocation = discover_port_allocation_rule(socket).await;
            my_settings.port_allocation = port_allocation;
        } else {
            eprintln!("We have a public IP!");
            my_settings.nat_type = Nat::None;
            my_settings.port_allocation = (PortAllocationRule::FullCone, 0);
        }
    } else {
        eprintln!("Unable to tell if there is NAT: {:?}", behind_a_nat);
    }
    my_settings
}
