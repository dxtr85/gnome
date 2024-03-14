use async_std::net::UdpSocket;
use async_std::task::sleep;
use bytes::{BufMut, Bytes, BytesMut};
use core::panic;
use futures::channel::mpsc;
use futures::join;
use futures::StreamExt;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::error::Error;
use std::fmt;

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
pub async fn run_networking_tasks(
    host_ip: IpAddr,
    broadcast_ip: IpAddr,
    server_port: u16,
    sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    let server_addr: SocketAddr = SocketAddr::new(host_ip, server_port);
    let bind_result = UdpSocket::bind(server_addr).await;
    if let Ok(socket) = bind_result {
        run_server(host_ip, socket, sender).await;
    } else {
        let futu_one = establish_connections_to_lan_servers(
            host_ip,
            broadcast_ip,
            server_port,
            sender.clone(),
        );

        let futu_two = async {
            let socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
                .await
                .expect("SKT couldn't bind to address");
            run_client(server_addr, socket, sender).await;
        };
        join!(futu_one, futu_two);
    };
}

async fn run_server(
    host_ip: IpAddr,
    socket: UdpSocket,
    mut sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    println!("--------------------------------------");
    println!("- - - - - - - - SERVER - - - - - - - -");
    println!("- Listens on: {:?}   -", socket.local_addr().unwrap());
    println!("--------------------------------------");
    let mut buf = BytesMut::zeroed(128);
    let mut bytes = buf.split();
    loop {
        let result = socket.recv_from(&mut bytes).await;
        if let Ok((count, _source)) = result {
            print!("SKT Received {} bytes: ", count);
            let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
            println!("{}", recv_str);
            let remote_addr: SocketAddr = recv_str.parse().unwrap();
            let dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0)).await.unwrap();
            buf.put(
                dedicated_socket
                    .local_addr()
                    .unwrap()
                    .to_string()
                    .as_bytes(),
            );
            let bytes_to_send = buf.split();
            let send_result = socket.send_to(&bytes_to_send, remote_addr).await;
            if let Ok(count) = send_result {
                println!("SKT Sent {} bytes: {:?}", count, bytes_to_send);
                let conn_result = dedicated_socket.connect(remote_addr).await;
                if let Ok(()) = conn_result {
                    println!("SKT Connected to client");
                    let (s1, r1) = mpsc::channel(16);
                    let (s2, r2) = mpsc::channel(16);
                    let send_result = sender.try_send((s1, r2));
                    // println!("send result: {:?}", send_result);
                    if send_result.is_ok() {
                        serve_socket(dedicated_socket, s2, r1).await;
                    }
                    println!("--------------------------------------");
                }
            }
        }
    }
}

async fn run_client(
    server_addr: SocketAddr,
    socket: UdpSocket,
    mut sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    println!("SKT CLIENT");
    // let (mut sender, mut receiver) = sender;
    let mut buf = BytesMut::new();
    let local_addr = socket.local_addr().unwrap().to_string();
    buf.put(local_addr.as_bytes());
    let mut bytes = buf.split();
    let send_result = socket.send_to(&bytes, server_addr).await;
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes: {}", count, local_addr);
    }
    let recv_result = socket.recv_from(&mut bytes).await;
    if let Ok((count, _source)) = recv_result {
        let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
        println!("SKT Received {} bytes: {}", count, recv_str);
        let conn_result = socket
            .connect(recv_str.parse::<SocketAddr>().unwrap())
            .await;
        if conn_result.is_ok() {
            println!("SKT Connected to server");
            // TODO: change following with new sender
            let (s1, r1) = mpsc::channel(16);
            let (s2, r2) = mpsc::channel(16);
            let send_result = sender.try_send((s1, r2));
            // println!("send result: {:?}", send_result);
            if send_result.is_ok() {
                serve_socket(socket, s2, r1).await;
            }
        } else {
            println!("SKT Failed to connect");
        }
    } else {
        println!("SKT recv result: {:?}", recv_result);
    }
    println!("SKT run_client complete");
}

async fn establish_connections_to_lan_servers(
    host_ip: IpAddr,
    broadcast_ip: IpAddr,
    server_port: u16,
    sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
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
    let mut buf = BytesMut::new();
    let local_addr = socket.local_addr().unwrap().to_string();
    println!("SKT sending broadcast {} to {:?}", local_addr, broadcast_ip);
    buf.put(local_addr.as_bytes());
    socket
        .send_to(&buf, SocketAddr::new(broadcast_ip, server_port))
        .await
        .expect("Unable to send broadcast packet");

    println!("SKT Listening for responses from LAN servers");
    while let Ok((count, _server_addr)) = socket.recv_from(&mut buf).await {
        let dedicated_socket = UdpSocket::bind(SocketAddr::new(host_ip, 0))
            .await
            .expect("SKT couldn't bind to address");
        let recv_str = String::from_utf8(Vec::from(&buf[..count])).unwrap();
        let server_addr: SocketAddr = recv_str.parse().expect("Received incorrect socket addr");
        run_client(server_addr, dedicated_socket, sender.clone()).await;
    }
}

async fn read_bytes_from_socket(socket: &UdpSocket) -> Result<Bytes, ConnError> {
    let mut buf = BytesMut::zeroed(1024);
    let recv_result = socket.peek(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        println!("<<<<< {:?}", String::from_utf8_lossy(&buf[..count]));
        Ok(Bytes::from(Vec::from(&buf[..count])))
    } else {
        println!("SKT {:?}", recv_result);
        Err(ConnError::Disconnected)
    }
}

async fn read_bytes_from_local_stream(
    receiver: &mut mpsc::Receiver<Bytes>,
) -> Result<Bytes, ConnError> {
    let next_option = receiver.next().await;
    if let Some(bytes) = next_option {
        println!(">>>>> {:?}", bytes);
        Ok(bytes)
    } else {
        Err(ConnError::LocalStreamClosed)
    }
}

async fn race_tasks(
    socket: &UdpSocket,
    sender: &mut mpsc::Sender<Bytes>,
    receiver: &mut mpsc::Receiver<Bytes>,
) {
    let mut buf = BytesMut::zeroed(1024);
    let t1 = read_bytes_from_socket(socket).fuse();
    let t2 = read_bytes_from_local_stream(receiver).fuse();

    pin_mut!(t1, t2);

    let (from_socket, result) = select! {
        result1 = t1 =>  (true, result1),
        result2 = t2 => (false, result2),
    };
    if let Err(err) = result {
        println!("SRCV Error received: {:?}", err);
        // TODO: should end serving this socket
    } else if let Ok(bytes) = result {
        if from_socket {
            let read_result = socket.recv(&mut buf).await;
            if let Ok(count) = read_result {
                if bytes.len() == count {
                    let _send_result = sender.try_send(bytes);
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

async fn serve_socket(
    socket: UdpSocket,
    mut sender: mpsc::Sender<Bytes>,
    mut receiver: mpsc::Receiver<Bytes>,
) {
    println!("SRCV Racing tasks");
    race_tasks(&socket, &mut sender, &mut receiver).await;
}
