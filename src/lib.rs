use async_std::task::spawn;
use async_std::task::yield_now;
use std::fs;
pub use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;
use swarm_consensus::GnomeId;
use swarm_consensus::NetworkSettings;
mod crypto;
mod data_conversion;
mod manager;
mod networking;
use crate::crypto::{
    get_key_pair_from_files, get_new_key_pair, store_key_pair_as_pem_files, Decrypter, Encrypter,
};
use crate::manager::ManagerRequest;
use crate::manager::ManagerResponse;
use manager::Manager;
use networking::run_networking_tasks;
use std::sync::mpsc::channel;
use std::sync::mpsc::Receiver;
use swarm_consensus::NotificationBundle;
use swarm_consensus::Request;
use swarm_consensus::Response;

pub mod prelude {
    pub use crate::init;
    pub use crate::manager::Manager as GManager;
    pub use crate::manager::ManagerRequest;
    pub use crate::manager::ManagerResponse;
    pub use swarm_consensus::CastID;
    pub use swarm_consensus::Data;
    pub use swarm_consensus::GnomeId;
    pub use swarm_consensus::Nat;
    pub use swarm_consensus::NetworkSettings;
    pub use swarm_consensus::PortAllocationRule;
    pub use swarm_consensus::Request;
    pub use swarm_consensus::Response;
}

pub fn init(work_dir: String) -> (Sender<ManagerRequest>, Receiver<ManagerResponse>) {
    let (req_sender, req_receiver) = channel();
    let (resp_sender, resp_receiver) = channel();

    let mut gnome_id = GnomeId(0);
    // println!("num of args: {}", num);
    // let server_ip: IpAddr = "192.168.0.106".parse().unwrap();
    let server_ip: IpAddr = "100.116.51.23".parse().unwrap();
    // let broadcast_ip: IpAddr = "192.168.0.255".parse().unwrap();
    let server_port: u16 = 1026;
    // let nic_buffer_size: u32 = 500000;
    let nic_buffer_size: u64 = 0;
    let upload_bytes_per_sec: u64 = 10240;
    let mut decrypter: Option<Decrypter> = None;
    // let mut encrypter: Option<Encrypter> = None;
    let priv_path = PathBuf::from(work_dir.clone()).join("id_rsa");
    let pub_path = PathBuf::from(work_dir.clone()).join("id_rsa.pub");
    let pub_key_pem; //= String::new();
    if pub_path.exists() && priv_path.exists() {
        pub_key_pem = fs::read_to_string(pub_path.clone()).unwrap();
        let res = Encrypter::create_from_data(&pub_key_pem);
        if let Ok(encr) = res {
            gnome_id = GnomeId(encr.hash());
        }
        if let Some((priv_key, _pub_key)) = get_key_pair_from_files(priv_path, pub_path) {
            decrypter = Some(Decrypter::create(priv_key));
        }
    } else if let Some((priv_key, pub_key)) = get_new_key_pair(512) {
        let res = store_key_pair_as_pem_files(&priv_key, &pub_key, PathBuf::from(work_dir));
        println!("Store key pair result: {:?} {:?}", res, priv_key);
        decrypter = Some(Decrypter::create(priv_key));
        pub_key_pem = fs::read_to_string(pub_path.clone()).unwrap();
        let res = Encrypter::create_from_data(&pub_key_pem);
        if let Ok(encr) = res {
            gnome_id = GnomeId(encr.hash());
        }
        // encrypter = Some(Encrypter::create(pub_key));
    } else {
        panic!("Unable to create RSA key pair!");
    }
    // pub_key_pem.retain(|c| !c.is_whitespace());
    // println!("Pub key: {:?}", res);
    let (mut gmgr, networking_receiver) =
        // create_manager_and_receiver(gnome_id, Some(network_settings));
        create_manager_and_receiver(gnome_id, None);

    if let Ok((swarm_id, (user_req, user_res))) =
        // gmgr.join_a_swarm("trzat".to_string(), Some(neighbor_network_settings), None)
        gmgr.join_a_swarm("/".to_string(), None, None)
    {
        println!("Joined `/` swarm");
    }

    let join = spawn(activate_gnome(
        // gnome_id,
        server_ip,
        // broadcast_ip,
        server_port,
        nic_buffer_size,
        upload_bytes_per_sec,
        networking_receiver,
        decrypter.unwrap(),
        pub_key_pem,
    ));
    // TODO spawn a loop with manager inside handling both user requests and
    //      swarm management
    spawn(async move {
        loop {
            if let Ok(request) = req_receiver.try_recv() {
                match request {
                    ManagerRequest::JoinSwarm(swarm_name) => {
                        if let Ok((swarm_id, (user_req, user_res))) =
                            // gmgr.join_a_swarm("trzat".to_string(), Some(neighbor_network_settings), None)
                            gmgr.join_a_swarm(swarm_name.clone(), None, None)
                        {
                            let _ = resp_sender.send(ManagerResponse::SwarmJoined(
                                swarm_id, swarm_name, user_req, user_res,
                            ));
                        } else {
                            println!("Unable to join a swarm.");
                        }
                    }
                    ManagerRequest::Disconnect => {
                        // TODO: do all necessary stuff
                        break;
                    }
                }
            }
            yield_now().await;
        }
        gmgr.finish();
        join.await;
    });
    (req_sender, resp_receiver)
}
fn start(
    gnome_id: GnomeId,
    network_settings: Option<NetworkSettings>,
    sender: Sender<NotificationBundle>,
) -> Manager {
    Manager::new(gnome_id, network_settings, sender)
}

pub fn create_manager_and_receiver(
    gnome_id: GnomeId,
    network_settings: Option<NetworkSettings>,
) -> (Manager, Receiver<NotificationBundle>) {
    let (networking_sender, networking_receiver) = channel();
    // let network_settings = None;
    let mgr = start(gnome_id, network_settings, networking_sender);
    (mgr, networking_receiver)
}

async fn activate_gnome(
    // _gnome_id: GnomeId,
    ip: IpAddr,
    // _broadcast: IpAddr,
    port: u16,
    buffer_size_bytes: u64,
    uplink_bandwith_bytes_sec: u64,
    receiver: Receiver<NotificationBundle>,
    decrypter: Decrypter,
    pub_key_pem: String,
) {
    run_networking_tasks(
        // gnome_id,
        // ip,
        // broadcast,
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
