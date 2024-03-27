use std::fmt;

use std::error::Error;
use swarm_consensus::BlockID;
use swarm_consensus::CastID;
use swarm_consensus::Data;
use swarm_consensus::Header;
use swarm_consensus::Message;
use swarm_consensus::NeighborRequest;
use swarm_consensus::NeighborResponse;
use swarm_consensus::Neighborhood;
use swarm_consensus::Payload;
use swarm_consensus::SwarmID;
use swarm_consensus::SwarmTime;

use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;

// 1234567890123456789012345678901234567890|
// _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ |
// HPPPNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS|
// H       = header: 0 - Sync
//                   1 - Block
//  PPP    = payload: 000 - KeepAlive
//                    100 - Block
//                    010 - Listing
//                    001 - NeighborRequest
//                    101 - Unicast   TODO
//                    110 - Multicast TODO
//                    011 - Broadcast TODO
//                    111 - Bye
//     NNNN = Neighborhood value
//         SS... = SwarmTime value

pub fn bytes_to_message(bytes: &Bytes) -> Result<Message, ConversionError> {
    // println!("Bytes to message: {:?}", bytes);
    // println!("decoding: {:#08b} {:?}", bytes[0], bytes);
    let bytes_len = bytes.len();
    let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[bytes[1], bytes[2], bytes[3], bytes[4]]));
    let neighborhood: Neighborhood = Neighborhood(bytes[0] & 0b0_000_1111);
    let header = if bytes[0] & 0b1_000_0000 > 0 {
        let block_id: u32 = as_u32_be(&[bytes[5], bytes[6], bytes[7], bytes[8]]);
        Header::Block(BlockID(block_id))
    } else {
        Header::Sync
    };
    let payload: Payload = if bytes[0] & 0b0_111_0000 == 112 {
        println!("bytes[0]: {:#b}", bytes[0]);
        Payload::Bye
    } else if bytes[0] & 0b0_101_0000 == 80 {
        // println!("UNICAST!!");
        let cid: CastID = CastID(bytes[5]);
        let data: Data = Data(as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]));
        Payload::Unicast(cid, data)
    } else if bytes[0] & 0b0_001_0000 == 16 {
        println!("len: {}", bytes_len);
        let request_type: u8 = bytes[5];
        let nr = match request_type {
            255 => {
                let st_value: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
                NeighborRequest::ListingRequest(SwarmTime(st_value))
            }
            254 => {
                let swarm_id = SwarmID(bytes[6]);
                let mut cast_ids = [CastID(0); 256];
                let mut inserted = 0;
                for c_id in &bytes[7..bytes_len] {
                    cast_ids[inserted] = CastID(*c_id);
                    inserted += 1;
                }
                NeighborRequest::UnicastRequest(swarm_id, cast_ids)
            }
            253 => {
                let count = bytes[7];
                let mut data = [BlockID(0); 128];
                for i in 0..count as usize {
                    let bid = as_u32_be(&[
                        bytes[4 * i + 7],
                        bytes[4 * i + 8],
                        bytes[4 * i + 9],
                        bytes[4 * i + 10],
                    ]);
                    data[i as usize] = BlockID(bid);
                }
                NeighborRequest::PayloadRequest(count, data)
            }
            other => {
                // TODO
                let data: u32 = as_u32_be(&[bytes[7], bytes[8], bytes[9], bytes[10]]);
                NeighborRequest::CustomRequest(other, Data(data))
            }
        };
        Payload::Request(nr)
    } else if bytes[0] & 0b0_100_0000 == 64 {
        // let bid: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
        // let data = Data(as_u32_be(&[bytes[10], bytes[11], bytes[12], bytes[13]]));
        let bid: u32 = as_u32_be(&[bytes[9], bytes[6], bytes[7], bytes[8]]);
        let data = Data(as_u32_be(&[bytes[9], bytes[10], bytes[11], bytes[12]]));
        Payload::Block(BlockID(bid), data)
    } else if bytes[0] & 0b0_010_0000 == 32 {
        let response_type = bytes[5];
        let nr = match response_type {
            255 => {
                let count = bytes[6];
                let mut data = [BlockID(0); 128];
                for i in 0..count as usize {
                    let bid: u32 = as_u32_be(&[
                        bytes[4 * i + 5],
                        bytes[4 * i + 6],
                        bytes[4 * i + 7],
                        bytes[4 * i + 8],
                    ]);
                    data[i] = BlockID(bid);
                }
                NeighborResponse::Listing(count, data)
            }
            254 => NeighborResponse::Unicast(SwarmID(bytes[6]), CastID(bytes[7])),
            253 => {
                let b_id: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);

                let data: u32 = as_u32_be(&[bytes[10], bytes[11], bytes[12], bytes[13]]);
                NeighborResponse::Block(BlockID(b_id), Data(data))
            }
            other => {
                // TODO
                NeighborResponse::CustomResponse(bytes[6], Data(0))
            }
        };
        Payload::Response(nr)
    } else {
        Payload::KeepAlive
    };
    Ok(Message {
        swarm_time,
        neighborhood,
        header,
        payload,
    })
}

fn as_u32_be(array: &[u8; 4]) -> u32 {
    ((array[0] as u32) << 24)
        + ((array[1] as u32) << 16)
        + ((array[2] as u32) << 8)
        + (array[3] as u32)
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

pub fn message_to_bytes(msg: Message) -> Bytes {
    // println!("Message to bytes: {:?}", msg);
    let mut bytes = BytesMut::with_capacity(1033);
    let nhood = msg.neighborhood.0;
    if nhood > 15 {
        panic!("Can't handle this!");
    }
    bytes.put_u8(nhood);
    bytes.put_u32(msg.swarm_time.0);

    let mut block_id_inserted = false;
    if let Header::Block(block_id) = msg.header {
        bytes[0] |= 0b1_000_0000;
        bytes.put_u32(block_id.0);
        block_id_inserted = true;
    }
    bytes[0] |= match msg.payload {
        Payload::KeepAlive => 0b0_000_0000,
        Payload::Unicast(cid, data) => {
            println!("Unicast into bytes");
            bytes.put_u8(cid.0);
            bytes.put_u32(data.0);
            0b0_101_0000
        }
        Payload::Multicast(mid, data) => 0b0_110_0000,
        Payload::Broadcast(bid, data) => 0b0_011_0000,
        Payload::Bye => 0b0_111_0000,
        Payload::Block(block_id, data) => {
            if !block_id_inserted {
                bytes.put_u32(block_id.0);
                // block_id_inserted = true;
            };
            bytes.put_u32(data.0);
            0b0_100_0000
        }
        Payload::Response(neighbor_response) => {
            match neighbor_response {
                NeighborResponse::Listing(count, data) => {
                    bytes.put_u8(255);
                    bytes.put_u8(count);
                    for chunk in data {
                        bytes.put_u32(chunk.0);
                    }
                }
                NeighborResponse::Unicast(swarm_id, cast_id) => {
                    bytes.put_u8(254);
                    bytes.put_u8(swarm_id.0);
                    bytes.put_u8(cast_id.0);
                }
                NeighborResponse::Block(b_id, data) => {
                    bytes.put_u8(253);
                    bytes.put_u32(b_id.0);
                    bytes.put_u32(data.0);
                }
                NeighborResponse::CustomResponse(id, data) => {
                    bytes.put_u8(id);
                    bytes.put_u32(data.0);
                }
            }
            0b0_010_0000
        }
        Payload::Request(nr) => {
            match nr {
                NeighborRequest::ListingRequest(st) => {
                    bytes.put_u8(255);
                    bytes.put_u32(st.0);
                }
                NeighborRequest::UnicastRequest(swarm_id, cast_ids) => {
                    bytes.put_u8(254);
                    bytes.put_u8(swarm_id.0);
                    for c_id in cast_ids {
                        bytes.put_u8(c_id.0);
                    }
                }
                NeighborRequest::PayloadRequest(count, data) => {
                    bytes.put_u8(253);
                    bytes.put_u8(count);
                    for chunk in data {
                        bytes.put_u32(chunk.0);
                    }
                }
                NeighborRequest::CustomRequest(id, data) => {
                    bytes.put_u8(id);
                    bytes.put_u32(data.0);
                }
            }
            0b0_001_0000
        }
    };
    // println!("encoded: {:#08b} {:?}", bytes[0], bytes);
    bytes.split().into()
}
