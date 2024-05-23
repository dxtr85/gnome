use super::Token;
use crate::crypto::{generate_symmetric_key, SessionKey};
use crate::networking::client::prepare_and_serve;
use crate::networking::common::{discover_network_settings, time_out};
use crate::networking::subscription::Subscription;
use crate::prelude::{Decrypter, Encrypter};
use async_std::net::UdpSocket;
use async_std::task::{sleep, spawn, yield_now};
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{channel, TryRecvError};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use swarm_consensus::{GnomeId, NetworkSettings};
use swarm_consensus::{Nat, PortAllocationRule};

// let puncher = "tudbut.de:4277";
// let swarm_name = "irdm".to_string();
// let (remote_addr, socket, server) = holepunch(puncher, swarm_name);

// In case of using external proxy like tudbut.de for neighbor discovery
// we can simply send a drgram to that server and wait for a response.
// When we receive that response we have a remote socket address.
// We first try to reach that received socket from our existing socket
// by using punch_it procedure.
// TODO:
// If that failed, we send another request to a proxy from a new socket, and we
// receive a reply - we note remote_port_1.
// We send another request to a proxy from yet another socket, and we
// note remote_port_2 from second reply.
// We calculate delta_p = remote_port_2 - remote_port_1.
// Now we calculate remote_port = remote_port_2 + delta_p.
// We start exchanging messages as described above to calculated remote_port.
// If no luck, we can turn back to proxy
// or some other mean to receive a neighbor's socket address.
pub async fn holepunch(
    puncher: &str,
    host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    receiver: Receiver<String>,
    pub_key_pem: String,
) {
    // ) -> (SocketAddr, UdpSocket, bool) {
    println!("Holepunch started");
    let mut magic_ports = MagicPorts::new();
    let sleep_duration = Duration::from_millis(50);

    loop {
        let recv_result = receiver.try_recv();
        if recv_result.is_err() {
            let err = recv_result.err().unwrap();
            match err {
                TryRecvError::Disconnected => break,
                _ => {
                    sleep(sleep_duration).await;
                    // yield_now().await;
                    continue;
                }
            }
            // println!("Error receiving swarm_name: {:?}", recv_result);
            // return;
        }
        let swarm_name: String = recv_result.unwrap();
        let bind_port = magic_ports.next();
        spawn(holepunch_task(
            // host_ip,
            puncher.to_string(),
            sub_sender.clone(),
            decrypter.clone(),
            pipes_sender.clone(),
            pub_key_pem.clone(),
            swarm_name,
            (host_ip, bind_port),
        ));
    }
}

async fn holepunch_task(
    // host_ip: IpAddr,
    puncher: String,
    sub_sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    swarm_name: String,
    mut bind_addr: (IpAddr, u16),
) {
    let magic_ports = MagicPorts::new();
    println!("Round '1'");
    // TODO: read only magic_ports!
    let mut holepunch = UdpSocket::bind(bind_addr)
        .await
        .expect("unable to create socket");

    let my_settings = discover_network_settings(&mut holepunch).await;
    let remote_socket_opt =
        ask_proxy_for_remote_socket(&holepunch, puncher.clone(), swarm_name.clone()).await;
    if remote_socket_opt.is_none() {
        println!("Unable to receive remote socket addr");
        return;
    }
    let (remote_1_addr, remote_1_port) = remote_socket_opt.unwrap();
    let remote_controls_port = magic_ports.is_magic(remote_1_port);
    let (remote_nat, remote_port_rule) = if remote_controls_port {
        (
            Nat::SymmetricWithPortControl,
            (PortAllocationRule::FullCone, 0),
        )
    } else {
        //TODO: we do not know PortAllocationRule
        (Nat::Unknown, (PortAllocationRule::PortSensitive, 1))
    };
    println!(
        "Holepunching {} (magic?: {:?}) and :{} (you) for {}.",
        remote_1_addr,
        remote_controls_port,
        holepunch.local_addr().unwrap().port(),
        swarm_name
    );
    let punch_it_result = punch_it(
        holepunch,
        (
            my_settings,
            NetworkSettings {
                pub_ip: remote_1_addr,
                pub_port: remote_1_port,
                nat_type: remote_nat,
                port_allocation: remote_port_rule,
            },
        ),
    )
    .await;
    if let Some(dedicated_socket) = punch_it_result {
        spawn(start_communication(
            dedicated_socket,
            swarm_name.clone(),
            pub_key_pem.clone(),
            // gnome_id,
            decrypter.clone(),
            pipes_sender.clone(),
            sub_sender.clone(),
        ));
        return;
    };

    println!("Round '2'");
    // 1. We need to define a common phrase for two endpoints - based on both ip addresses
    // Since we know our public IP and remote public IP
    let my_pub_ip = my_settings.pub_ip.to_string();
    let his_pub_ip = remote_1_addr.to_string();
    let mut common_phrase;
    if my_pub_ip > his_pub_ip {
        common_phrase = my_pub_ip;
        common_phrase.push_str(&his_pub_ip);
    } else {
        common_phrase = his_pub_ip;
        common_phrase.push_str(&my_pub_ip);
    }
    // 2. Then we create a new socket and send via that socket a request to connect with
    // other host via that common phrase
    bind_addr.1 -= 3;
    holepunch = UdpSocket::bind(bind_addr)
        .await
        .expect("unable to create socket");

    let remote_socket_opt =
        ask_proxy_for_remote_socket(&holepunch, puncher.clone(), common_phrase.clone()).await;
    if remote_socket_opt.is_none() {
        println!("Unable to receive remote socket addr");
        return;
    }
    // 3. We should receive a public IP and PORT of remote host
    let (remote_1_addr, remote_1_port) = remote_socket_opt.unwrap();
    println!("First remote socket: {:?}:{}", remote_1_addr, remote_1_port);
    // 4. We repeat steps 1, 2, 3 and now we have two remote socket addresses.
    bind_addr.1 += 1;
    holepunch = UdpSocket::bind(bind_addr)
        .await
        .expect("unable to create socket");
    common_phrase.push_str("bar");
    let remote_socket_opt =
        ask_proxy_for_remote_socket(&holepunch, puncher.clone(), common_phrase).await;
    if remote_socket_opt.is_none() {
        println!("Unable to receive remote socket addr");
        return;
    }
    let (remote_2_addr, remote_2_port) = remote_socket_opt.unwrap();
    println!(
        "Second remote socket: {:?}:{}",
        remote_2_addr, remote_2_port
    );

    // 5. We calculate delta_port based on two received public ports from proxy
    let delta_port = remote_2_port - remote_1_port;
    println!("Delta port: {}", delta_port);
    // 6. We add delta_port to last port number we received from proxy
    let target_remote_port = remote_2_port + delta_port;
    // 7. We create a new socket and try to punch through to remote host using
    //    known remote public IP and PORT calculated in point 6.
    bind_addr.1 += 1;
    holepunch = UdpSocket::bind(bind_addr)
        .await
        .expect("unable to create socket");
    // 8. We pass that socket to punch_it function and await it.
    let punch_it_result = punch_it(
        holepunch,
        (
            my_settings,
            NetworkSettings {
                pub_ip: remote_1_addr,
                pub_port: target_remote_port,
                nat_type: remote_nat,
                port_allocation: remote_port_rule, //TODO maybe we can learn some more on this?
            },
        ),
    )
    .await;
    if let Some(dedicated_socket) = punch_it_result {
        spawn(start_communication(
            dedicated_socket,
            swarm_name.clone(),
            pub_key_pem.clone(),
            decrypter.clone(),
            pipes_sender.clone(),
            sub_sender.clone(),
        ));
        return;
    };

    println!("Round '3'");
    let my_port_min = bind_addr.1 + 2;
    let his_port_min = target_remote_port + delta_port;
    let his_port_max = target_remote_port + (50 * delta_port);
    let timeout = Duration::from_secs(120);
    let punch_it_result = cluster_punch_it(
        bind_addr.0,
        remote_1_addr,
        my_port_min,
        50,
        (his_port_min, delta_port, his_port_max),
        timeout,
    )
    .await;
    if let Some(dedicated_socket) = punch_it_result {
        spawn(start_communication(
            dedicated_socket,
            swarm_name.clone(),
            pub_key_pem.clone(),
            decrypter.clone(),
            pipes_sender.clone(),
            sub_sender.clone(),
        ));
        // return;
    };

    // }
    // // TODO: end this brutality, follow the rigtheous path
    // let (remote_port_start, remote_port_end) = if remote_controls_port {
    //     (remote_1_port, remote_1_port + 50) //.collect::<std::vec::Vec<u16>>();
    //                                         // (remote_e_port, remote_e_port + 50) //.collect::<std::vec::Vec<u16>>();
    //                                         // VecDeque::from(vec)
    // } else {
    //     // let vec =
    //     (remote_1_port - 25, remote_1_port + 25) //.collect::<std::vec::Vec<u16>>();
    //                                              // (remote_e_port - 25, remote_e_port + 25) //.collect::<std::vec::Vec<u16>>();
    //                                              // VecDeque::from(vec)
    // };
    // println!("SADR: {:?}", s_addr);
    // holepunch
    //     .set_read_timeout(Some(Duration::from_secs(5)))
    //     .unwrap();
    // holepunch
    //     .set_write_timeout(Some(Duration::from_secs(1)))
    //     .unwrap();
    // println!("Holepunch and connection successful.");
    // let mut my_external_addr = vec![0; 200];
    // let _result = holepunch.recv(&mut my_external_addr).await;
    // my_external_addr.retain(|e| *e != 0);
    // if !my_external_addr.is_empty() {
    //     let my_addr = String::from_utf8_lossy(&my_external_addr).to_string();
    //     println!("Received external addr: {}", my_addr);
    //     let my_e_port = my_addr
    //         .split(':')
    //         .nth(1)
    //         .unwrap()
    //         .parse::<u16>()
    //         .expect("Unable to parse external port");
    //     let _ = holepunch.send(bind_addr.as_bytes()).await;
    //     // println!("Sent: {:?}", bind_addr);
    //     if holepunch.local_addr().expect("Dunno my local addr").port() == my_e_port {
    //         println!("I can define my external port number!");
    //     }
    //     let _result = holepunch.recv(&mut my_external_addr).await;
    //     let should_be_server = my_e_port < remote_adr.port();
    //     (
    //         // holepunch.local_addr().unwrap(),
    //         remote_adr,
    //         holepunch,
    //         should_be_server,
    //     )
    // } else {
    //     panic!("Did not receive data from remote host");
    // }
    // }
}

async fn ask_proxy_for_remote_socket(
    holepunch: &UdpSocket,
    puncher: String,
    swarm_name: String,
) -> Option<(IpAddr, u16)> {
    let bytes = swarm_name.as_bytes();
    let mut buf = vec![0_u8; 200];
    buf[..bytes.len().min(200)].copy_from_slice(&bytes[..bytes.len().min(200)]);
    // Initial request
    holepunch
        .send_to(&buf, puncher)
        .await
        .expect("unable to talk to helper");
    println!("Waiting for UDPunch server to respond");
    holepunch
        .recv_from(&mut buf)
        .await
        .expect("unable to receive from helper");
    // println!("Holepunch responded: {:?}", buf);
    // TODO: here we should temporarily send some invalid packet to proxy
    // let next_port = holepunch.local_addr().unwrap().port().saturating_add(1);
    // let holepunch = UdpSocket::bind((host_ip, next_port))
    //     .await
    //     .expect("unable to create second socket");
    // // Second request
    // holepunch
    //     .send_to(&buf, puncher)
    //     .await
    //     .expect("unable to talk to helper");
    // println!("Waiting for UDPunch server to respond");
    // holepunch
    //     .recv_from(&mut buf)
    //     .await
    //     .expect("unable to receive from helper");
    // // buf should now contain our partner's address data.
    // println!("Holepunch responded: {:?}", buf);
    buf.retain(|e| *e != 0);
    let remote_addr = String::from_utf8_lossy(buf.as_slice()).to_string();
    let remote_1_addr = remote_addr
        .split(':')
        .next()
        .unwrap()
        .parse::<IpAddr>()
        .expect("Unable to parse remote address");
    let remote_1_port = remote_addr
        .split(':')
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .expect("Unable to parse remote port");
    Some((remote_1_addr, remote_1_port))
    // // Third request
    // let holepunch = UdpSocket::bind((host_ip, next_port.saturating_add(1)))
    //     .await
    //     .expect("unable to create third socket");
    // holepunch
    //     .send_to(&buf, puncher)
    //     .await
    //     .expect("unable to talk to helper");
    // println!("Waiting for UDPunch server to respond");
    // holepunch
    //     .recv_from(&mut buf)
    //     .await
    //     .expect("unable to receive from helper");
    // println!("Holepunch responded: {:?}", buf);
    // // buf should now contain our partner's address data.
    // buf.retain(|e| *e != 0);
    // let remote_addr = String::from_utf8_lossy(buf.as_slice()).to_string();
    // let remote_2_addr = remote_addr
    //     .split(':')
    //     .next()
    //     .unwrap()
    //     .parse::<IpAddr>()
    //     .expect("Unable to parse remote address");
    // let remote_2_port = remote_addr
    //     .split(':')
    //     .nth(1)
    //     .unwrap()
    //     .parse::<u16>()
    //     .expect("Unable to parse remote port");
    // // println!("to be bind addr: {}", bind_addr);
    // let delta_port = remote_2_port - remote_1_port;
    // let remote_e_port = remote_2_port + delta_port;
}

struct MagicPorts(VecDeque<u16>);

impl MagicPorts {
    fn new() -> Self {
        let mut port_list = VecDeque::with_capacity(127);
        let low_end: u16 = 0b00000001_01010101;
        for i in 1..128u16 {
            port_list.push_front((i << 9) + low_end);
        }
        MagicPorts(port_list)
    }
    fn next(&mut self) -> u16 {
        let port = self.0.pop_front().unwrap();
        self.0.push_back(port);
        port
    }
    fn is_magic(&self, port: u16) -> bool {
        self.0.contains(&port)
    }
}

async fn timeout(time: &Duration) -> Result<(), u8> {
    sleep(*time).await;
    Err(0)
}
async fn try_communicate(socket: &UdpSocket, remote_addr: SocketAddr) -> Result<SocketAddr, u8> {
    let mut buf = [2u8; 32];
    let send_result = socket.send_to(&buf, remote_addr).await;
    if send_result.is_err() {
        println!("Unable to send first message: {:?}", send_result);
        return Err(2);
    }
    sleep(Duration::from_millis(1)).await;
    buf[0] = 1;
    let send_result = socket.send_to(&buf, remote_addr).await;
    if send_result.is_err() {
        println!("Unable to send second message: {:?}", send_result);
        return Err(1);
    }
    sleep(Duration::from_millis(10)).await;
    buf[0] = 0;
    let send_result = socket.send_to(&buf, remote_addr).await;
    if send_result.is_err() {
        println!("Unable to send third message: {:?}", send_result);
        return Err(0);
    }
    let recv_result = socket.recv_from(&mut buf).await;
    if recv_result.is_err() {
        println!("Error while waiting for datagram: {:?}", recv_result);
        Err(buf[0])
    } else {
        let (_count, remote) = recv_result.unwrap();
        if remote.ip() == remote_addr.ip() {
            println!("Recieved {} from {:?}", buf[0], remote);
            Ok(remote)
            // socket.send_to(&[0], remote).await;
            // remote_addr = remote;
            // break;
        } else {
            println!(
                "Unexpected response: {:?} from {:?} (sent to {:?})",
                buf, remote, remote_addr
            );
            Err(buf[0])
        }
    }
    // Ok(remote_addr)
}

// The connection procedure should be as follows:
// Once we receive other socket address we send nine one byte datagrams
// in 100ms intervals counting down from 9 to 1
// then we listen for dgrams from remote gnome and receive until we get
// one with a single byte 1.
// Now we can pass that socket for Neighbor creation.
// TODO: If we do not receive any dgrams after specified period of time,
// we can start over from creation of a new socket,
// but current procedure is not successful.
async fn probe_socket(
    socket: UdpSocket,
    remote_addr: SocketAddr,
    sender: Sender<(UdpSocket, Option<SocketAddr>)>,
) {
    // println!("running probe socket");
    let sleep_time = Duration::from_millis(100);
    let wait_time = Duration::from_millis(1100);
    print!("sent ");
    for i in (1..10).rev() {
        let _ = socket.send_to(&[i as u8], remote_addr).await;
        print!("{} ", i);
        sleep(sleep_time).await;
    }
    let mut socket_found = false;
    let mut timed_out = false;
    // println!("waiting for response");
    while !socket_found && !timed_out {
        let t1 = wait_for_bytes(&socket).fuse();
        let t2 = time_out(wait_time, None).fuse();

        pin_mut!(t1, t2);

        (socket_found, timed_out) = select! {
            _result1 = t1 =>  (true,false),
            _result2 = t2 => (false,true),
        };
    }
    // if socket_found {
    //     println!("socket found!");
    // } else {
    //     println!("timed out");
    // } // let sleep_time = Duration::from_millis(rand::random::<u8>() as u64 + 200);
    // loop {
    //     let new_port = port_list.pop_front().unwrap();
    //     remote_addr.set_port(new_port);
    //     port_list.push_back(new_port);
    //     let t1 = timeout(&sleep_time).fuse();
    //     let t2 = try_communicate(&socket, remote_addr).fuse();

    //     pin_mut!(t1, t2);

    //     let break_anyway = select! {
    //         _t1 = t1 =>  false,
    //         result2 = t2 => {match result2{
    //             Ok(rem) =>{
    //                 punch_back(&socket, rem).await;
    //                  remote_addr=rem;
    //                  socket_found = true;
    //                  true},
    //             Err(_e)=>false,
    //         }},
    //     };
    //     // let break_anyway = option.is_some();
    //     if !break_anyway {
    //         let send_result = sender.send(None);
    //         if send_result.is_err() {
    //             // println!("Unable to send: {:?}", send_result);
    //             // println!("Unable to send: {:?}", send_result.err().unwrap());
    //             // std::process::exit(255);
    //             break;
    //         }
    //     } else {
    //         break;
    //     };
    // }
    if socket_found {
        let _ = sender.send((socket, Some(remote_addr)));
    } else {
        let _ = sender.send((socket, None));
        // drop(socket);
    }
}

async fn wait_for_bytes(socket: &UdpSocket) {
    // println!("waiting for bytes");
    let mut bytes: [u8; 10] = [0; 10];
    loop {
        let recv_res = socket.recv_from(&mut bytes).await;
        // println!("Recv: {:?}", recv_res);
        if let Ok((count, from)) = recv_res {
            // println!("Recv: {} from {:?}", bytes[0], from);
            if count == 1 && bytes[0] == 1 {
                return;
            }
        }
    }
}

async fn punch_back(
    socket: &UdpSocket,
    remote_addr: SocketAddr,
    // sender: Sender<Option<(SocketAddr, UdpSocket)>>,
) {
    println!("Fixing target...");
    let con_res = socket.connect(remote_addr).await;
    println!("Con res: {:?}", con_res);
    println!("Sending back...");
    let mut i = 10;
    while i > 0 {
        let _res = socket.send(&[1]).await;
        i -= 1
    }
    let mut buf = [0; 200];
    println!("Waiting for response...");
    let _res = socket.recv(&mut buf).await;
    println!("Response {:?} received: {:?}", &buf, _res);
    // sender.send(Some((remote_addr, socket)));
}

// TODO: implement a timeout mechanism
// TODO: receive other NetworkSettings as argument and
// write more sophisticated procedure for connection
// establishment depending on our and their NetworkSettings
pub async fn punch_it(
    socket: UdpSocket,
    // socket_sender: Sender<Option<UdpSocket>>,
    (my_settings, other_settings): (NetworkSettings, NetworkSettings),
) -> Option<UdpSocket> {
    let mut remote_adr = SocketAddr::new(other_settings.pub_ip, other_settings.pub_port);
    println!(
        "Punching: {:?}:{:?}",
        other_settings.pub_ip, other_settings.pub_port
    );
    let (sender, reciever) = channel();
    spawn(probe_socket(socket, remote_adr, sender.clone()));
    let mut dedicated_socket: Option<UdpSocket> = None;
    let mut responsive_socket_result;
    let mut retries_remaining = 5;
    while retries_remaining > 0 {
        responsive_socket_result = reciever.try_recv();
        match responsive_socket_result {
            Ok((socket, None)) => {
                // println!("none");
                // if my_settings.nat_at_most_address_sensitive()
                if my_settings.port_allocation_predictable()
                    && other_settings.port_allocation_predictable()
                {
                    println!("\nHit me babe one more time");
                    // TODO: we try to cover case when NAT assigned next port
                    // while we were trying to initialize communication
                    let mut local_addr = socket.local_addr().unwrap();
                    local_addr.set_port(local_addr.port() + 1);
                    let socket = UdpSocket::bind(local_addr).await.unwrap();
                    let next_port = other_settings.port_increment(remote_adr.port());
                    remote_adr.set_port(next_port);
                    spawn(probe_socket(socket, remote_adr, sender.clone()));
                    retries_remaining -= 1;
                } else {
                    break;
                }
            }

            Ok((socket, Some(remote_addr))) => {
                println!("Got a sock!");
                let conn_result = socket.connect(remote_addr).await;
                if conn_result.is_err() {
                    println!("Unable to make dedicated connection");
                }
                dedicated_socket = Some(socket);
                break;
            }
            Err(_e) => {
                // println!("Receiver error: {:?}", e);
            }
        }
        yield_now().await;
    }
    drop(reciever);
    // if dedicated_socket.is_none() {
    //     println!("Failed to communicate with remote gnome");
    //     return;
    // }else{
    // let _ = socket_sender.send(dedicated_socket);
    dedicated_socket
}

pub async fn cluster_punch_it(
    my_ip: IpAddr,
    his_ip: IpAddr,
    my_port_min: u16,
    my_count: u16,
    (his_port_min, his_step, his_port_max): (u16, u16, u16),
    timeout_sec: Duration,
) -> Option<UdpSocket> {
    println!("Performing clusterpunch");
    println!(
        "my sockets: {:?} ports: {}-{}",
        my_ip,
        my_port_min,
        my_port_min + my_count
    );
    println!(
        "his sockets: {:?} ports: {}-{}",
        his_ip, his_port_min, his_port_max
    );
    let mut next_remote_port: HashMap<u16, u16> = HashMap::new();
    let mut his_current = his_port_min;
    let (send, recv) = channel();
    for i in 0..my_count {
        let a_socket = UdpSocket::bind((my_ip, my_port_min + i)).await.unwrap();
        spawn(probe_socket(
            a_socket,
            SocketAddr::new(his_ip, his_current),
            send.clone(),
        ));
        his_current += his_step;
        if his_current > his_port_max {
            his_current = his_port_min;
        }
        next_remote_port.insert(my_port_min + i, his_current);
    }

    // TODO: we need to create multiple socket instances: 50 maybe 100
    // for each instance we need to track what remote we are currently trying to reach
    // if a given socket has timed out, we increment the target socket port by step and
    // probe again, if port > max, we start from min
    // we repeat until timeout if given
    let sleep_duration = Duration::from_millis(50);
    let mut dedicated_socket = None;
    let (t_send, timeout) = channel();
    spawn(time_out(timeout_sec, Some(t_send)));
    while timeout.try_recv().is_err() {
        let recv_result = recv.try_recv();
        if recv_result.is_err() {
            sleep(sleep_duration).await;
            continue;
        }
        match recv_result {
            Ok((socket, None)) => {
                let local_port = socket.local_addr().unwrap().port();
                let mut his_next = next_remote_port.remove(&local_port).unwrap();
                spawn(probe_socket(
                    socket,
                    SocketAddr::new(his_ip, his_next),
                    send.clone(),
                ));
                his_next += his_step;
                if his_next > his_port_max {
                    his_next = his_port_min;
                }
                next_remote_port.insert(local_port, his_next);
            }
            Ok((socket, Some(remote_addr))) => {
                let _ = socket.connect(remote_addr).await;
                dedicated_socket = Some(socket);
                break;
            }
            other => {
                println!("Unexpected probe result: {:?}", other);
            }
        }
    }
    drop(recv);
    drop(timeout);
    dedicated_socket
}

pub async fn start_communication(
    dedicated_socket: UdpSocket,
    swarm_name: String,
    pub_key_pem: String,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    sub_sender: Sender<Subscription>,
) {
    // TODO: above is just one of possible procedures used for holepunching
    // below we need to wrap it into an async fn and call it when
    // above procedure or similar returns a punched socket and remote address
    // that are ready for message exchange
    // let dedicated_socket = dedicated_socket.unwrap();
    let remote_addr = dedicated_socket.peer_addr().unwrap();
    println!("We did it: {:?}", remote_addr);
    // if s_addr.is_err() {
    //     println!("Coś poszło nie tak");
    //     ::std::process::exit(1);
    // }
    // let remote_adr = s_addr.unwrap();
    // let sleep_time = Duration::from_millis(50);
    let _send_result = dedicated_socket.send(pub_key_pem.as_bytes()).await;
    // sleep(sleep_time).await;
    println!(
        "Send PUB key to: {:?} from: {:?} result: {:?}",
        remote_addr, //changed here
        dedicated_socket.local_addr().unwrap(),
        _send_result
    );
    println!("Waiting for some response...");
    let mut count;
    let mut buf = [0u8; 1100];
    loop {
        let recv_result = dedicated_socket.recv(&mut buf).await;
        if recv_result.is_err() {
            println!("Unable to receive from remote host on dedicated socket");
        }
        count = recv_result.unwrap();
        println!("Recieve {} count: {}", buf[0], count);
        if count > 100 {
            break;
        }
    }
    // let remote_addr = SocketAddr::from_str("")

    // dedicated_socket.connect(remote_addr).await.unwrap();
    let id_pub_key_pem = std::str::from_utf8(&buf[..count]).unwrap();
    let result = Encrypter::create_from_data(id_pub_key_pem);
    if result.is_err() {
        println!("Failed to build Encrypter from received PEM: {:?}", result);
        // return None;
    }
    let encr = result.unwrap();
    let remote_gnome_id = GnomeId(encr.hash());

    let key = generate_symmetric_key();
    let mut session_key = SessionKey::from_key(&key);
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    drop(loc_encr);
    if remote_gnome_id.0 < gnome_id.0 {
        // TODO maybe send remote external IP here?
        let bytes_to_send = Vec::from(&key);
        let encr_res = encr.encrypt(&bytes_to_send);
        if encr_res.is_err() {
            println!("Failed to encrypt symmetric key: {:?}", encr_res);
            // return None;
        }
        println!("Encrypted symmetric key");

        let encrypted_data = encr_res.unwrap();
        // let res = socket.send_to(&encrypted_data, remote_addr).await;
        let res = dedicated_socket.send(&encrypted_data).await;
        if res.is_err() {
            println!("Failed to send encrypted symmetric key: {:?}", res);
            // return None;
        }
    } else {
        let recv_result = dedicated_socket.recv(&mut buf).await;
        if recv_result.is_err() {
            println!("Failed to received encrypted session key");
            // return
        }
        // let count = recv_result.unwrap();
        // let mut decoded_key: Option<[u8; 32]> = None;
        // println!("Dec key: {:?}", decoded_key);
        if let Ok(count) = recv_result {
            // remote_addr = remote_adr;
            println!("Received {}bytes", count);
            let decoded_key = decrypter.decrypt(&buf[..count]);

            // let _res = req_sender.send(Vec::from(&buf[..count]));
            // println!("Sent decode request: {:?}", _res);
            // loop {
            //     let response = resp_receiver.try_recv();
            //     if let Ok(symmetric_key) = response {
            //         // match subs_resp {
            //         //     Subscription::KeyDecoded(symmetric_key) => {
            //         //         // decoded_port = port;
            //         decoded_key = Some(symmetric_key);
            //         break;
            //         //     }
            //         //     Subscription::DecodeFailure => {
            //         //         println!("Failed decoding symmetric key!");
            //         //         break;
            //         //     }
            //         //     _ => println!("Unexpected message: {:?}", subs_resp),
            //         // }
            //     }
            //     yield_now().await
            // }
            if let Ok(sym_key) = decoded_key {
                println!("Got session key: {:?}", sym_key);
                session_key = SessionKey::from_key(&sym_key.try_into().unwrap());
            } else {
                println!("Unable to decode key");
                // return receiver;
            }
        }
    }
    let encr_address = session_key.encrypt(remote_addr.to_string().as_bytes());

    let send_remote_addr_result = dedicated_socket.send(&encr_address).await;
    if send_remote_addr_result.is_err() {
        println!(
            "Failed to send Neighbor's external address: {:?}",
            send_remote_addr_result
        );
    } else {
        println!("External address sent with success");
    }
    let mut buf = [0u8; 128];
    let recv_my_ext_addr_res = dedicated_socket.recv(&mut buf).await;
    if recv_my_ext_addr_res.is_err() {
        println!(
            "Unable to recv my external address from remote: {:?}",
            recv_my_ext_addr_res
        );
    }
    let count = recv_my_ext_addr_res.unwrap();
    let decode_result = session_key.decrypt(&buf[..count]);
    if decode_result.is_err() {
        println!(
            "Unable to decode received data with session key: {:?}",
            decode_result
        );
    }
    let decoded_data = decode_result.unwrap();
    let my_ext_addr_res = std::str::from_utf8(&decoded_data);
    if my_ext_addr_res.is_err() {
        println!(
            "Failed to parse string from received data: {:?}",
            my_ext_addr_res
        );
    }
    let my_e_addr_str = my_ext_addr_res.unwrap();
    let my_pub_ip = my_e_addr_str
        .split(':')
        .next()
        .unwrap()
        .parse::<IpAddr>()
        .expect("Unable to parse my external address");
    let my_pub_port = my_e_addr_str
        .split(':')
        .nth(1)
        .unwrap()
        .parse::<u16>()
        .expect("Unable to parse my external port");
    let has_port_control = my_pub_port == dedicated_socket.local_addr().unwrap().port();
    let my_nat = if has_port_control {
        Nat::SymmetricWithPortControl
    } else {
        Nat::Symmetric
    };

    let _ = sub_sender.send(Subscription::Distribute(my_pub_ip, my_pub_port, my_nat));
    // println!("My external addr: {}", my_ext_addr_res.unwrap());
    // }
    let swarm_names = vec![swarm_name];
    spawn(prepare_and_serve(
        dedicated_socket,
        remote_gnome_id,
        session_key,
        swarm_names,
        sub_sender.clone(),
        pipes_sender.clone(),
    ));
    // resp_receiver
}
