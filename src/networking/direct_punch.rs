use crate::networking::{holepunch::punch_it, subscription::Subscription};
use crate::prelude::{Decrypter, Encrypter};
use crate::GnomeId;
use async_std::task::{spawn, yield_now};
use std::net::IpAddr;
use std::sync::mpsc::{Receiver, Sender};
use swarm_consensus::NetworkSettings;

use super::token::Token;

pub async fn direct_punching_service(
    host_ip: IpAddr,
    sub_sender: Sender<Subscription>,
    // req_sender: Sender<Vec<u8>>,
    // mut resp_receiver: Receiver<[u8; 32]>,
    decrypter: Decrypter,
    pipes_sender: Sender<(Sender<Token>, Receiver<Token>)>,
    // receiver: Receiver<String>,
    pub_key_pem: String,
    swarm_name: String,
    net_set_recv: Receiver<(NetworkSettings, Option<NetworkSettings>)>,
) {
    println!("Waiting for direct connect requests for {}", swarm_name);
    let loc_encr = Encrypter::create_from_data(&pub_key_pem).unwrap();
    let gnome_id = GnomeId(loc_encr.hash());
    drop(loc_encr);

    // loop {
    //     let settings_result = net_set_recv.try_recv();
    //     if let Ok((my_settings, option_other)) = settings_result {
    //         if let Some(other_settings) = option_other {
    //             println!("My: {:?}, his: {:?}", my_settings, other_settings);
    //             spawn(punch_it(
    //                 host_ip,
    //                 swarm_name.clone(),
    //                 pub_key_pem.clone(),
    //                 gnome_id,
    //                 decrypter.clone(),
    //                 my_settings.port_range,
    //                 other_settings.port_range,
    //                 (other_settings.pub_ip, other_settings.pub_port),
    //                 // req_sender.clone(),
    //                 pipes_sender.clone(),
    //                 sub_sender.clone(),
    //                 None,
    //             ));
    //         }
    //     }
    //     yield_now().await;
    // }
}
