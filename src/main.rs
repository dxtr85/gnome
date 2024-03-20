// use async_std::task::spawn_blocking;
use async_std::task::spawn;
use std::net::IpAddr;
// use std::time::Duration;
use swarm_consensus::start;
use swarm_consensus::GnomeId;
use swarm_consensus::Manager;
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
    let (networking_sender, networking_receiver) = channel();
    // spawn_blocking(move || run_service(networking_sender, receiver));
    spawn(run_networking_tasks(
        server_ip,
        broadcast_ip,
        server_port,
        sender,
        networking_receiver,
    ));
    // .await;
    run_service(networking_sender, receiver);
}

fn run_service(
    sender: Sender<(String, Sender<swarm_consensus::Request>)>,
    receiver: Receiver<(Sender<Message>, Receiver<Message>)>,
) -> Manager {
    println!("SRVC In run_service");
    let neighbor = if let Ok((msg_sender, msg_receiver)) = receiver.try_recv() {
        Some(vec![Neighbor::from_id_channel_time(
            GnomeId(1),
            msg_receiver,
            msg_sender,
            SwarmTime(0),
            SwarmTime(0),
        )])
    } else {
        println!("nic nie dostałem");
        None
    };
    println!("after neighbor assignment");
    let mut mgr = start(sender);
    println!("mgr");
    let (_user_req, _user_res) = mgr.join_a_swarm("swarm".to_string(), neighbor);
    println!("join");
    // let mut swarm_created = false;
    let request_sender = _user_req.clone();
    let mut number = 1;
    println!("loop");
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
            let _ = request_sender.send(swarm_consensus::Request::AddNeighbor(neighbor));
            // if swarm_created {
            //     mgr.add_neighbor_to_a_swarm("swarm".to_string(), neighbor);
            // } else {
            //     swarm_created = true;
            // }
        }
    }
    // println!("SRVC: finish");
}
