use super::common::swarm_names_from_bytes;
use super::tcp_common::serve_socket;
// use super::Token;
use crate::crypto::Decrypter;
use crate::crypto::Encrypter;
use crate::crypto::SessionKey;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::time_out;
use crate::networking::status::Transport;
use crate::networking::subscription::Subscription;
use crate::networking::tcp_common::send_subscribed_swarm_names;
use crate::networking::Nat;
use crate::networking::NetworkSettings;
use crate::networking::PortAllocationRule;
use crate::networking::Transport as GTransport;
use async_std::io::ReadExt;
use async_std::net::TcpStream;
use async_std::task::sleep;
use async_std::task::spawn;
use futures::AsyncWriteExt;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::sync::mpsc::{channel, Sender};
use std::time::Duration;
use swarm_consensus::GnomeId;
use swarm_consensus::SwarmName;

pub async fn run_tcp_client(
    my_id: GnomeId,
    swarm_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    // pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    (ip, port): (IpAddr, u16),
) {
    eprintln!("TCP CLIENT {:?}:{}", ip, port);
    // TODO: we need to race two tasks:
    // 1 - connect task
    // 2 - timeout
    // Since we could wait forever and consume resources
    //
    let t1 = try_to_connect(ip, port).fuse();
    let t2 = start_timer(Duration::from_secs(1)).fuse();
    pin_mut!(t1, t2);
    let (timeout, result) = select! {
        result1 = t1 =>  (false, result1),
        result2 = t2 => (true, result2),
    };
    if timeout || result.is_none() {
        eprintln!("TCP connect timeout/error");
        return;
    }
    let mut stream = result.unwrap();

    // let send_result = socket.send_to(pub_key_pem.as_bytes(), send_addr).await;
    let send_result = stream.write(pub_key_pem.as_bytes()).await;
    let _flush_result = stream.flush().await;
    // It appears that there is a bug in async scheduler that
    // sometimes ignores given task if it does not have any
    // IO operations.
    // For that reason following eprintln! call is necessary
    // to establish a new TCP connection with a neighbor
    // In other places some eprintln! calls also serve this fn
    eprintln!("Pubkey sent: {:?}({:?})", send_result, _flush_result);
    if send_result.is_err() {
        eprintln!("Unable te send my pub key: {:?}", send_result);
        let _ = stream.close().await;
        return;
        // return receiver;
        // } else {
        //     eprintln!("Send result: {:?}", send_result);
    }

    let timeout_sec = Duration::from_secs(5);
    let (t_send, _timeout) = channel();
    spawn(time_out(timeout_sec, Some(t_send)));
    // while timeout.try_recv().is_err() {
    if swarm_names.is_empty() {
        eprintln!("User is not subscribed to any Swarms");
        let _ = stream.close().await;
        return;
        // return receiver;
    }
    // eprintln!("TCP Client swarm_names: {:?}", swarm_names);
    establish_secure_connection(
        my_id,
        stream.clone(),
        sender.clone(),
        // req_sender.clone(),
        // resp_receiver,
        decrypter.clone(),
        // pipes_sender.clone(),
        swarm_names.clone(),
    )
    .await;
    // }
    // eprintln!("TCP Client is done");
    // receiver
}
async fn try_to_connect(ip: IpAddr, port: u16) -> Option<TcpStream> {
    let stream = TcpStream::connect((ip, port)).await;
    eprintln!("TCP connect returned");
    if stream.is_err() {
        eprintln!("Could not establish TCP connection with {:?}", (ip, port));
        None
    } else {
        eprintln!("Established TCP connection with {:?}", (ip, port));
        Some(stream.unwrap())
    }
}

async fn start_timer(timespan: Duration) -> Option<TcpStream> {
    sleep(timespan).await;
    None
}

async fn establish_secure_connection(
    my_id: GnomeId,
    mut stream: TcpStream,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    // pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_names: Vec<SwarmName>,
) {
    eprintln!("In establish secure TCP connection ");
    // let mut remote_gnome_id: GnomeId = GnomeId(0);
    let session_key: SessionKey; // = SessionKey::from_key(&[0; 32]);
                                 // let remote_addr: SocketAddr; // = "0.0.0.0:0".parse().unwrap();
    let mut recv_buf = [0u8; 1100];

    // let mut socket_found = false;
    let wait_time = Duration::from_secs(1);
    let t1 = stream.read(&mut recv_buf).fuse();
    let t2 = time_out(wait_time, None).fuse();

    pin_mut!(t1, t2);
    let mut count = 0;

    let timed_out = select! {
        _result1 = t1 =>{
            count = _result1.unwrap();
            false},
        _result2 = t2 => true,
    };
    if timed_out {
        eprintln!("I am done, timed out");
        let _ = stream.close().await;
        return;
    } else if count == 0 {
        // TODO: fix this
        eprintln!("zero bytes received");
        //     let _ = stream.close().await;
        //     return;
    }
    // let recv_result = stream.recv_from(&mut recv_buf).await;
    // if recv_result.is_ok() {
    // (count, remote_addr) = recv_result.unwrap();
    // eprintln!("Got {} bytes back", count);
    // } else {
    //     return;
    // }

    // let mut decoded_key: Option<[u8; 32]> = None;
    // println!("Dec key: {:?}", decoded_key);
    // if let Ok((count, remote_adr)) = recv_result {
    //     remote_addr = remote_adr;
    eprintln!("Received {} bytes 1", count);
    let decoded_key = decrypter.decrypt(&recv_buf[..count]);

    if let Ok(sym_key) = decoded_key {
        // eprintln!("Got session key: {:?}", sym_key);
        session_key = SessionKey::from_key(&sym_key.try_into().unwrap());
    } else {
        eprintln!("Unable to decode key: {}", decoded_key.err().unwrap());
        // return resp_receiver;
        let _ = stream.close().await;
        return;
    }

    // let dedicated_socket = UdpSocket::bind(SocketAddr::new(stream.local_addr().unwrap().ip(), 0))
    //     .await
    //     .unwrap();
    // dedicated_socket.connect(remote_addr).await.unwrap();
    // let _ = dedicated_socket.send(&[0u8]).await;

    let mut recv_buf = [0u8; 1100];
    let recv_result = stream.read(&mut recv_buf).await;
    if let Ok(count) = recv_result {
        eprintln!("Received {} bytes 2", count);
        let decr_res = session_key.decrypt(&recv_buf[..count]);
        if let Ok(remote_pubkey_pem) = decr_res {
            let mut r_buf = [0u8; 64];
            let recv_result2 = stream.read(&mut r_buf).await;
            if let Ok(count) = recv_result2 {
                eprintln!("TCP 2 Received {} bytes", count);
                let decr_res = session_key.decrypt(&r_buf[..count]);
                let mut my_public_address = None;
                if let Ok(pib) = decr_res {
                    let port = u16::from_be_bytes([pib[0], pib[1]]);
                    match pib.len() {
                        6 => {
                            //TODO: ip_v4
                            my_public_address = Some((
                                IpAddr::V4(Ipv4Addr::new(pib[2], pib[4], pib[4], pib[5])),
                                port,
                            ));
                        }
                        18 => {
                            let a: u16 = u16::from_be_bytes([pib[2], pib[3]]);
                            let b: u16 = u16::from_be_bytes([pib[4], pib[5]]);
                            let c: u16 = u16::from_be_bytes([pib[6], pib[7]]);
                            let d: u16 = u16::from_be_bytes([pib[8], pib[9]]);
                            let e: u16 = u16::from_be_bytes([pib[10], pib[11]]);
                            let f: u16 = u16::from_be_bytes([pib[12], pib[13]]);
                            let g: u16 = u16::from_be_bytes([pib[14], pib[15]]);
                            let h: u16 = u16::from_be_bytes([pib[16], pib[17]]);
                            my_public_address =
                                Some((IpAddr::V6(Ipv6Addr::new(a, b, c, d, e, f, g, h)), port));
                        }
                        other => {
                            eprintln!("Received {} bytes as my public IP", other);
                            // IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
                        }
                    };
                } else {
                    eprintln!("Failed to decrypt my public address");
                }

                if let Some(pub_ip) = my_public_address {
                    let transport = if pub_ip.0.is_ipv4() {
                        GTransport::TCPoverIP4
                    } else {
                        GTransport::TCPoverIP6
                    };
                    let mut own_nsettings = None;
                    // compare against local socket & build NetworkSettings
                    if let Ok(local_addr) = stream.local_addr() {
                        if local_addr.port() == pub_ip.1 {
                            //TODO: same port
                            if local_addr.ip() == pub_ip.0 {
                                //TODO: no NAT?
                                own_nsettings = Some(NetworkSettings::new_not_natted(
                                    pub_ip.0, pub_ip.1, transport,
                                ));
                            } else {
                                //TODO: 1-1 port mapping
                                let mset = NetworkSettings {
                                    pub_ip: pub_ip.0,
                                    pub_port: pub_ip.1,
                                    nat_type: Nat::Unknown,
                                    port_allocation: (PortAllocationRule::FullCone, 127),
                                    transport,
                                };
                                own_nsettings = Some(mset);
                            }
                        } else {
                            //TODO: different ports, can not determine
                            let mset = NetworkSettings {
                                pub_ip: pub_ip.0,
                                pub_port: pub_ip.1,
                                nat_type: Nat::Unknown,
                                port_allocation: (PortAllocationRule::Random, 127),
                                transport,
                            };
                            own_nsettings = Some(mset);
                        }
                    }
                    if let Some(settings) = own_nsettings {
                        eprintln!("Distribute from tcp_client");
                        let _ = sender.send(Subscription::Distribute(
                            settings.pub_ip,
                            settings.pub_port,
                            settings.nat_type,
                            settings.port_allocation,
                            Transport::Tcp,
                        ));
                    }
                }
            } else {
                eprintln!("UDP Failed to receive additional data from remote");
                return;
            }
            let remote_id_pub_key_pem =
                std::str::from_utf8(&remote_pubkey_pem).unwrap().to_string();
            spawn(prepare_and_serve(
                my_id,
                stream,
                // remote_gnome_id,
                session_key,
                swarm_names,
                sender.clone(),
                // pipes_sender.clone(),
                // encr,
                remote_id_pub_key_pem,
            ));
        } else {
            eprintln!("Failed to decrypt message");
            stream.close().await;
            // return;
        }
    } else {
        eprintln!("Failed to receive data from remote");
        stream.close().await;
        // return;
    }
}

pub async fn prepare_and_serve(
    my_id: GnomeId,
    mut stream: TcpStream,
    // remote_gnome_id: GnomeId,
    session_key: SessionKey,
    swarm_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    // pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // encrypter: Encrypter,
    pub_key_pem: String,
) {
    let encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let remote_gnome_id = GnomeId(encr.hash());
    eprintln!("TCP client Remote GnomeId: {}", remote_gnome_id);
    // println!("Decrypted PEM using session key:\n {:?}", pub_key_pem);
    send_subscribed_swarm_names(&mut stream, &swarm_names).await;

    let mut remote_names: Vec<SwarmName> = vec![];
    receive_remote_swarm_names(&mut stream, &mut remote_names).await;
    if remote_names.is_empty() {
        eprintln!("Neighbor {} did not provide swarm list", remote_gnome_id);
        stream.close().await;
        return;
        // } else {
        //     eprintln!("TCP Client Remote names from bytes: {:?}", remote_names);
    }

    let mut common_names = vec![];
    // eprintln!("My swarm names: {:?}", swarm_names);
    // eprintln!("His swarm names: {:?}", remote_names);
    distil_common_names(
        my_id,
        remote_gnome_id,
        &mut common_names,
        swarm_names,
        &mut remote_names,
    );
    if common_names.is_empty() {
        eprintln!("No common interests with {}", remote_gnome_id);
        stream.close().await;
        return;
    }
    // eprintln!("TCP Common swarm names: {:?}", common_names);
    let (shared_sender, swarm_extend_receiver) = channel();

    let mut ch_pairs = vec![];
    create_a_neighbor_for_each_swarm(
        common_names,
        remote_names,
        sender.clone(),
        remote_gnome_id,
        &mut ch_pairs,
        shared_sender.clone(),
        pub_key_pem,
    );

    // spawn a task to serve socket
    // let (token_send, token_recv) = channel();
    // let (token_send_two, token_recv_two) = channel();
    // let _ = pipes_sender.send((token_send, token_recv_two));
    serve_socket(
        session_key,
        stream,
        ch_pairs,
        // token_send_two,
        // token_recv,
        sender,
        shared_sender,
        swarm_extend_receiver,
    )
    .await;
}

async fn receive_remote_swarm_names(
    reader: &mut TcpStream,
    // recv_buf: &mut BytesMut,
    remote_names: &mut Vec<SwarmName>,
) {
    let mut recv_buf = [0u8; 1450];
    *remote_names = if let Ok(count) = reader.read(&mut recv_buf).await {
        eprintln!(
            "Recv buf (count: {}): {:?}",
            count,
            &recv_buf[..count] // String::from_utf8(recv_buf[..count].try_into().unwrap()).unwrap()
        );
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
