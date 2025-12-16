use crate::crypto::sha_hash;
use crate::crypto::Decrypter;
use crate::networking::status::NetworkSummary;
use crate::networking::status::Transport;
// use async_std::task::sleep;
// use async_std::task::spawn;
use a_swarm_consensus::ByteSet;
use a_swarm_consensus::Capabilities;
use a_swarm_consensus::CastData;
use a_swarm_consensus::Policy;
use a_swarm_consensus::Requirement;
use smol::spawn;
use smol::Timer;
use std::collections::VecDeque;
// use std::hash::{DefaultHasher, Hash, Hasher};
use crate::networking::Nat;
use crate::networking::NetworkSettings;
use crate::networking::PortAllocationRule;
use a_swarm_consensus::SwarmName;
use a_swarm_consensus::SwarmTime;
use std::net::IpAddr;
use std::time::Duration;
// use crate::gnome::NetworkSettings;
// use crate::swarm::{Swarm, SwarmID};
use crate::networking::Notification;
use crate::networking::NotificationBundle;
// use async_std::channel::Receiver as AReceiver;
// use async_std::channel::Sender as ASender;
use a_swarm_consensus::GnomeToManager;
use a_swarm_consensus::ManagerToGnome;
use a_swarm_consensus::{GnomeId, Neighbor};
use a_swarm_consensus::{GnomeToApp, ToGnome};
use a_swarm_consensus::{Swarm, SwarmID};
use rsa::{
    pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey},
    pkcs1v15::{Signature, SigningKey, VerifyingKey},
    // pkcs8::LineEnding,
    sha2::{Digest, Sha256},
    signature::Verifier,
    // traits::SignatureScheme,
    // Pkcs1v15Encrypt,
    // RsaPrivateKey,
    RsaPublicKey,
};
use smol::channel::Receiver as AReceiver;
use smol::channel::Sender as ASender;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender};

pub enum ToGnomeManager {
    JoinRandom(Option<SwarmName>), // We ask GMgr to join a swarm of his own selection
    //TODO: maybe JoinSwarm could contain optional Bandwidth value?
    JoinSwarm(SwarmName, Option<GnomeId>), //GID represents source of information about this new swarm
    StartListeningSwarm(Vec<(GnomeId, NetworkSettings)>), //Use this when we loose all existing neighbors.
    ExtendSwarm(SwarmName, SwarmID, GnomeId), // SwarmID has GnomeId who belongs to SwarmName
    // LeaveSwarm(SwarmName),
    LeaveSwarm(SwarmID),
    FromGnome(GnomeToManager),
    ProvideNeighboringSwarms(SwarmID),
    DisconnectSwarmIfNoNeighbors(SwarmID),
    PublicAddress(IpAddr, u16, Nat, (PortAllocationRule, i8), Transport),
    ProvidePublicIPs,
    SetRunningPolicy(SwarmName, Policy, Requirement),
    SetRunningCapability(SwarmName, Capabilities, Vec<GnomeId>),
    SetRunningByteSet(SwarmName, u8, ByteSet),
    CustomNeighborRequest(SwarmName, GnomeId, u8, CastData),
    CustomNeighborResponse(SwarmName, GnomeId, u8, CastData),
    Quit,
}

#[derive(Debug)]
pub enum FromGnomeManager {
    SelectedSwarm(Option<SwarmName>),
    SwarmJoined(SwarmID, SwarmName, Sender<ToGnome>, Receiver<GnomeToApp>), //TODO: process inside AppMgr
    SwarmNeighbors(SwarmID, HashSet<GnomeId>),
    SwarmNeighborLeft(SwarmID, GnomeId),
    // NeighboringSwarms(SwarmID, Vec<(SwarmName, GnomeId)>),
    SwarmBusy(SwarmID, bool), //TODO: process inside AppMgr
    NewSwarmsAvailable(SwarmID, HashSet<(SwarmName, GnomeId, bool)>), //TODO: process inside AppMgr
    SwarmFounderDetermined(SwarmID, SwarmName),
    MyName(SwarmName),
    // MyPublicIPs(Vec<(IpAddr, u16, Nat, (PortAllocationRulej, i8))>),
    MyPublicIPs(Vec<NetworkSettings>),
    AllNeighborsGone,
    RunningPolicies(Vec<(Policy, Requirement)>),
    RunningCapabilities(Vec<(Capabilities, Vec<GnomeId>)>),
    RunningByteSets(Vec<(u8, ByteSet)>),
    Disconnected(Vec<(SwarmID, SwarmName)>),
}
pub struct Manager {
    gnome_id: GnomeId,
    pub_key_der: Vec<u8>,
    priv_key_pem: String,
    sender: Sender<GnomeToManager>,
    // receiver: Receiver<GnomeToManager>,
    swarms: HashMap<SwarmID, (Sender<ManagerToGnome>, SwarmName)>,
    neighboring_swarms: HashMap<SwarmName, HashSet<GnomeId>>,
    neighboring_gnomes: HashMap<GnomeId, HashSet<SwarmName>>,
    name_to_id: HashMap<SwarmName, SwarmID>,
    // comm_channels: CommunicationChannels,
    // network_settings: NetworkSettings,
    to_networking: Sender<Notification>,
    decrypter: Decrypter,
    req_receiver: AReceiver<ToGnomeManager>,
    req_sender: ASender<ToGnomeManager>,
    resp_sender: ASender<FromGnomeManager>,
    waiting_for_neighbors: Option<SwarmID>,
    waiting_for_settings: VecDeque<(SwarmID, u8, GnomeId)>,
    join_queue: VecDeque<(SwarmName, Option<GnomeId>)>,
    network_summary: NetworkSummary,
}

//TODO: Manager should hold Gnome's pub_key_pem in order to provide it
//      when required (when we want to equip him with a Capability).
//TODO: decrypter has a sign method that can be used to create signed messages.
//      Those can be verified by calling Encrypter::verify
//      on corresponding to given Decrypter entity.
//      We should do signing and verification within gnome crate, since
//      swarm-consensus crate should stay dependence-free.
//      We need to introduce Capabilities (can be defined in swarm-consensus).
//      Gnome will ask Manager to create a signed message,
//      Manager will send back a signed message and Gnome will propagate it
//
//      Upon receiving a SignedMessage from a Neighbor Gnome needs to verify it.
//      Gnome will check if given remote GnomeId instance has capabilities
//      required for building SignedMessage.
//      If not - message gets discarded, otherwise
//      Gnome looks up supposed message author's pub_key_pem in swarm's data structure
//
//      Then Gnome will ask Manager to verify a signed message by providing:
//      - message,
//      - signature,
//      - pub_key_pem.
//      Manager will create Encrypter from received data and
//      confirm or deny validity of given message.
//      (We do not want any dependencies in swarm-consensus crate.)
//
//      If message is Valid - Gnome proceeds, otherwise message is discarded.
//
//      Neighbors should not store pub_key_pems. We can have a Capability only
//      for our (selected) Neighbors and store their pub_key_pems there.
//      But only in case we need to send SignedMessages between direct peers.
//      In case we want to create a ChainedUnicast stream for communication
//      between two gnomes that are not direct Neighbors we will need a Capability
//      for such known Gnome in order to verify this was indeed sent by him.
//
//      Signing and verification can be also useful for Multicasting in order
//      to prevent spoofing.
//      Swarm reconfiguration should be done only with SignedMessages.
//      Sending a message uplink to broadcast's origin in order for him to
//      transmit this message via broadcast channel can be also done via
//      SignedMessage.
//
//      In order to make the Capabilities system work we need to create a mapping
//      between SignedMessage and a list of Capabilities required from a Gnome
//      that wants to instantiate such message.
//      When a SignedMessage is accepted, it is passed to user.

impl Manager {
    pub fn new(
        gnome_id: GnomeId,
        pub_key_der: Vec<u8>,
        priv_key_pem: String,
        _network_settings: Option<NetworkSettings>, //TODO: use this info
        to_networking: Sender<Notification>,
        decrypter: Decrypter,
        req_receiver: AReceiver<ToGnomeManager>,
        req_sender: ASender<ToGnomeManager>,
        resp_sender: ASender<FromGnomeManager>,
        sender: Sender<GnomeToManager>,
    ) -> Manager {
        // let network_settings = network_settings.unwrap_or_default();
        // let comm_channels = CommunicationChannels {
        //     udp: (false, ChannelAvailability::Unknown),
        //     tcp: (false, ChannelAvailability::Unknown),
        //     udpv6: (false, ChannelAvailability::Unknown),
        //     tcpv6: (false, ChannelAvailability::Unknown),
        // };
        Manager {
            gnome_id,
            pub_key_der,
            priv_key_pem,
            sender,
            // receiver,
            swarms: HashMap::new(),
            neighboring_swarms: HashMap::new(),
            neighboring_gnomes: HashMap::new(),
            name_to_id: HashMap::new(),
            // comm_channels,
            // network_settings,
            to_networking, // Send a message to networking about new swarm subscription, and where to send Neighbors
            decrypter,
            req_receiver,
            req_sender,
            resp_sender,
            waiting_for_neighbors: None,
            waiting_for_settings: VecDeque::new(),
            join_queue: VecDeque::with_capacity(8),
            network_summary: NetworkSummary::empty(gnome_id.get_port()),
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
        bandwidth_per_swarm: u64,
        // req_receiver: Receiver<ToGnomeManager>,
        // resp_sender: Sender<FromGnomeManager>,
        // app_sync_hash: u64,
    ) {
        // let min_sleep_nanos: u64 = 1 << 7;
        // let max_sleep_nanos: u64 = 1 << 27;
        let sleep_nanos: u64 = 1 << 25;
        let mut quit = false;
        let mut prev_join_swarm = SwarmName::new(GnomeId(0), "/".to_string()).unwrap();
        loop {
            if let Ok(request) = self.req_receiver.recv().await {
                match request {
                    ToGnomeManager::JoinRandom(exclude_swarm_opt) => {
                        eprintln!(
                            "JoinRandom request (exclude: {})",
                            exclude_swarm_opt.is_some()
                        );
                        if self.swarms.is_empty() {
                            eprintln!("JoinRandom when not in touch with any swarm!");
                        }
                        let to_exclude = if exclude_swarm_opt.is_some() {
                            exclude_swarm_opt
                        } else {
                            Some(prev_join_swarm.clone())
                        };
                        if let Some((s_name, aware_gnome)) =
                            self.next_avail_neighboring_swarm(to_exclude)
                        {
                            prev_join_swarm = s_name.clone();
                            eprintln!("JoinRandom found {}", s_name);
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::SelectedSwarm(Some(s_name.clone())))
                                .await;
                            let _ = self
                                .req_sender
                                .send(ToGnomeManager::JoinSwarm(s_name, aware_gnome))
                                .await;
                        } else {
                            eprintln!("JoinRandom not found any swarm to join");
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::SelectedSwarm(None))
                                .await;
                        }
                    }
                    ToGnomeManager::JoinSwarm(swarm_name, aware_gnome) => {
                        eprintln!("Waiting 4 neighbors: {:?}", self.waiting_for_neighbors);
                        if self.waiting_for_neighbors.is_some() {
                            eprintln!(
                                "Delaying Join {}(qlen: {})",
                                swarm_name,
                                self.join_queue.len()
                            );
                            self.join_queue.push_back((swarm_name, aware_gnome));
                            continue;
                        }
                        eprintln!("Manager Received JoinSwarm({})", swarm_name);
                        if let Some(_es_id) = self.name_to_id.get(&swarm_name) {
                            eprintln!("Not joining {}, we are already there", swarm_name);
                            //     if let Some((sender, es_name)) = self.swarms.get(es_id){
                            //         if swarm_name ==*es_name{
                            //     let _ = self
                            //         .resp_sender
                            //         .send(FromGnomeManager::SwarmJoined(es_id, swarm_name, Sender<ToGnome>, Receiver<GnomeToApp>))
                            //         .await;

                            //         }

                            // }
                            continue;
                        }
                        if let Some(n_ids) = self.neighboring_swarms.get(&swarm_name) {
                            eprintln!("Existing neighbors already in {}: ", swarm_name);
                            for n_id in n_ids {
                                // for (s_id, g_id) in n_ids {
                                if let Some(ns_names) = self.neighboring_gnomes.get(&n_id) {
                                    for ns_name in ns_names {
                                        if *ns_name == swarm_name {
                                            continue;
                                        }
                                        if let Some(existing_id) = self.name_to_id.get(ns_name) {
                                            if let Some((to_gnome, _name)) =
                                                self.swarms.get(existing_id)
                                            {
                                                eprint!("{} in {}… ", n_id, ns_name);
                                                let _res = to_gnome.send(
                                                    ManagerToGnome::ProvideNeighborsToSwarm(
                                                        swarm_name.clone(),
                                                        *n_id,
                                                    ),
                                                );
                                                eprintln!("request sent: {:?}", _res);
                                                break;
                                            }
                                        } else {
                                            eprintln!("offline");
                                        }
                                    }
                                }
                                // if let Some((to_gnome, _neighbors)) = self.swarms.get(s_id) {
                                //     let _res =
                                //         to_gnome.send(ManagerToGnome::ProvideNeighborsToSwarm(
                                //             swarm_name.clone(),
                                //             *g_id,
                                //         ));
                                //     eprintln!("request sent: {:?}", _res);
                                // }
                            }
                        } else {
                            eprintln!("No pairs found in neighboring swarms");
                        }
                        eprintln!("Joining {}… ", swarm_name,);
                        if let Ok((swarm_id, (user_req, user_res))) =
                            self.join_a_swarm(swarm_name.clone(), bandwidth_per_swarm, None, None)
                        {
                            // Start a timer for a few seconds.
                            // When it ends Mgr checks if it received
                            // ActiveNeighbors message from this new swarm.
                            // If not send Disconnect request,
                            // and maybe try again.
                            eprintln!("Seting waiting_for_neighbors to {}", swarm_id);
                            self.waiting_for_neighbors = Some(swarm_id);
                            // if !quit {
                            let message = ToGnomeManager::DisconnectSwarmIfNoNeighbors(swarm_id);
                            spawn(start_a_timer(
                                self.req_sender.clone(),
                                message,
                                Duration::from_secs(10),
                            ))
                            .detach();
                            // }
                            if !swarm_name.founder.is_any() {
                                self.notify_other_swarms(swarm_id, swarm_name.clone(), aware_gnome);
                            }
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::SwarmJoined(
                                    swarm_id, swarm_name, user_req, user_res,
                                ))
                                .await;
                        } else {
                            eprintln!("Unable to join a swarm.");
                        }
                    }
                    ToGnomeManager::StartListeningSwarm(ns) => {
                        let swarm_name = SwarmName::new(GnomeId::any(), "/".to_string()).unwrap();
                        //TODO: filter neighbor settings
                        let neighbor_settings = if ns.is_empty() {
                            None
                        } else {
                            let mut neighbors = Vec::with_capacity(ns.len());
                            for (g_id, setting) in ns {
                                if g_id == self.gnome_id {
                                    continue;
                                }
                                neighbors.push(setting);
                            }
                            Some(neighbors)
                        };
                        if neighbor_settings.is_some() {
                            eprintln!("Starting a listening swarm with Neighbor endpoints");
                        } else {
                            eprintln!("Starting a listening swarm");
                        }
                        if let Ok((swarm_id, (user_req, user_res))) = self.join_a_swarm(
                            swarm_name.clone(),
                            bandwidth_per_swarm,
                            neighbor_settings,
                            None,
                        ) {
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::SwarmJoined(
                                    swarm_id, swarm_name, user_req, user_res,
                                ))
                                .await;
                        } else {
                            eprintln!("Unable to join default swarm.");
                        }
                    }
                    ToGnomeManager::ExtendSwarm(swarm_name, swarm_id, gnome_id) => {
                        eprintln!(
                            "GMgr Received ExtendSwarm({}, {} {})",
                            swarm_name, swarm_id, gnome_id
                        );
                        // eprint!("Name to ids: ");
                        // for name in self.name_to_id.keys() {
                        //     eprint!("{} ", name);
                        // }
                        // eprintln!("Existing swarms:");
                        // for (id, swarm_data) in self.swarms.iter() {
                        //     eprint!("{}: ", id);
                        //     for n_id in &swarm_data.1 {
                        //         eprint!("{} ", n_id);
                        //     }
                        //     eprintln!("");
                        // }
                        if self.name_to_id.get(&swarm_name).is_some() {
                            if let Some(neighbors) = self.neighboring_swarms.get(&swarm_name) {
                                // if let Some((_sender, neighbors)) = self.swarms.get(s_id) {
                                if neighbors.contains(&gnome_id) {
                                    eprintln!("But it already has this neighbor");
                                    continue;
                                }
                            }
                        }
                        if let Some((to_gnome, _name)) = self.swarms.get(&swarm_id) {
                            // for (to_gnome, _neighbors) in self.swarms.values() {
                            if let Some(g_ids) = self.neighboring_swarms.get(&swarm_name) {
                                eprintln!("{} res2: {:?}", swarm_id, _name);
                                if g_ids.contains(&gnome_id) {
                                    let _res =
                                        to_gnome.send(ManagerToGnome::ProvideNeighborsToSwarm(
                                            swarm_name.clone(),
                                            gnome_id,
                                        ));
                                    eprintln!("Extend request to {} sent: {:?}", gnome_id, _res);
                                } else {
                                    eprintln!("Unable to send Extend request: missing neighbor");
                                    //TODO: a mechanism to retry after some timeout.
                                    // spawn a task that will sleep for a few seconds and then
                                    // send this request again.
                                    // Make sure this procedure runs at most 3 times!
                                }
                            }
                        } else {
                            eprintln!("Unable to send Extend request, {} not found", swarm_id);
                        }
                    }
                    ToGnomeManager::ProvideNeighboringSwarms(s_id) => {
                        eprintln!("Gnome Manager received ProvideNeighboringSwarms {}", s_id);
                        let mut query_name = None;
                        for (name, sid) in self.name_to_id.iter() {
                            if s_id == *sid {
                                query_name = Some(name.clone());
                                break;
                            }
                        }
                        let mut neighboring_swarms = HashSet::new();
                        if let Some(q_name) = query_name {
                            //TODO:
                            // we take n_ids from neighboring_swarm
                            if let Some(n_ids) = self.neighboring_swarms.get(&q_name) {
                                for n_id in n_ids {
                                    // for each n_id we iterate neighboring_gnome
                                    // and see whan result_names it contains
                                    if let Some(s_names) = self.neighboring_gnomes.get(n_id) {
                                        for s_name in s_names {
                                            eprintln!("Adding {} to list", s_name);
                                            neighboring_swarms.insert((
                                                s_name.clone(),
                                                *n_id,
                                                self.name_to_id.contains_key(&s_name),
                                            ));
                                        }
                                    }
                                }
                            }
                            if !neighboring_swarms.is_empty() {
                                // let _ = self
                                //     .resp_sender
                                //     .send(FromGnomeManager::NewSwarmsAvailable(
                                //         s_id,
                                //         neighboring_swarms,
                                //     ))
                                //     // .send(FromGnomeManager::NewSwarmsAvailable(s_id, HashSet::new()))
                                //     .await;
                            } else {
                                eprintln!("Neighboring swarms empty");
                            }
                        } else {
                            eprintln!("Can not provide neighboring swarms");
                        }
                        // for (s_name, n_ids) in self.neighboring_swarms.iter() {
                        //     if self.name_to_id.contains_key(&s_name) {
                        //         continue;
                        //     }
                        //     for (sw_id, g_id) in n_ids {
                        //         if sw_id == &s_id {
                        //             eprintln!("Adding {} to list", s_name);
                        //             neighboring_swarms.insert((
                        //                 s_name.clone(),
                        //                 *g_id,
                        //                 self.name_to_id.contains_key(&s_name),
                        //             ));
                        //         }
                        //     }
                        // }

                        // if let Some((to_gnome, _n)) = self.swarms.get(&s_id) {
                        //     let _ = to_gnome.send(ManagerToGnome::ListNeighboringSwarms);
                        // } else {
                        //     eprintln!("GMgr does not know anything about {}", s_id);
                        // }
                    }
                    ToGnomeManager::ProvidePublicIPs => {
                        let nss = self.network_summary.get_network_settings();
                        if !nss.is_empty() {
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::MyPublicIPs(nss))
                                .await;
                        }
                    }
                    ToGnomeManager::LeaveSwarm(s_id) => {
                        eprintln!("GMgr requested LeaveSwarm({})", s_id);
                        // for key in self.neighboring_swarms.keys() {
                        //     eprintln!("GMgr knows about {}", key);
                        // }
                        // for (s_name, n_ids) in self.neighboring_swarms.iter_mut() {
                        //     let existing_pairs = std::mem::replace(n_ids, Vec::new());
                        //     for (sw_id, g_id) in existing_pairs.into_iter() {
                        //         if sw_id.0 == s_id.0 {
                        //             continue;
                        //         }
                        //         n_ids.push((sw_id, g_id));
                        //     }
                        //     if n_ids.is_empty() {
                        //         eprintln!("TODO: We should remove {} from neighboring_swarms after it's update logic is done",s_name);
                        //     }
                        // }
                        // for (name, id) in self.name_to_id.iter() {
                        //     if id.0 == s_id.0 {
                        //         eprint!("GMgr has to remove {}...", name);
                        //         let removed = self.neighboring_swarms.remove(&name);
                        //         eprintln!("removed: {}", removed.is_some());
                        //     }
                        // }
                        if let Some((to_gnome, s_name)) = self.swarms.get(&s_id) {
                            eprintln!("Requesting networking to remove {}", s_name);
                            let _ = self
                                .to_networking
                                .send(Notification::RemoveSwarm(vec![s_name.clone()]));
                            let _ = to_gnome.send(ManagerToGnome::Disconnect);
                        } else {
                            eprintln!("Can not leave {} - not found in Swarms set", s_id);
                        }
                        // } else {
                        //     eprintln!("Unable to find SwarmID for {} ", s_name);
                        // }
                    }
                    ToGnomeManager::PublicAddress(ip, port, nat, port_rule, tr) => {
                        eprintln!("Queue for PubIP: {}", self.waiting_for_settings.len());
                        // if let Some(pub_ips) =
                        self.network_summary.process(ip, port, nat, port_rule, tr);
                        // let nss = self.network_summary.get_network_settings();
                        // if !nss.is_empty() {
                        //     let _ = self
                        //         .resp_sender
                        //         .send(FromGnomeManager::MyPublicIPs(nss))
                        //         .await;
                        // }
                        while let Some((s_id, conn_id, g_id)) =
                            self.waiting_for_settings.pop_front()
                        {
                            let nsl = self.network_summary.get_network_settings_bytes();
                            if let Some((sender, _sname)) = self.swarms.get(&s_id) {
                                // for ns in nsl {
                                eprintln!("Sending NS to Gnome");
                                let _ = sender
                                    .send(ManagerToGnome::ReplyNetworkSettings(nsl, conn_id, g_id));
                                // }
                            }
                        }
                    }
                    // ToGnomeManager::TryTcpConnect(ns) => {
                    //     //todo:
                    //     // send this to networking
                    //     // self.to_networking.send(Notification::AddSwarm(()));
                    //     eprintln!("GMgrNS: {:?}", ns);
                    // }
                    ToGnomeManager::FromGnome(request) => {
                        // eprintln!("GMgr Got {:?}", request);
                        let disconnected_opt = self.serve_gnome_requests(request).await;
                        if let Some(disconnected_swarms) = disconnected_opt {
                            let mut removed_names = Vec::with_capacity(disconnected_swarms.len());
                            for (_s_id, s_name) in &disconnected_swarms {
                                removed_names.push(s_name.clone());
                            }

                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::Disconnected(disconnected_swarms))
                                .await;
                            if quit {
                                eprintln!("quit is true");
                                let sleep_time = Duration::from_nanos(sleep_nanos);
                                // sleep(sleep_time).await;
                                Timer::after(sleep_time).await;
                                break;
                            }
                            let _ = self
                                .to_networking
                                .send(Notification::RemoveSwarm(removed_names));
                        }
                    }
                    ToGnomeManager::CustomNeighborRequest(s_name, g_id, req_id, data) => {
                        eprintln!("GMgr got CustomNeighborRequest");
                        if let Some(s_id) = self.name_to_id.get(&s_name) {
                            let (to_gnome, _n) = self.swarms.get(s_id).unwrap();
                            let _ =
                                to_gnome.send(ManagerToGnome::SendCustom(true, g_id, req_id, data));
                        }
                    }
                    ToGnomeManager::CustomNeighborResponse(s_name, g_id, req_id, data) => {
                        eprintln!("GMgr got CustomNeighborResponse");
                        if let Some(s_id) = self.name_to_id.get(&s_name) {
                            let (to_gnome, _n) = self.swarms.get(s_id).unwrap();
                            let _ = to_gnome
                                .send(ManagerToGnome::SendCustom(false, g_id, req_id, data));
                        }
                    }
                    ToGnomeManager::Quit => {
                        eprintln!("GMgr Got quit");
                        let mut disconected = vec![];
                        for (s_id, (to_gnome, _neighbors)) in &self.swarms {
                            eprintln!("Sending disconnect request to swarm: {:?}", s_id);
                            disconected.push((
                                *s_id,
                                SwarmName::new(GnomeId::any(), String::new()).unwrap(),
                            ));
                            let _ = to_gnome.send(ManagerToGnome::Disconnect);
                        }
                        // eprintln!("Swarms: {:?}", self.swarms);
                        // if self.swarms.is_empty() {
                        let _ = self
                            .resp_sender
                            .send(FromGnomeManager::Disconnected(disconected))
                            .await;
                        // }
                        quit = true;
                    }
                    ToGnomeManager::DisconnectSwarmIfNoNeighbors(s_id) => {
                        eprintln!("Timeout for neighbors gathering for {}", s_id);
                        if let Some(waiting_for_neighbors) = self.waiting_for_neighbors.take() {
                            // eprintln!("Disconnecting.");
                            if s_id == waiting_for_neighbors {
                                if let Some((mgr_to_gnome, s_name)) = self.swarms.get(&s_id) {
                                    let _ = mgr_to_gnome.send(ManagerToGnome::Disconnect);
                                    let _ = self
                                        .resp_sender
                                        .send(FromGnomeManager::Disconnected(vec![(
                                            s_id,
                                            s_name.clone(),
                                        )]))
                                        .await;
                                }
                            } else {
                                eprintln!("Seting back waiting_for_neighbors ");
                                self.waiting_for_neighbors = Some(waiting_for_neighbors);
                            }
                        } else {
                            if let Some((s_name, aware_gnome)) = self.join_queue.pop_front() {
                                eprintln!("Requesting join {}", s_name);
                                let _ = self
                                    .req_sender
                                    .send(ToGnomeManager::JoinSwarm(s_name, aware_gnome))
                                    .await;
                            }
                        }
                    }
                    ToGnomeManager::SetRunningPolicy(s_name, pol, req) => {
                        eprintln!("GMgr rcvd SetRunningPolicy for {s_name}");
                        if let Some(s_id) = self.name_to_id.get(&s_name) {
                            if let Some((sender, _n)) = self.swarms.get(s_id) {
                                let _ = sender.send(ManagerToGnome::SetRunningPolicy(pol, req));
                            }
                        }
                    }
                    ToGnomeManager::SetRunningCapability(s_name, cap, v_gids) => {
                        eprintln!("GMgr rcvd SetRunningCapability for {s_name}");
                        if let Some(s_id) = self.name_to_id.get(&s_name) {
                            if let Some((sender, _n)) = self.swarms.get(s_id) {
                                let _ =
                                    sender.send(ManagerToGnome::SetRunningCapability(cap, v_gids));
                            }
                        }
                    }
                    ToGnomeManager::SetRunningByteSet(s_name, b_id, bset) => {
                        eprintln!("GMgr rcvd SetRunningByteSet for {s_name}");
                        if let Some(s_id) = self.name_to_id.get(&s_name) {
                            if let Some((sender, _n)) = self.swarms.get(s_id) {
                                let _ = sender.send(ManagerToGnome::SetRunningByteSet(b_id, bset));
                            }
                        }
                    }
                }
            }
            // sleep(sleep_time).await;
        }
        eprintln!("GnomeMgr is done");
        // self.finish();
        // join.await;
    }

    async fn serve_gnome_requests(
        &mut self,
        request: GnomeToManager,
    ) -> Option<Vec<(SwarmID, SwarmName)>> {
        let mut disconnected_swarms = vec![];
        // while let Ok(request) = self.receiver.try_recv() {
        match request {
            GnomeToManager::FounderDetermined(swarm_id, s_name) => {
                let _res = self
                    .resp_sender
                    .send(FromGnomeManager::SwarmFounderDetermined(
                        swarm_id,
                        s_name.clone(),
                    ))
                    .await;
                if s_name.founder != self.gnome_id {
                    eprintln!("Starting my own Catalog Swarm (after timeout)");
                    let my_sname = SwarmName::new(self.gnome_id, "/".to_string()).unwrap();
                    let message = ToGnomeManager::JoinSwarm(my_sname, None);
                    spawn(start_a_timer(
                        self.req_sender.clone(),
                        message,
                        Duration::from_secs(3),
                    ))
                    .detach();
                }
                eprintln!("Founder Det: {} {} {:?}", swarm_id, s_name, _res);
                eprintln!("existing: {:?}", self.name_to_id.keys());
                let mut g_name = SwarmName {
                    founder: GnomeId::any(),
                    name: "/".to_string(),
                };
                if let Some((sender, _old_name)) = self.swarms.remove(&swarm_id) {
                    self.swarms.insert(swarm_id, (sender, s_name.clone()));
                }
                if let Some(e_id) = self.name_to_id.remove(&g_name) {
                    let founder = s_name.founder;
                    if e_id == swarm_id {
                        eprintln!("Updating existing name_to_id mapping");
                        g_name = s_name;
                    }
                    self.name_to_id.insert(g_name, e_id);
                    let _ = self.to_networking.send(Notification::SetFounder(founder));
                } else {
                    eprintln!("No generic swarm name to overwrite");
                    self.name_to_id.insert(s_name, swarm_id);
                }
            }
            GnomeToManager::ProvidePublicAddress(s_id, conn_id, g_id) => {
                //we want to wait for updated network settings
                // from networking
                eprintln!("Received Pub Addr Request from Gnome");
                if let Some((sender, s_name)) = self.swarms.get(&s_id) {
                    if s_name.founder == g_id {
                        let ns = self.network_summary.get_network_settings_bytes();
                        eprintln!("Sending immediately {:?}", ns);
                        let _ =
                            sender.send(ManagerToGnome::ReplyNetworkSettings(ns, conn_id, g_id));
                    } else {
                        self.waiting_for_settings.push_back((s_id, conn_id, g_id));
                    }
                } else {
                    self.waiting_for_settings.push_back((s_id, conn_id, g_id));
                }
                eprintln!(
                    "waiting_for_settings size: {}",
                    self.waiting_for_settings.len()
                );
            }

            GnomeToManager::NeighboringSwarms(swarm_id, swarm_names) => {
                eprintln!("GMgr got info from {} about:", swarm_id,);
                let mut new_swarms_available = HashSet::new();
                let mut potential_new_members = vec![];
                for (g_id, s_n) in &swarm_names {
                    eprintln!("{}  {}", g_id, s_n);
                    if self.name_to_id.contains_key(&s_n) {
                        potential_new_members.push((s_n.clone(), *g_id, swarm_id));
                    }
                    if let Some(name_ids) = self.neighboring_gnomes.get_mut(g_id) {
                        name_ids.insert(s_n.clone());
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(s_n.clone());
                        self.neighboring_gnomes.insert(*g_id, h_set);
                    }
                    if let Some(n_ids) = self.neighboring_swarms.get_mut(s_n) {
                        n_ids.insert(*g_id);
                        //
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(*g_id);
                        self.neighboring_swarms.insert(s_n.clone(), h_set);
                    }
                    if !self.name_to_id.contains_key(&s_n) {
                        new_swarms_available.insert((s_n.clone(), *g_id, false));
                    }
                }
                // TODO: we also need to implement a way to update this list once
                // a neighbor gets dropped from a swarm
                //
                // for (gnome_id, swarm_name) in &swarm_names {
                //     if let Some((_s, neighbors)) = self.swarms.get_mut(&swarm_id) {
                //         if !neighbors.contains(&gnome_id) {
                //             eprintln!("Updating neighbor info");
                //             neighbors.push(*gnome_id);
                //             eprintln!("{} res: {:?}", swarm_id, neighbors);
                //             // } else {
                //             //     eprintln!("Ignoring, neighbor is already in this swarm");
                //             //     return None;
                //         }
                //     }
                //     let mut skip_notification = false;
                //     eprintln!(
                //         "All known neighboring swarms #{}",
                //         self.neighboring_swarms.len()
                //     );
                //     for name in self.neighboring_swarms.keys() {
                //         eprintln!("NS {}", name);
                //     }
                //     if let Some(list) = self.neighboring_swarms.get_mut(&swarm_name) {
                //         eprintln!("List of {} members:", swarm_name);
                //         for (s_id, g_id) in list.iter() {
                //             eprintln!("{} {}", s_id, g_id);
                //         }
                //         if !list.contains(&(swarm_id, *gnome_id)) {
                //             list.push((swarm_id, *gnome_id));
                //         } else if self.name_to_id.contains_key(&swarm_name) {
                //             // This is important, otherwise we will enter infinite loop
                //             eprintln!("Skipping in order to avoid infinite loop");
                //             skip_notification = true;
                //         } else {
                //             eprintln!("Name to id has no mapping for {}", swarm_name);
                //         }
                //     } else {
                //         eprintln!("GMgr adding {} to neighboring_swarms", swarm_name);
                //         self.neighboring_swarms
                //             .insert(swarm_name.clone(), vec![(swarm_id, *gnome_id)]);
                //     }
                //     if !skip_notification {
                //         let swarm_exists = self.name_to_id.get(&swarm_name).is_some();
                //         filtered_names.insert((swarm_name.clone(), *gnome_id, swarm_exists));
                //     }
                // }
                if new_swarms_available.is_empty() {
                    // try to find all Neighbors who are founders of
                    // any of neighboring_swarms that we are not currently
                    // connected to
                    for name in self.neighboring_swarms.keys() {
                        if self.name_to_id.contains_key(&name) {
                            continue;
                        }
                        for (gnome_id, swarm_name) in &swarm_names {
                            if name.founder == *gnome_id {
                                new_swarms_available.insert((swarm_name.clone(), *gnome_id, false));
                            }
                        }
                    }
                }
                for (s_name, g_id, via_s_id) in potential_new_members {
                    if let Some((sender, _name)) = self.swarms.get(&via_s_id) {
                        let _ = sender.send(ManagerToGnome::ProvideNeighborsToSwarm(s_name, g_id));
                    }
                }
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::NewSwarmsAvailable(
                        swarm_id,
                        new_swarms_available,
                        // swarm_exists,
                    ))
                    .await;
            }
            GnomeToManager::AddNeighborToSwarm(_swarm_id, swarm_name, new_neighbor) => {
                eprintln!("GMgr recvd add {} to {}", new_neighbor.id, swarm_name);
                // eprintln!("Founder: {:?}", swarm_name.founder.0.to_be_bytes());
                // for name in self.name_to_id.keys() {
                //     eprintln!("Key: {:?}", name.founder.0.to_be_bytes());
                // }
                if let Some(id) = self.name_to_id.get(&swarm_name) {
                    eprintln!("found this swarm!");
                    self.add_neighbor_to_a_swarm(*id, new_neighbor);
                    //TODO: notify gnome that provided new neighbor
                } else {
                    eprintln!("{} not found", swarm_name);
                }
            }
            GnomeToManager::GnomeLeft(s_id, s_name, g_id) => {
                if let Some(n_swarms) = self.neighboring_gnomes.get_mut(&g_id) {
                    n_swarms.remove(&s_name);
                }
                if let Some(n_gnomes) = self.neighboring_swarms.get_mut(&s_name) {
                    n_gnomes.remove(&g_id);
                }
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::SwarmNeighborLeft(s_id, g_id))
                    .await;
            }
            GnomeToManager::ActiveNeighbors(s_id, s_name, new_ids) => {
                if let Some(waiting_for_n) = self.waiting_for_neighbors.take() {
                    if s_id != waiting_for_n {
                        eprintln!("Seting back 2 waiting_for_neighbors ");
                        self.waiting_for_neighbors = Some(waiting_for_n);
                    }
                }
                for g_id in &new_ids {
                    if let Some(n_swarms) = self.neighboring_gnomes.get_mut(&g_id) {
                        n_swarms.insert(s_name.clone());
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(s_name.clone());
                        self.neighboring_gnomes.insert(*g_id, h_set);
                    }
                }
                self.neighboring_swarms.insert(s_name, new_ids.clone());
                //TODO: notify app mgr
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::SwarmNeighbors(s_id, new_ids.clone()))
                    .await;
                eprintln!("Neighbors updated");
                if let Some((s_name, n_opt)) = self.join_queue.pop_front() {
                    eprintln!("Joining from queue: {}", s_name);
                    let _ = self
                        .req_sender
                        .send(ToGnomeManager::JoinSwarm(s_name, n_opt))
                        .await;
                }
                // if let Some((sender, n_ids)) = self.swarms.get_mut(&s_id) {
                //     *n_ids = new_ids;
                // }
            }
            GnomeToManager::SwarmBusy(s_id, is_busy) => {
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::SwarmBusy(s_id, is_busy))
                    .await;
            }
            GnomeToManager::RunningPolicies(policies) => {
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::RunningPolicies(policies))
                    .await;
            }
            GnomeToManager::RunningCapabilities(caps) => {
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::RunningCapabilities(caps))
                    .await;
            }
            GnomeToManager::RunningByteSets(bsets) => {
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::RunningByteSets(bsets))
                    .await;
            }
            GnomeToManager::Disconnected(s_id, s_name) => {
                if let Some(n_set) = self.neighboring_swarms.remove(&s_name) {
                    for n_id in n_set {
                        if let Some(ns_set) = self.neighboring_gnomes.get_mut(&n_id) {
                            ns_set.remove(&s_name);
                        }
                    }
                }
                let mut disconnected_pair = None;
                for (name, id) in &self.name_to_id {
                    if *id == s_id {
                        disconnected_pair = Some((s_id, name.clone()));
                        break;
                    }
                }
                if let Some((s_id, name)) = disconnected_pair {
                    self.name_to_id.remove(&name);
                    disconnected_swarms.push((s_id, name));
                } else {
                    eprintln!("Unable to find name for {}", s_id);
                }
                self.swarms.remove(&s_id);
                if self.swarms.is_empty() {
                    // eprintln!("We should start a template swarm!");
                    // let _ = self
                    //     .req_sender
                    //     .send(ToGnomeManager::StartListeningSwarm)
                    //     .await;
                    let _ = self
                        .resp_sender
                        .send(FromGnomeManager::AllNeighborsGone)
                        .await;
                }
            }
        }
        if disconnected_swarms.is_empty() {
            None
        } else {
            Some(disconnected_swarms)
        }
    }

    pub fn add_neighbor_to_a_swarm(&mut self, id: SwarmID, neighbor: Neighbor) {
        if let Some((to_gnome, s_name)) = self.swarms.get_mut(&id) {
            let n_id = neighbor.id;
            match to_gnome.send(ManagerToGnome::AddNeighbor(neighbor)) {
                Ok(()) => {
                    // neighbors.push(n_id);
                    //TODO: not sure if we should update neighboring_swarms and _gnomes at this point
                    if let Some(n_gnomes) = self.neighboring_swarms.get_mut(&s_name) {
                        n_gnomes.insert(n_id);
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(n_id);
                        self.neighboring_swarms.insert(s_name.clone(), h_set);
                    }
                    if let Some(n_swarms) = self.neighboring_gnomes.get_mut(&n_id) {
                        n_swarms.insert(s_name.clone());
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(s_name.clone());
                        self.neighboring_gnomes.insert(n_id, h_set);
                    }
                    eprintln!("Added {} to existing swarm {}", n_id, id.0)
                }
                Err(e) => eprintln!("Failed adding neighbor to existing swarm: {:?}", e),
            }
        } else {
            eprintln!("No swarm with id: {:?}", id);
        }
    }
    fn notify_other_swarms(
        &self,
        swarm_id: SwarmID,
        swarm_name: SwarmName,
        aware_gnome: Option<GnomeId>,
    ) {
        //TODO: We want to send at most a single netification to each of our Neighbors
        // So we need to know what Neighbors are currently active under each Swarm
        // So we create a list of already notified neighbors containing current neighbors
        // of newly created swarm
        // Now we iterate our swarms and only send a notification if given swarm has
        // at least one neighbor that was not notified
        // In SwarmJoined we also include only those neighbors we want to be notified

        eprintln!("notify_other_swarms {}", swarm_id);
        for (n, idi) in self.name_to_id.iter() {
            eprintln!("name:{} id: {}", n, idi);
        }
        let mut already_notified = if let Some(n_set) = self.neighboring_swarms.get(&swarm_name) {
            n_set.clone()
            // let mut al_not = Vec::with_capacity(n_set.len());
            // for n_id in n_set {
            //     al_not.push(*n_id);
            // }
            // al_not
            // let mut already_notified = if let Some((_sender, neighbors)) = self.swarms.get(&swarm_id) {
            //     neighbors.clone()
        } else {
            // vec![]
            HashSet::new()
        };
        if let Some(aware_gnome) = aware_gnome {
            eprintln!("He knows: {}", aware_gnome);
            already_notified.insert(aware_gnome);
            // if !already_notified.contains(&aware_gnome) {
            //     already_notified.push(aware_gnome);
            // }
        }
        // eprintln!("Already notified: {}", already_notified.len());
        // for n_id in &already_notified {
        //     eprintln!("noti {}", n_id);
        // }
        let mut not_yet_notified = HashSet::new();
        for n_id in self.neighboring_gnomes.keys() {
            if !already_notified.contains(n_id) {
                // eprint!("yes");
                not_yet_notified.insert(*n_id);
            }
            // eprintln!("");
        }
        // eprintln!("not yet notified: ");
        // for nyn in &not_yet_notified {
        //     eprintln!("{}", nyn);
        // }
        // eprintln!("Swarms:");
        // for swarm in &self.swarms {
        //     eprintln!("{}", swarm.0);
        // }
        // eprintln!("NSwarms:");
        // for (swarm_name, n_ids) in self.neighboring_swarms.iter() {
        //     eprint!("{}: ", swarm_name);
        //     for n_id in n_ids {
        //         eprint!("{} ", n_id);
        //     }
        //     eprintln!("");
        // }
        // eprintln!("NGnomes:");
        // for (g_id, names) in self.neighboring_gnomes.iter() {
        //     eprint!("{}: ", g_id);
        //     for name in names {
        //         eprint!("{} ", name);
        //     }
        //     eprintln!("");
        // }
        if not_yet_notified.is_empty() {
            eprintln!("Not sending anything out - everyone is already notified");
            return;
        }
        for (id, (sender, cur_s_name)) in &self.swarms {
            if *id == swarm_id {
                eprintln!("omitting {}", swarm_id);
                continue;
            }
            // if *id != swarm_id {
            eprintln!("looping {}", cur_s_name);
            let mut n_to_notify = vec![];
            if let Some(cur_n_ids) = self.neighboring_swarms.get(cur_s_name) {
                eprintln!("inside");
                for n_id in cur_n_ids {
                    eprintln!("n {}", n_id);
                }
                let intersect = not_yet_notified.intersection(cur_n_ids).cloned();
                for n_id in intersect {
                    eprintln!("adding {} to notification", n_id);
                    n_to_notify.push(n_id);
                    already_notified.insert(n_id);
                }
                if n_to_notify.is_empty() {
                    continue;
                }
                for n_id in &n_to_notify {
                    eprintln!("removing {} from not_yet_notified", n_id);
                    not_yet_notified.remove(n_id);
                }
            } else {
                eprintln!("None");
            }
            // let mut not_yet_notified = vec![];
            // for n in n_ids {
            //     // eprintln!("testing n_id: {:?}", n);
            //     if !already_notified.contains(n) {
            //         // eprintln!("this one was not notified!");
            //         not_yet_notified.push(*n);
            //     }
            // }
            // if !not_yet_notified.is_empty() {
            eprintln!("Informing {} about new swarm", n_to_notify.len());
            let _ = sender.send(ManagerToGnome::SwarmJoined(
                swarm_name.clone(),
                n_to_notify, // not_yet_notified.clone(),
            ));
            if not_yet_notified.is_empty() {
                break;
            }
            // already_notified.append(&mut not_yet_notified);
            // }
            // }
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

    fn next_avail_neighboring_swarm(
        &self,
        exclude_swarm: Option<SwarmName>,
    ) -> Option<(SwarmName, Option<GnomeId>)> {
        eprintln!(
            "next_avail_neighboring_swarm (exclude: {:?})\n name_to_id:",
            exclude_swarm
        );
        let mut _excluded_swarm = if let Some(s_name) = &exclude_swarm {
            if s_name.founder.is_any() {
                None
            } else {
                Some((s_name.clone(), None))
            }
        } else {
            None
        };
        if self.swarms.is_empty() {
            eprintln!("Not connected to any swarm, returning template swarm");
            return Some((
                SwarmName::new(GnomeId::any(), "/".to_string()).unwrap(),
                None,
            ));
        }
        for key in self.name_to_id.keys() {
            eprintln!("{}", key);
        }
        // TODO: some better way to get a random name
        let mut randomizer = HashSet::new();
        // let mut exclude_neighbor = None;
        eprintln!("neighboring_swarms:");
        if let Some(exclude_name) = exclude_swarm.clone() {
            for (neighboring_swarm, neighbors) in &self.neighboring_swarms {
                eprintln!("{}", neighboring_swarm);
                if !self.name_to_id.contains_key(&neighboring_swarm)
                // &&
                {
                    if neighboring_swarm != &exclude_name {
                        let to_add = if let Some(neighbor) = neighbors.iter().next() {
                            (neighboring_swarm.clone(), Some(*neighbor))
                        } else {
                            (neighboring_swarm.clone(), None)
                        };
                        randomizer.insert(to_add);
                    } else if let Some(n) = neighbors.iter().next() {
                        _excluded_swarm = Some((exclude_name.clone(), Some(*n)));
                        // exclude_neighbor = Some(*n);
                    }
                }
            }
        } else {
            for (neighboring_swarm, neighbors) in &self.neighboring_swarms {
                eprintln!("{}", neighboring_swarm);
                if !self.name_to_id.contains_key(&neighboring_swarm) {
                    let to_add = if let Some(neighbor) = neighbors.iter().next() {
                        (neighboring_swarm.clone(), Some(*neighbor))
                    } else {
                        (neighboring_swarm.clone(), None)
                    };
                    randomizer.insert(to_add);
                }
            }
        }
        if randomizer.is_empty() {
            None
            // eprintln!("Returning excluded");
            // excluded_swarm
        } else {
            randomizer.into_iter().next()
        }
        // ;
        // if selection.is_none() {
        //     if let Some(e_swarm) = exclude_swarm {
        //         if !e_swarm.founder.is_any() {
        //             eprintln!("returning exclude");
        //             Some((e_swarm, exclude_neighbor))
        //         } else {
        //             None
        //         }
        //     } else {
        //         None
        //     }
        // } else {
        //     selection
        // }
    }

    pub fn join_a_swarm(
        &mut self,
        name: SwarmName,
        assigned_bandwidth: u64,
        neighbor_network_settings: Option<Vec<NetworkSettings>>,
        neighbors: Option<Vec<Neighbor>>,
    ) -> Result<(SwarmID, (Sender<ToGnome>, Receiver<GnomeToApp>)), String> {
        if let Some(swarm_id) = self.next_avail_swarm_id() {
            // let (band_send, band_recv) = channel();
            let (net_settings_send, net_settings_recv) = channel();
            if let Some(neighbor_settings) = neighbor_network_settings {
                let mut bytes = vec![];
                for setting in neighbor_settings {
                    eprintln!("Sending neighbor settings: {:?}", setting);
                    bytes.append(&mut setting.bytes());
                }
                if !bytes.is_empty() {
                    let _ = net_settings_send.send(bytes);
                }
            }
            let mgr_sender = self.sender.clone();
            let (send, recv) = channel();

            fn verify(
                gnome_id: GnomeId,
                key: &Vec<u8>,
                swarm_time: SwarmTime,
                data: &mut Vec<u8>,
                signature: &[u8],
            ) -> bool {
                // eprintln!("Verify time: {:?}", swarm_time);
                // println!("PubKey DER: '{:?}' len: {}", key, key.len());
                let res: std::result::Result<RsaPublicKey, rsa::pkcs1::Error> =
                    DecodeRsaPublicKey::from_pkcs1_der(key);
                eprintln!("SwarmTime: {:?}", swarm_time);
                // let res = DecodeRsaPublicKey::from_pkcs1_pem(&key[..key.len() - 4]);
                // let pub_key: RsaPublicKey;
                if let Ok(pub_key) = res {
                    // let mut hasher = DefaultHasher::new();
                    // pub_key.hash(&mut hasher);
                    // let id: u64 = hasher.finish();
                    // let pkbytes = pub_key.to_pkcs1_der().unwrap();
                    let id = sha_hash(key);
                    if id != gnome_id.0 {
                        eprintln!("Verify FAIL: GnomeId mismatch!");
                        return false;
                    }
                    // println!("PubKey decoded!");
                    // println!("Signature [{} bytes]:\n{:?}", signature.len(), signature);
                    let signature_three = Signature::try_from(signature).unwrap();

                    let verifier: VerifyingKey<Sha256> = VerifyingKey::new(pub_key);
                    // Include timestamp before signing
                    for st_byte in swarm_time.0.to_be_bytes() {
                        data.push(st_byte);
                    }
                    let mut result = verifier.verify(data, &signature_three);
                    eprintln!("Ver res: {:?}", result);

                    // TODO: dirty hack to check against two more neighboring SwarmTimes
                    let mut last_byte = swarm_time.0 + 1;
                    if result.is_err() {
                        data.pop();
                        data.pop();
                        data.pop();
                        data.pop();
                        for st_byte in last_byte.to_be_bytes() {
                            data.push(st_byte);
                        }
                        if last_byte > 1 {
                            last_byte -= 2;
                            result = verifier.verify(data, &signature_three);
                        }
                    }
                    eprintln!("Ver res 2: {:?}", result);
                    if result.is_err() {
                        data.pop();
                        data.pop();
                        data.pop();
                        data.pop();
                        for st_byte in last_byte.to_be_bytes() {
                            data.push(st_byte);
                        }
                        result = verifier.verify(data, &signature_three);
                    }
                    eprintln!("verify: {:?}", result);
                    // Remove timestamp
                    data.pop();
                    data.pop();
                    data.pop();
                    data.pop();
                    result.is_ok()
                } else {
                    eprintln!("Unable to decode PubKey");
                    false
                }
            }

            fn sign(
                priv_key_pem: &str,
                swarm_time: SwarmTime,
                data: &mut Vec<u8>,
            ) -> Result<Vec<u8>, ()> {
                // println!(
                //     "Sign time: {:?} blen: {}\nSign head: ",
                //     swarm_time,
                //     data.len()
                // );
                // for i in 0..20 {
                //     if let Some(byte) = data.get(i) {
                //         print!("{}-", byte);
                //     }
                // }
                // println!("Sign tail:");
                // for i in data.len() - 20..data.len() {
                //     if let Some(byte) = data.get(i) {
                //         print!("{}-", byte);
                //     }
                // }
                if let Ok(priv_key) = DecodeRsaPrivateKey::from_pkcs1_pem(priv_key_pem) {
                    let signer: SigningKey<Sha256> = SigningKey::new(priv_key);
                    // Include timestamp before signing
                    for st_byte in swarm_time.0.to_be_bytes() {
                        data.push(st_byte);
                    }
                    // let mut rng = OsRng;
                    let mut digest = <Sha256 as Digest>::new();
                    digest.update(&data);
                    use rsa::signature::DigestSigner;
                    let signature_result = signer.sign_digest(digest);
                    let signature_string = signature_result.to_string();
                    let mut bytes: Vec<u8> = vec![];
                    for i in (0..signature_string.len()).step_by(2) {
                        bytes.push(u8::from_str_radix(&signature_string[i..i + 2], 16).unwrap());
                    }
                    // println!("Signature [{} bytes]:\n{:?}", bytes.len(), bytes);
                    // Remove timestamp
                    data.pop();
                    data.pop();
                    data.pop();
                    data.pop();
                    Ok(bytes)
                } else {
                    Err(())
                }
            }
            let _n_ids = if let Some(ns) = &neighbors {
                let mut list = vec![];
                for n in ns {
                    list.push(n.id);
                }
                list
            } else {
                vec![]
            };
            let (sender, receiver) = Swarm::join(
                name.clone(),
                // app_sync_hash,
                swarm_id,
                self.gnome_id,
                self.pub_key_der.clone(),
                self.priv_key_pem.clone(),
                neighbors,
                mgr_sender,
                recv,
                // band_recv,
                net_settings_send,
                // self.network_settings,
                assigned_bandwidth,
                verify,
                sign,
                sha_hash,
            );
            // println!("swarm '{}' created ", name);
            // let sender = swarm.sender.clone();
            // let receiver = swarm.receiver.take();
            // eprintln!("{} Joined `{}`", swarm_id, name);
            self.notify_networking(
                name.clone(),
                sender.clone(),
                // band_send,
                net_settings_recv,
            );
            eprintln!("inserting swarm");
            self.swarms.insert(swarm_id, (send, name.clone()));
            self.name_to_id.insert(name, swarm_id);
            Ok((swarm_id, (sender, receiver)))
        } else {
            Err("Could not connect to a swarm, all SwarmIDs taken".to_string())
        }
    }

    pub fn notify_networking(
        &mut self,
        swarm_name: SwarmName,
        sender: Sender<ToGnome>,
        // avail_bandwith_sender: Sender<u64>,
        // network_settings_receiver: Receiver<NetworkSettings>,
        network_settings_receiver: Receiver<Vec<u8>>,
    ) {
        eprintln!("Notifying network about {}", swarm_name);
        let _r = self
            .to_networking
            .send(Notification::AddSwarm(NotificationBundle {
                swarm_name,
                request_sender: sender,
                // token_sender: avail_bandwith_sender,
                network_settings_receiver,
            }));
        // eprintln!("Network notified: {:?}", r);
    }

    pub fn print_status(&self, id: &SwarmID) {
        if let Some((mgr_send, _ns)) = self.swarms.get(id) {
            let _ = mgr_send.send(ManagerToGnome::Status);
        }
    }

    pub fn get_sender(&self) -> ASender<ToGnomeManager> {
        self.req_sender.clone()
    }

    pub fn finish(self) {
        for (id, (mgr_sender, _ns)) in self.swarms.into_iter() {
            let _ = mgr_sender.send(ManagerToGnome::Disconnect);
            // let jh = swarm.join_handle.take().unwrap();
            // let _ = jh.join();
            eprintln!("Leaving SwarmID:`{:?}`", id);
        }
        // drop(self)
    }
}
pub async fn start_a_timer(
    sender: ASender<ToGnomeManager>,
    message: ToGnomeManager,
    timeout: Duration,
) {
    // let timeout = Duration::from_secs(5);
    // sleep(timeout).await;
    Timer::after(timeout).await;
    // eprintln!("Timeout for {} is over", message);
    let _ = sender.send(message).await;
}
