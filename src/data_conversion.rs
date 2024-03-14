use std::fmt;

use swarm_consensus::BlockID;
use swarm_consensus::Data;
use swarm_consensus::Header;
use swarm_consensus::Message;
use swarm_consensus::NeighborRequest;
// use swarm_consensus::;
use std::error::Error;
use swarm_consensus::Neighborhood;
use swarm_consensus::Payload;
use swarm_consensus::SwarmTime;

// use bytes::Buf;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;

// 1234567890123456789012345678901234567890|
// _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ |
// HPPNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS|
// H       = header: 0 - Sync
//                   1 - Block
//  PP     = payload: 00 - KeepAlive
//                    10 - Block
//                    01 - Listing
//                    11 - NeighborRequest
//    NNNNN = Neighborhood value
//         SS... = SwarmTime value

pub fn bytes_to_message(bytes: &Bytes) -> Result<Message, ConversionError> {
    println!("Bytes to message: {:?}", bytes);
    let swarm_time: SwarmTime = SwarmTime(as_u32_be(&[bytes[1], bytes[2], bytes[3], bytes[4]]));
    let neighborhood: Neighborhood = Neighborhood(bytes[0] & 0b0001_1111);
    let header = if bytes[0] & 0b1000_0000 > 0 {
        let block_id: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
        Header::Block(BlockID(block_id))
    } else {
        Header::Sync
    };
    let payload: Payload = if bytes[0] & 0b0110_0000 > 0 {
        let count: u8 = bytes[5];
        let nr = if count == 0 {
            let st_value: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
            NeighborRequest::ListingRequest(SwarmTime(st_value))
        } else {
            let mut data = [BlockID(0); 128];
            for i in 0..count as usize {
                let bid = as_u32_be(&[
                    bytes[4 * i + 6],
                    bytes[4 * i + 7],
                    bytes[4 * i + 8],
                    bytes[4 * i + 9],
                ]);
                data[i as usize] = BlockID(bid);
            }
            NeighborRequest::PayloadRequest(count, data)
        };
        Payload::Request(nr)
    } else if bytes[0] & 0b0100_0000 > 0 {
        let bid: u32 = as_u32_be(&[bytes[6], bytes[7], bytes[8], bytes[9]]);
        let data = Data(as_u32_be(&[bytes[10], bytes[11], bytes[12], bytes[13]]));
        Payload::Block(BlockID(bid), data)
    } else if bytes[0] & 0b0010_0000 > 0 {
        let count = bytes[5];
        let mut data = [BlockID(0); 128];
        for i in 0..count as usize {
            let bid: u32 = as_u32_be(&[
                bytes[4 * i + 6],
                bytes[4 * i + 7],
                bytes[4 * i + 8],
                bytes[4 * i + 9],
            ]);
            data[i as usize] = BlockID(bid);
        }
        Payload::Listing(count, data)
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
        + ((array[3] as u32) << 0)
}

#[derive(Debug)]
pub enum ConversionError {
    Failed,
}
impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversionError::Failed => write!(f, "ConversionError: Failed"),
        }
    }
}

impl Error for ConversionError {}

pub fn message_to_bytes(msg: Message) -> Bytes {
    println!("Message to bytes: {:?}", msg);
    let mut bytes = BytesMut::with_capacity(1033);
    let nhood = msg.neighborhood.0;
    if nhood > 63 {
        panic!("Can't handle this!");
    }
    bytes.put_u8(nhood);
    bytes.put_u32(msg.swarm_time.0);

    let mut block_id_inserted = false;
    if let Header::Block(block_id) = msg.header {
        bytes[0] |= 0b1000_0000;
        bytes.put_u32(block_id.0);
        block_id_inserted = true;
    }
    bytes[0] |= match msg.payload {
        Payload::KeepAlive => 0b0000_0000,
        Payload::Block(block_id, data) => {
            if !block_id_inserted {
                bytes.put_u32(block_id.0);
                block_id_inserted = true;
            };
            bytes.put_u32(data.0);
            0b0100_0000
        }
        Payload::Listing(count, listing) => {
            bytes.put_u8(count);
            for chunk in listing {
                bytes.put_u32(chunk.0);
            }
            0b0010_0000
        }
        Payload::Request(nr) => {
            match nr {
                NeighborRequest::ListingRequest(st) => {
                    bytes.put_u8(0u8);
                    bytes.put_u32(st.0);
                }
                NeighborRequest::PayloadRequest(count, data) => {
                    bytes.put_u8(count);
                    for chunk in data {
                        bytes.put_u32(chunk.0);
                    }
                }
            }
            0b0110_0000
        }
    };
    bytes.split().into()
}
