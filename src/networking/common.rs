use crate::networking::subscription::Subscription;
use async_std::net::UdpSocket;
use async_std::task::yield_now;
use bytes::{BufMut, BytesMut};
use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, Sender};

pub async fn collect_subscribed_swarm_names(
    names: &mut Vec<String>,
    sender: Sender<Subscription>,
    receiver: Receiver<Subscription>,
) -> Receiver<Subscription> {
    println!("Collecting swarm names...");
    let _ = sender.send(Subscription::ProvideList);
    loop {
        if let Ok(subs_msg) = receiver.try_recv() {
            // recv_result = Ok(recv_rslt);
            match subs_msg {
                Subscription::Added(ref name) => {
                    names.push(name.to_owned());
                    continue;
                }
                Subscription::List(ref nnames) => {
                    *names = nnames.to_owned();
                    break;
                }
                _ => println!("Unexpected message: {:?}", subs_msg),
            };
        }
        yield_now().await;
    }
    receiver
}

pub async fn send_subscribed_swarm_names(
    socket: &UdpSocket,
    names: &Vec<String>,
    remote_addr: SocketAddr,
) {
    let mut buf = BytesMut::with_capacity(1030);
    for name in names {
        buf.put(name.as_bytes());
        // TODO split with some other value
        buf.put_u8(255);
    }
    let bytes = buf.split();
    println!("After split: {:?}", bytes);
    let send_result = socket.send_to(&bytes, remote_addr).await;
    if let Ok(count) = send_result {
        println!("SKT Sent {} bytes", count);
    }
}

pub fn distil_common_names(
    common_names: &mut Vec<String>,
    names: Vec<String>,
    remote_names: Vec<String>,
) {
    for name in remote_names {
        // let name = String::from_utf8(bts).unwrap();
        if names.contains(&name) {
            common_names.push(name.to_owned());
        }
    }
}

pub async fn receive_remote_swarm_names(
    socket: &UdpSocket,
    mut recv_buf: &mut BytesMut,
    remote_names: &mut Vec<String>,
) {
    *remote_names = if let Ok((count, _from)) = socket.recv_from(&mut recv_buf).await {
        recv_buf[..count]
            // TODO split by some reasonable delimiter
            .split(|n| n == &255u8)
            .map(|bts| String::from_utf8(bts.to_vec()).unwrap())
            .collect()
    } else {
        Vec::new()
    };
}
