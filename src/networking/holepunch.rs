use super::Token;
use crate::crypto::{generate_symmetric_key, SessionKey};
use crate::networking::client::prepare_and_serve;
use crate::networking::subscription::Subscription;
use crate::prelude::Encrypter;
use async_std::net::UdpSocket;
use async_std::task::{sleep, spawn, yield_now};
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::channel;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use swarm_consensus::GnomeId;

// let puncher = "tudbut.de:4277";
// let swarm_name = "irdm".to_string();
// let (remote_addr, socket, server) = holepunch(puncher, swarm_name);
pub async fn holepunch(
    puncher: &str,
    host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    req_sender: Sender<Vec<u8>>,
    resp_receiver: Receiver<[u8; 32]>,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    receiver: Receiver<String>,
    pub_key_pem: String,
) {
    // ) -> (SocketAddr, UdpSocket, bool) {
    println!("Holepunch started");
    let mut magic_ports = MagicPorts::new();
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    //TODO: loop here
    loop {
        let recv_result = receiver.try_recv();
        if recv_result.is_err() {
            // println!("Error receiving swarm_name: {:?}", recv_result);
            yield_now().await;
            continue;
            // return;
        }
        let swarm_name: String = recv_result.unwrap();
        println!("Establishing connection with helper...");
        let bind_port = magic_ports.next();
        let bind_addr = (host_ip, bind_port);
        let holepunch = UdpSocket::bind(bind_addr)
            .await
            .expect("unable to create socket");
        // holepunch
        //     .connect(puncher)
        //     .await
        //     .expect("unable to connect to helper");
        let bytes = swarm_name.as_bytes();
        let mut buf = vec![0_u8; 200];
        buf[..bytes.len().min(200)].copy_from_slice(&bytes[..bytes.len().min(200)]);
        holepunch
            .send_to(&buf, puncher)
            .await
            .expect("unable to talk to helper");
        println!("Waiting for UDPunch server to respond");
        holepunch
            .recv_from(&mut buf)
            .await
            .expect("unable to receive from helper");
        // buf should now contain our partner's address data.
        buf.retain(|e| *e != 0);
        let remote_addr = String::from_utf8_lossy(buf.as_slice()).to_string();
        let remote_e_addr = remote_addr
            .split(':')
            .next()
            .unwrap()
            .parse::<IpAddr>()
            .expect("Unable to parse remote address");
        let remote_e_port = remote_addr
            .split(':')
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .expect("Unable to parse remote port");
        // println!("to be bind addr: {}", bind_addr);
        let remote_controls_port = magic_ports.is_magic(remote_e_port);
        println!(
            "Holepunching {} (magic?: {:?}) and :{} (you) for {}.",
            remote_addr,
            remote_controls_port,
            holepunch.local_addr().unwrap().port(),
            swarm_name
        );
        let local_ports = (bind_port + 1..bind_port + 50).collect::<std::vec::Vec<u16>>();
        let mut remote_ports = if remote_controls_port {
            let vec = (remote_e_port..remote_e_port + 50).collect::<std::vec::Vec<u16>>();
            VecDeque::from(vec)
        } else {
            let vec = (remote_e_port - 25..remote_e_port + 25).collect::<std::vec::Vec<u16>>();
            VecDeque::from(vec)
        };
        // println!("Using remote ports: {:?}", remote_ports);
        let remote_adr = SocketAddr::new(remote_e_addr, remote_e_port);
        let (sender, reciever) = channel();
        spawn(probe_socket(
            holepunch,
            remote_adr,
            remote_ports.clone(),
            sender.clone(),
        ));
        for my_port in local_ports {
            let port = remote_ports.pop_front().unwrap();
            remote_ports.push_back(port);
            let mut b_addr: SocketAddr = bind_addr.into();
            b_addr.set_port(my_port);
            let bind_result = UdpSocket::bind(b_addr).await;
            if bind_result.is_err() {
                println!("Unable to bind {:?}: {:?}", bind_addr, bind_result);
                continue;
            }
            let socket = bind_result.unwrap();
            spawn(probe_socket(
                socket,
                remote_adr,
                remote_ports.clone(),
                sender.clone(),
            ));
        }
        let bind_addr = (host_ip, 0);
        let mut dedicated_socket = UdpSocket::bind(bind_addr).await.unwrap();
        println!(
            "Waiting for some socket on {:?}",
            dedicated_socket.local_addr()
        );
        let mut responsive_socket_result;
        loop {
            responsive_socket_result = reciever.try_recv();
            match responsive_socket_result {
                Ok(None) => continue,
                Ok(Some((remote_addr, socket))) => {
                    println!("Got a sock!");
                    dedicated_socket = socket;
                    let conn_result = dedicated_socket.connect(remote_addr).await;
                    if conn_result.is_err() {
                        println!("Unable to make dedicated connection");
                    }
                    break;
                }
                Err(_e) => {
                    // println!("Receiver error: {:?}", e);
                }
            }
        }
        drop(reciever);
        println!("We did it: {:?}", dedicated_socket.peer_addr().unwrap());
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
            remote_adr,
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
            let mut decoded_key: Option<[u8; 32]> = None;
            println!("Dec key: {:?}", decoded_key);
            if let Ok(count) = recv_result {
                // remote_addr = remote_adr;
                println!("Received {}bytes", count);

                let _res = req_sender.send(Vec::from(&buf[..count]));
                println!("Sent decode request: {:?}", _res);
                loop {
                    let response = resp_receiver.try_recv();
                    if let Ok(symmetric_key) = response {
                        // match subs_resp {
                        //     Subscription::KeyDecoded(symmetric_key) => {
                        //         // decoded_port = port;
                        decoded_key = Some(symmetric_key);
                        break;
                        //     }
                        //     Subscription::DecodeFailure => {
                        //         println!("Failed decoding symmetric key!");
                        //         break;
                        //     }
                        //     _ => println!("Unexpected message: {:?}", subs_resp),
                        // }
                    }
                    yield_now().await
                }
                if let Some(sym_key) = decoded_key {
                    session_key = SessionKey::from_key(&sym_key);
                    println!("Got session key: {:?}", sym_key);
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
        // let my_ext_addr = SocketAddr::from(my_ext_addr_res.unwrap());
        println!("My external addr: {}", my_ext_addr_res.unwrap());
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
    }
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

async fn probe_socket(
    socket: UdpSocket,
    mut remote_addr: SocketAddr,
    mut port_list: VecDeque<u16>,
    sender: Sender<Option<(SocketAddr, UdpSocket)>>,
) {
    let sleep_time = Duration::from_millis(rand::random::<u8>() as u64 + 200);
    loop {
        let new_port = port_list.pop_front().unwrap();
        remote_addr.set_port(new_port);
        port_list.push_back(new_port);
        let t1 = timeout(&sleep_time).fuse();
        let t2 = try_communicate(&socket, remote_addr).fuse();

        pin_mut!(t1, t2);

        let break_anyway = select! {
            _t1 = t1 =>  false,
            result2 = t2 => {match result2{
                Ok(rem) =>{
                    punch_back(&socket, rem).await;
                     remote_addr=rem;
                     true},
                Err(_e)=>false,
            }},
        };
        // let break_anyway = option.is_some();
        if !break_anyway {
            let send_result = sender.send(None);
            if send_result.is_err() {
                // println!("Unable to send: {:?}", send_result);
                // println!("Unable to send: {:?}", send_result.err().unwrap());
                // std::process::exit(255);
                return;
            }
        } else {
            break;
        };
    }
    let _ = sender.send(Some((remote_addr, socket)));
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
