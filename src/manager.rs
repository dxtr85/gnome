use swarm_consensus::NetworkSettings;
// use crate::gnome::NetworkSettings;
// use crate::swarm::{Swarm, SwarmID};
use crate::NotificationBundle;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{GnomeId, Neighbor};
use swarm_consensus::{Request, Response};
use swarm_consensus::{Swarm, SwarmID};

pub enum ManagerRequest {
    JoinSwarm(String),
    Disconnect,
}
pub enum ManagerResponse {
    SwarmJoined(SwarmID, String, Sender<Request>, Receiver<Response>),
}
pub struct Manager {
    gnome_id: GnomeId,
    swarms: HashMap<SwarmID, Sender<Request>>,
    network_settings: NetworkSettings,
    to_networking: Sender<NotificationBundle>,
}

impl Manager {
    pub fn new(
        gnome_id: GnomeId,
        network_settings: Option<NetworkSettings>,
        to_networking: Sender<NotificationBundle>,
    ) -> Manager {
        let network_settings = network_settings.unwrap_or_default();
        Manager {
            gnome_id,
            swarms: HashMap::new(),
            network_settings,
            to_networking, // Send a message to networking about new swarm subscription, and where to send Neighbors
        }
    }

    pub fn add_neighbor_to_a_swarm(&mut self, id: SwarmID, neighbor: Neighbor) {
        if let Some(sender) = self.swarms.get_mut(&id) {
            match sender.send(Request::AddNeighbor(neighbor)) {
                Ok(()) => println!("Added neighbor to existing swarm"),
                Err(e) => println!("Failed adding neighbor to existing swarm: {:?}", e),
            }
        } else {
            println!("No swarm with id: {:?}", id);
        }
    }

    fn next_avail_swarm_id(&self) -> Option<SwarmID> {
        for i in 0..=255 {
            let swarm_id = SwarmID(i);
            if !self.swarms.contains_key(&swarm_id) {
                return Some(swarm_id);
            }
        }
        None
    }

    pub fn join_a_swarm(
        &mut self,
        name: String,
        neighbor_network_settings: Option<NetworkSettings>,
        neighbors: Option<Vec<Neighbor>>,
    ) -> Result<(SwarmID, (Sender<Request>, Receiver<Response>)), String> {
        if let Some(swarm_id) = self.next_avail_swarm_id() {
            let (band_send, band_recv) = channel();
            let (net_settings_send, net_settings_recv) = channel();
            if let Some(neighbor_settings) = neighbor_network_settings {
                let _ = net_settings_send.send(neighbor_settings);
            }
            let (sender, receiver) = Swarm::join(
                name.clone(),
                swarm_id,
                self.gnome_id,
                neighbors,
                band_recv,
                net_settings_send,
                self.network_settings,
            );
            // println!("swarm '{}' created ", name);
            // let sender = swarm.sender.clone();
            // let receiver = swarm.receiver.take();
            println!("Joined `{}` swarm", name);
            self.notify_networking(name.clone(), sender.clone(), band_send, net_settings_recv);
            println!("inserting swarm");
            self.swarms.insert(swarm_id, sender.clone());
            Ok((swarm_id, (sender, receiver)))
        } else {
            Err("Could not connect to a swarm, all SwarmIDs taken".to_string())
        }
    }

    pub fn notify_networking(
        &mut self,
        swarm_name: String,
        sender: Sender<Request>,
        avail_bandwith_sender: Sender<u64>,
        network_settings_receiver: Receiver<NetworkSettings>,
    ) {
        // println!("About to send notification");
        let r = self.to_networking.send(NotificationBundle {
            swarm_name,
            request_sender: sender,
            token_sender: avail_bandwith_sender,
            network_settings_receiver,
        });
        println!("notification sent: {:?}", r);
    }

    pub fn print_status(&self, id: &SwarmID) {
        if let Some(sender) = self.swarms.get(id) {
            let _ = sender.send(Request::Status);
        }
    }

    pub fn finish(self) {
        for (id, sender) in self.swarms.into_iter() {
            let _ = sender.send(Request::Disconnect);
            // let jh = swarm.join_handle.take().unwrap();
            // let _ = jh.join();
            println!("Leaving SwarmID:`{:?}`", id);
        }
        // drop(self)
    }
}
