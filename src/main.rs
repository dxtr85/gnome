use async_std::task::{sleep, spawn};
use bytes::Bytes;
use futures::channel::mpsc;
use futures::StreamExt;
use std::error::Error;
use std::fmt;
use std::net::IpAddr;
use std::time::Duration;
use swarm_consensus::start;
use swarm_consensus::GnomeId;
use swarm_consensus::Neighbor;
use swarm_consensus::SwarmTime;
mod data_conversion;
use data_conversion::{bytes_to_message, message_to_bytes};
mod networking;
use networking::run_networking_tasks;

#[async_std::main]
async fn main() {
    let server_ip: IpAddr = "192.168.237.196".parse().unwrap();
    let broadcast_ip: IpAddr = "192.168.237.255".parse().unwrap();
    let server_port: u16 = 1026;
    let (sender, receiver) = mpsc::channel(64usize);
    spawn(run_networking_tasks(
        server_ip,
        broadcast_ip,
        server_port,
        sender,
    ));
    run_service(receiver).await;
}

async fn run_service(mut receiver: mpsc::Receiver<(mpsc::Sender<Bytes>, mpsc::Receiver<Bytes>)>) {
    println!("SRVC In run_service");
    let mut mgr = start();
    while let Some((mut bytes_sender, mut bytes_receiver)) = receiver.next().await {
        println!("SRVC New neighbor connected");
        // TODO: insert an adapter here that plugs into application logic

        // local_sender sends Message that was constructed from Bytes received over UDPSocket
        let (local_sender, local_receiver) = std::sync::mpsc::channel();
        // remote_receiver receives Message constructed locally and turns it into Bytes to send over UDPSocket
        let (remote_sender, remote_receiver) = std::sync::mpsc::channel();
        let neighbor = Neighbor::from_id_channel_time(
            GnomeId(1),
            local_receiver,
            remote_sender,
            SwarmTime(0),
            SwarmTime(7),
        );
        let (_user_req, _user_res) = mgr.join_a_swarm("swarm".to_string(), Some(vec![neighbor]));

        sleep(Duration::from_millis(1600)).await;
        match remote_receiver.try_recv() {
            Err(e) => {
                println!("Unable to receive data from local service {:?}", e);
            }
            Ok(msg) => {
                let bytes = message_to_bytes(msg);
                match bytes_sender.try_send(bytes) {
                    Ok(()) => println!("Bytes sent over network"),
                    Err(e) => println!("Error sending bytes: {:?}", e),
                };
            }
        }
        // let send_result = bytes_sender.try_send(Bytes::from("Hello world!"));
        // if let Err(err) = send_result {
        //     println!("SRVC Unable to send to channel {:?}", err);
        // }
        sleep(Duration::from_millis(500)).await;
        match bytes_receiver.try_next() {
            Err(err) => println!("SRVC Unable to recieve from channel {:?}", err),
            Ok(Some(bytes)) => match bytes_to_message(&bytes) {
                Ok(message) => {
                    println!("Sending: {:?}", message);
                    let res = local_sender.send(message);
                    match res {
                        Ok(()) => println!("Sent Message to service"),
                        Err(e) => println!("send result: {:?}", e),
                    }
                }
                Err(_) => println!("Unable to convert bytes to Message"),
            },
            Ok(_) => (),
        };
        sleep(Duration::from_millis(500)).await;
        // mgr.finish();
    }
    println!("SRVC: finish");
}
