use super::tcp_common::serve_socket;
use super::Token;
use crate::crypto::Encrypter;
use crate::crypto::{generate_symmetric_key, SessionKey};
// use crate::networking::common::collect_subscribed_swarm_names;
use crate::networking::common::create_a_neighbor_for_each_swarm;
use crate::networking::common::distil_common_names;
use crate::networking::subscription::Subscription;
use async_std::io::{ReadExt, WriteExt};
use async_std::net::{TcpListener, TcpStream};
use async_std::stream::StreamExt;
use async_std::task::spawn;
// use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeId;
use swarm_consensus::SwarmName;

pub async fn run_tcp_server(
    listener: TcpListener,
    sub_sender: Sender<Subscription>,
    // mut sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    pub_key_pem: String,
    swarm_names: Vec<SwarmName>,
) {
    let mut incoming = listener.incoming();
    eprintln!("Server listening for TCP connections…");
    while let Some(stream) = incoming.next().await {
        if let Ok(stream) = stream {
            // TODO: sub_receiver
            // sub_receiver =
            //     collect_subscribed_swarm_names(&mut swarm_names, sub_sender.clone(), sub_receiver)
            //         .await;
            spawn(serve_dedicated_connection(
                stream,
                pub_key_pem.clone(),
                sub_sender.clone(),
                token_pipes_sender.clone(),
                swarm_names.clone(),
            ));
        }
    }
}

async fn serve_dedicated_connection(
    stream: TcpStream,
    pub_key_pem: String,
    sub_sender: Sender<Subscription>,
    // mut sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    swarm_names: Vec<SwarmName>,
) {
    let (mut reader, mut writer) = (stream.clone(), stream);
    eprintln!("Got reader: {:?} and writer: {:?}", reader, writer);
    // TODO: below is a copy from UDP, needs a rewrite for TcpStreams
    // since we are already connected it can be compressed into one fn and spawned
    let mut remote_gnome_id: GnomeId = GnomeId(0);
    let mut session_key: SessionKey = SessionKey::from_key(&[0; 32]);
    let optional_sock = establish_secure_connection(
        // host_ip,
        &mut reader,
        &mut writer,
        &mut remote_gnome_id,
        &mut session_key,
        &pub_key_pem,
    )
    .await;
    if optional_sock.is_none() {
        // println!("Failed to establish secure connection with Neighbor");
        return;
    }
    let remote_pub_key_pem = optional_sock.unwrap();

    let swarm_names = vec![SwarmName::new(GnomeId::any(), "/".to_string()).unwrap()];

    // swarm_names.sort();
    spawn(prepare_and_serve(
        reader,
        writer,
        remote_gnome_id,
        session_key,
        sub_sender.clone(),
        swarm_names,
        token_pipes_sender.clone(),
        // encrypter,
        remote_pub_key_pem,
    ));
    eprintln!("--------------------------------------");
}

async fn establish_secure_connection(
    // host_ip: IpAddr,
    reader: &mut TcpStream,
    writer: &mut TcpStream,
    remote_gnome_id: &mut GnomeId,
    session_key: &mut SessionKey,
    pub_key_pem: &str,
    // ) -> Option<(UdpSocket, Encrypter)> {
) -> Option<String> {
    let mut bytes = [0u8; 1100];
    // loop {
    let result = reader.read(&mut bytes).await;
    if result.is_err() {
        eprintln!("Failed to receive data on TCP stream: {:?}", result);
        return None;
    }
    let count = result.unwrap();
    // if count > 1 {
    //     break;
    // }
    // }
    let id_pub_key_pem = std::str::from_utf8(&bytes[..count]).unwrap();
    if id_pub_key_pem == pub_key_pem {
        return None;
    }
    eprintln!("TCP stream received {} bytes", count);
    let result = Encrypter::create_from_data(id_pub_key_pem);
    if result.is_err() {
        eprintln!("Failed to build Encripter from received PEM: {:?}", result);
        return None;
    }
    let encr = result.unwrap();

    // let dedicated_socket = UdpSocket::bind(SocketAddr::new(socket.local_addr().unwrap().ip(), 0))
    //     .await
    //     .unwrap();
    // let dedicated_port = dedicated_socket.local_addr().unwrap().port();
    // let mut bytes_to_send = Vec::from(dedicated_port.to_be_bytes());
    let key = generate_symmetric_key();
    // bytes_to_send.append(&mut Vec::from(key));
    // TODO maybe send remote external IP here?
    let bytes_to_send = Vec::from(&key);
    let encr_res = encr.encrypt(&bytes_to_send);
    if encr_res.is_err() {
        eprintln!("Failed to encrypt symmetric key: {:?}", encr_res);
        return None;
    }
    eprintln!("Encrypted symmetric key");

    let encrypted_data = encr_res.unwrap();
    // let res = socket.send_to(&encrypted_data, remote_addr).await;
    let res = writer.write(&encrypted_data).await;
    // let res = dedicated_socket.send_to(&encrypted_data, remote_addr).await;
    if res.is_err() {
        eprintln!("Failed to send encrypted symmetric key: {:?}", res);
        return None;
    }
    eprintln!("Sent encrypted symmetric key {}", encrypted_data.len());

    *session_key = SessionKey::from_key(&key);

    // let mut r_buf = [0u8; 32];
    // // let r_res = dedicated_socket.recv_from(&mut r_buf).await;
    // let r_res = reader.read(&mut r_buf).await;
    // if r_res.is_err() {
    //     eprintln!("Failed to receive ping from Neighbor");
    //     return None;
    // }
    // let _count = r_res.unwrap();
    // let conn_result = dedicated_socket.connect(remote_addr).await;
    // if conn_result.is_err() {
    //     eprintln!("Unable to connect dedicated socket: {:?}", conn_result);
    //     return None;
    // }

    let my_encrypted_pubkey = session_key.encrypt(pub_key_pem.as_bytes());
    // let res2 = dedicated_socket.send(&my_encrypted_pubkey).await;
    let res2 = writer.write(&my_encrypted_pubkey).await;
    if res2.is_err() {
        eprintln!("Error sending encrypted pubkey response: {:?}", res2);
        return None;
    }
    eprintln!("Sent encrypted public key");

    *remote_gnome_id = GnomeId(encr.hash());
    eprintln!("Remote GnomeId: {}", remote_gnome_id);
    Some(id_pub_key_pem.to_string())
}

async fn prepare_and_serve(
    mut reader: TcpStream,
    mut writer: TcpStream,
    remote_gnome_id: GnomeId,
    session_key: SessionKey,
    sub_sender: Sender<Subscription>,
    swarm_names: Vec<SwarmName>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // encrypter: Encrypter,
    pub_key_pem: String,
) {
    eprintln!("Waiting for data from remote...");
    let mut remote_names = vec![];
    receive_remote_swarm_names(&mut reader, &mut remote_names).await;

    let mut common_names = vec![];

    send_subscribed_swarm_names(&mut writer, &swarm_names).await;
    distil_common_names(&mut common_names, swarm_names, &remote_names);

    eprintln!("Common names: {:?}", common_names);
    let (shared_sender, swarm_extend_receiver) = channel();
    let mut ch_pairs = vec![];
    create_a_neighbor_for_each_swarm(
        common_names,
        remote_names,
        sub_sender.clone(),
        remote_gnome_id,
        &mut ch_pairs,
        shared_sender.clone(),
        // encrypter,
        pub_key_pem,
    );

    let (token_send, token_recv) = channel();
    let (token_send_two, token_recv_two) = channel();
    let _ = token_pipes_sender.send((token_send, token_recv_two));
    //TODO: serve connection
    eprintln!("Now it's time for work!");
    serve_socket(
        session_key,
        reader,
        ch_pairs,
        token_send_two,
        token_recv,
        sub_sender,
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
        recv_buf[..count]
            // TODO split by some reasonable delimiter
            .split(|n| n == &255u8)
            .map(|bts| SwarmName::from(bts.to_vec()).unwrap())
            .collect()
    } else {
        Vec::new()
    };
}
pub async fn send_subscribed_swarm_names(
    socket: &mut TcpStream,
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
    let send_result = socket.write(&buf).await;
    if let Ok(count) = send_result {
        eprintln!("SKT Sent {} bytes", count);
    }
}