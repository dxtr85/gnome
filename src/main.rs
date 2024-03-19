use async_std::task::spawn_blocking;
// use async_std::task::{sleep, spawn};
use std::net::IpAddr;
// use std::time::Duration;
use swarm_consensus::start;
use swarm_consensus::GnomeId;
use swarm_consensus::Message;
use swarm_consensus::Neighbor;
use swarm_consensus::SwarmTime;
mod data_conversion;
mod networking;
use networking::run_networking_tasks;
use std::sync::mpsc::{channel, Receiver, Sender};

pub fn gnome_start(ip: IpAddr, broadcast: IpAddr, port: u16, swarm: String) {}

#[async_std::main]
async fn main() {
    let server_ip: IpAddr = "192.168.1.23".parse().unwrap();
    let broadcast_ip: IpAddr = "192.168.1.255".parse().unwrap();
    let server_port: u16 = 1026;
    let (sender, receiver) = channel();
    spawn_blocking(move || run_service(receiver));
    // spawn(
    run_networking_tasks(
        server_ip,
        broadcast_ip,
        server_port,
        sender,
        // )
    )
    .await;
    // run_service(receiver);
}

fn run_service(receiver: Receiver<(Sender<Message>, Receiver<Message>)>) {
    println!("SRVC In run_service");
    let mut mgr = start();
    let mut swarm_created = false;
    let mut number = 1;
    loop {
        if let Ok((msg_sender, msg_receiver)) = receiver.try_recv() {
            println!("SRVC New neighbor connected");
            let neighbor = Neighbor::from_id_channel_time(
                GnomeId(number),
                msg_receiver,
                msg_sender,
                SwarmTime(0),
                SwarmTime(7),
            );
            number += 1;
            if swarm_created {
                mgr.add_neighbor_to_a_swarm("swarm".to_string(), neighbor);
            } else {
                let (_user_req, _user_res) =
                    mgr.join_a_swarm("swarm".to_string(), Some(vec![neighbor]));
                swarm_created = true;
            }
        }
    }
    // println!("SRVC: finish");
}
