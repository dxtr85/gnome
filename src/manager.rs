use crate::crypto::Decrypter;
use async_std::task::sleep;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;
use swarm_consensus::NetworkSettings;
use swarm_consensus::SwarmTime;
// use crate::gnome::NetworkSettings;
// use crate::swarm::{Swarm, SwarmID};
use crate::NotificationBundle;
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
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::GnomeToManager;
use swarm_consensus::ManagerToGnome;
use swarm_consensus::{GnomeId, Neighbor};
use swarm_consensus::{GnomeToApp, ToGnome};
use swarm_consensus::{Swarm, SwarmID};

pub enum ToGnomeManager {
    JoinSwarm(String),
    Disconnect,
}
pub enum FromGnomeManager {
    SwarmJoined(SwarmID, String, Sender<ToGnome>, Receiver<GnomeToApp>),
}
pub struct Manager {
    gnome_id: GnomeId,
    pub_key_der: Vec<u8>,
    priv_key_pem: String,
    sender: Sender<GnomeToManager>,
    receiver: Receiver<GnomeToManager>,
    swarms: HashMap<SwarmID, Sender<ManagerToGnome>>,
    neighboring_swarms: HashMap<String, Vec<(SwarmID, GnomeId)>>,
    name_to_id: HashMap<String, SwarmID>,
    network_settings: NetworkSettings,
    to_networking: Sender<NotificationBundle>,
    decrypter: Decrypter,
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
        to_networking: Sender<NotificationBundle>,
        decrypter: Decrypter,
    ) -> Manager {
        let network_settings = network_settings.unwrap_or_default();
        let (sender, receiver) = channel();
        Manager {
            gnome_id,
            pub_key_der,
            priv_key_pem,
            sender,
            receiver,
            swarms: HashMap::new(),
            neighboring_swarms: HashMap::new(),
            name_to_id: HashMap::new(),
            network_settings,
            to_networking, // Send a message to networking about new swarm subscription, and where to send Neighbors
            decrypter,
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
        req_receiver: Receiver<ToGnomeManager>,
        resp_sender: Sender<FromGnomeManager>,
        app_sync_hash: u64,
    ) {
        // let min_sleep_nanos: u64 = 1 << 7;
        // let max_sleep_nanos: u64 = 1 << 27;
        let sleep_nanos: u64 = 1 << 25;
        loop {
            if let Ok(request) = req_receiver.try_recv() {
                match request {
                    // ManagerRequest::UpdateAppRootHash(swarm_id, new_app_hash) => {
                    //     println!("Manager updating hash to {}", new_app_hash);
                    //     if let Some(sender) = self.swarms.get(&swarm_id) {
                    //         let _ = sender.send(ManagerToGnome::UpdateAppRootHash(new_app_hash));
                    //     }
                    // }
                    ToGnomeManager::JoinSwarm(swarm_name) => {
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
                            self.join_a_swarm(swarm_name.clone(), app_sync_hash, None, None)
                        {
                            self.notify_other_swarms(swarm_id, swarm_name.clone());
                            let _ = resp_sender.send(FromGnomeManager::SwarmJoined(
                                swarm_id, swarm_name, user_req, user_res,
                            ));
                        } else {
                            println!("Unable to join a swarm.");
                        }
                    }
                    ToGnomeManager::Disconnect => {
                        // TODO: do all necessary stuff
                        break;
                    }
                }
            }
            self.serve_gnome_requests();
            let sleep_time = Duration::from_nanos(sleep_nanos);
            sleep(sleep_time).await;
        }
        self.finish();
        // join.await;
    }

    fn serve_gnome_requests(&mut self) {
        while let Ok(request) = self.receiver.try_recv() {
            match request {
                GnomeToManager::NeighboringSwarm(swarm_id, gnome_id, swarm_name) => {
                    println!("Manager got info about a swarm: '{}'", swarm_name);
                    // println!("DER size: {}", self.pub_key_der.len());
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
        app_sync_hash: u64,
        neighbor_network_settings: Option<NetworkSettings>,
        neighbors: Option<Vec<Neighbor>>,
    ) -> Result<(SwarmID, (Sender<ToGnome>, Receiver<GnomeToApp>)), String> {
        if let Some(swarm_id) = self.next_avail_swarm_id() {
            let (band_send, band_recv) = channel();
            let (net_settings_send, net_settings_recv) = channel();
            if let Some(neighbor_settings) = neighbor_network_settings {
                let _ = net_settings_send.send(neighbor_settings);
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
                println!("Verify time: {:?}", swarm_time);
                // println!("PubKey DER: '{:?}' {}", key, key.len());
                let res: std::result::Result<RsaPublicKey, rsa::pkcs1::Error> =
                    DecodeRsaPublicKey::from_pkcs1_der(key);
                // println!("decode res: {:?}", res);
                // let res = DecodeRsaPublicKey::from_pkcs1_pem(&key[..key.len() - 4]);
                // let pub_key: RsaPublicKey;
                if let Ok(pub_key) = res {
                    let mut hasher = DefaultHasher::new();
                    pub_key.hash(&mut hasher);
                    let id: u64 = hasher.finish();
                    if id != gnome_id.0 {
                        println!("Verify FAIL: GnomeId mismatch!");
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
                    // println!("Ver res: {:?}", result);

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
                    // println!("Ver res 2: {:?}", result);
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
                    // println!("verify: {:?}", result);
                    // Remove timestamp
                    data.pop();
                    data.pop();
                    data.pop();
                    data.pop();
                    result.is_ok()
                } else {
                    println!("Unable to decode PubKey");
                    false
                }
            }

            fn sign(
                priv_key_pem: &str,
                swarm_time: SwarmTime,
                data: &mut Vec<u8>,
            ) -> Result<Vec<u8>, ()> {
                println!("Sign time: {:?}", swarm_time);
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
            let (sender, receiver) = Swarm::join(
                name.clone(),
                app_sync_hash,
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
        sender: Sender<ToGnome>,
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
