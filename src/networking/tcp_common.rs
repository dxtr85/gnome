use super::common::read_bytes_from_local_stream;
use super::common::swarm_names_as_bytes;
use crate::crypto::SessionKey;
use crate::data_conversion::bytes_to_cast_message;
use crate::data_conversion::bytes_to_message;
use crate::data_conversion::bytes_to_neighbor_request;
use crate::data_conversion::bytes_to_neighbor_response;
use async_std::io::ReadExt;
use async_std::io::WriteExt;
use async_std::task;
use core::panic;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use swarm_consensus::CastID;
use swarm_consensus::CastType;
use swarm_consensus::NeighborRequest;
use swarm_consensus::SwarmName;
use swarm_consensus::{CastContent, CastMessage, Message, Neighbor, SwarmTime, WrappedMessage};

use super::sock::TokenDispenser;
use super::Token;
use crate::networking::subscription::Subscription;
use async_std::net::TcpStream;

pub async fn serve_socket(
    session_key: SessionKey,
    mut stream: TcpStream,
    send_recv_pairs: Vec<(
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
    sub_sender: Sender<Subscription>,
    shared_sender: Sender<(
        SwarmName,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    extend_receiver: Receiver<(
        SwarmName,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
) {
    let mut senders: HashMap<u8, (Sender<Message>, Sender<CastMessage>)> = HashMap::new();
    let mut receivers: HashMap<u8, Receiver<WrappedMessage>> = HashMap::new();
    let mut out_of_order_recvd = HashMap::new();
    let min_tokens_threshold: u64 = 1500;
    let mut available_tokens: u64 = min_tokens_threshold;
    let mut new_channel_mappings: HashMap<u8, SwarmName> = HashMap::new();
    // println!("racing: {:?}", send_recv_pairs);
    for (i, (sender, c_sender, receiver)) in send_recv_pairs.into_iter().enumerate() {
        senders.insert(i as u8, (sender, c_sender));
        receivers.insert(i as u8, receiver);
    }
    // let buf = [0u8; 1500];
    // let mut buf2 = [0u8; 1500];
    // let count: u16 = 0;
    // let mut buf2 = [0u8; 1];
    eprintln!("Waiting for initial tokens");
    task::sleep(Duration::from_millis(1)).await;
    let max_tokens = if let Ok(Token::Provision(tkns)) = token_reciever.try_recv() {
        tkns * 1000
    } else {
        eprintln!("Did not receive Provision as first token message");
        eprintln!("setting max_tokens to 10kB/s");
        10240
    };
    let mut token_dispenser = TokenDispenser::new(max_tokens);
    eprintln!("Max tokens: {}", max_tokens);
    let mut requested_tokens: u64 = 0;
    let mut incomplete_msg: Option<(usize, Vec<u8>)> = None;
    loop {
        // print!("l");
        if let Ok((swarm_name, snd, cast_snd, recv)) = extend_receiver.try_recv() {
            eprintln!("TCP Extend req: {}", swarm_name);
            //TODO: extend senders and receivers, force send message
            // informing remote about new swarm neighbor
            let mut new_id = 255;
            for (stored_id, name) in new_channel_mappings.iter() {
                if name.founder.0 == swarm_name.founder.0
                    && name.name == swarm_name.name
                    && !senders.contains_key(stored_id)
                {
                    new_id = *stored_id;
                }
            }
            new_channel_mappings.remove(&new_id);
            if new_id < 64 {
                // eprintln!("Mamma mia!");
                if let std::collections::hash_map::Entry::Vacant(e) = senders.entry(new_id) {
                    e.insert((snd, cast_snd));
                    receivers.insert(new_id, recv);
                } else {
                    eprintln!("This was not expected to happen");
                }
            } else {
                for i in 0u8..64 {
                    if let std::collections::hash_map::Entry::Vacant(e) = senders.entry(i) {
                        e.insert((snd, cast_snd));
                        receivers.insert(i, recv);
                        // TODO: define a preamble containing i and some recognizable pattern
                        // in order to be sent in newly defined channel
                        // let bytes = swarm_name.as_bytes();
                        // let ciphered = session_key.encrypt(bytes);
                        // let _send_result = socket.send(&ciphered).await;
                        break;
                    }
                }
            }
            if let Some((id, msg)) = out_of_order_recvd.remove(&new_id) {
                if let Some((_snd, c_snd)) = senders.get(&new_id) {
                    eprintln!("TCP Sending delayed request: {:?}", msg);
                    let _ = c_snd.send(CastMessage {
                        c_type: CastType::Unicast,
                        id,
                        content: CastContent::Request(msg),
                    });
                }
            }
        }
        // TODO: a different approach to token provisioning:
        // we keep here a VecDeque containing always 8 u64 elements
        // every 128ms we receive some tokens (total 1024ms ~ 1sec)
        // (it can also be a mutable list [u64;8])
        // we sum all the numbers in VecDeque = this is how many tokens
        // we have not used
        // we push front recently received tokens
        // and pop_back & discard oldest provisioned tokens
        // now we use this front value for sending messages out.
        // when we run out of tokens in this front value, or have not enough
        // to send current message we decrement from next value.
        // If we run out of all the tokens we request more

        // let mut provisioned_tokens_count = 0;
        while let Ok(token_msg) = token_reciever.try_recv() {
            match token_msg {
                Token::Provision(count) => {
                    if requested_tokens > 0 {
                        // println!("Got {} tokens (requested: {})!", count, requested_tokens);
                        if count >= requested_tokens {
                            requested_tokens = 0;
                        } else {
                            requested_tokens -= count;
                        }
                    }
                    token_dispenser.add(count);
                    // println!("Now I have: {}", token_dispenser.available_tokens);
                }
                other => {
                    eprintln!("Unexpected Token message: {:?}", other);
                }
            }
        }
        let _ = token_sender.send(Token::Unused(token_dispenser.available_tokens));
        if requested_tokens > 0 && token_dispenser.available_tokens < min_tokens_threshold {
            task::sleep(Duration::from_millis(1)).await;
            continue;
        }
        // println!(
        //     "{} unused, more tokens: {}",
        //     available_tokens, provisioned_tokens_count
        // );
        // available_tokens += provisioned_tokens_count;
        // if available_tokens > max_tokens {
        //     available_tokens = max_tokens;
        // }
        if token_dispenser.available_tokens < min_tokens_threshold {
            // println!(
            //     "Requesting more tokens (have: {}, required: {})",
            //     token_dispenser.available_tokens, min_tokens_threshold
            // );
            requested_tokens = (2 * min_tokens_threshold) - token_dispenser.available_tokens;
            let _ = token_sender.send(Token::Request(requested_tokens));
            task::sleep(Duration::from_millis(1)).await;
            continue;
        }
        let mut s_clone = stream.clone();
        // let mut s_clone_2 = stream.clone();
        let t1 = read_bytes_from_remote(&mut s_clone).fuse();
        // TODO: serv pairs of sender-receiver
        // let t2 = read_bytes_from_local(&mut s_clone_2, &mut receivers).fuse();
        let t2 = read_bytes_from_local_stream(&mut receivers).fuse();

        pin_mut!(t1, t2);

        // TODO: upper layer requests creation of a new channel, and then
        // sends CreateNeighbor over new channel.
        // This means that gnome joining a new swarm establishes new channel with
        // his Neighbor over existing UDP socket
        // TODO: rework this for TCPSTream
        let (from_socket, result) = select! {
            result1 = t1 =>  {
                (true, result1)},
            result2 = t2 => (false, result2),
        };
        if let Err(_err) = result {
            eprintln!("SRVC Error received: {:?}", _err);
            if &_err == "No receivers" {
                eprintln!("No receivers, terminating!");
                break;
            } else if from_socket {
                eprintln!("Connection error, terminating!");
                break;
            }
            // TODO: should end serving this socket
            for (sender, _c_snd) in senders.values() {
                let _send_result = sender.send(Message::bye());
            }
            break;
            // } else {
        }
        if from_socket {
            let mut b_iter = result.unwrap().into_iter();
            // TODO: first two bytes always indicate the size of encrypted message,
            //       and take only this many following bytes to decrypt a message
            let mut encrypted_messages = vec![];
            if let Some((missing_bytes, mut msg_bytes)) = incomplete_msg.take() {
                // eprintln!("Some incomplete msg, missing size: {}", missing_bytes);
                for i in 0..missing_bytes {
                    if let Some(byte) = b_iter.next() {
                        msg_bytes.push(byte);
                    } else {
                        incomplete_msg = Some((missing_bytes - i, msg_bytes.clone()));
                        break;
                    }
                }
                if incomplete_msg.is_some() {
                    continue;
                } else {
                    // eprintln!("completed a partial message");
                    encrypted_messages.push(msg_bytes);
                }
            }
            'outer: while let Some(b1) = b_iter.next() {
                let b2 = b_iter.next().unwrap();
                let m_size = u16::from_be_bytes([b1, b2]) as usize;
                // eprintln!("incoming size: {}", m_size);
                let mut encrypted = Vec::with_capacity(m_size);
                for i in 0..m_size {
                    if let Some(byte) = b_iter.next() {
                        encrypted.push(byte);
                    } else {
                        // eprintln!("incomplete msg size: {}", m_size);
                        incomplete_msg = Some((m_size - i, encrypted));
                        // eprintln!("Built incomplete msg, missing size: {}", m_size - i);
                        break 'outer;
                    }
                }
                encrypted_messages.push(encrypted);
            }
            // TODO: we do not need to race tasks, so this logic can be moved
            // to read_bytes_from_remote
            // let read_result = stream.read(&mut buf2).await;
            // if let Ok(count) = read_result {
            // println!("Read {} bytes", count);
            // if bytes.len() <= count {
            // println!("Count {}", count);
            // if count > 0 {
            // eprintln!("Got {} bytes to decipher", count);
            // eprintln!("{:?}", bytes);
            let mut decrypted_messages = vec![];
            for enc_m in encrypted_messages {
                let decr_res = session_key.decrypt(&enc_m);
                if let Ok(deciph) = decr_res {
                    decrypted_messages.push(deciph);
                } else {
                    eprintln!("Failed to decrypt\n{:?}", enc_m);
                }
            }
            // let decr_res = session_key.decrypt(&buf2);
            // buf = BytesMut::zeroed(1100);
            for mut deciph in decrypted_messages {
                // eprintln!("Decrypted: {:?}", deciph);
                // let deciph = Bytes::from(deciph);
                // let mut byte_iterator = deciph.into_iter();
                // let dgram_header = byte_iterator.next().unwrap();
                let dgram_header = deciph.first().unwrap();
                let swarm_id = dgram_header & 0b00111111;
                if let Some((sender, cast_sender)) = senders.get(&swarm_id) {
                    if dgram_header & 0b11000000 == 0 {
                        // TODO: regular message
                        deciph.drain(0..1);
                        // let deciphered = &deciph[1..];
                        // println!("received a regular message: {:?}", deciphered);
                        if let Ok(message) = bytes_to_message(deciph) {
                            // eprintln!("decode OK: {:?}", message);
                            let _send_result = sender.send(message);
                            if _send_result.is_err() {
                                // TODO: if failed maybe we are no longer interested?
                                // Maybe send back a bye if possible?
                                // Remove given Sender/Receiver pair
                                // println!("send result2: {:?}", _send_result);
                            }
                        } else {
                            eprintln!("Failed to decode incoming stream");
                        }
                    } else if dgram_header & 0b11000000 == 64 {
                        let c_id = deciph[1];
                        match c_id {
                            255 => {
                                // TODO N_Req
                                deciph.drain(0..2);
                                // let n_req = bytes_to_neighbor_request(&deciph[2..]);
                                let n_req = bytes_to_neighbor_request(deciph);
                                let message = CastMessage::new_request(n_req);
                                // eprintln!("Decr req: {:?}", message);
                                let _send_result = cast_sender.send(message);
                                if _send_result.is_err() {
                                    eprintln!("Unable to pass NeighborRequest to gnome");
                                }
                            }
                            254 => {
                                // TODO N_Resp
                                deciph.drain(0..2);
                                let n_resp = bytes_to_neighbor_response(deciph);
                                let message = CastMessage::new_response(n_resp);
                                // eprintln!("Decr resp: {:?}", message);
                                // eprintln!("Socket sending {:?} up…", message.content);
                                let _send_result = cast_sender.send(message);
                                if _send_result.is_err() {
                                    eprintln!(
                                        "Unable to pass NeighborResponseto gnome: {}",
                                        _send_result.err().unwrap()
                                    );
                                }
                            }
                            _ => {
                                if let Ok(message) = bytes_to_cast_message(&deciph) {
                                    // eprintln!("Decr other: {:?}", message);
                                    let _send_result = cast_sender.send(message);
                                    if _send_result.is_err() {
                                        // TODO: if failed maybe we are no longer interested?
                                        // Maybe send back a bye if possible?
                                        // Remove given Sender/Receiver pair
                                        eprintln!("send result3: {:?}", _send_result);
                                    }
                                }
                            }
                        }
                        // } else {
                        //     println!("Failed to decode incoming stream");
                    } else if let Ok(message) = bytes_to_cast_message(&deciph) {
                        // eprintln!("Decr other2: {:?}", message);
                        let _send_result = cast_sender.send(message);
                        if _send_result.is_err() {
                            // TODO: if failed maybe we are no longer interested?
                            // Maybe send back a bye if possible?
                            // Remove given Sender/Receiver pair
                            eprintln!("send result4: {:?}", _send_result);
                        }
                        // }
                    } else {
                        eprintln!("Unable to decode message");
                    }
                } else {
                    eprintln!("TCP Got a message on new channel...");
                    // eprintln!("Defined channels count: {}", senders.len());
                    // TODO: maybe we should instantiate a new
                    // Neighbor for a new swarm?
                    // For this we need Sender<Subscription> ?

                    // TODO: unmask it
                    deciph.drain(0..1);
                    let cast_id = CastID(deciph.drain(0..1).next().unwrap());
                    let neighbor_request = bytes_to_neighbor_request(deciph);
                    eprintln!("NR: {:?}", neighbor_request);
                    match neighbor_request {
                        NeighborRequest::CreateNeighbor(remote_gnome_id, swarm_name) => {
                            eprintln!(
                                "TCP Create neighbor {} for {} {}",
                                remote_gnome_id, swarm_id, swarm_name
                            );
                            new_channel_mappings.insert(swarm_id, swarm_name.clone());
                            let (s1, r1) = channel();
                            let (s2, r2) = channel();
                            let (s3, r3) = channel();
                            if let Some((id, msg)) = out_of_order_recvd.remove(&swarm_id) {
                                // for (id, msg) in msgs.into_iter() {
                                let _ = s2.send(CastMessage {
                                    c_type: CastType::Unicast,
                                    id,
                                    content: CastContent::Request(msg),
                                });
                                // }
                            }
                            let neighbor = Neighbor::from_id_channel_time(
                                remote_gnome_id,
                                r1,
                                r2,
                                s3,
                                shared_sender.clone(),
                                SwarmTime(0),
                                SwarmTime(7),
                                vec![],
                            );
                            let _ = shared_sender.send((swarm_name.clone(), s1, s2, r3));

                            // TODO: we need to pass this neighbor up
                            // eprintln!(
                            //     "Socket requests add {} to {}",
                            //     neighbor.id, swarm_name
                            // );
                            let _ = sub_sender
                                .send(Subscription::IncludeNeighbor(swarm_name, neighbor));
                        }
                        other => {
                            eprintln!(
                                "TCP Unexpected Neighbor request for {:?} received on new channel!: {:?}",
                                swarm_id,other
                            );
                            //TODO: maybe store this message temporarily until we receive CreateNeighbor?
                            out_of_order_recvd.insert(swarm_id, (cast_id, other));
                            // .entry(swarm_id)
                            // .or_insert(vec![])
                            // .push((cast_id, other));
                            eprintln!("Was expecting: NeighborRequest::CreateNeighbor");
                        }
                    }
                }
                // } else {
                //     eprintln!("Failed to decipher incoming stream ");
            }
            // }
            // } else {
            //     eprintln!("SRCV Unable to recv supposedly ready data");
            // }
        } else {
            let bytes = result.unwrap();
            if bytes.is_empty() {
                continue;
            }
            // eprintln!("Encrypting {} bytes…", bytes.len());
            let mut ciphered = session_key.encrypt(&bytes);
            // eprintln!("{:?}", ciphered);
            let actual_len = ciphered.len();
            // eprintln!("Encrypted {} bytes…", actual_len);
            let size_bytes = (actual_len as u16).to_be_bytes();
            let mut total_bytes = Vec::with_capacity(2 + actual_len);
            total_bytes.push(size_bytes[0]);
            total_bytes.push(size_bytes[1]);
            total_bytes.append(&mut ciphered);
            let all_header_bytes = 45; //TODO: TCP has more than UDP
            let len = all_header_bytes + actual_len as u64;
            let taken = token_dispenser.take(len);
            if taken == len {
                let _send_result = stream.write(&total_bytes).await;
                let _flush_result = stream.flush().await;
                // available_tokens = if len > available_tokens {
                //     0
                // } else {
                //     available_tokens - len
                // };
                available_tokens = available_tokens.saturating_sub(len);
            } else {
                eprintln!("Waiting for tokens...");
                let _ = token_sender.send(Token::Request(2 * len));
                task::sleep(Duration::from_millis(1)).await;
                let res = token_reciever.recv();
                match res {
                    Ok(Token::Provision(amount)) => {
                        token_dispenser.add(amount);
                        token_dispenser.take(len);
                        let _send_result = stream.write(&ciphered).await;
                    }
                    Ok(other) => eprintln!("Received unexpected Token: {:?}", other),
                    Err(e) => {
                        panic!("Error while waiting for Tokens: {:?}", e);
                    }
                }
            }
        }
    }
}

async fn read_bytes_from_remote(
    stream: &mut TcpStream,
    // buf: &mut [u8; 1500],
) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; 1500];
    let read_result = stream.read(&mut buf).await;
    if let Ok(count) = read_result {
        // eprintln!("TCP read {} bytes:\n{:?}", count, &buf[0..count]);
        // let bytes: Vec<u8> = (count as u16).to_be_bytes().into_iter().collect();
        Ok(Vec::from(&buf[0..count]))
    } else {
        eprintln!("TCP read error: {:?}", read_result.err().unwrap());
        Err(String::new())
    }
}

pub async fn send_subscribed_swarm_names(socket: &mut TcpStream, names: &Vec<SwarmName>) {
    let buf = swarm_names_as_bytes(names, 1450);
    let send_result = socket.write(&buf).await;
    let _f_result = socket.flush().await;
    // if let Ok(count) = send_result {
    //     eprintln!("TCP Sent {} bytes", count);
    // }
}
