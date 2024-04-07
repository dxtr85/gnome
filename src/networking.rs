use async_std::net::UdpSocket;
use async_std::task::{self, sleep, spawn, yield_now};
use bytes::{BufMut, Bytes, BytesMut};
use core::panic;
// use futures::join;
use std::collections::{HashMap, VecDeque};
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
    gnome_id: GnomeId,
    host_ip: IpAddr,
    _broadcast_ip: IpAddr,
    server_port: u16,
    buffer_size_bytes: u32,
    uplink_bandwith_bytes_sec: u32,
    notification_receiver: Receiver<(String, Sender<Request>, Sender<u32>)>,
) {
    let server_addr: SocketAddr = SocketAddr::new(host_ip, server_port);
    let bind_result = UdpSocket::bind(server_addr).await;
    let (sub_send_one, sub_recv_one) = channel();
    let (sub_send_two, sub_recv_two) = channel();
    let (token_dispenser_send, token_dispenser_recv) = channel();
    spawn(subscriber(
        sub_send_one,
        sub_recv_two,
        notification_receiver,
        token_dispenser_send,
    ));
    let (token_pipes_sender, token_pipes_receiver) = channel();
    // let (token_msg_sender_two, token_msg_receiver_two) = channel();
    spawn(token_dispenser(
        buffer_size_bytes,
        uplink_bandwith_bytes_sec,
        // token_msg_sender,
        token_pipes_receiver,
        token_dispenser_recv,
    ));

    //TODO: make use of token_msg_sender_two, token_msg_receiver

    if let Ok(socket) = bind_result {
        run_server(
            gnome_id,
            host_ip,
            socket,
            sub_send_two,
            sub_recv_one,
            token_pipes_sender,
        )
        .await;
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
        let (token_send, token_recv) = channel();
        let (token_send_two, token_recv_two) = channel();
        token_pipes_sender.send((token_send, token_recv_two));
        run_client(
            gnome_id,
            server_addr,
            socket,
            sub_recv_one,
            sub_send_two,
            token_send_two,
            token_recv,
        )
        .await;
        //     };
        //     join!(futu_one, futu_two);
        // }
    };
}

async fn run_server(
    gnome_id: GnomeId,
    host_ip: IpAddr,
    socket: UdpSocket,
    sub_sender: Sender<Subscription>,
    sub_receiver: Receiver<Subscription>,
    token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
) {
    println!("--------------------------------------");
    println!("- - - - - - - - SERVER - - - - - - - -");
    println!("- Listens on: {:?}   -", socket.local_addr().unwrap());
    println!("--------------------------------------");
    let mut buf = BytesMut::zeroed(128);
    let mut bytes = buf.split();
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
                    // Here we send our GnomeId
                    let _send_result = dedicated_socket.send(&gnome_id.0.to_be_bytes()).await;
                    println!("Send result: {:?}", _send_result);
                    let mut rbuf = BytesMut::zeroed(4);
                    let recv_result = dedicated_socket.recv(&mut rbuf).await;
                    println!("Recv result: {:?}", recv_result);
                    println!("{:?}", rbuf);
                    let mut neighbor_id = GnomeId(0);
                    if let Ok(size) = recv_result {
                        if size == 4 {
                            // TODO: fix this
                            // let num: u32 = u32::from_be(rbuf.a);
                            // println!("r3: {}", rbuf[3]);
                            // let num: u32 = (rbuf[0] as u32)
                            //     << 24 + (rbuf[1] as u32)
                            //     << 16 + (rbuf[2] as u32)
                            //     << 8 + rbuf[3];
                            let num: u32 = rbuf[3] as u32;
                            neighbor_id = GnomeId(num);
                            println!("NeighborId updated: {}", num);
                        }
                    }
                    println!("Neighbor: {}", neighbor_id);
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
                    let mut ch_pairs = vec![];
                    for name in common_names {
                        let (s1, r1) = channel();
                        let (s2, r2) = channel();
                        let neighbor = Neighbor::from_id_channel_time(
                            neighbor_id,
                            r2,
                            s1,
                            SwarmTime(0),
                            SwarmTime(7),
                        );
                        let _ = sub_sender.send(Subscription::IncludeNeighbor(name, neighbor));
                        ch_pairs.push((s2, r1));
                    }
                    if send_result.is_err() {
                        println!("send result: {:?}", send_result);
                    }
                    if send_result.is_ok() {
                        let (token_send, token_recv) = channel();
                        let (token_send_two, token_recv_two) = channel();
                        token_pipes_sender.send((token_send, token_recv_two));
                        spawn(serve_socket(
                            dedicated_socket,
                            ch_pairs,
                            token_send_two,
                            token_recv,
                        ));
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

#[derive(Debug)]
enum TokenMessage {
    Add((Sender<Token>, Receiver<Token>)),
    AvailBandwith(u32),
    SlowDown,
}

#[derive(Debug)]
enum Token {
    Provision(u32),
    Unused(u32),
    Request(u32),
}

async fn token_dispenser(
    buffer_size_bytes: u32,
    bandwith_bytes_sec: u32,
    // sender: Sender<TokenMessage>,
    reciever: Receiver<(Sender<Token>, Receiver<Token>)>,
    band_reciever: Receiver<Sender<u32>>,
) {
    let mut available_buffer = buffer_size_bytes;
    let bytes_per_msec: u32 = bandwith_bytes_sec / 1000;
    let mut socket_pipes: VecDeque<(Sender<Token>, Receiver<Token>, u32, bool)> = VecDeque::new();
    let mut bandwith_notification_senders: VecDeque<Sender<u32>> = VecDeque::new();
    // let dur = Duration::from_micros(1000);
    let dur = Duration::from_millis(1000);
    let (timer_sender, timer_reciever) = channel();

    async fn timer(duration: Duration, sender: Sender<()>) {
        loop {
            task::sleep(duration).await;
            let res = sender.send(());
            if res.is_err() {
                break;
            }
        }
    }
    task::spawn(timer(dur, timer_sender));

    let mut sent_avail_bandwith: u32 = bandwith_bytes_sec;
    let mut token_size = 2048;
    let min_token_size = 256;
    let min_overhead = 4;
    let max_overhead = 64;
    let mut overhead = 16;
    let mut used_bandwith_msec: u32 = 0;
    let mut used_bandwith: VecDeque<u32> = VecDeque::from(vec![0; 100]);
    let mut used_bandwith_ring: VecDeque<(bool, u32)> = VecDeque::from(vec![(false, 0); 9]);
    used_bandwith_ring.push_back((true, 0));
    let mut additional_request_received = false;

    loop {
        let mut send_tokens = false;

        while let Ok(sender) = band_reciever.try_recv() {
            let res = sender.send(sent_avail_bandwith);
            if res.is_ok() {
                bandwith_notification_senders.push_back(sender);
            }
        }
        if let Ok((s, r)) = reciever.try_recv() {
            // match tm {
            //     TokenMessage::Add((s, r)) => {
            socket_pipes.push_back((s, r, token_size, false));
            //     }
            //     TokenMessage::AvailBandwith(_) => {
            //         let _ = sender.send(TokenMessage::AvailBandwith(std::cmp::max(
            //             bandwith_bytes_sec - used_bandwith.iter().sum::<u32>(),
            //             0,
            //         )));
            //     }
            //     TokenMessage::SlowDown => {
            //         //TODO: better algo
            //         token_size = std::cmp::max(min_token_size, token_size >> 1);
            //     }
            // }
        }
        if let Ok(_) = timer_reciever.try_recv() {
            available_buffer = std::cmp::min(buffer_size_bytes, available_buffer + bytes_per_msec);
            send_tokens = true;

            if let Some((push, value)) = used_bandwith_ring.pop_front() {
                used_bandwith_ring.push_back((push, used_bandwith_msec));
                used_bandwith_msec = 0;
                if push {
                    let sum = used_bandwith_ring
                        .iter()
                        .fold(value, |acc, (_i, val)| acc + val);
                    let _ = used_bandwith.pop_front();
                    used_bandwith.push_back(sum);
                    let used_bandwith_one_sec = used_bandwith.iter().sum::<u32>();
                    let new_avail_bandwith = bandwith_bytes_sec
                        .checked_sub(used_bandwith_one_sec)
                        .unwrap_or(0);
                    if sent_avail_bandwith.abs_diff(new_avail_bandwith) * 10 > sent_avail_bandwith {
                        let how_many = bandwith_notification_senders.len();
                        for _i in [0..how_many] {
                            if let Some(sender) = bandwith_notification_senders.pop_front() {
                                let res = sender.send(new_avail_bandwith);
                                if res.is_ok() {
                                    bandwith_notification_senders.push_back(sender);
                                }
                            };
                        }
                        sent_avail_bandwith = new_avail_bandwith;
                    }
                }
            }

            // TODO: some better algo, maybe individual overhead per socket?
            if additional_request_received {
                overhead = std::cmp::min(max_overhead, overhead + 32);
            } else {
                overhead = std::cmp::max(min_overhead, overhead - 1);
            }
            additional_request_received = false;
        }

        for _i in 0..socket_pipes.len() {
            if let Some((s, r, prev_size, additional_request)) = socket_pipes.pop_front() {
                let mut token_sent = false;
                let mut broken_pipe = false;
                let mut req_size = 0;
                let mut unused_size = 0;
                let mut new_size: u32 = prev_size;
                let mut new_add_req = additional_request;
                while let Ok(request) = r.try_recv() {
                    match request {
                        Token::Request(size) => {
                            req_size += size;
                            new_add_req = true;
                            additional_request_received = true;
                        }
                        Token::Unused(size) => unused_size += size,
                        _ => (),
                    }
                }
                if req_size > 0 {
                    if available_buffer >= req_size {
                        let res = s.send(Token::Provision(req_size));
                        if res.is_err() {
                            broken_pipe = true;
                        }
                        available_buffer -= req_size;
                        used_bandwith_msec += req_size;
                        new_size += req_size;
                        token_sent = true;
                    } else {
                        let res = s.send(Token::Provision(available_buffer));
                        if res.is_err() {
                            broken_pipe = true;
                        }
                        used_bandwith_msec += available_buffer;
                        new_size += available_buffer;
                        available_buffer = 0;
                        token_sent = true;
                    }
                }
                // println!("unused: {}, req: {}", unused_size, req_size);
                if unused_size > 0 && req_size == 0 {
                    // println!("unused>0");
                    available_buffer += unused_size;
                    used_bandwith_msec.checked_sub(unused_size).unwrap_or(0);
                    // print!(
                    //     "max(min:{}, prev:{} - unused:{} + overhead:{}) ",
                    //     min_token_size, new_size, unused_size, overhead
                    // );
                    new_size = std::cmp::max(
                        min_token_size,
                        new_size.checked_sub(unused_size).unwrap_or(0) + overhead,
                    );
                    // println!("computed: {}", new_size);
                }
                if send_tokens && !token_sent {
                    // println!("sending:{}", new_size);
                    let res = s.send(Token::Provision(new_size));
                    if res.is_err() {
                        broken_pipe = true;
                    }
                    available_buffer -= new_size;
                    used_bandwith_msec += new_size;
                }
                if !broken_pipe {
                    socket_pipes.push_back((s, r, new_size, new_add_req));
                }
            }
        }
    }
}
async fn subscriber(
    sub_sender: Sender<Subscription>,
    sub_receiver: Receiver<Subscription>,
    notification_receiver: Receiver<(String, Sender<Request>, Sender<u32>)>,
    token_dispenser_send: Sender<Sender<u32>>,
) {
    let mut swarms: HashMap<String, Sender<Request>> = HashMap::with_capacity(10);
    let mut names: Vec<String> = Vec::with_capacity(10);
    println!("Subscriber service started");
    loop {
        let recv_result = notification_receiver.try_recv();
        match recv_result {
            Ok((swarm_name, sender, band_sender)) => {
                swarms.insert(swarm_name.clone(), sender);
                names.push(swarm_name.clone());
                // TODO: inform existing sockets about new subscription
                println!("Added swarm: {}", swarm_name);
                let _ = sub_sender.send(Subscription::Added(swarm_name));
                let _ = token_dispenser_send.send(band_sender);
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
        yield_now().await;
    }
}

async fn run_client(
    gnome_id: GnomeId,
    server_addr: SocketAddr,
    socket: UdpSocket,
    receiver: Receiver<Subscription>,
    sender: Sender<Subscription>,
    token_send: Sender<Token>,
    token_recv: Receiver<Token>,
    // token_pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
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
        yield_now().await;
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
            let _send_result = socket.send(&gnome_id.0.to_be_bytes()).await;
            println!("client Send result: {:?}", _send_result);
            let recv_result = socket.recv(&mut recv_buf).await;
            println!("Recv result: {:?}", recv_result);
            let mut neighbor_id = GnomeId(0);
            if let Ok(size) = recv_result {
                println!("ok");
                if size == 4 {
                    println!("size = 4");
                    // TODO: fix this
                    // let num: u32 = (buf[0] as u32)
                    //     << 24 + (buf[1] as u32)
                    //     << 16 + (buf[2] as u32)
                    //     << 8 + buf[3];
                    println!("b[3]: {}", recv_buf[3]);
                    let num: u32 = recv_buf[3] as u32;
                    neighbor_id = GnomeId(num);
                }
            }
            println!("Neighbor: {}", neighbor_id);
            let mut ch_pairs = vec![];
            // println!("komon names: {:?}", common_names);
            for name in common_names {
                let (s1, r1) = channel();
                let (s2, r2) = channel();
                let neighbor =
                    Neighbor::from_id_channel_time(neighbor_id, r2, s1, SwarmTime(0), SwarmTime(7));
                let _ = sender.send(Subscription::IncludeNeighbor(name, neighbor));
                ch_pairs.push((s2, r1));
            }
            spawn(serve_socket(socket, ch_pairs, token_send, token_recv)).await;
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
    // TODO: increase size of buffer everywhere
    let mut buf = BytesMut::zeroed(1024);
    let recv_result = socket.peek(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        // println!("<<<<< {:?}", String::from_utf8_lossy(&buf[..count]));
        // When first byte starts with 1_111, next byte identifies swarm
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
    send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
) {
    let mut senders: HashMap<u8, Sender<Message>> = HashMap::new();
    let mut receivers: HashMap<u8, Receiver<Message>> = HashMap::new();
    let min_tokens_threshold: u32 = 128;
    let mut available_tokens: u32 = min_tokens_threshold;
    // println!("racing: {:?}", send_recv_pairs);
    for (i, (sender, receiver)) in send_recv_pairs.into_iter().enumerate() {
        senders.insert(i as u8, sender);
        receivers.insert(i as u8, receiver);
    }
    let mut buf = BytesMut::zeroed(1024);
    // if let Some((sender, mut receiver)) = send_recv_pairs.pop() {
    loop {
        if let Ok(token_msg) = token_reciever.try_recv() {
            match token_msg {
                Token::Provision(count) => {
                    let _ = token_sender.send(Token::Unused(available_tokens));
                    // println!("{} unused, more tokens: {}", available_tokens, count);
                    available_tokens = count;
                }
                other => {
                    println!("Unexpected Token message: {:?}", other);
                }
            }
        }
        if available_tokens < min_tokens_threshold {
            println!("Requesting more tokens");
            let _ = token_sender.send(Token::Request(2 * min_tokens_threshold));
        }
        let t1 = read_bytes_from_socket(&socket).fuse();
        // TODO: serv pairs of sender-receiver
        let t2 = read_bytes_from_local_stream(&mut receivers).fuse();

        pin_mut!(t1, t2);

        let (from_socket, result) = select! {
            result1 = t1 =>  (true, result1),
            result2 = t2 => (false, result2),
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
                let len = 42 + bytes.len() as u32;
                if len <= available_tokens {
                    let _send_result = socket.send(&bytes).await;
                    available_tokens -= len;
                } else {
                    println!("Waiting for tokens...");
                    let _ = token_sender.send(Token::Request(2 * len));
                    let res = token_reciever.recv();
                    match res {
                        Ok(Token::Provision(amount)) => {
                            available_tokens += amount;
                            let _send_result = socket.send(&bytes).await;
                            available_tokens -= len;
                        }
                        Ok(other) => println!("Received unexpected Token: {:?}", other),
                        Err(e) => {
                            panic!("Error while waiting for Tokens: {:?}", e);
                        }
                    }
                }
            }
        }
    }
    // }
}

async fn serve_socket(
    socket: UdpSocket,
    send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
) {
    println!("SRVC Racing tasks");
    race_tasks(socket, send_recv_pairs, token_sender, token_reciever).await;
    println!("SRVC Racing tasks over");
}
