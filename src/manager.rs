use crate::crypto::sha_hash;
use crate::crypto::Decrypter;
use async_std::task::sleep;
// use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::IpAddr;
use std::time::Duration;
use swarm_consensus::{Nat, SwarmName};
use swarm_consensus::{NetworkSettings, Notification};
use swarm_consensus::{PortAllocationRule, SwarmTime};
// use crate::gnome::NetworkSettings;
// use crate::swarm::{Swarm, SwarmID};
use crate::NotificationBundle;
use async_std::channel::Receiver as AReceiver;
use async_std::channel::Sender as ASender;
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
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeToManager;
use swarm_consensus::ManagerToGnome;
use swarm_consensus::{GnomeId, Neighbor};
use swarm_consensus::{GnomeToApp, ToGnome};
use swarm_consensus::{Swarm, SwarmID};

pub enum ToGnomeManager {
    JoinRandom(Option<SwarmName>), // We ask GMgr to join a swarm of his own selection
    JoinSwarm(SwarmName, Option<GnomeId>), //GID represents source of information about this new swarm
    ExtendSwarm(SwarmName, SwarmID, GnomeId), // SwarmID has GnomeId who belongs to SwarmName
    // LeaveSwarm(SwarmName),
    LeaveSwarm(SwarmID),
    FromGnome(GnomeToManager),
    ProvideNeighboringSwarms(SwarmID),
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
    MyPublicIPs(Vec<(IpAddr, u16, Nat, (PortAllocationRule, i8))>),
    Disconnected(Vec<(SwarmID, SwarmName)>),
}
enum ChannelAvailability {
    Unknown,
    Supported,
    Available(IpAddr, u16, Nat, (PortAllocationRule, i8)),
    NotAvailable,
}
// bool in each transport says if gnome's networking service has tried to bind it
struct CommunicationChannels {
    udp: (bool, ChannelAvailability),
    tcp: (bool, ChannelAvailability),
    udpv6: (bool, ChannelAvailability),
    tcpv6: (bool, ChannelAvailability),
}
impl CommunicationChannels {
    fn get_public_comm_channels(
        &mut self,
        ip: IpAddr,
        port: u16,
        nat: Nat,
        port_rule: (PortAllocationRule, i8),
    ) -> Vec<(IpAddr, u16, Nat, (PortAllocationRule, i8))> {
        // eprintln!("get public comm channels: {}:{} {:?}", ip, port, nat);
        //TODO: we need to hold a structure for all 4 possible IP/Transport
        // combinations.
        // First we should receive info whether or not given pair is supported
        // by this host
        // Later, for those supported we are expected to receive our public IP&Port
        // Once we have all those public facing data we can send it to a swarm
        // that we are founder of in order for it to update Manifest
        // We currently only use STUN via UDP to query about public IP, so in some
        // cases this might fail to provide us with required data.
        //
        // if port == 0 then if ip = 0.0.0.2 then network failed to bind udp socket
        // if port == 1 then if ip = 0.0.0.2 then network bind udp socket is ok
        // if port == 0 then if ip = ::2 then network failed to bind udpv6 socket
        // if port == 0 then if ip = ::2 then network bind udpv6 socket is ok
        // if port == 0 then if ip = 0.0.0.3 then network failed to bind tcp socket
        // if port == 1 then if ip = 0.0.0.3 then network bind tcp socket is ok
        // if port == 0 then if ip = ::3 then network failed to bind tcpv6 socket
        // if port == 0 then if ip = ::3 then network bind tcpv6 socket is ok
        // other port value means that we have a public ip and port
        // we assign those values to both udp & tcp (or udpv6 & tcpv6),
        // since over tcp we do not currently have any pub ip discovery logic
        // match port {
        //     0 => {
        let (ipv6, oct) = match ip {
            IpAddr::V4(ip) => (false, *ip.octets().last().take().unwrap()),
            IpAddr::V6(ip) => (true, *ip.octets().last().take().unwrap()),
        };
        if port < 2 {
            match oct {
                2 => {
                    if port == 0 {
                        if ipv6 {
                            //TODO: udpv6 disabled
                            self.udpv6 = (true, ChannelAvailability::NotAvailable);
                        } else {
                            //TODO: udp disabled
                            self.udp = (true, ChannelAvailability::NotAvailable);
                        }
                    } else if port == 1 {
                        if ipv6 {
                            //TODO: udpv6 enabled
                            self.udpv6 = (true, ChannelAvailability::Supported);
                        } else {
                            //TODO: udp enabled
                            self.udp = (true, ChannelAvailability::Supported);
                        }
                    } else {
                        eprintln!("Unexpected port value: {}", port);
                    }
                }
                3 => {
                    if port == 0 {
                        if ipv6 {
                            //TODO: tcpv6 disabled
                            self.tcpv6 = (true, ChannelAvailability::NotAvailable);
                        } else {
                            //TODO: tcp disabled
                            self.tcp = (true, ChannelAvailability::NotAvailable);
                        }
                    } else if port == 1 {
                        if ipv6 {
                            //TODO: tcpv6 enabled
                            self.tcpv6 = (true, ChannelAvailability::Supported);
                        } else {
                            //TODO: tcp enabled
                            self.tcp = (true, ChannelAvailability::Supported);
                        }
                    } else {
                        eprintln!("Unexpected port value: {}", port);
                    }
                }
                other => {
                    eprintln!("Unexpected last ip octet value: {}", other);
                }
            }
        } else if ipv6 {
            if self.udpv6.0 {
                match nat {
                    Nat::None | Nat::FullCone => {
                        self.udpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
                    }
                    other => self.udpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule),
                }
            }
            if self.tcpv6.0 {
                match nat {
                    Nat::None | Nat::FullCone => {
                        self.tcpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
                    }
                    other => self.tcpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule),
                }
            } else {
                if self.udp.0 {
                    match nat {
                        Nat::None | Nat::FullCone => {
                            self.udp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
                        }
                        other => {
                            self.udp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
                        }
                    }
                }
                if self.tcp.0 {
                    match nat {
                        Nat::None | Nat::FullCone => {
                            self.tcp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
                        }
                        other => {
                            self.tcp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
                        }
                    }
                }
            }
        }
        //TODO: add logic to check if all 4 conn_channels sent any response
        // if so, we check if there are any public ip assigned
        // if so, we send this info to our swarm
        // This might result in Manifest changing multiple times,
        // but for now it should be fine
        let mut comm_channels = vec![];
        // Only when we hear back from all possible networking channel services
        if self.udp.0 && self.tcp.0 && self.udpv6.0 && self.tcpv6.0 {
            if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udp.1 {
                comm_channels.push((ip, port, nat, port_rule));
            }
            if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udpv6.1 {
                comm_channels.push((ip, port, nat, port_rule));
            }
        }
        comm_channels
    }
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
    comm_channels: CommunicationChannels,
    network_settings: NetworkSettings,
    to_networking: Sender<Notification>,
    decrypter: Decrypter,
    req_receiver: AReceiver<ToGnomeManager>,
    req_sender: ASender<ToGnomeManager>,
    resp_sender: ASender<FromGnomeManager>,
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
        network_settings: Option<NetworkSettings>,
        to_networking: Sender<Notification>,
        decrypter: Decrypter,
        req_receiver: AReceiver<ToGnomeManager>,
        req_sender: ASender<ToGnomeManager>,
        resp_sender: ASender<FromGnomeManager>,
        sender: Sender<GnomeToManager>,
    ) -> Manager {
        let network_settings = network_settings.unwrap_or_default();
        let comm_channels = CommunicationChannels {
            udp: (false, ChannelAvailability::Unknown),
            tcp: (false, ChannelAvailability::Unknown),
            udpv6: (false, ChannelAvailability::Unknown),
            tcpv6: (false, ChannelAvailability::Unknown),
        };
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
            comm_channels,
            network_settings,
            to_networking, // Send a message to networking about new swarm subscription, and where to send Neighbors
            decrypter,
            req_receiver,
            req_sender,
            resp_sender,
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
        // req_receiver: Receiver<ToGnomeManager>,
        // resp_sender: Sender<FromGnomeManager>,
        // app_sync_hash: u64,
    ) {
        // let min_sleep_nanos: u64 = 1 << 7;
        // let max_sleep_nanos: u64 = 1 << 27;
        let sleep_nanos: u64 = 1 << 25;
        let mut quit = false;
        let mut prev_join_swarm = SwarmName::new(GnomeId(1), String::new()).unwrap();
        loop {
            if let Ok(request) = self.req_receiver.recv().await {
                match request {
                    ToGnomeManager::JoinRandom(exclude_swarm_opt) => {
                        eprintln!(
                            "JoinRandom request (exclude: {})",
                            exclude_swarm_opt.is_some()
                        );
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
                        eprintln!("Manager Received JoinSwarm({})", swarm_name);
                        if self.name_to_id.get(&swarm_name).is_some() {
                            eprintln!("Not joining {}, we are already there", swarm_name);
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
                            eprintln!("Joining {}… ", swarm_name,);
                        }
                        if let Ok((swarm_id, (user_req, user_res))) =
                            self.join_a_swarm(swarm_name.clone(), None, None)
                        {
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
                                .send(Notification::RemoveSwarm(s_name.clone()));
                            let _ = to_gnome.send(ManagerToGnome::Disconnect);
                        } else {
                            eprintln!("Can not leave {} - not found in Swarms set", s_id);
                        }
                        // } else {
                        //     eprintln!("Unable to find SwarmID for {} ", s_name);
                        // }
                    }
                    ToGnomeManager::FromGnome(request) => {
                        // eprintln!("GMgr Got {:?}", request);
                        let disconnected_opt = self.serve_gnome_requests(request).await;
                        if let Some(disconnected_swarms) = disconnected_opt {
                            let _ = self
                                .resp_sender
                                .send(FromGnomeManager::Disconnected(disconnected_swarms))
                                .await;
                            if quit {
                                eprintln!("quit is true");
                                let sleep_time = Duration::from_nanos(sleep_nanos);
                                sleep(sleep_time).await;
                                break;
                            }
                        }
                    }
                    ToGnomeManager::Quit => {
                        eprintln!("GMgr Got quit");
                        for (s_id, (to_gnome, _neighbors)) in &self.swarms {
                            eprintln!("Sending disconnect request to swarm: {:?}", s_id);
                            let _ = to_gnome.send(ManagerToGnome::Disconnect);
                        }
                        // eprintln!("Swarms: {:?}", self.swarms);
                        // if self.swarms.is_empty() {
                        //     self.resp_sender.send(FromGnomeManager::Disconnected).await;
                        // }
                        quit = true;
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
                eprintln!("Founder Det: {} {} {:?}", swarm_id, s_name, _res);
                eprintln!("existing: {:?}", self.name_to_id.keys());
                let mut g_name = SwarmName {
                    founder: GnomeId::any(),
                    name: "/".to_string(),
                };
                if let Some((sender, old_name)) = self.swarms.remove(&swarm_id) {
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
            GnomeToManager::PublicAddress(ip, port, nat, port_rule) => {
                let pub_ips = self
                    .comm_channels
                    .get_public_comm_channels(ip, port, nat, port_rule);
                if !pub_ips.is_empty() {
                    let _ = self
                        .resp_sender
                        .send(FromGnomeManager::MyPublicIPs(pub_ips))
                        .await;
                }
            }
            GnomeToManager::NeighboringSwarms(swarm_id, swarm_names) => {
                eprintln!("GMgr got info from {} about:", swarm_id,);
                let mut filtered_names = HashSet::new();
                for (g_id, s_n) in &swarm_names {
                    eprintln!("{}  {}", g_id, s_n);
                    if let Some(name_ids) = self.neighboring_gnomes.get_mut(g_id) {
                        name_ids.insert(s_n.clone());
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(s_n.clone());
                        self.neighboring_gnomes.insert(*g_id, h_set);
                    }
                    if let Some(n_ids) = self.neighboring_swarms.get_mut(s_n) {
                        n_ids.insert(*g_id);
                    } else {
                        let mut h_set = HashSet::new();
                        h_set.insert(*g_id);
                        self.neighboring_swarms.insert(s_n.clone(), h_set);
                    }
                    if !self.name_to_id.contains_key(&s_n) {
                        filtered_names.insert((s_n.clone(), *g_id, false));
                    }
                }
                // eprintln!("Name_to_id mapping:");
                // for (name, s_id) in self.name_to_id.iter() {
                //     eprintln!("{} {} ", s_id, name);
                // }
                // println!("DER size: {}", self.pub_key_der.len());
                //
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
                if filtered_names.is_empty() {
                    // try to find all Neighbors who are founders of
                    // any of neighboring_swarms that we are not currently
                    // connected to
                    for name in self.neighboring_swarms.keys() {
                        if self.name_to_id.contains_key(&name) {
                            continue;
                        }
                        for (gnome_id, swarm_name) in &swarm_names {
                            if name.founder == *gnome_id {
                                filtered_names.insert((swarm_name.clone(), *gnome_id, false));
                            }
                        }
                    }
                }
                let _ = self
                    .resp_sender
                    .send(FromGnomeManager::NewSwarmsAvailable(
                        swarm_id,
                        filtered_names,
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
                self.resp_sender
                    .send(FromGnomeManager::SwarmNeighborLeft(s_id, g_id))
                    .await;
            }
            GnomeToManager::ActiveNeighbors(s_id, s_name, new_ids) => {
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
            GnomeToManager::Disconnected(s_id, s_name) => {
                // for (name, id) in self.name_to_id.iter() {
                //     if id.0 == s_id.0 {
                // eprintln!("Requesting networking to remove {}", s_name);
                // let _ = self
                //     .to_networking
                //     .send(Notification::RemoveSwarm(s_name.clone()));
                //     }
                // }
                // for ns_list in self.neighboring_swarms.values_mut() {
                //     if ns_list.is_empty() {
                //         continue;
                //     }
                //     for i in ns_list.len() - 1..=0 {
                //         if ns_list[i].0 == s_id {
                //             eprintln!("remove ns: {:?}", ns_list[i]);
                //             ns_list.remove(i);
                //         }
                //     }
                // }
                // let mut remove_swarm = false;
                // if new_ids.is_empty() {
                // remove_swarm = true;
                // let _ = sender.send(ManagerToGnome::Disconnect);
                let mut disconnected_pair = None;
                // let mut name_to_remove = None;
                for (name, id) in &self.name_to_id {
                    if *id == s_id {
                        // name_to_remove = Some(name.clone());
                        disconnected_pair = Some((s_id, name.clone()));
                        break;
                    }
                }
                // }
                if let Some((s_id, name)) = disconnected_pair {
                    self.name_to_id.remove(&name);
                    disconnected_swarms.push((s_id, name));
                } else {
                    eprintln!("Unable to find name for {}", s_id);
                }
                // }
                // }
                // if remove_swarm {
                self.swarms.remove(&s_id);
                // }
            }
        }
        // }
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
            already_notified.insert(aware_gnome);
            // if !already_notified.contains(&aware_gnome) {
            //     already_notified.push(aware_gnome);
            // }
        }
        eprintln!("Already notified: {}", already_notified.len());
        for n_id in &already_notified {
            eprintln!("noti {}", n_id);
        }
        let mut not_yet_notified = HashSet::new();
        for n_id in self.neighboring_gnomes.keys() {
            if !already_notified.contains(n_id) {
                not_yet_notified.insert(*n_id);
            }
        }
        eprintln!("not yet notified: ");
        for nyn in &not_yet_notified {
            eprintln!("{}", nyn);
        }
        eprintln!("Swarms:");
        for swarm in &self.swarms {
            eprintln!("{}", swarm.0);
        }
        eprintln!("NSwarms:");
        for (swarm_name, n_ids) in self.neighboring_swarms.iter() {
            eprint!("{}: ", swarm_name);
            for n_id in n_ids {
                eprint!("{} ", n_id);
            }
            eprintln!("");
        }
        eprintln!("NGnomes:");
        for (g_id, names) in self.neighboring_gnomes.iter() {
            eprint!("{}: ", g_id);
            for name in names {
                eprint!("{} ", name);
            }
            eprintln!("");
        }
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
        eprintln!("next_avail_neighboring_swarm\n name_to_id:");
        for key in self.name_to_id.keys() {
            eprintln!("{}", key);
        }
        // TODO: some better way to get a random name
        let mut randomizer = HashSet::new();
        eprintln!("neighboring_swarms:");
        if let Some(exclude_name) = exclude_swarm {
            for (neighboring_swarm, neighbors) in &self.neighboring_swarms {
                eprintln!("{}", neighboring_swarm);
                if !self.name_to_id.contains_key(&neighboring_swarm)
                    && neighboring_swarm != &exclude_name
                {
                    let to_add = if let Some(neighbor) = neighbors.iter().next() {
                        (neighboring_swarm.clone(), Some(*neighbor))
                    } else {
                        (neighboring_swarm.clone(), None)
                    };
                    randomizer.insert(to_add);
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
        randomizer.into_iter().next()
    }

    pub fn join_a_swarm(
        &mut self,
        name: SwarmName,
        // app_sync_hash: u64,
        neighbor_network_settings: Option<Vec<NetworkSettings>>,
        neighbors: Option<Vec<Neighbor>>,
    ) -> Result<(SwarmID, (Sender<ToGnome>, Receiver<GnomeToApp>)), String> {
        if let Some(swarm_id) = self.next_avail_swarm_id() {
            let (band_send, band_recv) = channel();
            let (net_settings_send, net_settings_recv) = channel();
            if let Some(neighbor_settings) = neighbor_network_settings {
                for setting in neighbor_settings {
                    eprintln!("Sending neighbor settings: {:?}", setting);
                    let _ = net_settings_send.send(setting);
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
            let n_ids = if let Some(ns) = &neighbors {
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
                band_recv,
                net_settings_send,
                self.network_settings,
                verify,
                sign,
                sha_hash,
            );
            // println!("swarm '{}' created ", name);
            // let sender = swarm.sender.clone();
            // let receiver = swarm.receiver.take();
            // eprintln!("{} Joined `{}`", swarm_id, name);
            self.notify_networking(name.clone(), sender.clone(), band_send, net_settings_recv);
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
        avail_bandwith_sender: Sender<u64>,
        network_settings_receiver: Receiver<NetworkSettings>,
    ) {
        eprintln!("Notifying network about {}", swarm_name);
        let r = self
            .to_networking
            .send(Notification::AddSwarm(NotificationBundle {
                swarm_name,
                request_sender: sender,
                token_sender: avail_bandwith_sender,
                network_settings_receiver,
            }));
        // eprintln!("Network notified: {:?}", r);
    }

    pub fn print_status(&self, id: &SwarmID) {
        if let Some((mgr_send, _ns)) = self.swarms.get(id) {
            let _ = mgr_send.send(ManagerToGnome::Status);
        }
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
