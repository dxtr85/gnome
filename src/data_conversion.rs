#![allow(clippy::unusual_byte_groupings)]
use std::fmt;
use std::ops::Deref;

use std::error::Error;
use swarm_consensus::BlockID;
use swarm_consensus::CastID;
use swarm_consensus::Configuration;
use swarm_consensus::Data;
use swarm_consensus::GnomeId;
use swarm_consensus::Header;
use swarm_consensus::Message;
use swarm_consensus::Nat;
use swarm_consensus::NeighborRequest;
use swarm_consensus::NeighborResponse;
use swarm_consensus::Neighborhood;
use swarm_consensus::NetworkSettings;
// use swarm_consensus::NetworkSettings;
use swarm_consensus::CastMessage;
use swarm_consensus::Payload;
use swarm_consensus::PortAllocationRule;
use swarm_consensus::SwarmID;
use swarm_consensus::SwarmTime;

// use bytes::BufMut;
// use bytes::Bytes;
// use bytes::BytesMut;
use std::net::IpAddr;

// 123456789012345678901234567890123456789012345678
// _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _
// HPPPNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSRRRRRRRR
// H       = header: 0    - Sync
//                   1    - Block
//                   1111 - RESERVED for Swarm identification within UDP channel
//                   0111 - Reconfigure (including Bye)
//  PPP    = payload: 000 - KeepAlive
//                    100 - Block
//                    010 - NeighborResponse
//                    001 - NeighborRequest
//                    101 - Unicast   TODO
//                    110 - Multicast TODO
//                    011 - Broadcast TODO
//     NNNN = Neighborhood value
//         SS... = SwarmTime value
//           RRR = Reconfigure byte (when payload = Reconfigure)

// TODO: In future we should prepend our header with additional byte(and second optional)
// to always send local Swarm identification (0-63) and info whether it is
// a regular Message or one of Casts (Uni/Multi/Broad).
// In case it is a Cast, next byte is CastID:
// TTIIIIII[CCCCCCCC] (and later HPPPNNNNSS... but only for Regular Message)
// TT - Datagram Type 00 - Regular Message
//                    01 - Unicast
//                    10 - Multicast
//                    11 - Broadcast
//   IIIIII - local Swarm identification in order to support up to 64 different Swarms
//            on a single UPD socket
//         CCCCCCCC - optional CastID, when TT is not Regular Message
//
// When this is implemented casting messages can become independent from Sync mechanism.
// Then we can define a separate channel for Casts per Swarm to be handled independently.
// Also for casts there is only need for CastID and Data, rest does not need to be sent
// and in case upper abstraction layer requires it, can be filled with defaults.
// This can be easily mitigated by creating a separate CastMessage enum.
// Also PPP field can have additional 3 payload types, since Casts are freed.
// Since having a single Swarm is expected to be an extreme rarity, this change should
// get implemented soon.

pub fn bytes_to_message(bytes: &[u8]) -> Result<Message, ConversionError> {
    // println!("Bytes to message: {:?}", bytes);
    // println!("decoding: {:#08b} {:?}", bytes[0], bytes);
    let bytes_len = bytes.len();
    let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[bytes[1], bytes[2], bytes[3], bytes[4]]));
    let neighborhood: Neighborhood = Neighborhood(bytes[0] & 0b0_000_1111);
    let mut gnome_id: GnomeId = GnomeId(0);
    let (header, data_idx) = if bytes[0] & 0b1_000_0000 == 128 {
        let block_id: u32 = as_u32_be(&[bytes[5], bytes[6], bytes[7], bytes[8]]);
        (Header::Block(BlockID(block_id)), 9)
    } else if bytes[0] & 0b0_111_0000 == 112 {
        if bytes[5] == 255 {
            (Header::Sync, 5)
        } else {
            gnome_id = GnomeId(as_u64_be(&[
                bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13],
            ]));
            (Header::Reconfigure(bytes[5], gnome_id), 5)
        }
    } else {
        (Header::Sync, 5)
    };
    let payload: Payload = if bytes[0] & 0b0_111_0000 == 112 {
        // println!("bytes[0]: {:#b}", bytes[0]);
        // let data = as_u32_be(&[
        //     bytes[data_idx],
        //     bytes[data_idx + 1],
        //     bytes[data_idx + 2],
        //     bytes[data_idx + 3],
        // ]);
        // if data == std::u32::MAX {
        if bytes[data_idx] == 255 {
            Payload::Bye
        } else {
            // println!("Configuration!");
            let config = match bytes[data_idx] {
                254 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::StartBroadcast(gnome_id, c_id)
                }
                253 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::ChangeBroadcastOrigin(gnome_id, c_id)
                }
                252 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::EndBroadcast(c_id)
                }
                251 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::StartMulticast(gnome_id, c_id)
                }
                250 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::ChangeMulticastOrigin(gnome_id, c_id)
                }
                249 => {
                    let c_id: CastID = CastID(bytes[data_idx + 9]);
                    Configuration::EndMulticast(c_id)
                }
                248 => Configuration::CreateGroup,
                247 => Configuration::DeleteGroup,
                246 => Configuration::ModifyGroup,
                other => Configuration::UserDefined(other),
            };
            Payload::Reconfigure(config)
        }
    // } else if bytes[0] & 0b0_111_0000 == 80 {
    //     // println!("UNICAST!!");
    //     let cid: CastID = CastID(bytes[data_idx]);
    //     let data: Data = Data(as_u32_be(&[
    //         bytes[data_idx + 1],
    //         bytes[data_idx + 2],
    //         bytes[data_idx + 3],
    //         bytes[data_idx + 4],
    //     ]));
    //     Payload::Unicast(cid, data)
    // } else if bytes[0] & 0b0_111_0000 == 16 {
    //     println!("len: {}", bytes_len);
    //     // let request_type: u8 = bytes[data_idx];
    //     let nr = bytes_to_neighbor_request(&bytes[data_idx..]);
    //     Payload::Request(nr)
    } else if bytes[0] & 0b0_111_0000 == 64 {
        // let bid: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
        // let data = Data(as_u32_be(&[bytes[10], bytes[11], bytes[12], bytes[13]]));
        let bid: u32 = as_u32_be(&[bytes[5], bytes[6], bytes[7], bytes[8]]);
        let data = Data(as_u32_be(&[
            bytes[data_idx],
            bytes[data_idx + 1],
            bytes[data_idx + 2],
            bytes[data_idx + 3],
        ]));
        Payload::Block(BlockID(bid), data)
    // } else if bytes[0] & 0b0_111_0000 == 32 {
    //     let nr = bytes_to_neighbor_response(&bytes[1..]);
    //     Payload::Response(nr)
    // } else if bytes[0] & 0b0_111_0000 == 48 {
    //     let c_id = CastID(bytes[data_idx]);
    //     let data: Data = Data(as_u32_be(&[
    //         bytes[data_idx + 1],
    //         bytes[data_idx + 2],
    //         bytes[data_idx + 3],
    //         bytes[data_idx + 4],
    //     ]));
    //     Payload::Broadcast(c_id, data)
    // } else if bytes[0] & 0b0_111_0000 == 96 {
    //     let c_id = CastID(bytes[data_idx]);
    //     let data: Data = Data(as_u32_be(&[
    //         bytes[data_idx + 1],
    //         bytes[data_idx + 2],
    //         bytes[data_idx + 3],
    //         bytes[data_idx + 4],
    //     ]));
    //     Payload::Multicast(c_id, data)
    } else {
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
    };
    Ok(Message {
        swarm_time,
        neighborhood,
        header,
        payload,
    })
}
pub fn bytes_to_cast_message(bytes: &[u8]) -> Result<CastMessage, ConversionError> {
    let dgram_header = bytes[0];
    let c_id = CastID(bytes[1]);
    let data = Data(as_u32_be(&[bytes[2], bytes[3], bytes[4], bytes[5]]));
    if dgram_header & 0b11000000 == 192 {
        // TODO: broadcast
        // println!("received a broadcast");
        Ok(CastMessage::new_broadcast(c_id, data))
    } else if dgram_header & 0b11000000 == 128 {
        // TODO: multicast
        println!("received a multicast");
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
    // println!("Message to bytes: {:?}", msg);
    let mut bytes = Vec::with_capacity(1033);
    let nhood = msg.neighborhood.0;
    if nhood > 15 {
        panic!("Can't handle this!");
    }
    bytes.push(nhood);
    put_u32(&mut bytes, msg.swarm_time.0);
    // bytes.append(&mut msg.swarm_time.to_bytes());

    let mut block_id_inserted = false;
    if let Header::Block(block_id) = msg.header {
        bytes[0] |= 0b1_000_0000;
        put_u32(&mut bytes, block_id.0);
        block_id_inserted = true;
    }
    bytes[0] |= match msg.payload {
        Payload::KeepAlive(bandwith) => {
            put_u64(&mut bytes, bandwith);
            // bytes.put_u64(bandwith);
            0b0_000_0000
        }
        // Payload::Unicast(cid, data) => {
        //     println!("Unicast into bytes");
        //     bytes.push(cid.0);
        //     put_u32(&mut bytes, data.0);
        //     // bytes.put_u32(data.0);
        //     0b0_101_0000
        // }
        // Payload::Multicast(mid, data) => {
        //     bytes.push(mid.0);
        //     put_u32(&mut bytes, data.0);
        //     // bytes.put_u32(data.0);
        //     0b0_110_0000
        // }
        // Payload::Broadcast(bid, data) => {
        //     bytes.push(bid.0);
        //     put_u32(&mut bytes, data.0);
        //     // bytes.put_u32(data.0);
        //     0b0_011_0000
        // }
        Payload::Bye => {
            bytes.push(255);
            bytes.push(255);
            bytes.push(255);
            bytes.push(255);
            // bytes.put_u32(std::u32::MAX);
            0b0_111_0000
        }
        Payload::Reconfigure(config) => {
            match config {
                Configuration::StartBroadcast(g_id, c_id) => {
                    bytes.push(254);
                    put_u64(&mut bytes, g_id.0);
                    // bytes.put_u64(g_id.0);
                    bytes.push(c_id.0);
                }
                Configuration::ChangeBroadcastOrigin(g_id, c_id) => {
                    bytes.push(253);
                    put_u64(&mut bytes, g_id.0);
                    // bytes.put_u64(g_id.0);
                    bytes.push(c_id.0);
                }
                Configuration::EndBroadcast(c_id) => {
                    bytes.push(252);
                    bytes.push(c_id.0);
                }
                Configuration::StartMulticast(g_id, c_id) => {
                    bytes.push(251);
                    put_u64(&mut bytes, g_id.0);
                    // bytes.put_u64(g_id.0);
                    bytes.push(c_id.0);
                }
                Configuration::ChangeMulticastOrigin(g_id, c_id) => {
                    bytes.push(250);
                    put_u64(&mut bytes, g_id.0);
                    // bytes.put_u64(g_id.0);
                    bytes.push(c_id.0);
                }
                Configuration::EndMulticast(c_id) => {
                    bytes.push(249);
                    bytes.push(c_id.0);
                }
                Configuration::CreateGroup => {
                    bytes.push(248);
                    // TODO
                    // bytes.put_u8(c_id.0);
                }
                Configuration::DeleteGroup => {
                    bytes.push(247);
                    // TODO
                    // bytes.put_u8(c_id.0);
                }
                Configuration::ModifyGroup => {
                    bytes.push(246);
                    // TODO
                    // bytes.put_u8(c_id.0);
                }
                Configuration::UserDefined(id) => {
                    bytes.push(id);
                    // TODO
                    // bytes.put_u8(c_id.0);
                }
            }
            put_u32(&mut bytes, config.as_u32());
            // bytes.put_u32(config.as_u32());
            0b0_111_0000
        }
        Payload::Block(block_id, data) => {
            if !block_id_inserted {
                put_u32(&mut bytes, block_id.0);
                // bytes.put_u32(block_id.0);
            };
            put_u32(&mut bytes, data.0);
            // bytes.put_u32(data.0);
            0b0_100_0000
        } // Payload::Response(neighbor_response) => {
          //     neighbor_response_to_bytes(neighbor_response, &mut bytes);
          //     0b0_010_0000
          // }
          // Payload::Request(n_req) => {
          //     neighbor_request_to_bytes(n_req, &mut bytes);
          //     0b0_001_0000
          // }
    };
    // println!("encoded: {:#08b} {:?}", bytes[0], bytes);
    // bytes.split().into()
    bytes
}

pub fn neighbor_response_to_bytes(n_resp: NeighborResponse, mut bytes: &mut Vec<u8>) {
    match n_resp {
        NeighborResponse::Listing(count, data) => {
            bytes.push(255);
            bytes.push(count);
            for chunk in data.deref() {
                put_u32(&mut bytes, chunk.0);
                // bytes.put_u32(chunk.0);
            }
        }
        NeighborResponse::Unicast(swarm_id, cast_id) => {
            bytes.push(254);
            bytes.push(swarm_id.0);
            bytes.push(cast_id.0);
        }
        NeighborResponse::Block(b_id, data) => {
            bytes.push(253);
            put_u32(&mut bytes, b_id.0);
            // bytes.put_u32(b_id.0);
            put_u32(&mut bytes, data.0);
            // bytes.put_u32(data.0);
        }
        NeighborResponse::ForwardConnectResponse(network_settings) => {
            bytes.push(252);
            insert_network_settings(&mut bytes, network_settings);
        }
        NeighborResponse::ForwardConnectFailed => {
            bytes.push(251);
        }
        NeighborResponse::AlreadyConnected(id) => {
            bytes.push(250);
            bytes.push(id);
        }
        NeighborResponse::ConnectResponse(id, network_settings) => {
            bytes.push(249);
            bytes.push(id);
            insert_network_settings(&mut bytes, network_settings);
        }
        NeighborResponse::SwarmSync(chill_phase, swarm_time, bcasts, mcasts) => {
            bytes.push(248);
            put_u16(bytes, chill_phase);
            // bytes.push(chill_phase);
            put_u32(bytes, swarm_time.0);
            bytes.push(bcasts.len() as u8);
            bytes.push(mcasts.len() as u8);
            // TODO: make sure we do not send more than 1kB of data
            for (b_id, origin) in bcasts {
                bytes.push(b_id.0);
                put_u64(&mut bytes, origin.0);
            }
            for (m_id, origin) in mcasts {
                bytes.push(m_id.0);
                put_u64(&mut bytes, origin.0);
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
            // bytes.put_u64(origin_id.0);
        }
        NeighborResponse::CustomResponse(id, data) => {
            bytes.push(id);
            put_u32(bytes, data.0);
            // bytes.put_u32(data.0);
        } // _ => todo!(),
    }
}

pub fn neighbor_request_to_bytes(n_req: NeighborRequest, mut bytes: &mut Vec<u8>) {
    match n_req {
        NeighborRequest::ListingRequest(st) => {
            bytes.push(255);
            put_u32(bytes, st.0);
            // bytes.put_u32(st.0);
        }
        NeighborRequest::UnicastRequest(swarm_id, cast_ids) => {
            bytes.push(254);
            bytes.push(swarm_id.0);
            for c_id in cast_ids.deref() {
                bytes.push(c_id.0);
            }
        }
        NeighborRequest::BlockRequest(count, data) => {
            bytes.push(253);
            bytes.push(count);
            for chunk in data.deref() {
                put_u32(bytes, chunk.0);
                // bytes.put_u32(chunk.0);
            }
        }
        NeighborRequest::ForwardConnectRequest(network_settings) => {
            bytes.push(252);
            insert_network_settings(&mut bytes, network_settings);
        }
        NeighborRequest::ConnectRequest(id, gnome_id, network_settings) => {
            bytes.push(251);
            bytes.push(id);
            put_u64(&mut bytes, gnome_id.0);
            // bytes.put_u64(gnome_id.0);
            insert_network_settings(&mut bytes, network_settings);
        }
        NeighborRequest::SwarmSyncRequest => {
            bytes.push(250);
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
        NeighborRequest::CustomRequest(id, data) => {
            bytes.push(id);
            put_u32(&mut bytes, data.0);
            // bytes.put_u32(data.0);
        }
    }
}
pub fn bytes_to_neighbor_request(bytes: &[u8]) -> NeighborRequest {
    let data_idx = 0;
    let bytes_len = bytes.len();
    let nr = match bytes[0] {
        255 => {
            let st_value: u32 = as_u32_be(&[
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
            ]);
            NeighborRequest::ListingRequest(SwarmTime(st_value))
        }
        254 => {
            let swarm_id = SwarmID(bytes[data_idx + 1]);
            let mut cast_ids = [CastID(0); 256];
            // let mut inserted = 0;
            for (inserted, c_id) in bytes[data_idx + 2..bytes_len].iter().enumerate() {
                cast_ids[inserted] = CastID(*c_id);
                // inserted += 1;
            }
            NeighborRequest::UnicastRequest(swarm_id, Box::new(cast_ids))
        }
        253 => {
            let count = bytes[data_idx + 2];
            let mut data = [BlockID(0); 128];
            for i in 0..count as usize {
                let bid = as_u32_be(&[
                    bytes[4 * i + data_idx + 3],
                    bytes[4 * i + data_idx + 4],
                    bytes[4 * i + data_idx + 5],
                    bytes[4 * i + data_idx + 6],
                ]);
                data[i] = BlockID(bid);
            }
            NeighborRequest::BlockRequest(count, Box::new(data))
        }
        252 => {
            let net_set = parse_network_settings(&bytes[data_idx + 1..bytes_len]);
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
            let net_set = parse_network_settings(&bytes[data_idx + 10..bytes_len]);
            NeighborRequest::ConnectRequest(id, GnomeId(g_id), net_set)
        }
        250 => NeighborRequest::SwarmSyncRequest,
        249 => {
            let is_bcast = bytes[data_idx + 1] > 0;
            let cast_id = CastID(bytes[data_idx + 2]);
            NeighborRequest::SubscribeRequest(is_bcast, cast_id)
        }
        other => {
            // TODO
            let data: u32 = as_u32_be(&[
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
            ]);
            NeighborRequest::CustomRequest(other, Data(data))
        }
    };
    nr
}

pub fn bytes_to_neighbor_response(bytes: &[u8]) -> NeighborResponse {
    let data_idx = 0;
    let bytes_len = bytes.len();

    let response_type = bytes[data_idx];
    match response_type {
        255 => {
            let count = bytes[data_idx + 1];
            let mut data = vec![];
            for i in 0..count as usize {
                let bid: u32 = as_u32_be(&[
                    bytes[4 * i + data_idx + 2],
                    bytes[4 * i + data_idx + 3],
                    bytes[4 * i + data_idx + 4],
                    bytes[4 * i + data_idx + 5],
                ]);
                data.push(BlockID(bid));
            }
            NeighborResponse::Listing(count, data)
        }
        254 => NeighborResponse::Unicast(SwarmID(bytes[data_idx + 1]), CastID(bytes[data_idx + 2])),
        253 => {
            let b_id: u32 = as_u32_be(&[
                bytes[data_idx + 1],
                bytes[data_idx + 2],
                bytes[data_idx + 3],
                bytes[data_idx + 4],
            ]);

            let data: u32 = as_u32_be(&[
                bytes[data_idx + 5],
                bytes[data_idx + 6],
                bytes[data_idx + 7],
                bytes[data_idx + 8],
            ]);
            NeighborResponse::Block(BlockID(b_id), Data(data))
        }
        252 => {
            let net_set = parse_network_settings(&bytes[data_idx + 1..bytes_len]);
            NeighborResponse::ForwardConnectResponse(net_set)
        }
        251 => NeighborResponse::ForwardConnectFailed,
        250 => {
            let id = bytes[data_idx + 1];
            NeighborResponse::AlreadyConnected(id)
        }
        249 => {
            let id = bytes[data_idx + 1];
            let net_set = parse_network_settings(&bytes[data_idx + 2..bytes_len]);
            NeighborResponse::ConnectResponse(id, net_set)
        }
        248 => {
            let chill_phase = as_u16_be(&[bytes[data_idx + 1], bytes[data_idx + 2]]);
            let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[
                bytes[data_idx + 3],
                bytes[data_idx + 4],
                bytes[data_idx + 5],
                bytes[data_idx + 6],
            ]));
            let b_count = bytes[data_idx + 7];
            let m_count = bytes[data_idx + 8];
            let mut b_casts = vec![];
            let mut m_casts = vec![];
            let data_idx = data_idx + 9;
            let mut i: usize = 0;
            while i < (b_count * 9) as usize {
                let b_id = CastID(bytes[data_idx + i]);
                let origin = GnomeId(as_u64_be(&[
                    bytes[data_idx + i + 1],
                    bytes[data_idx + i + 2],
                    bytes[data_idx + i + 3],
                    bytes[data_idx + i + 4],
                    bytes[data_idx + i + 5],
                    bytes[data_idx + i + 6],
                    bytes[data_idx + i + 7],
                    bytes[data_idx + i + 8],
                ]));
                b_casts.push((b_id, origin));
                i += 9;
            }
            let data_idx = data_idx + (b_count as usize * 9);
            i = 0;
            while i < m_count as usize {
                let m_id = CastID(bytes[data_idx + i]);
                let origin = GnomeId(as_u64_be(&[
                    bytes[data_idx + i + 1],
                    bytes[data_idx + i + 2],
                    bytes[data_idx + i + 3],
                    bytes[data_idx + i + 4],
                    bytes[data_idx + i + 5],
                    bytes[data_idx + i + 6],
                    bytes[data_idx + i + 7],
                    bytes[data_idx + i + 8],
                ]));
                m_casts.push((m_id, origin));
                i += 9;
            }
            NeighborResponse::SwarmSync(chill_phase, swarm_time, b_casts, m_casts)
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
        _other => {
            // TODO
            NeighborResponse::CustomResponse(bytes[data_idx + 1], Data(0))
        }
    }
    // nr
}

fn insert_network_settings(bytes: &mut Vec<u8>, network_settings: NetworkSettings) {
    bytes.push(network_settings.nat_type as u8);
    put_u16(bytes, network_settings.pub_port);
    // bytes.put_u16(network_settings.pub_port);
    bytes.push(network_settings.port_allocation.0 as u8);
    // TODO: fix this!
    // bytes.put_i8(network_settings.port_allocation.1);
    bytes.push(network_settings.port_allocation.1 as u8);
    let pub_ip = network_settings.pub_ip;
    match pub_ip {
        std::net::IpAddr::V4(ip4) => {
            for b in ip4.octets() {
                bytes.push(b);
            }
        }
        std::net::IpAddr::V6(ip4) => {
            for b in ip4.octets() {
                bytes.push(b);
            }
        }
    }
}
fn parse_network_settings(bytes: &[u8]) -> NetworkSettings {
    let mut bytes_iter = bytes.iter();
    let raw_nat_type = bytes_iter.next().unwrap();
    let nat_type = match raw_nat_type {
        0 => Nat::Unknown,
        1 => Nat::None,
        2 => Nat::FullCone,
        4 => Nat::AddressRestrictedCone,
        8 => Nat::PortRestrictedCone,
        16 => Nat::SymmetricWithPortControl,
        32 => Nat::Symmetric,
        _ => {
            println!("Unrecognized NatType while parsing: {}", raw_nat_type);
            Nat::Unknown
        }
    };

    let mut port_bytes: [u8; 2] = [0, 0];
    port_bytes[0] = *bytes_iter.next().unwrap();
    port_bytes[1] = *bytes_iter.next().unwrap();
    let pub_port: u16 = ((port_bytes[0]) as u16) << 8 | port_bytes[1] as u16;

    let port_allocation_rule = match bytes_iter.next().unwrap() {
        0 => PortAllocationRule::Random,
        1 => PortAllocationRule::FullCone,
        2 => PortAllocationRule::AddressSensitive,
        4 => PortAllocationRule::PortSensitive,
        _ => PortAllocationRule::Random,
    };
    let delta_port = *bytes_iter.next().unwrap() as i8;
    // port_bytes[0] = bytes_iter.next().unwrap();
    // port_bytes[1] = bytes_iter.next().unwrap();
    // let port_range_min: u16 = ((port_bytes[0]) as u16) << 8 | port_bytes[1] as u16;
    // port_bytes[0] = bytes_iter.next().unwrap();
    // port_bytes[1] = bytes_iter.next().unwrap();
    // let port_range_max: u16 = ((port_bytes[0]) as u16) << 8 | port_bytes[1] as u16;
    let mut ip_bytes: Vec<u8> = vec![];
    for a_byte in bytes_iter {
        ip_bytes.push(*a_byte);
    }
    let bytes_len = ip_bytes.len();
    let pub_ip = if bytes_len == 4 {
        let array: [u8; 4] = ip_bytes.try_into().unwrap();
        IpAddr::from(array)
    } else if bytes_len == 16 {
        let array: [u8; 16] = ip_bytes.try_into().unwrap();
        IpAddr::from(array)
    } else {
        println!("Unable to parse IP addr from: {:?}", ip_bytes);
        IpAddr::from([0, 0, 0, 0])
    };
    NetworkSettings {
        pub_ip,
        pub_port,
        nat_type,
        port_allocation: (port_allocation_rule, delta_port),
    }
}
