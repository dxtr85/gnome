use super::Token;
use crate::crypto::{generate_symmetric_key, SessionKey};
use crate::networking::client::prepare_and_serve;
use crate::networking::subscription::Subscription;
use crate::prelude::Encrypter;
use async_std::net::UdpSocket;
use async_std::task::{spawn, yield_now};
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{Receiver, Sender};
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
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    //TODO: loop here
    loop {
        let recv_result = receiver.try_recv();
        if recv_result.is_err() {
            println!("Error receiving swarm_name: {:?}", recv_result);
            yield_now().await;
            continue;
            // return;
        }
        let swarm_name: String = recv_result.unwrap();
        println!("Establishing connection with helper...");
        let bind_addr = (host_ip, 0);
        let holepunch = UdpSocket::bind(bind_addr)
            .await
            .expect("unable to create socket");
        holepunch
            .connect(puncher)
            .await
            .expect("unable to connect to helper");
        let bytes = swarm_name.as_bytes();
        let mut buf = vec![0_u8; 200];
        buf[..bytes.len().min(200)].copy_from_slice(&bytes[..bytes.len().min(200)]);
        holepunch
            .send(&buf)
            .await
            .expect("unable to talk to helper");
        println!("Waiting for UDPunch server to respond");
        holepunch
            .recv(&mut buf)
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
        println!(
            "Holepunching {} (partner) and :{} (you) for {}.",
            remote_addr,
            holepunch.local_addr().unwrap().port(),
            swarm_name
        );
        let remote_adr = SocketAddr::new(remote_e_addr, remote_e_port);
        // if s_addr.is_err() {
        //     println!("Coś poszło nie tak");
        //     ::std::process::exit(1);
        // }
        // let remote_adr = s_addr.unwrap();
        let dedicated_socket = UdpSocket::bind(bind_addr).await.unwrap();
        let _send_result = dedicated_socket
            .send_to(pub_key_pem.as_bytes(), remote_adr)
            .await;
        let mut buf = [0u8; 1100];
        let recv_result = dedicated_socket.recv_from(&mut buf).await;
        if recv_result.is_err() {
            println!("Unable to receive from remote host on dedicated socket");
        }
        let (count, remote_addr) = recv_result.unwrap();
        dedicated_socket.connect(remote_addr).await.unwrap();
        let id_pub_key_pem = std::str::from_utf8(&buf[..count]).unwrap();
        let result = Encrypter::create_from_data(id_pub_key_pem);
        if result.is_err() {
            println!("Failed to build Encripter from received PEM: {:?}", result);
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
                let encr_address = session_key.encrypt(remote_addr.to_string().as_bytes());

                let send_remote_addr_result = dedicated_socket.send(&encr_address).await;
                if send_remote_addr_result.is_err() {
                    println!("Failed to send Neighbor's external address");
                } else {
                    println!("External address sent with success");
                }
                let mut buf = [0u8; 128];
                let recv_my_ext_addr_res = dedicated_socket.recv(&mut buf).await;
                if recv_my_ext_addr_res.is_err() {
                    println!("Unable to recv my external address from remote");
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

                let swarm_names = vec![swarm_name];
                spawn(prepare_and_serve(
                    dedicated_socket,
                    remote_gnome_id,
                    session_key,
                    swarm_names,
                    sub_sender.clone(),
                    pipes_sender.clone(),
                ));
            }
        }
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
