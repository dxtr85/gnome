use async_std::channel as achannel;
use async_std::channel::Receiver as AReceiver;
use async_std::channel::Sender as ASender;
use async_std::task::sleep;
use async_std::task::spawn;
use networking::Notification;
use rsa::pkcs1::RsaPublicKey;
use rsa::pkcs8::der::Decode;
use std::fs::read_to_string;
pub use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use swarm_consensus::GnomeId;
use swarm_consensus::GnomeToManager;
// use swarm_consensus::NetworkSettings;

// use swarm_consensus::Notification;
use swarm_consensus::SwarmName;
mod crypto;
mod data_conversion;
mod manager;
mod networking;
use crate::crypto::{
    get_key_pair_from_files, get_new_key_pair, store_key_pair_as_pem_files, Decrypter, Encrypter,
};
use crate::manager::FromGnomeManager;
use crate::manager::ToGnomeManager;
use manager::Manager;
use networking::run_networking_tasks;
use networking::NetworkSettings;
use std::sync::mpsc::channel;
use std::sync::mpsc::Receiver;
// use swarm_consensus::NotificationBundle;
// use swarm_consensus::Request;
// use swarm_consensus::Response;

pub mod prelude {
    pub use crate::crypto::sha_hash;
    pub use crate::init;
    pub use crate::manager::FromGnomeManager;
    pub use crate::manager::Manager as GManager;
    pub use crate::manager::ToGnomeManager;
    pub use crate::networking::Nat;
    pub use crate::networking::NetworkSettings;
    pub use crate::networking::PortAllocationRule;
    pub use crate::networking::Transport;
    pub use swarm_consensus::ByteSet;
    pub use swarm_consensus::CapabiliTree;
    pub use swarm_consensus::Capabilities;
    pub use swarm_consensus::CastData;
    pub use swarm_consensus::CastID;
    pub use swarm_consensus::GnomeId;
    pub use swarm_consensus::GnomeToApp;
    pub use swarm_consensus::NeighborRequest;
    pub use swarm_consensus::NeighborResponse;
    pub use swarm_consensus::Policy;
    pub use swarm_consensus::Requirement;
    pub use swarm_consensus::SwarmID;
    pub use swarm_consensus::SwarmName;
    pub use swarm_consensus::SyncData;
    pub use swarm_consensus::ToGnome;
}

pub async fn init(
    work_dir: PathBuf,
    neighbor_settings: Option<Vec<(GnomeId, NetworkSettings)>>,
    default_bandwidth_per_swarm: u64,
) -> (
    ASender<ToGnomeManager>,
    AReceiver<FromGnomeManager>,
    SwarmName,
) {
    // ) {
    // println!("init start");
    let (req_sender, req_receiver) = achannel::unbounded();
    let (resp_sender, resp_receiver) = achannel::unbounded();
    let mut my_name = SwarmName {
        founder: GnomeId::any(),
        name: format!("/"),
    };
    // println!("num of args: ");
    // let server_ip: IpAddr = "192.168.0.106".parse().unwrap();
    // let server_ip: IpAddr = "100.116.51.23".parse().unwrap();
    // let broadcast_ip: IpAddr = "192.168.0.255".parse().unwrap();
    // let server_port: u16 = 1026;
    // let nic_buffer_size: u64 = 500000;
    // let upload_bytes_per_sec: u64 = 102400;
    let mut decrypter: Option<Decrypter> = None;
    // let mut encrypter: Option<Encrypter> = None;
    let priv_path = PathBuf::from(work_dir.clone()).join("id_rsa");
    let pub_path = PathBuf::from(work_dir.clone()).join("id_rsa.pub");
    let pub_key_pem; //= String::new();
    let mut pub_key_der = vec![];
    let priv_key_pem;
    // println!("check if key files exist");
    if pub_path.exists() && priv_path.exists() {
        // println!("both exist");
        pub_key_pem = read_to_string(pub_path.clone()).unwrap();
        priv_key_pem = read_to_string(priv_path.clone()).unwrap();
        let res = Encrypter::create_from_data(&pub_key_pem);
        if let Ok(encr) = res {
            pub_key_der = encr.pub_key_der();
            // println!("\n\n\n\n\n\nDER len: {:?}", pub_key_der.len());
            let ki = RsaPublicKey::from_der(&pub_key_der);
            if ki.is_err() {
                println!("key err: {:?}", ki);
            }
            my_name.founder = GnomeId(encr.hash());
        }
        if let Some((priv_key, _pub_key)) = get_key_pair_from_files(priv_path, pub_path) {
            // println!("This is to verify signing works");
            // let decr = Decrypter::create(priv_key.clone());
            // let encr = Encrypter::create(_pub_key);
            // if let Ok(signature) = decr.sign("Test message".as_bytes()) {
            //     if encr.verify("Test message".as_bytes(), signature.as_slice()) {
            //         println!("Message verified! (sig len: {})", signature.len());
            //     } else {
            //         println!("Failed to verify message");
            //     }
            // }
            decrypter = Some(Decrypter::create(priv_key));
        }
    // } else if let Some((priv_key, pub_key)) = get_new_key_pair(512) {
    } else if let Some((priv_key, pub_key)) = get_new_key_pair() {
        // println!("new keys created");
        let _res = store_key_pair_as_pem_files(&priv_key, &pub_key, PathBuf::from(work_dir));
        // println!("Store key pair result: {:?} {:?}", res, priv_key);
        decrypter = Some(Decrypter::create(priv_key));
        pub_key_pem = read_to_string(pub_path.clone()).unwrap();
        priv_key_pem = read_to_string(priv_path.clone()).unwrap();
        let res = Encrypter::create_from_data(&pub_key_pem);
        if let Ok(encr) = res {
            pub_key_der = encr.pub_key_der();
            // println!("DER size: {}", pub_key_der.len());
            // if let Ok(signature) = decrypter.clone().unwrap().sign("Test message".as_bytes()) {
            //     if encr.verify("Test message".as_bytes(), &signature) {
            //         panic!("Message verified!");
            //     } else {
            //         panic!("Failed to verify message");
            //     }
            // }
            my_name.founder = GnomeId(encr.hash());
        }
        // encrypter = Some(Encrypter::create(pub_key));
    } else {
        panic!("Unable to create RSA key pair!");
    }
    // pub_key_pem.retain(|c| !c.is_whitespace());
    // println!("Pub key: {:?}", res);
    let (mut gmgr, networking_receiver) =
        // create_manager_and_receiver(gnome_id, Some(network_settings));
        create_manager_and_receiver(my_name.founder, pub_key_der, priv_key_pem, None, decrypter.clone().unwrap(), (req_sender.clone(),req_receiver),
        resp_sender.clone());

    // let app_sync_hash = 0; // TODO when we read data from disk we update app_sync_hash
    // let ns = NetworkSettings::new_not_natted(IpAddr::V4(Ipv4Addr::new(100, 127, 65, 53)), 1026);
    // let ns = NetworkSettings::new_not_natted(IpAddr::V4(Ipv4Addr::new(100, 112, 244, 78)), 1026);
    // let neighbor_settings = Some(ns);
    // let neighbor_settings = None;
    let mut filtered_neighbor_settings = vec![];
    if let Some(neighbors) = neighbor_settings {
        eprintln!("I am {}", my_name.founder);
        for (g_id, ns) in neighbors {
            if g_id == my_name.founder {
                eprintln!("Skipping my Pub IP ({:?})", ns);
                continue;
            }
            if !filtered_neighbor_settings.contains(&ns) {
                eprintln!("Adding {} setting ({:?})", g_id, ns);
                filtered_neighbor_settings.push(ns);
            }
        }
    }
    let filtered_neighbor_settings = if filtered_neighbor_settings.is_empty() {
        None
    } else {
        Some(filtered_neighbor_settings)
    };
    if let Ok((swarm_id, (user_req, user_res))) = gmgr.join_a_swarm(
        SwarmName::new(GnomeId::any(), "/".to_string()).unwrap(),
        default_bandwidth_per_swarm,
        filtered_neighbor_settings,
        None,
    ) {
        let _ = resp_sender
            .send(FromGnomeManager::MyName(my_name.clone()))
            .await;
        let _ = resp_sender
            .send(FromGnomeManager::SwarmJoined(
                swarm_id,
                SwarmName::new(GnomeId::any(), "/".to_string()).unwrap(),
                user_req,
                user_res,
            ))
            .await;
        eprintln!("Joined `any /` swarm");
    }

    let server_port: u16 = my_name.founder.get_port();
    //     {
    //     let modulo = (my_name.founder.0 % (u16::MAX as u64)) as u16;
    //     if modulo >= 1024 {
    //         modulo
    //     } else {
    //         modulo * 64
    //     }
    // };
    let _join = spawn(run_networking_tasks(
        gmgr.get_sender(),
        // server_ip,
        // broadcast_ip,
        server_port,
        // nic_buffer_size,
        // upload_bytes_per_sec,
        networking_receiver,
        decrypter.unwrap(),
        pub_key_pem,
    ));
    // TODO: spawn a loop with manager inside handling both user requests and
    //      swarm management
    //      Here we need to store information about swarms that our neighbors
    //      are subscribed to, probably as a hashmap of swarm_name -> Vec<SwarmID>.
    //      Now user will be able to see what swarms he is able to join immediately.
    //      For this we need to create a two way communication channel between manager/loop
    //      and each gnome individually.
    //      Now manager can ask a gnome to provide Neighbors for requested swarm_name.
    //      Gnome will send NeighborRequest to his neighbors.
    //      For each collected Response gnome will send it back to Manager.
    //      Manager will collect those responses from gnome and instantiate
    //      a new gnome for swarm name requested by User with initial pool of Neighbors
    //      collected from existing gnome(s).
    //
    // TODO: In future we might need to create a mechanism to avoid bandwith drainage.
    //       In case we run out of available bandwith we need to start conserving it.
    //       To do so we have to limit the number of swarms we are being part of.
    //       But user wants to stay in sync with all the swarms!
    //       In order to keep user happy and still have some bandwith available
    //       we need to introduce some sort of swarm_ring, maybe in form of VecDeque.
    //       Now we need to define a min_bandwith threshold - at this level we have
    //       to increase the number of swarms that are being served under swarm_ring.
    //       Being in swarm_ring means that you only stay in a given swarm until you
    //       become synced (or some defined timeout has passed).
    //       Once given swarm is synced it is being moved back to swarm_ring's end.
    //       We only move swarms into ring to stay above min_bandwith.
    //       If available_bandwith is way above min_off then we can decide
    //       to gradually move a swarm out of swarm_ring into regular swarm that
    //       is constantly updated.
    //       So swarm_ring has three modes of operation: increasing available bandwith,
    //       decreasing it, or leaving it as-is, if we are not using enough of it.
    //       Probably a sane default threshold should be min: 20%, min_off: 30%
    //       of total bandwith available.
    //       If we are between min and min_off we can stay in this configuration.
    //
    spawn(gmgr.do_your_job(default_bandwidth_per_swarm));
    (req_sender, resp_receiver, my_name)
}

pub fn create_manager_and_receiver(
    gnome_id: GnomeId,
    pub_key_der: Vec<u8>,
    priv_key_pem: String,
    network_settings: Option<NetworkSettings>,
    decrypter: Decrypter,
    (req_sender, req_receiver): (ASender<ToGnomeManager>, AReceiver<ToGnomeManager>),
    resp_sender: ASender<FromGnomeManager>,
) -> (Manager, Receiver<Notification>) {
    let (networking_sender, networking_receiver) = channel();
    // let network_settings = None;
    // let mgr = start(gnome_id, network_settings, networking_sender);
    let (sender, receiver) = channel();
    spawn(serve_gnome_requests(receiver, req_sender.clone()));
    let mgr = Manager::new(
        gnome_id,
        pub_key_der,
        priv_key_pem,
        network_settings,
        networking_sender,
        decrypter,
        req_receiver,
        req_sender,
        resp_sender,
        sender,
    );
    (mgr, networking_receiver)
}

async fn serve_gnome_requests(
    from_gnome: Receiver<GnomeToManager>,
    to_manager: ASender<ToGnomeManager>,
) {
    let recv_timeout = Duration::from_millis(8);
    loop {
        if let Ok(request) = from_gnome.recv_timeout(recv_timeout) {
            let _ = to_manager.send(ToGnomeManager::FromGnome(request)).await;
        } else {
            sleep(recv_timeout).await
        }
    }
}
// async fn activate_gnome(
//     // _gnome_id: GnomeId,
//     // ip: IpAddr,
//     // _broadcast: IpAddr,
//     port: u16,
//     // TODO: we need to be able to modify uplink_bandwith_bytes_sec
//     //       on the go as we decrease/increase number of swarms
//     //       we are participating to
//     // I'm probably wrong, since this will only be used when user decides to
//     // modify his initial uplink bandwith
//     buffer_size_bytes: u64,
//     uplink_bandwith_bytes_sec: u64,
//     receiver: Receiver<NotificationBundle>,
//     decrypter: Decrypter,
//     pub_key_pem: String,
// ) {
//     run_networking_tasks(
//         // gnome_id,
//         // ip,
//         // broadcast,
//         port,
//         buffer_size_bytes,
//         uplink_bandwith_bytes_sec,
//         receiver,
//         decrypter,
//         pub_key_pem,
//     )
//     .await;
// }

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
