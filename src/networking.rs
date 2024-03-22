use async_std::net::UdpSocket;
use async_std::task::{self, sleep, spawn, yield_now};
use bytes::{BufMut, Bytes, BytesMut};
use core::panic;
// use futures::join;
use std::collections::HashMap;
// use futures::StreamExt;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::error::Error;
use std::fmt;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{GnomeId, Message, Neighbor, Request, SwarmTime};

#[derive(Debug)]
enum ConnError {
    Disconnected,
    LocalStreamClosed,
}
impl fmt::Display for ConnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnError::Disconnected => write!(f, "ConnError: Disconnected"),
            ConnError::LocalStreamClosed => write!(f, "ConnError: LocalStreamClosed"),
        }
    }
}

impl Error for ConnError {}

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::data_conversion::bytes_to_message;
use crate::data_conversion::message_to_bytes;
pub async fn run_networking_tasks(
    host_ip: IpAddr,
    _broadcast_ip: IpAddr,
    server_port: u16,
    // _sender: Sender<(Sender<Message>, Receiver<Message>)>,
    notification_receiver: Receiver<(String, Sender<Request>)>,
) {
    let server_addr: SocketAddr = SocketAddr::new(host_ip, server_port);
    let bind_result = UdpSocket::bind(server_addr).await;
    let (sub_send_one, sub_recv_one) = channel();
    let (sub_send_two, sub_recv_two) = channel();
    spawn(subscriber(
        sub_send_one,
        sub_recv_two,
        notification_receiver,
    ));
    if let Ok(socket) = bind_result {
        run_server(host_ip, socket, sub_send_two, sub_recv_one).await;
    } else {
        // if let Ok((swarm_name, req_sender)) = subscription_receiver.try_recv() {
        //     let futu_one = establish_connections_to_lan_servers(
        //         swarm_name.clone(),
        //         host_ip,
        //         broadcast_ip,
        //         server_port,
        //         sender.clone(),
        //         req_sender.clone(),
        //     );

        //     let futu_two = async {
        let socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
            .await
            .expect("SKT couldn't bind to address");
        run_client(server_addr, socket, sub_recv_one, sub_send_two).await;
        //     };
        //     join!(futu_one, futu_two);
        // }
    };
}

async fn run_server(
    host_ip: IpAddr,
    socket: UdpSocket,
    // sender: Sender<(Sender<Message>, Receiver<Message>)>,
    // subscription_receiver: Receiver<(String, Sender<Request>)>,
    sub_sender: Sender<Subscription>,
    sub_receiver: Receiver<Subscription>,
) {
    println!("--------------------------------------");
    println!("- - - - - - - - SERVER - - - - - - - -");
    println!("- Listens on: {:?}   -", socket.local_addr().unwrap());
    println!("--------------------------------------");
    let mut buf = BytesMut::zeroed(128);
    let mut bytes = buf.split();
    // let mut swarms: HashMap<String, Sender<Request>> = HashMap::with_capacity(10);
    loop {
        // println!("loopa");
        // if let Ok((swarm_name, sender)) = subscription_receiver.try_recv() {
        // swarms.insert(swarm_name, sender);
        // TODO: inform existing sockets about new subscription
        // TODO: sockets should be able to respond if they want to join
        // }
        let result = socket.recv_from(&mut bytes).await;
        if let Ok((count, remote_addr)) = result {
            print!("SKT Received {} bytes: ", count);
            // TODO: recv_str should contain a list of swarms that remote gnome
            //       wants to join
            let names: &Vec<String> = &bytes[..count]
                .split(|n| n == &255u8)
                .map(|bts| String::from_utf8(bts.to_vec()).unwrap())
                .collect();
            // let bytes_to_names = &bytes[..count].split(|n| n == &0u8).collect();
            // let mut names = vec![];
            // for bts in bytes_to_names {
            //     let name = String::from_utf8(bts).unwrap();
            //     names.push(name);
            // }
            let mut common_names = vec![];
            // let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
            // println!("his: {:?}", names);
            // let remote_addr: SocketAddr = recv_str.parse().unwrap();
            let dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await.unwrap();
            // TODO: send swarm names we subscribe and match remote interests
            let _ = sub_sender.send(Subscription::ProvideList);
            let recv_result = sub_receiver.recv();
            let swarm_names = match recv_result {
                Ok(Subscription::Added(name)) => vec![name],
                Ok(Subscription::List(names)) => names,
                Ok(_) => vec![],
                Err(e) => {
                    println!("Error: {:?}", e);
                    vec![]
                }
            };
            // println!("my: {:?}", swarm_names);
            // if let Ok(Subscription::List(swarms)) = sub_receiver.recv() {
            // } else {
            //     println!("did not recv subscription list");
            // }
            buf.put(
                dedicated_socket
                    .local_addr()
                    .unwrap()
                    .to_string()
                    .as_bytes(),
            );
            let bytes_to_send = buf.split();
            // println!("sending: {:?}", bytes_to_send);
            let send_result = socket.send_to(&bytes_to_send, remote_addr).await;
            if let Ok(count) = send_result {
                println!("SKT Sent {} bytes: {:?}", count, bytes_to_send);
                let conn_result = dedicated_socket.connect(remote_addr).await;
                if let Ok(()) = conn_result {
                    println!("SKT Connected to client");
                    for swarm_name in swarm_names {
                        if names.contains(&swarm_name) {
                            common_names.push(swarm_name.clone());
                            buf.put(swarm_name.as_bytes());
                            buf.put_u8(255);
                        }
                    }
                    let to_send = buf.split();
                    let _ = dedicated_socket.send(&to_send).await;
                    // TODO: for each element we sent create a Neighbor
                    // and send it down the pipe
                    // let (s1, r1) = channel();
                    // let (s2, r2) = channel();
                    // TODO: put s1 & r2 into neighbor instead of following
                    // let send_result = sender.send((s1, r2));
                    // TODO: read subscribed swarm names from socket
                    // for each matching sname create a neighbor and
                    // send it to service via sender from swarms hashmap
                    // rewrite serve socket and message exchange to
                    // include agreed upon preamble informing of
                    // swarm that is supposed to receive given message

                    // TODO: how to update a socket about changed list of
                    // subscribed swarms? (Probably with channels ;)
                    // TODO: same as above goes to run_client
                    let mut num = 1;
                    let mut ch_pairs = vec![];
                    for name in common_names {
                        let (s1, r1) = channel();
                        let (s2, r2) = channel();
                        let neighbor = Neighbor::from_id_channel_time(
                            GnomeId(num),
                            r2,
                            s1,
                            SwarmTime(0),
                            SwarmTime(7),
                        );
                        let _ = sub_sender.send(Subscription::IncludeNeighbor(name, neighbor));
                        num += 1;
                        ch_pairs.push((s2, r1));
                    }
                    if send_result.is_err() {
                        println!("send result: {:?}", send_result);
                    }
                    if send_result.is_ok() {
                        spawn(serve_socket(dedicated_socket, ch_pairs));
                        // spawn(serve_socket(dedicated_socket, networking_receiver));
                    }
                    println!("--------------------------------------");
                }
            }
        }
    }
}

#[derive(Debug)]
enum Subscription {
    Added(String),
    Removed(String),
    ProvideList,
    List(Vec<String>),
    IncludeNeighbor(String, Neighbor),
}

async fn subscriber(
    sub_sender: Sender<Subscription>,
    sub_receiver: Receiver<Subscription>,
    notification_receiver: Receiver<(String, Sender<Request>)>,
) {
    let mut swarms: HashMap<String, Sender<Request>> = HashMap::with_capacity(10);
    let mut names: Vec<String> = Vec::with_capacity(10);
    println!("Subscriber service started");
    loop {
        let recv_result = notification_receiver.try_recv();
        match recv_result {
            Ok((swarm_name, sender)) => {
                swarms.insert(swarm_name.clone(), sender);
                names.push(swarm_name.clone());
                // TODO: inform existing sockets about new subscription
                println!("Added swarm: {}", swarm_name);
                let _ = sub_sender.send(Subscription::Added(swarm_name));
                // TODO: sockets should be able to respond if they want to join
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                println!("subscriber disconnected from Manager");
                break;
            }
            Err(_) => {}
        }
        if let Ok(sub) = sub_receiver.try_recv() {
            match sub {
                Subscription::IncludeNeighbor(swarm, neighbor) => {
                    if let Some(sender) = swarms.get(&swarm) {
                        let _ = sender.send(Request::AddNeighbor(neighbor));
                    } else {
                        println!("No sender for {} found", swarm);
                    }
                }
                Subscription::ProvideList => {
                    println!("sub sending: {:?}", names);
                    let _ = sub_sender.send(Subscription::List(names.clone()));
                }
                other => {
                    println!("Unexpected msg: {:?}", other)
                }
            }
        }
        yield_now();
    }
}

async fn run_client(
    // _swarm_name: String,
    server_addr: SocketAddr,
    socket: UdpSocket,
    receiver: Receiver<Subscription>,
    sender: Sender<Subscription>,
) {
    // TODO: we should be able to use given socket with multiple swarms
    // when subscriptions change

    // if let Ok((swarm_name, sender)) = subscription_receiver.recv() {
    //     swarms.insert(swarm_name, sender);
    // }
    println!("SKT CLIENT");
    let mut buf = BytesMut::with_capacity(128);
    // let local_addr = socket.local_addr().unwrap().to_string();
    // TODO: send a list of swarms we are subscribed to
    let mut names = vec![];
    loop {
        let _ = sender.send(Subscription::ProvideList);
        let _recv_result = receiver.recv();
        println!("res: {:?}", _recv_result);
        let recv_result = receiver.recv();
        println!("res: {:?}", recv_result);
        match recv_result {
            Ok(Subscription::Added(ref name)) => names = vec![name.to_owned()],
            Ok(Subscription::List(ref nnames)) => names = nnames.to_owned(),
            Ok(_) => names = vec![],
            Err(e) => {
                println!("Error: {:?}", e);
                // vec![]
            }
        };
        if names.len() > 0 {
            break;
        }
        yield_now();
    }
    // let names = if let Ok(Subscription::List(swarms)) = recv_result {
    //     println!("Sub list: {:?}", swarms);
    //     swarms
    // } else if let Ok(Subscription::Added(swarm_name)) = recv_result {
    //     // println!("Nothing received from sub service! {:?}", recv_result);
    //     vec![swarm_name]
    // };
    for name in &names {
        buf.put(name.as_bytes());
        buf.put_u8(255);
    }
    // buf.put(local_addr.as_bytes());
    let bytes = buf.split();
    println!("After split: {:?}", bytes);
    let send_result = socket.send_to(&bytes, server_addr).await;
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes", count);
    }
    let mut recv_buf = BytesMut::zeroed(1024);
    let recv_result = socket.recv_from(&mut recv_buf).await;
    //TODO: compare received list of swarms with swarm_name and if matches continue
    if let Ok((count, _remote_addr)) = recv_result {
        // TODO: in future compare with list of swarms we are subscribed to
        let recv_str = String::from_utf8(Vec::from(&recv_buf[..count])).unwrap();

        println!("SKT Received {} bytes: {:?}", count, recv_str);
        let conn_result = socket.connect(recv_str).await;
        if conn_result.is_ok() {
            println!("SKT Connected to server");
            let remote_names: Vec<String> =
                if let Ok((count, _from)) = socket.recv_from(&mut recv_buf).await {
                    recv_buf[..count]
                        .split(|n| n == &255u8)
                        .map(|bts| String::from_utf8(bts.to_vec()).unwrap())
                        .collect()
                } else {
                    Vec::new()
                };
            let mut common_names = vec![];
            for name in remote_names {
                // let name = String::from_utf8(bts).unwrap();
                if names.contains(&name) {
                    common_names.push(name.to_owned());
                }
            }
            let mut ch_pairs = vec![];
            println!("komon names: {:?}", common_names);
            for name in common_names {
                let (s1, r1) = channel();
                let (s2, r2) = channel();
                let neighbor =
                    Neighbor::from_id_channel_time(GnomeId(1), r2, s1, SwarmTime(0), SwarmTime(7));
                let _ = sender.send(Subscription::IncludeNeighbor(name, neighbor));
                ch_pairs.push((s2, r1));
            }
            spawn(serve_socket(socket, ch_pairs)).await;
        } else {
            println!("SKT Failed to connect");
        }
    } else {
        println!("SKT recv result: {:?}", recv_result);
    }
    println!("SKT run_client complete");
}

async fn establish_connections_to_lan_servers(
    swarm_name: String,
    host_ip: IpAddr,
    broadcast_ip: IpAddr,
    server_port: u16,
    _sender: Sender<(Sender<Message>, Receiver<Message>)>,
    _request_sender: Sender<Request>,
) {
    //TODO: extend to send a broadcast dgram to local network
    // and create a dedicated connection for each response
    sleep(Duration::from_millis(10)).await;
    let socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
        .await
        .expect("Unable to bind socket");
    println!("Socket bind");
    socket
        .set_broadcast(true)
        .expect("Unable to set broadcast on socket");
    let mut buf = BytesMut::from(swarm_name.clone().as_bytes());
    let local_addr = socket.local_addr().unwrap().to_string();
    println!("SKT sending broadcast {} to {:?}", local_addr, broadcast_ip);
    buf.put(local_addr.as_bytes());
    socket
        .send_to(&buf, SocketAddr::new(broadcast_ip, server_port))
        .await
        .expect("Unable to send broadcast packet");

    println!("SKT Listening for responses from LAN servers");
    while let Ok((count, _server_addr)) = socket.recv_from(&mut buf).await {
        let _dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
            .await
            .expect("SKT couldn't bind to address");
        let recv_str = String::from_utf8(Vec::from(&buf[..count])).unwrap();
        let _server_addr: SocketAddr = recv_str.parse().expect("Received incorrect socket addr");
        // TODO: figure this out later
        // run_client(
        //     // swarm_name.clone(),
        //     server_addr,
        //     dedicated_socket,
        //     // sender.clone(),
        //     request_sender.clone(),
        // )
        // .await;
    }
}

async fn read_bytes_from_socket(socket: &UdpSocket) -> Result<(u8, Bytes), ConnError> {
    // println!("read_bytes_from_socket");
    let mut buf = BytesMut::zeroed(1024);
    let recv_result = socket.peek(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        // println!("<<<<< {:?}", String::from_utf8_lossy(&buf[..count]));
        if buf[0] & 0b1_111_0000 == 240 {
            Ok((buf[1], Bytes::from(Vec::from(&buf[2..count - 2]))))
        } else {
            Ok((0, Bytes::from(Vec::from(&buf[..count]))))
        }
    } else {
        println!("SKTd{:?}", recv_result);
        Err(ConnError::Disconnected)
    }
}

async fn read_bytes_from_local_stream(
    receivers: &mut HashMap<u8, Receiver<Message>>,
) -> Result<(u8, Bytes), ConnError> {
    // println!("read_bytes_from_local_stream");
    loop {
        for (id, receiver) in receivers.iter_mut() {
            let next_option = receiver.try_recv();
            if let Ok(message) = next_option {
                let bytes = message_to_bytes(message);
                // println!(">>>>> {:?}", bytes);

                if *id == 0 {
                    return Ok((*id, bytes));
                } else {
                    let mut extended_bytes = BytesMut::with_capacity(bytes.len() + 2);
                    extended_bytes.put_u8(240);
                    extended_bytes.put_u8(*id);
                    extended_bytes.put(bytes);
                    return Ok((*id, Bytes::from(extended_bytes)));
                }
            }
        }
        task::yield_now().await;
    }
    // Err(ConnError::LocalStreamClosed)
}

async fn race_tasks(
    socket: UdpSocket,
    mut send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    // sender: Sender<Message>,
    // mut receiver: Receiver<Message>,
    // , subscription_receiver: Receiver<(bool, String, Option<(Sender<Message>, Receiver<Message>)>),
) {
    let mut senders: HashMap<u8, Sender<Message>> = HashMap::new();
    let mut receivers: HashMap<u8, Receiver<Message>> = HashMap::new();
    // println!("racing: {:?}", send_recv_pairs);
    for (i, (sender, receiver)) in send_recv_pairs.into_iter().enumerate() {
        senders.insert(i as u8, sender);
        receivers.insert(i as u8, receiver);
    }
    let mut buf = BytesMut::zeroed(1024);
    // if let Some((sender, mut receiver)) = send_recv_pairs.pop() {
    loop {
        let t1 = read_bytes_from_socket(&socket).fuse();
        // TODO: serv pairs of sender-receiver
        let t2 = read_bytes_from_local_stream(&mut receivers).fuse();

        pin_mut!(t1, t2);

        let (from_socket, result) = select! {
            result1 = t1 => {print!(""); (true, result1)},
            result2 = t2 => {print!("");(false, result2)},
        };
        if let Err(_err) = result {
            println!("SRVC Error received: {:?}", _err);
            // TODO: should end serving this socket
            for sender in senders.values() {
                let _send_result = sender.send(Message::bye());
            }
            break;
        } else if let Ok((id, bytes)) = result {
            if from_socket {
                // println!("From soket");
                let read_result = socket.recv(&mut buf).await;
                if let Ok(count) = read_result {
                    // println!("Read {} bytes", count);
                    if bytes.len() <= count {
                        // println!("Count match");
                        if count > 0 {
                            if let Ok(message) = bytes_to_message(&bytes) {
                                // println!("decode OK");
                                if let Some(sender) = senders.get(&id) {
                                    // if message.is_bye() {
                                    //     println!("Sending: {:?}", message);
                                    // }
                                    let _send_result = sender.send(message);
                                    if _send_result.is_err() {
                                        println!("{:?} send result2: {:?}", message, _send_result);
                                    }
                                } else {
                                    println!("Did not find sender for {}", id);
                                }
                            } else {
                                println!("Failed to decode incoming stream");
                            }
                        }
                    } else {
                        println!("SRCV Peeked != recv");
                    }
                } else {
                    println!("SRCV Unable to recv supposedly ready data");
                }
            } else {
                let _send_result = socket.send(&bytes).await;
            }
        }
    }
    // }
}

async fn serve_socket(
    socket: UdpSocket,
    send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    // subscription_receiver: Receiver<(bool, String, Option<(Sender<Message>, Receiver<Message>)>)
) {
    println!("SRVC Racing tasks");
    race_tasks(socket, send_recv_pairs).await;
    println!("SRVC Racing tasks over");
}
