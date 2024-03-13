use async_std::net::UdpSocket;
use async_std::task::spawn;
use bytes::{BufMut, Bytes, BytesMut};
use futures::channel::mpsc;
// use futures::future::join;
use futures::join;
use futures::StreamExt;
use std::error::Error;
// use std::io::Result;
// use futures::SinkExt;
use std::fmt;
use std::net::SocketAddr;
// use std::net::UdpSocket;

use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};

#[derive(Debug)]
struct ConnectionError {
    num: u8,
}
impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ConnectionError{}", self.num)
    }
}

impl Error for ConnectionError {
    // fn source(&self) -> Option<&(dyn Error + 'static)> {
    //     Some(&self.num)
    // }
}

#[async_std::main]
async fn main() {
    let server_addr: SocketAddr = "127.0.0.1:1026".parse().unwrap();
    let bind_result = UdpSocket::bind(server_addr).await;
    let (sender, receiver) = mpsc::channel(64usize);
    spawn(run_service(receiver));
    if let Ok(socket) = bind_result {
        run_server(socket, sender).await;
    } else {
        let futu_one = establish_connections_to_lan_servers(sender.clone());
        let futu_two = async {
            let socket = UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("couldn't bind to address");
            run_client(server_addr, socket, sender).await;
        };
        join!(futu_one, futu_two);
    };
}

async fn run_server(
    socket: UdpSocket,
    mut sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    println!("----------------------------------");
    println!("- - - - - - - SERVER - - - - - - -");
    println!("----------------------------------");
    let mut buf = BytesMut::zeroed(128);
    let mut bytes = buf.split();
    loop {
        let result = socket.recv_from(&mut bytes).await;
        if let Ok((count, _source)) = result {
            print!("Received {} bytes: ", count);
            let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
            println!("{}", recv_str);
            let remote_addr: SocketAddr = recv_str.parse().unwrap();
            let dedicated_socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
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
                println!("Sent {} bytes: {:?}", count, bytes_to_send);
                let conn_result = dedicated_socket.connect(remote_addr).await;
                if let Ok(()) = conn_result {
                    println!("Connected to client");
                    let (s1, r1) = mpsc::channel(16);
                    let (s2, r2) = mpsc::channel(16);
                    let send_result = sender.try_send((s1, r2));
                    // println!("send result: {:?}", send_result);
                    if send_result.is_ok() {
                        serve_socket(dedicated_socket, s2, r1).await;
                    }
                    println!("----------------------------------");
                }
                // Now we should:
                // 1 - inform remote host about switching to new socket
                // 2 - run_client async fn for given connection
                //     that should be established on dedicated socket on server side
                // 3 - send communication channels over sender to run_service
            }
            // }
        }
    }
}

async fn run_client(
    server_addr: SocketAddr,
    socket: UdpSocket,
    mut sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    println!("CLIENT");
    // let (mut sender, mut receiver) = sender;
    let mut buf = BytesMut::new();
    let local_addr = socket.local_addr().unwrap().to_string();
    buf.put(local_addr.as_bytes());
    let mut bytes = buf.split();
    let send_result = socket.send_to(&bytes, server_addr).await;
    if let Ok(count) = send_result {
        println!("Sent {} bytes: {}", count, local_addr);
    }
    let recv_result = socket.recv_from(&mut bytes).await;
    if let Ok((count, _source)) = recv_result {
        let recv_str = String::from_utf8(Vec::from(&bytes[..count])).unwrap();
        println!("Received {} bytes: {}", count, recv_str);
        let conn_result = socket
            .connect(recv_str.parse::<SocketAddr>().unwrap())
            .await;
        if conn_result.is_ok() {
            println!("Connected to server");
            // TODO: change following with new sender
            let (s1, r1) = mpsc::channel(16);
            let (s2, r2) = mpsc::channel(16);
            let send_result = sender.try_send((s1, r2));
            // println!("send result: {:?}", send_result);
            if send_result.is_ok() {
                serve_socket(socket, s2, r1).await;
            }
        } else {
            println!("Failed to connect");
        }
    } else {
        println!("recv result: {:?}", recv_result);
    }
    println!("run_client complete");
}

async fn run_service(mut receiver: mpsc::Receiver<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>) {
    println!("In run_service");
    while let Some((mut sender, mut receiver)) = receiver.next().await {
        let send_result = sender.try_send(Bytes::from("Hello world!"));
        let recv_result = receiver.try_next();
        println!("New neighbor connected {:?} {:?}", send_result, recv_result);
    }
    println!("run_service finish");
}

async fn establish_connections_to_lan_servers(
    _sender: mpsc::Sender<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>,
) {
    //TODO: extend to send a broadcast dgram to local network
    // and create a dedicated connection for each response
}

async fn read_bytes_from_socket(socket: &UdpSocket) -> Result<Bytes, ConnectionError> {
    let mut buf = BytesMut::zeroed(1024);
    let recv_result = socket.recv(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        println!(
            "read_bytes_from_socket: {:?}",
            String::from_utf8_lossy(&buf[..count])
        );
        Ok(Bytes::from(Vec::from(&buf[..count])))
    } else {
        Err(ConnectionError { num: 0 })
    }
}

async fn read_bytes_from_local_stream(
    receiver: &mut mpsc::Receiver<Bytes>,
) -> Result<Bytes, ConnectionError> {
    let next_option = receiver.next().await;
    if let Some(bytes) = next_option {
        println!("read_bytes_from_local_stream: {:?}", bytes);
        Ok(bytes)
    } else {
        Err(ConnectionError { num: 0 })
    }
}

async fn race_tasks(
    socket: &UdpSocket,
    sender: &mut mpsc::Sender<Bytes>,
    receiver: &mut mpsc::Receiver<Bytes>,
) {
    let t1 = read_bytes_from_socket(socket).fuse();
    let t2 = read_bytes_from_local_stream(receiver).fuse();

    pin_mut!(t1, t2);

    let (from_socket, result) = select! {
        result1 = t1 =>  (true, result1),
        result2 = t2 => (false, result2),
    };
    let bytes = if let Ok(bts) = result {
        bts
    } else {
        Bytes::new()
    };
    if from_socket {
        let _send_result = sender.try_send(bytes);
    } else {
        let _send_result = socket.send(&bytes).await;
    }
}

async fn serve_socket(
    socket: UdpSocket,
    mut sender: mpsc::Sender<Bytes>,
    mut receiver: mpsc::Receiver<Bytes>,
) {
    println!("Racing tasks");
    race_tasks(&socket, &mut sender, &mut receiver).await;
}
