#![allow(clippy::unusual_byte_groupings)]
use core::panic;
use std::fmt;
use std::ops::Deref;

use crate::networking::Nat;
use crate::networking::NetworkSettings;
use crate::networking::PortAllocationRule;
use crate::networking::Transport;
use std::error::Error;
use std::net::IpAddr;
use swarm_consensus::BlockID;
use swarm_consensus::Capabilities;
use swarm_consensus::CastData;
use swarm_consensus::CastID;
use swarm_consensus::CastMessage;
use swarm_consensus::Configuration;
use swarm_consensus::GnomeId;
use swarm_consensus::Header;
use swarm_consensus::Message;
use swarm_consensus::NeighborRequest;
use swarm_consensus::NeighborResponse;
use swarm_consensus::Neighborhood;
use swarm_consensus::Payload;
use swarm_consensus::Policy;
use swarm_consensus::Requirement;
use swarm_consensus::Signature;
use swarm_consensus::SwarmID;
use swarm_consensus::SwarmName;
use swarm_consensus::SwarmSyncRequestParams;
use swarm_consensus::SwarmSyncResponse;
use swarm_consensus::SwarmTime;
use swarm_consensus::SwarmType;
use swarm_consensus::SyncData;

// Every received Dgram starts with message identification header
// (and optional second byte for casting messages).
// This byte is used to send local Swarm ID (0-63) and info whether it is
// a regular Message or one of Casts (Uni/Multi/Broad).
// In case it is a Cast, next byte is CastID:
// TTIIIIII[CCCCCCCC] (and later HTPNNNNNSS... but only for Regular Message)
// TT - Datagram Type 00 - Regular Message
//                    01 - Unicast
//                    10 - Multicast
//                    11 - Broadcast
//   IIIIII - local Swarm identification in order to support up to 64 different Swarms
//            on a single UPD socket
//         CCCCCCCC - optional CastID, only when TT is not Regular Message
//
// With above casting messages are independent from Sync mechanism.
//
//
// If we have received a regular message, it starts with following:
//
// 123456789012345678901234567890123456789012345678
// _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _
// HTPNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSRRRRRRRR
// HTP    = header: 000  Sync,           payload: KeepAlive
//                  010  Sync,           payload: Bye
//                  100  Reconf,         payload: KeepAlive
//                  101  Reconf,         payload: Reconfigure
//                  110  Block,          payload: KeepAlive
//                  111  Block,          payload: Block
//
// remaining ten possible bit configs are currently UNIMPLEMENTED
//
//    NNNNN = Neighborhood value
//         SS... = SwarmTime value
//           RRR = Reconfigure byte (when payload = Reconfigure)
//              ___ rest of Header, Payload with Signature...
// With above we can send Reconf/Block only once per neighbor.
// When we know our neighbor is aware of that data, we save bandwith
// First bit set to 1 indicates we should read more bytes
// to identify Reconf(second bit is 0)/Block(second bit is 1) header.
// Only then we read bytes for Payload.

// pub fn bytes_to_message(bytes: &[u8]) -> Result<Message, ConversionError> {
pub fn bytes_to_message(bytes: Vec<u8>) -> Result<Message, ConversionError> {
    // eprintln!("Bytes to message: {:?}", bytes.len());
    // println!("decoding: {:#08b} {:?}", bytes[0], bytes);
    // let bytes_len = bytes.len();
    let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[bytes[1], bytes[2], bytes[3], bytes[4]]));
    let neighborhood: Neighborhood = Neighborhood(bytes[0] & 0b0_000_1111);
    let mut gnome_id: GnomeId = GnomeId(0);
    let (header, mut data_idx) = if bytes[0] & 0b1_000_0000 == 128 {
        if bytes[0] & 0b0110_0000 == 96 {
            let block_id: u64 = as_u64_be(&[
                bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12],
            ]);
            (Header::Block(BlockID(block_id)), 21)
        } else if bytes[0] & 0b0110_0000 == 64 {
            let block_id: u64 = as_u64_be(&[
                bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12],
            ]);
            (Header::Block(BlockID(block_id)), 13)
        } else if bytes[0] & 0b0110_0000 == 32 {
            gnome_id = GnomeId(as_u64_be(&[
                bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13],
            ]));
            // println!("Reconf byte: {}", bytes[5]);
            // println!("Decoded GID: {}", gnome_id);
            (Header::Reconfigure(bytes[5], gnome_id), 5)
        } else {
            gnome_id = GnomeId(as_u64_be(&[
                bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13],
            ]));
            // println!("Reconf byte: {}", bytes[5]);
            // println!("GnomeId: {}", gnome_id);
            (Header::Reconfigure(bytes[5], gnome_id), 14)
        }
    // } else if bytes[0] & 0b0_111_0000 == 112 {
    } else {
        // if bytes[5] == 255 {
        (Header::Sync, 5)
    };
    // let payload: Payload = if bytes[0] & 0b0_111_0000 == 112 {
    let payload: Payload = if header.is_sync() {
        if bytes[0] & 0b0110_0000 == 0 {
            let bandwith: u64 = as_u64_be(&[
                bytes[data_idx],
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
            ]);
            Payload::KeepAlive(bandwith)
        } else if bytes[0] & 0b0110_0000 == 64 {
            Payload::Bye
        } else {
            panic!("Uncovered header value: {}", bytes[0]);
        }
    } else if header.is_reconfigure() {
        // println!("Reconf header");
        if bytes[0] & 0b0010_0000 == 0 {
            // println!("Reconf KA");
            let bandwith: u64 = as_u64_be(&[
                bytes[data_idx],
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
            ]);
            Payload::KeepAlive(bandwith)
        // } else if bytes[0] & 0b0010_0000 == 32 {
        } else {
            // println!("Reconf conf {} ({})", bytes[data_idx], bytes.len());
            // println!("Configuration! {}", bytes[data_idx]);
            // println!("Configuration! {:?}", bytes);
            let config = match bytes[data_idx] {
                254 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    // eprintln!("StartBroadcast config {}, {:?}", gnome_id, c_id);
                    data_idx += 10;
                    Configuration::StartBroadcast(gnome_id, c_id)
                }
                253 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    data_idx += 10;
                    Configuration::ChangeBroadcastOrigin(gnome_id, c_id)
                }
                252 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    data_idx += 10;
                    Configuration::EndBroadcast(gnome_id, c_id)
                }
                251 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    data_idx += 10;
                    Configuration::StartMulticast(gnome_id, c_id)
                }
                250 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    data_idx += 10;
                    Configuration::ChangeMulticastOrigin(gnome_id, c_id)
                }
                249 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    data_idx += 10;
                    Configuration::EndMulticast(gnome_id, c_id)
                }
                248 => {
                    data_idx += 1;
                    Configuration::CreateGroup
                }
                247 => {
                    data_idx += 1;
                    Configuration::DeleteGroup
                }
                246 => {
                    data_idx += 1;
                    Configuration::ModifyGroup
                }
                245 => {
                    let mut key = vec![];
                    for i in data_idx + 9..data_idx + 279 {
                        key.push(bytes[i]);
                    }
                    data_idx += 279;
                    // println!("read: {}, {:?}", gnome_id, key);
                    Configuration::InsertPubkey(gnome_id, key)
                }
                244 => {
                    // eprintln!("Bytes: {:?} (idx: {})", bytes, data_idx);
                    let new_value = bytes[data_idx + 9];
                    // eprintln!("new_value: {}", new_value);
                    data_idx += 10;
                    // eprintln!("next_byte: {}", bytes[data_idx]);
                    // println!("read: {}, {:?}", gnome_id, key);
                    Configuration::ChangeDiameter(gnome_id, new_value)
                }
                243 => {
                    let mut new_bytes = Vec::from(&bytes[data_idx + 9..]);
                    let init_len = new_bytes.len();

                    eprintln!("New bytes: {:?}", new_bytes);
                    let pol = Policy::from(&mut new_bytes);
                    eprintln!("pol: {:?}", pol);
                    eprintln!("New bytes: {:?}", new_bytes);

                    let req = Requirement::from(&mut new_bytes);
                    let final_len = new_bytes.len();
                    data_idx += 9 + init_len - final_len;
                    Configuration::SetRunningPolicy(gnome_id, pol, req)
                    // TODO:
                }
                242 => {
                    let mut new_bytes = Vec::from(&bytes[data_idx + 9..]);
                    let init_len = new_bytes.len();

                    eprintln!("New bytes: {:?}", new_bytes);
                    let cap = Capabilities::from(new_bytes.remove(0));
                    eprintln!("cap: {:?}", cap);
                    eprintln!("New bytes: {:?}", new_bytes);
                    let b1 = new_bytes.remove(0);
                    let b2 = new_bytes.remove(0);
                    let how_many = u16::from_be_bytes([b1, b2]);
                    let mut v_gids = Vec::with_capacity(how_many as usize);
                    for _i in 0..how_many {
                        let b1 = new_bytes.remove(0);
                        let b2 = new_bytes.remove(0);
                        let b3 = new_bytes.remove(0);
                        let b4 = new_bytes.remove(0);
                        let b5 = new_bytes.remove(0);
                        let b6 = new_bytes.remove(0);
                        let b7 = new_bytes.remove(0);
                        let b8 = new_bytes.remove(0);
                        let gnome_id: u64 = as_u64_be(&[b1, b2, b3, b4, b5, b6, b7, b8]);
                        v_gids.push(GnomeId(gnome_id));
                    }

                    let final_len = new_bytes.len();
                    data_idx += 9 + init_len - final_len;
                    Configuration::SetRunningCapability(gnome_id, cap, v_gids)
                    // TODO:
                }
                other => {
                    // println!("Custom: {}", other);
                    // 1 byte for Reconf id
                    // 8 bytes for GnomeId
                    // 2 bytes for sync_data len
                    // println!("Data idx: {}", data_idx);
                    let s_data_len =
                        u16::from_be_bytes([bytes[data_idx + 9], bytes[data_idx + 10]]) as usize;
                    // println!("SData len: {}", s_data_len);
                    data_idx += 11;
                    let mut s_data_bytes = Vec::with_capacity(s_data_len);
                    for i in data_idx..data_idx + s_data_len {
                        s_data_bytes.push(bytes[i]);
                    }
                    data_idx += s_data_len;
                    Configuration::UserDefined(other, SyncData::new(s_data_bytes).unwrap())
                }
            };
            // println!("Reading sig type at index: {}", data_idx);
            let sig_type = bytes[data_idx];
            // println!("Sig type: {}, bytes_len: {}", sig_type, bytes.len());
            let signature = if sig_type == 0 {
                // println!("Regular sig");
                let mut sig_bytes = vec![];
                for i in data_idx + 1..data_idx + 257 {
                    sig_bytes.push(bytes[i]);
                }
                // data_idx += 74;
                Signature::Regular(gnome_id, sig_bytes)
            } else if sig_type == 255 {
                // println!("Extended sig,all bytes len: {}", bytes.len());
                // let mut pub_key = Vec::with_capacity(74);
                let mut pub_key = Vec::with_capacity(270);
                for i in data_idx + 1..data_idx + 271 {
                    pub_key.push(bytes[i]);
                }
                let mut sig_bytes = Vec::with_capacity(256);

                // for i in data_idx + 75..data_idx + 139 {
                for i in data_idx + 271..data_idx + 527 {
                    sig_bytes.push(bytes[i]);
                }
                // data_idx += 148;
                Signature::Extended(gnome_id, pub_key, sig_bytes)
            } else {
                panic!("Unexpected value: {}", sig_type);
            };
            Payload::Reconfigure(signature, config)
        }

    // } else if bytes[0] & 0b0_111_0000 == 64 {
    } else if bytes[0] & 0b0010_0000 == 0 {
        // eprintln!("Header Block, Payload KeepAlive");
        let bandwith: u64 = as_u64_be(&[
            bytes[data_idx],
            bytes[data_idx + 1],
            bytes[data_idx + 2],
            bytes[data_idx + 3],
            bytes[data_idx + 4],
            bytes[data_idx + 5],
            bytes[data_idx + 6],
            bytes[data_idx + 7],
        ]);
        Payload::KeepAlive(bandwith)
    // } else if bytes[0] & 0b0010_0000 == 32 {
    } else {
        let bid: u64 = as_u64_be(&[
            bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12],
        ]);
        eprintln!("Header Block, Payload Block id: {}", bid);
        let gnome_id: u64 = as_u64_be(&[
            bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19], bytes[20],
        ]);
        eprintln!("GnomeId: {}", gnome_id);
        let sig_type = bytes[data_idx];
        let signature = if sig_type == 0 {
            // println!("Regular sig");
            let mut sig_bytes = Vec::with_capacity(256);
            for i in data_idx + 1..data_idx + 257 {
                sig_bytes.push(bytes[i]);
            }
            data_idx += 257;
            Signature::Regular(GnomeId(gnome_id), sig_bytes)
        } else if sig_type == 255 {
            // println!("Extended sig,all bytes len: {}", bytes.len());
            // let mut pub_key = Vec::with_capacity(74);
            // for i in data_idx + 1..data_idx + 75 {
            let mut pub_key = Vec::with_capacity(270);
            for i in data_idx + 1..data_idx + 271 {
                pub_key.push(bytes[i]);
            }
            // let mut sig_bytes = Vec::with_capacity(64);
            let mut sig_bytes = Vec::with_capacity(256);
            for i in data_idx + 271..data_idx + 527 {
                sig_bytes.push(bytes[i]);
            }
            // data_idx += 139;
            data_idx += 527;
            Signature::Extended(GnomeId(gnome_id), pub_key, sig_bytes)
        } else {
            panic!("Unexpected value: {}", sig_type);
        };
        // println!("Data idx: {:?}", &bytes[data_idx..]);
        // println!("len: {}", bytes.len());
        let data = SyncData::new(Vec::from(&bytes[data_idx..])).unwrap();
        Payload::Block(BlockID(bid), signature, data)
    };
    // eprintln!("DCHeader: {}", header);
    // eprintln!("DCPayload: {}", payload);
    // eprintln!("DCSwarmTime: {}", swarm_time);
    Ok(Message {
        swarm_time,
        neighborhood,
        header,
        payload,
    })
}
pub fn bytes_to_cast_message(bytes: &[u8]) -> Result<CastMessage, ConversionError> {
    // eprintln!("bytes_to_cast_message: {:?}", bytes);
    let dgram_header = bytes[0];
    let c_id = CastID(bytes[1]);
    let data = CastData::new(Vec::from(&bytes[2..])).unwrap();
    if dgram_header & 0b11000000 == 192 {
        // TODO: broadcast
        // println!("received a broadcast");
        Ok(CastMessage::new_broadcast(c_id, data))
    } else if dgram_header & 0b11000000 == 128 {
        // TODO: multicast
        // eprintln!("received a multicast");
        Ok(CastMessage::new_multicast(c_id, data))
    } else if dgram_header & 0b11000000 == 64 {
        // TODO: unicast
        // println!("received a unicast");
        Ok(CastMessage::new_unicast(c_id, data))
    } else {
        Err(ConversionError)
    }
}

fn as_u16_be(array: &[u8; 2]) -> u16 {
    ((array[0] as u16) << 8) + (array[1] as u16)
}
fn as_u32_be(array: &[u8; 4]) -> u32 {
    ((array[0] as u32) << 24)
        + ((array[1] as u32) << 16)
        + ((array[2] as u32) << 8)
        + (array[3] as u32)
}

fn as_u64_be(array: &[u8; 8]) -> u64 {
    ((array[0] as u64) << 56)
        + ((array[1] as u64) << 48)
        + ((array[2] as u64) << 40)
        + ((array[3] as u64) << 32)
        + ((array[4] as u64) << 24)
        + ((array[5] as u64) << 16)
        + ((array[6] as u64) << 8)
        + (array[7] as u64)
}

#[derive(Debug)]
pub struct ConversionError;
//     Failed,
// }
impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // match self {
        //     ConversionError::Failed => write!(f, "ConversionError: Failed"),
        // }
        write!(f, "ConversionError")
    }
}

impl Error for ConversionError {}

fn put_u16(vec: &mut Vec<u8>, value: u16) {
    for a_byte in value.to_be_bytes() {
        vec.push(a_byte);
    }
}
fn put_u32(vec: &mut Vec<u8>, value: u32) {
    for a_byte in value.to_be_bytes() {
        vec.push(a_byte);
    }
}
fn put_u64(vec: &mut Vec<u8>, value: u64) {
    for a_byte in value.to_be_bytes() {
        vec.push(a_byte);
    }
}
pub fn message_to_bytes(msg: Message) -> Vec<u8> {
    // eprintln!("Message to bytes: {:?}", msg);
    let mut bytes = Vec::with_capacity(1033);
    let nhood = msg.neighborhood.0;
    if nhood > 31 {
        panic!("Can't handle this!");
    }
    bytes.push(nhood);
    put_u32(&mut bytes, msg.swarm_time.0);
    // bytes.append(&mut msg.swarm_time.to_bytes());

    let mut block_id_inserted = false;
    if let Header::Block(block_id) = msg.header {
        if matches!(msg.payload, Payload::Block(_b, ref _s, ref _d)) {
            bytes[0] |= 0b111_00000;
        } else {
            bytes[0] |= 0b110_00000;
        }
        put_u64(&mut bytes, block_id.0);
        block_id_inserted = true;
    } else if let Header::Reconfigure(conf_id, gnome_id) = msg.header {
        // println!("Header::Refonfigure: {}, {}", conf_id, gnome_id);
        if matches!(msg.payload, Payload::Reconfigure(ref _s, ref _d)) {
            bytes[0] |= 0b101_00000;
        } else {
            bytes[0] |= 0b100_00000;
        }
        bytes.push(conf_id);
        put_u64(&mut bytes, gnome_id.0);
    } else if matches!(msg.payload, Payload::Bye) {
        bytes[0] |= 0b010_00000;
        return bytes;
    }

    match msg.payload {
        Payload::KeepAlive(bandwith) => {
            put_u64(&mut bytes, bandwith);
        }
        Payload::Bye => {
            panic!("Bye should be short circuiting this fn");
        }
        Payload::Reconfigure(signature, config) => {
            // println!("msg->bytes sign len: {}", signature.len());
            if config.is_user_defined() {
                let [len_1, len_2] = (config.len() as u16 - 1).to_be_bytes();
                bytes.push(len_1);
                bytes.push(len_2);
            }
            for byte in config.content_bytes(false) {
                bytes.push(byte);
            }
            // put_u64(&mut bytes, signature.gnome_id().0);
            // print!("Pushing at index {} sig header byte: ", bytes.len());
            bytes.push(signature.header_byte());
            for sign_byte in signature.bytes() {
                bytes.push(sign_byte);
            }
        }
        Payload::Block(block_id, signature, data) => {
            // eprintln!("PB: {}", signature.len());
            if !block_id_inserted {
                put_u64(&mut bytes, block_id.0);
                // bytes.put_u32(block_id.0);
            };
            put_u64(&mut bytes, signature.gnome_id().0);
            bytes.push(signature.header_byte());
            for sign_byte in signature.bytes() {
                bytes.push(sign_byte);
            }
            // put_u32(&mut bytes, data.0);
            for byte in data.bytes() {
                bytes.push(byte);
            }
        }
    };
    // eprintln!("Message to bytes: {}[{:#b}]", bytes.len(), bytes[0]);
    bytes
}

pub fn neighbor_response_to_bytes(n_resp: NeighborResponse, bytes: &mut Vec<u8>) {
    match n_resp {
        NeighborResponse::BroadcastSync(part_no, total_parts, cast_id_origin_pairs) => {
            bytes.push(255);
            bytes.push(part_no);
            bytes.push(total_parts);
            for (c_id, origin) in cast_id_origin_pairs {
                bytes.push(c_id.0);
                put_u64(bytes, origin.0);
            }
        }
        NeighborResponse::MulticastSync(part_no, total_parts, cast_id_origin_pairs) => {
            bytes.push(254);
            bytes.push(part_no);
            bytes.push(total_parts);
            for (c_id, origin) in cast_id_origin_pairs {
                bytes.push(c_id.0);
                put_u64(bytes, origin.0);
            }
        }
        NeighborResponse::Unicast(swarm_id, cast_id) => {
            bytes.push(253);
            bytes.push(swarm_id.0);
            bytes.push(cast_id.0);
        }
        NeighborResponse::ForwardConnectResponse(mut network_settings) => {
            bytes.push(252);
            bytes.append(&mut network_settings);
            // insert_network_settings(bytes, network_settings);
        }
        NeighborResponse::ForwardConnectFailed => {
            bytes.push(251);
        }
        NeighborResponse::AlreadyConnected(id) => {
            bytes.push(250);
            bytes.push(id);
        }
        NeighborResponse::ConnectResponse(id, mut network_settings) => {
            bytes.push(249);
            bytes.push(id);
            bytes.append(&mut network_settings);
            // insert_network_settings(bytes, network_settings);
        }
        NeighborResponse::SwarmSync(sync_response) => {
            bytes.push(248);
            put_u16(bytes, sync_response.chill_phase);
            put_u64(bytes, sync_response.founder.0);
            put_u32(bytes, sync_response.swarm_time.0);
            put_u32(bytes, sync_response.round_start.0);
            bytes.push(sync_response.swarm_type.as_byte());
            bytes.push(sync_response.swarm_diameter.0 as u8);
            bytes.push(sync_response.key_reg_size);
            bytes.push(sync_response.capability_size);
            bytes.push(sync_response.policy_size);
            bytes.push(sync_response.broadcast_size);
            bytes.push(sync_response.multicast_size);
            bytes.push(sync_response.more_key_reg_messages as u8);
            for (g_id, mut pubkey) in sync_response.key_reg_pairs {
                put_u64(bytes, g_id.0);
                bytes.append(&mut pubkey);
            }
        }
        NeighborResponse::Subscribed(is_bcast, cast_id, origin_id, _source_opt) => {
            bytes.push(247);
            if is_bcast {
                bytes.push(1);
            } else {
                bytes.push(0);
            }
            bytes.push(cast_id.0);
            put_u64(bytes, origin_id.0);
        }
        NeighborResponse::KeyRegistrySync(part_no, total_parts, id_pubkey_pairs) => {
            bytes.push(246);
            bytes.push(part_no);
            bytes.push(total_parts);
            for (g_id, mut pubkey) in id_pubkey_pairs {
                put_u64(bytes, g_id.0);
                bytes.append(&mut pubkey);
            }
        }
        NeighborResponse::CapabilitySync(part_no, total_parts, caps_ids_pairs) => {
            bytes.push(245);
            bytes.push(part_no);
            bytes.push(total_parts);
            for (c_id, g_ids) in caps_ids_pairs {
                bytes.push(c_id.byte());
                bytes.push(g_ids.len() as u8);
                for g_id in g_ids {
                    put_u64(bytes, g_id.0);
                }
            }
        }
        NeighborResponse::PolicySync(part_no, total_parts, pol_req_pairs) => {
            bytes.push(244);
            bytes.push(part_no);
            bytes.push(total_parts);
            for (pol, req) in pol_req_pairs {
                pol.append_bytes_to(bytes);
                req.append_bytes_to(bytes);
            }
        }
        NeighborResponse::Custom(id, data) => {
            bytes.push(id);
            for byte in data.bytes() {
                bytes.push(byte);
            }
        }
    }
}

pub fn neighbor_request_to_bytes(n_req: NeighborRequest, bytes: &mut Vec<u8>) {
    match n_req {
        NeighborRequest::SwarmJoinedInfo(swarm_name) => {
            bytes.push(255);
            for byte in swarm_name.as_bytes() {
                bytes.push(byte);
            }
        }
        NeighborRequest::UnicastRequest(swarm_id, cast_ids) => {
            bytes.push(254);
            bytes.push(swarm_id.0);
            for c_id in cast_ids.deref() {
                bytes.push(c_id.0);
            }
        }
        NeighborRequest::CreateNeighbor(g_id, swarm_name) => {
            eprintln!("CreateNeighbor {} for {} to bytes", g_id, swarm_name);
            bytes.push(253);
            for byte in g_id.0.to_be_bytes() {
                bytes.push(byte);
            }
            for byte in swarm_name.as_bytes() {
                bytes.push(byte);
            }
        }
        NeighborRequest::ForwardConnectRequest(mut network_settings) => {
            bytes.push(252);
            bytes.append(&mut network_settings);
            // insert_network_settings(bytes, network_settings);
        }
        NeighborRequest::ConnectRequest(id, gnome_id, mut network_settings) => {
            bytes.push(251);
            bytes.push(id);
            put_u64(bytes, gnome_id.0);
            bytes.append(&mut network_settings);
            // insert_network_settings(bytes, network_settings);
        }
        NeighborRequest::SwarmSyncRequest(sync_req_params) => {
            bytes.push(250);
            bytes.push(sync_req_params.sync_key_reg as u8);
            bytes.push(sync_req_params.sync_capability as u8);
            bytes.push(sync_req_params.sync_policy as u8);
            bytes.push(sync_req_params.sync_broadcast as u8);
            bytes.push(sync_req_params.sync_multicast as u8);
            // put_u64(bytes, sync_req_params.app_root_hash);
        }
        NeighborRequest::SubscribeRequest(is_bcast, cast_id) => {
            bytes.push(249);
            if is_bcast {
                bytes.push(1);
            } else {
                bytes.push(0);
            }
            bytes.push(cast_id.0);
        }
        NeighborRequest::UnsubscribeRequest(is_bcast, cast_id) => {
            bytes.push(248);
            if is_bcast {
                bytes.push(1);
            } else {
                bytes.push(0);
            }
            bytes.push(cast_id.0);
        }
        NeighborRequest::SourceDrained(is_bcast, cast_id) => {
            bytes.push(247);
            if is_bcast {
                bytes.push(1);
            } else {
                bytes.push(0);
            }
            bytes.push(cast_id.0);
        }
        NeighborRequest::SendToCastSource(is_bcast, cast_id, c_data) => {
            bytes.push(246);
            if is_bcast {
                bytes.push(1);
            } else {
                bytes.push(0);
            }
            bytes.push(cast_id.0);
            bytes.append(&mut c_data.bytes());
        }
        NeighborRequest::Custom(id, data) => {
            bytes.push(id);
            for byte in data.bytes() {
                bytes.push(byte);
            }
        }
    }
}
pub fn bytes_to_neighbor_request(bytes: Vec<u8>) -> NeighborRequest {
    let data_idx = 0;
    let bytes_len = bytes.len();
    match bytes[0] {
        255 => {
            // eprintln!("Reading SwarmNames gnome/data_conversion");
            let swarm_name = SwarmName::from(&bytes[data_idx + 1..bytes_len]).unwrap();
            NeighborRequest::SwarmJoinedInfo(swarm_name)
        }
        254 => {
            let swarm_id = SwarmID(bytes[data_idx + 1]);
            let mut cast_ids = [CastID(0); 256];
            for (inserted, c_id) in bytes[data_idx + 2..bytes_len].iter().enumerate() {
                cast_ids[inserted] = CastID(*c_id);
            }
            NeighborRequest::UnicastRequest(swarm_id, Box::new(cast_ids))
        }
        253 => {
            let gnome_id = GnomeId(as_u64_be(&[
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
                bytes[data_idx + 8],
            ]));
            // let swarm_name = String::from_utf8(bytes[data_idx + 9..bytes_len].to_vec()).unwrap();
            let swarm_name = SwarmName::from(&bytes[data_idx + 9..bytes_len]).unwrap();
            // eprintln!(
            //     "Reading SwarmNames gnome/data_conversion (CreateNeighbor for {})",
            //     swarm_name
            // );
            NeighborRequest::CreateNeighbor(gnome_id, swarm_name)
        }
        252 => {
            // let net_set = parse_network_settings(&bytes[data_idx + 1..bytes_len]);
            let net_set = bytes[data_idx + 1..bytes_len].to_vec();
            NeighborRequest::ForwardConnectRequest(net_set)
        }
        251 => {
            let id = bytes[data_idx + 1];
            let mut g_id: u64 = ((bytes[data_idx + 2]) as u64) << 56;
            g_id += ((bytes[data_idx + 3]) as u64) << 48;
            g_id += ((bytes[data_idx + 4]) as u64) << 40;
            g_id += ((bytes[data_idx + 5]) as u64) << 32;
            g_id += ((bytes[data_idx + 6]) as u64) << 24;
            g_id += ((bytes[data_idx + 7]) as u64) << 16;
            g_id += ((bytes[data_idx + 8]) as u64) << 8;
            g_id += (bytes[data_idx + 9]) as u64;
            // let net_set = parse_network_settings(&bytes[data_idx + 10..bytes_len]);
            let net_set = bytes[data_idx + 10..bytes_len].to_vec();
            NeighborRequest::ConnectRequest(id, GnomeId(g_id), net_set)
        }
        250 => {
            let sync_key_reg = bytes[data_idx + 1] > 0;
            let sync_capability = bytes[data_idx + 2] > 0;
            let sync_policy = bytes[data_idx + 3] > 0;
            let sync_broadcast = bytes[data_idx + 4] > 0;
            let sync_multicast = bytes[data_idx + 5] > 0;
            // let mut app_root_hash: u64 = ((bytes[data_idx + 6]) as u64) << 56;
            // app_root_hash += ((bytes[data_idx + 7]) as u64) << 48;
            // app_root_hash += ((bytes[data_idx + 8]) as u64) << 40;
            // app_root_hash += ((bytes[data_idx + 9]) as u64) << 32;
            // app_root_hash += ((bytes[data_idx + 10]) as u64) << 24;
            // app_root_hash += ((bytes[data_idx + 11]) as u64) << 16;
            // app_root_hash += ((bytes[data_idx + 12]) as u64) << 8;
            // app_root_hash += (bytes[data_idx + 13]) as u64;
            let sync_req_params = SwarmSyncRequestParams {
                sync_key_reg,
                sync_capability,
                sync_policy,
                sync_broadcast,
                sync_multicast,
                // app_root_hash,
            };
            NeighborRequest::SwarmSyncRequest(sync_req_params)
        }
        249 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            NeighborRequest::SubscribeRequest(is_bcast, cast_id)
        }
        248 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            NeighborRequest::UnsubscribeRequest(is_bcast, cast_id)
        }
        247 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            NeighborRequest::SourceDrained(is_bcast, cast_id)
        }
        246 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            let c_bytes = Vec::from(&bytes[3..]);
            let c_data = CastData::new(c_bytes).unwrap();
            NeighborRequest::SendToCastSource(is_bcast, cast_id, c_data)
        }
        other => {
            let dta = if bytes.len() > data_idx + 1 {
                Vec::from(&bytes[data_idx + 1..])
            } else {
                vec![]
            };
            NeighborRequest::Custom(other, CastData::new(dta).unwrap())
        }
    }
}

pub fn bytes_to_neighbor_response(mut bytes: Vec<u8>) -> NeighborResponse {
    let data_idx = 0;
    let bytes_len = bytes.len();

    let response_type = bytes[data_idx];
    // println!("Parsing response type: {}", response_type);
    match response_type {
        255 => {
            let part_no = bytes[data_idx + 1];
            let total_parts = bytes[data_idx + 2];
            let mut idx = data_idx + 3;
            let mut cid_origin_pairs = vec![];
            while idx < bytes_len {
                let c_id = bytes[idx];
                idx += 1;
                let gnome_id: GnomeId = GnomeId(as_u64_be(&[
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]));
                idx += 8;
                cid_origin_pairs.push((CastID(c_id), gnome_id));
            }
            NeighborResponse::BroadcastSync(part_no, total_parts, cid_origin_pairs)
        }
        254 => {
            let part_no = bytes[data_idx + 1];
            let total_parts = bytes[data_idx + 2];
            let mut idx = data_idx + 3;
            let mut cid_origin_pairs = vec![];
            while idx < bytes_len {
                let c_id = bytes[idx];
                idx += 1;
                let gnome_id: GnomeId = GnomeId(as_u64_be(&[
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]));
                idx += 8;
                cid_origin_pairs.push((CastID(c_id), gnome_id));
            }
            NeighborResponse::MulticastSync(part_no, total_parts, cid_origin_pairs)
        }
        253 => NeighborResponse::Unicast(SwarmID(bytes[data_idx + 1]), CastID(bytes[data_idx + 2])),
        252 => {
            // let net_set = parse_network_settings(&bytes[data_idx + 1..bytes_len]);
            let net_set = bytes[data_idx + 1..bytes_len].to_vec();
            NeighborResponse::ForwardConnectResponse(net_set)
        }
        251 => NeighborResponse::ForwardConnectFailed,
        250 => {
            let id = bytes[data_idx + 1];
            NeighborResponse::AlreadyConnected(id)
        }
        249 => {
            let id = bytes[data_idx + 1];
            // let net_set = parse_network_settings(&bytes[data_idx + 2..bytes_len]);
            let net_set = bytes[data_idx + 2..bytes_len].to_vec();
            NeighborResponse::ConnectResponse(id, net_set)
        }
        248 => {
            let chill_phase = as_u16_be(&[bytes[data_idx + 1], bytes[data_idx + 2]]);
            let founder = GnomeId(as_u64_be(&[
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
                bytes[data_idx + 8],
                bytes[data_idx + 9],
                bytes[data_idx + 10],
            ]));
            let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[
                bytes[data_idx + 11],
                bytes[data_idx + 12],
                bytes[data_idx + 13],
                bytes[data_idx + 14],
            ]));
            let round_start: SwarmTime = SwarmTime(as_u32_be(&[
                bytes[data_idx + 15],
                bytes[data_idx + 16],
                bytes[data_idx + 17],
                bytes[data_idx + 18],
            ]));
            let swarm_type = SwarmType::from(bytes[data_idx + 19]);
            // let app_root_hash = as_u64_be(&[
            //     bytes[data_idx + 20],
            //     bytes[data_idx + 21],
            //     bytes[data_idx + 22],
            //     bytes[data_idx + 23],
            //     bytes[data_idx + 24],
            //     bytes[data_idx + 25],
            //     bytes[data_idx + 26],
            //     bytes[data_idx + 27],
            // ]);
            let swarm_diameter = SwarmTime(bytes[data_idx + 20] as u32);
            let key_reg_size = bytes[data_idx + 21];
            let capability_size = bytes[data_idx + 22];
            let policy_size = bytes[data_idx + 23];
            let broadcast_size = bytes[data_idx + 24];
            let multicast_size = bytes[data_idx + 25];
            let more_key_reg_messages = bytes[data_idx + 26] != 0;
            let mut key_reg_pairs = vec![];
            let mut idx = data_idx + 27;
            while idx < bytes_len {
                let mut g_vec = [0; 8];
                for i in 0..8 {
                    g_vec[i] = bytes[idx + i];
                }
                let mut pubkey_vec = Vec::with_capacity(270);
                for i in 0..270 {
                    pubkey_vec.push(bytes[idx + 8 + i]);
                }
                idx += 278;
                key_reg_pairs.push((GnomeId(u64::from_be_bytes(g_vec)), pubkey_vec));
            }
            let sync_response = SwarmSyncResponse {
                chill_phase,
                founder,
                swarm_time,
                round_start,
                swarm_type,
                swarm_diameter,
                key_reg_size,
                capability_size,
                policy_size,
                broadcast_size,
                multicast_size,
                more_key_reg_messages,
                key_reg_pairs,
            };
            NeighborResponse::SwarmSync(sync_response)
        }
        247 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            let data_idx = data_idx + 3;
            let origin_id: GnomeId = GnomeId(as_u64_be(&[
                bytes[data_idx],
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
            ]));
            NeighborResponse::Subscribed(is_bcast, cast_id, origin_id, None)
        }
        246 => {
            let part_no = bytes[data_idx + 1];
            let total_parts = bytes[data_idx + 2];
            let mut idx = data_idx + 3;
            let mut id_pubkey_pairs = vec![];
            while idx < bytes_len {
                let gnome_id: GnomeId = GnomeId(as_u64_be(&[
                    bytes[idx],
                    bytes[idx + 1],
                    bytes[idx + 2],
                    bytes[idx + 3],
                    bytes[idx + 4],
                    bytes[idx + 5],
                    bytes[idx + 6],
                    bytes[idx + 7],
                ]));
                let mut pubkey = Vec::with_capacity(270);
                for i in idx + 8..idx + 278 {
                    pubkey.push(bytes[i]);
                }
                id_pubkey_pairs.push((gnome_id, pubkey));
                idx += 278;
            }
            NeighborResponse::KeyRegistrySync(part_no, total_parts, id_pubkey_pairs)
        }
        245 => {
            let part_no = bytes[data_idx + 1];
            let total_parts = bytes[data_idx + 2];
            let mut idx = data_idx + 3;
            let mut cap_ids_pairs = vec![];
            while idx < bytes_len {
                let cap_id = bytes[idx];
                let ids_len = bytes[idx + 1];
                idx += 2;
                let mut ids = Vec::with_capacity(ids_len as usize);
                for _ix in 0..ids_len {
                    let gnome_id: GnomeId = GnomeId(as_u64_be(&[
                        bytes[idx],
                        bytes[idx + 1],
                        bytes[idx + 2],
                        bytes[idx + 3],
                        bytes[idx + 4],
                        bytes[idx + 5],
                        bytes[idx + 6],
                        bytes[idx + 7],
                    ]));
                    ids.push(gnome_id);
                    idx += 8;
                }
                cap_ids_pairs.push((Capabilities::from(cap_id), ids));
            }
            NeighborResponse::CapabilitySync(part_no, total_parts, cap_ids_pairs)
        }
        244 => {
            let mut b_drain = bytes.drain(0..3);
            let _ = b_drain.next();
            let part_no = b_drain.next().unwrap();
            let total_parts = b_drain.next().unwrap();
            let mut pol_req_pairs = vec![];
            drop(b_drain);
            while !bytes.is_empty() {
                let policy = Policy::from(&mut bytes);
                let req = Requirement::from(&mut bytes);
                pol_req_pairs.push((policy, req));
            }
            NeighborResponse::PolicySync(part_no, total_parts, pol_req_pairs)
        }
        _other => {
            let dta = if bytes.len() > data_idx + 1 {
                Vec::from(&bytes[data_idx + 1..])
            } else {
                vec![]
            };
            NeighborResponse::Custom(bytes[data_idx], CastData::new(dta).unwrap())
        }
    }
}

fn insert_network_settings(bytes: &mut Vec<u8>, network_settings_vec: Vec<NetworkSettings>) {
    //TODO: support sending multiple NSs.
    // TODO: add transport byte
    for network_settings in network_settings_vec {
        bytes.append(&mut network_settings.bytes());
        // bytes.push(network_settings.nat_type as u8);
        // put_u16(bytes, network_settings.pub_port);
        // // bytes.put_u16(network_settings.pub_port);
        // bytes.push(network_settings.port_allocation.0 as u8);
        // // TODO: fix this!
        // // bytes.put_i8(network_settings.port_allocation.1);
        // bytes.push(network_settings.port_allocation.1 as u8);
        // bytes.push(network_settings.transport.byte());
        // let pub_ip = network_settings.pub_ip;
        // match pub_ip {
        //     std::net::IpAddr::V4(ip4) => {
        //         for b in ip4.octets() {
        //             bytes.push(b);
        //         }
        //     }
        //     std::net::IpAddr::V6(ip4) => {
        //         for b in ip4.octets() {
        //             bytes.push(b);
        //         }
        //     }
        // }
    }
}
fn parse_network_settings(bytes: &[u8]) -> Vec<NetworkSettings> {
    NetworkSettings::from(bytes)
    // let mut bytes_iter = bytes.iter();
    // let mut ns_vec = vec![];
    // //TODO: support multiple NSs
    // while let Some(raw_nat_type) = bytes_iter.next() {
    //     let nat_type = match raw_nat_type {
    //         0 => Nat::Unknown,
    //         1 => Nat::None,
    //         2 => Nat::FullCone,
    //         4 => Nat::AddressRestrictedCone,
    //         8 => Nat::PortRestrictedCone,
    //         16 => Nat::SymmetricWithPortControl,
    //         32 => Nat::Symmetric,
    //         _ => {
    //             eprintln!("Unrecognized NatType while parsing: {}", raw_nat_type);
    //             Nat::Unknown
    //         }
    //     };

    //     let mut port_bytes: [u8; 2] = [0, 0];
    //     port_bytes[0] = *bytes_iter.next().unwrap();
    //     port_bytes[1] = *bytes_iter.next().unwrap();
    //     let pub_port: u16 = ((port_bytes[0]) as u16) << 8 | port_bytes[1] as u16;

    //     let port_allocation_rule = match bytes_iter.next().unwrap() {
    //         0 => PortAllocationRule::Random,
    //         1 => PortAllocationRule::FullCone,
    //         2 => PortAllocationRule::AddressSensitive,
    //         4 => PortAllocationRule::PortSensitive,
    //         _ => PortAllocationRule::Random,
    //     };
    //     let delta_port = *bytes_iter.next().unwrap() as i8;
    //     let transport = Transport::from(*bytes_iter.next().unwrap()).unwrap();
    //     let mut ip_bytes: Vec<u8> = vec![];
    //     let pub_ip = match transport {
    //         Transport::UDPoverIP4 | Transport::TCPoverIP4 => {
    //             for _i in 0..4 {
    //                 ip_bytes.push(*bytes_iter.next().unwrap());
    //             }
    //             let array: [u8; 4] = ip_bytes.try_into().unwrap();
    //             IpAddr::from(array)
    //         }
    //         Transport::TCPoverIP6 | Transport::UDPoverIP6 => {
    //             for _i in 0..16 {
    //                 ip_bytes.push(*bytes_iter.next().unwrap());
    //             }
    //             let array: [u8; 16] = ip_bytes.try_into().unwrap();
    //             IpAddr::from(array)
    //         }
    //     };
    //     // for a_byte in bytes_iter {
    //     //     ip_bytes.push(*a_byte);
    //     // }
    //     // let bytes_len = ip_bytes.len();
    //     // let pub_ip = if bytes_len == 4 {
    //     //     let array: [u8; 4] = ip_bytes.try_into().unwrap();
    //     //     IpAddr::from(array)
    //     // } else if bytes_len == 16 {
    //     //     let array: [u8; 16] = ip_bytes.try_into().unwrap();
    //     //     IpAddr::from(array)
    //     // } else {
    //     //     eprintln!("Unable to parse IP addr from: {:?}", ip_bytes);
    //     //     IpAddr::from([0, 0, 0, 0])
    //     // };
    //     ns_vec.push(NetworkSettings {
    //         pub_ip,
    //         pub_port,
    //         nat_type,
    //         port_allocation: (port_allocation_rule, delta_port),
    //         transport,
    //     })
    // }
    // ns_vec
}
