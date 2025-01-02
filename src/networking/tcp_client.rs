use super::common::swarm_names_from_bytes;
use super::tcp_common::serve_socket;
use super::Token;
use crate::crypto::Decrypter;
use crate::crypto::Encrypter;
use crate::crypto::SessionKey;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::common::time_out;
use crate::networking::subscription::Subscription;
use crate::networking::tcp_common::send_subscribed_swarm_names;
use async_std::io::ReadExt;
use async_std::io::WriteExt;
use async_std::net::TcpStream;
use async_std::task::spawn;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::net::IpAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::GnomeId;
use swarm_consensus::SwarmName;

pub async fn run_tcp_client(
    my_id: GnomeId,
    swarm_names: Vec<SwarmName>,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    (ip, port): (IpAddr, u16),
) {
    eprintln!("TCP CLIENT {:?}-{}", ip, port);
    let stream = TcpStream::connect((ip, port)).await;
    if stream.is_err() {
        eprintln!("Could not establish TCP connection with {:?}", (ip, port));
        return;
    }
    let mut stream = stream.unwrap();

    // let send_result = socket.send_to(pub_key_pem.as_bytes(), send_addr).await;
    let send_result = stream.write(pub_key_pem.as_bytes()).await;
    let _flush_result = stream.flush().await;
    if send_result.is_err() {
        eprintln!("Unable te send broadcast message: {:?}", send_result);
        return;
        // return receiver;
    } else {
        eprintln!("Send result: {:?}", send_result);
    }

    let timeout_sec = Duration::from_secs(5);
    let (t_send, _timeout) = channel();
    spawn(time_out(timeout_sec, Some(t_send)));
    // while timeout.try_recv().is_err() {
    if swarm_names.is_empty() {
        eprintln!("User is not subscribed to any Swarms");
        return;
        // return receiver;
    }
    eprintln!("TCP Client swarm_names: {:?}", swarm_names);
    establish_secure_connection(
        my_id,
        stream.clone(),
        sender.clone(),
        // req_sender.clone(),
        // resp_receiver,
        decrypter.clone(),
        pipes_sender.clone(),
        swarm_names.clone(),
    )
    .await;
    // }
    eprintln!("Client is done");
    // receiver
}

async fn establish_secure_connection(
    my_id: GnomeId,
    mut stream: TcpStream,
    sender: Sender<Subscription>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
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
        return;
    }
    // let recv_result = stream.recv_from(&mut recv_buf).await;
    // if recv_result.is_ok() {
    // (count, remote_addr) = recv_result.unwrap();
    eprintln!("Got {} bytes back", count);
    // } else {
    //     return;
    // }

    // let mut decoded_key: Option<[u8; 32]> = None;
    // println!("Dec key: {:?}", decoded_key);
    // if let Ok((count, remote_adr)) = recv_result {
    //     remote_addr = remote_adr;
    // eprintln!("Received {} bytes 1", count);
    let decoded_key = decrypter.decrypt(&recv_buf[..count]);

    if let Ok(sym_key) = decoded_key {
        // eprintln!("Got session key: {:?}", sym_key);
        session_key = SessionKey::from_key(&sym_key.try_into().unwrap());
    } else {
        eprintln!("Unable to decode key");
        // return resp_receiver;
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
        // eprintln!("Received {} bytes 2", count);
        let decr_res = session_key.decrypt(&recv_buf[..count]);
        if let Ok(remote_pubkey_pem) = decr_res {
            let remote_id_pub_key_pem =
                std::str::from_utf8(&remote_pubkey_pem).unwrap().to_string();
            spawn(prepare_and_serve(
                my_id,
                stream,
                // remote_gnome_id,
                session_key,
                swarm_names,
                sender.clone(),
                pipes_sender.clone(),
                // encr,
                remote_id_pub_key_pem,
            ));
        } else {
            eprintln!("Failed to decrypt message");
            // return;
        }
    } else {
        eprintln!("Failed to receive data from remote");
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
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
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
        return;
    } else {
        eprintln!("TCP Client Remote names from bytes: {:?}", remote_names);
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
        return;
    }
    eprintln!("TCP Common swarm names: {:?}", common_names);
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
    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = pipes_sender.send((token_send, token_recv_two));
    serve_socket(
        session_key,
        stream,
        ch_pairs,
        token_send_two,
        token_recv,
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
    let mut recv_buf = [0u8; 1024];
    *remote_names = if let Ok(count) = reader.read(&mut recv_buf).await {
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
