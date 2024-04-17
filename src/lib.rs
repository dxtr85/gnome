// use async_std::task::spawn_blocking;
// use async_std::task::spawn;
use prelude::Decrypter;
use prelude::Encrypter;
pub use std::net::IpAddr;
use swarm_consensus::GnomeId;
use swarm_consensus::Request;
// use std::time::Duration;
use swarm_consensus::start;
// use swarm_consensus::GnomeId;
use swarm_consensus::Manager;
// use swarm_consensus::Message;
// use swarm_consensus::Neighbor;
// use swarm_consensus::SwarmTime;
mod crypto;
mod data_conversion;
mod networking;
use networking::run_networking_tasks;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::{channel, Sender};

pub mod prelude {
    pub use crate::crypto::{
        get_key_pair_from_files, get_new_key_pair, store_key_pair_as_pem_files, Decrypter,
        Encrypter,
    };
    pub use async_std::task::spawn;
    pub use swarm_consensus::Data;
    pub use swarm_consensus::GnomeId;
    pub use swarm_consensus::Manager as GManager;
    pub use swarm_consensus::Request;
}

pub fn create_manager_and_receiver(
    gnome_id: GnomeId,
) -> (Manager, Receiver<(String, Sender<Request>, Sender<u32>)>) {
    let (networking_sender, networking_receiver) = channel();
    let mgr = start(gnome_id, networking_sender);
    (mgr, networking_receiver)
}

pub async fn activate_gnome(
    gnome_id: GnomeId,
    ip: IpAddr,
    broadcast: IpAddr,
    port: u16,
    buffer_size_bytes: u32,
    uplink_bandwith_bytes_sec: u32,
    receiver: Receiver<(String, Sender<Request>, Sender<u32>)>,
    decrypter: Decrypter,
    pub_key_pem: String,
) {
    run_networking_tasks(
        // gnome_id,
        ip,
        broadcast,
        port,
        buffer_size_bytes,
        uplink_bandwith_bytes_sec,
        receiver,
        decrypter,
        pub_key_pem,
    )
    .await;
}

// #[async_std::main]
// async fn main() {
//     let server_ip: IpAddr = "192.168.1.23".parse().unwrap();
//     let broadcast_ip: IpAddr = "192.168.1.255".parse().unwrap();
//     let server_port: u16 = 1026;
//     // let (sender, receiver) = channel();
//     // let (networking_sender, networking_receiver) = channel();
//     // spawn_blocking(move || run_service(networking_sender, receiver));
//     // spawn(run_networking_tasks(
//     //     server_ip,
//     //     broadcast_ip,
//     //     server_port,
//     //     sender,
//     //     networking_receiver,
//     // ));
//     // .await;
//     // println!("after neighbor assignment");
//     let (mut mgr, networking_receiver) = create_manager_and_receiver(); // = start(networking_sender);
//                                                                         // println!("mgr");
//     let (_user_req, _user_res) = mgr.join_a_swarm("swarm".to_string(), None);
//     // loop {}
//     // run_networking_tasks(
//     //     server_ip,
//     //     broadcast_ip,
//     //     server_port,
//     //     // sender,
//     //     networking_receiver,
//     // )
//     // .await;
//     let _gnome = activate_gnome(server_ip, broadcast_ip, server_port, networking_receiver).await;
//     // run_service(user_req.clone(), receiver);
// }

// fn run_service(
//     // sender: Sender<(String, Sender<swarm_consensus::Request>)>,
//     request_sender: Sender<Request>,
//     receiver: Receiver<(Sender<Message>, Receiver<Message>)>,
// ) -> Manager {
//     println!("SRVC In run_service");
//     // let neighbor = if let Ok((msg_sender, msg_receiver)) = receiver.try_recv() {
//     //     Some(vec![Neighbor::from_id_channel_time(
//     //         GnomeId(1),
//     //         msg_receiver,
//     //         msg_sender,
//     //         SwarmTime(0),
//     //         SwarmTime(0),
//     //     )])
//     // } else {
//     //     // println!("nic nie dostałem");
//     //     None
//     // };
//     // println!("join");
//     // let mut swarm_created = false;
//     // let request_sender = _user_req.clone();
//     let mut number = 1;
//     // println!("loop");
//     loop {
//         if let Ok((msg_sender, msg_receiver)) = receiver.try_recv() {
//             println!("SRVC New neighbor connected");
//             let neighbor = Neighbor::from_id_channel_time(
//                 GnomeId(number),
//                 msg_receiver,
//                 msg_sender,
//                 SwarmTime(0),
//                 SwarmTime(7),
//             );
//             number += 1;
//             let _ = request_sender.send(swarm_consensus::Request::AddNeighbor(neighbor));
//             // if swarm_created {
//             //     mgr.add_neighbor_to_a_swarm("swarm".to_string(), neighbor);
//             // } else {
//             //     swarm_created = true;
//             // }
//         }
//     }
//     // println!("SRVC: finish");
// }
