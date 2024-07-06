use async_std::task::yield_now;
use swarm_consensus::NetworkSettings;
// use crate::gnome::NetworkSettings;
// use crate::swarm::{Swarm, SwarmID};
use crate::NotificationBundle;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeToManager;
use swarm_consensus::ManagerToGnome;
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
    sender: Sender<GnomeToManager>,
    receiver: Receiver<GnomeToManager>,
    swarms: HashMap<SwarmID, Sender<ManagerToGnome>>,
    neighboring_swarms: HashMap<String, Vec<(SwarmID, GnomeId)>>,
    name_to_id: HashMap<String, SwarmID>,
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
        let (sender, receiver) = channel();
        Manager {
            gnome_id,
            sender,
            receiver,
            swarms: HashMap::new(),
            neighboring_swarms: HashMap::new(),
            name_to_id: HashMap::new(),
            network_settings,
            to_networking, // Send a message to networking about new swarm subscription, and where to send Neighbors
        }
    }

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
    pub async fn do_your_job(
        mut self,
        req_receiver: Receiver<ManagerRequest>,
        resp_sender: Sender<ManagerResponse>,
    ) {
        loop {
            if let Ok(request) = req_receiver.try_recv() {
                match request {
                    ManagerRequest::JoinSwarm(swarm_name) => {
                        // let mut neighbors = vec![];
                        if let Some(pairs) = self.neighboring_swarms.get(&swarm_name) {
                            for (s_id, g_id) in pairs {
                                if let Some(sender) = self.swarms.get(s_id) {
                                    // TODO
                                    let _ = sender.send(ManagerToGnome::ProvideNeighborsToSwarm(
                                        swarm_name.clone(),
                                        *g_id,
                                    ));
                                }
                            }
                            // // TODO this needs rework, maybe some ongoing requests queue
                            // if let Ok(response) = self.receiver.recv() {
                            //     if let GnomeToManager::AddNeighborToSwarm(_s_id, s_name, neighbor) =
                            //         response
                            //     {
                            //         if s_name == swarm_name {
                            //             neighbors.push(neighbor);
                            //         }
                            //     }
                            // }
                        }
                        if let Ok((swarm_id, (user_req, user_res))) =
                            // gmgr.join_a_swarm("trzat".to_string(), Some(neighbor_network_settings), None)
                            self.join_a_swarm(swarm_name.clone(), None, None)
                        {
                            self.notify_other_swarms(swarm_id, swarm_name.clone());
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
            self.serve_gnome_requests();
            yield_now().await;
        }
        self.finish();
        // join.await;
    }

    fn serve_gnome_requests(&mut self) {
        while let Ok(request) = self.receiver.try_recv() {
            match request {
                GnomeToManager::NeighboringSwarm(swarm_id, gnome_id, swarm_name) => {
                    println!("Manager got info about a swarm: '{}'", swarm_name);
                    if let Some(list) = self.neighboring_swarms.get_mut(&swarm_name) {
                        list.push((swarm_id, gnome_id));
                    } else {
                        self.neighboring_swarms
                            .insert(swarm_name, vec![(swarm_id, gnome_id)]);
                    }
                }
                GnomeToManager::AddNeighborToSwarm(_swarm_id, swarm_name, new_neighbor) => {
                    println!("Mgr recvd add neigh to {}", swarm_name);
                    if let Some(id) = self.name_to_id.get(&swarm_name) {
                        println!("found this swarm!");
                        self.add_neighbor_to_a_swarm(*id, new_neighbor);
                        //TODO: notify gnome that provided new neighbor
                    }
                }
            }
        }
    }

    pub fn add_neighbor_to_a_swarm(&mut self, id: SwarmID, neighbor: Neighbor) {
        if let Some(mgr_sender) = self.swarms.get_mut(&id) {
            match mgr_sender.send(ManagerToGnome::AddNeighbor(neighbor)) {
                Ok(()) => println!("Added neighbor to existing swarm"),
                Err(e) => println!("Failed adding neighbor to existing swarm: {:?}", e),
            }
        } else {
            println!("No swarm with id: {:?}", id);
        }
    }
    fn notify_other_swarms(&self, swarm_id: SwarmID, swarm_name: String) {
        for (id, sender) in &self.swarms {
            if *id != swarm_id {
                println!("Informing others");
                let _ = sender.send(ManagerToGnome::SwarmJoined(swarm_name.clone()));
            }
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
            let mgr_sender = self.sender.clone();
            let (send, recv) = channel();
            let (sender, receiver) = Swarm::join(
                name.clone(),
                swarm_id,
                self.gnome_id,
                neighbors,
                mgr_sender,
                recv,
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
            self.swarms.insert(swarm_id, send);
            self.name_to_id.insert(name, swarm_id);
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
        if let Some(mgr_send) = self.swarms.get(id) {
            let _ = mgr_send.send(ManagerToGnome::Status);
        }
    }

    pub fn finish(self) {
        for (id, mgr_sender) in self.swarms.into_iter() {
            let _ = mgr_sender.send(ManagerToGnome::Disconnect);
            // let jh = swarm.join_handle.take().unwrap();
            // let _ = jh.join();
            println!("Leaving SwarmID:`{:?}`", id);
        }
        // drop(self)
    }
}
