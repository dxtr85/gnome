use crate::crypto::SessionKey;
use crate::data_conversion::bytes_to_cast_message;
use crate::data_conversion::bytes_to_message;
use crate::data_conversion::bytes_to_neighbor_request;
use crate::data_conversion::bytes_to_neighbor_response;
use crate::data_conversion::message_to_bytes;
use crate::data_conversion::neighbor_request_to_bytes;
use crate::data_conversion::neighbor_response_to_bytes;
use crate::networking::subscription::Subscription;
use crate::networking::token::Token;

use async_std::net::UdpSocket;
use async_std::task;
use swarm_consensus::NeighborRequest;
// TODO: get rid of bytes crate
// use bytes::BufMut;
use core::panic;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use swarm_consensus::{CastContent, CastMessage, Message, Neighbor, SwarmTime, WrappedMessage};

async fn read_bytes_from_socket(
    socket: &UdpSocket,
    buf: &mut [u8; 1100],
) -> Result<Vec<u8>, String> {
    // println!("read_bytes_from_socket");
    // TODO: increase size of buffer everywhere
    // let mut buf = [0u8; 1100];
    let recv_result = socket.peek(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        // println!("<<<<< {:?}", String::from_utf8_lossy(&buf[..count]));
        // When first byte starts with 1_111, next byte identifies swarm
        // if buf[0] & 0b1_111_0000 == 240 {
        //     Ok((buf[1], Bytes::from(Vec::from(&buf[2..count - 2]))))
        // } else {
        Ok(Vec::from(&buf[..count]))
        // }
    } else {
        println!("SKTd{:?}", recv_result);
        Err("Disconnected".to_string())
    }
}

async fn read_bytes_from_local_stream(
    receivers: &mut HashMap<u8, Receiver<WrappedMessage>>,
) -> Result<Vec<u8>, String> {
    // println!("read_bytes_from_local_stream");
    loop {
        for (id, receiver) in receivers.iter_mut() {
            let next_option = receiver.try_recv();
            if let Ok(message) = next_option {
                // TODO: here we need to add a Datagram header
                // indicating type of message and id
                let mut dgram_header = *id;
                match message {
                    WrappedMessage::NoOp => return Ok(vec![]),
                    WrappedMessage::Cast(c_msg) => {
                        dgram_header += if c_msg.is_unicast() {
                            64
                        } else if c_msg.is_multicast() {
                            128
                        } else {
                            192
                        };
                        // println!("Cast: {:?}", c_msg);
                        let c_id = c_msg.id();
                        let mut merged_bytes = Vec::with_capacity(10);
                        merged_bytes.push(dgram_header);
                        merged_bytes.push(c_id.0);
                        match c_msg.content {
                            CastContent::Data(data) => {
                                // let data = c_msg.get_data().unwrap();
                                for a_byte in data.bytes() {
                                    merged_bytes.push(a_byte);
                                }
                            }
                            CastContent::Request(n_req) => {
                                neighbor_request_to_bytes(n_req, &mut merged_bytes);
                            }
                            CastContent::Response(n_resp) => {
                                neighbor_response_to_bytes(n_resp, &mut merged_bytes);
                            }
                        }
                        return Ok(merged_bytes);
                    }
                    WrappedMessage::Regular(message) => {
                        let mut bytes = message_to_bytes(message);
                        let mut merged_bytes = Vec::with_capacity(bytes.len() + 1);
                        merged_bytes.push(dgram_header);
                        merged_bytes.append(&mut bytes);
                        return Ok(merged_bytes);
                    }
                }
                // println!(">>>>> {:?}", bytes);

                // if *id == 0 {
                // } else {
                //     let mut extended_bytes = BytesMut::with_capacity(bytes.len() + 2);
                //     extended_bytes.put_u8(240);
                //     extended_bytes.put_u8(*id);
                //     extended_bytes.put(bytes);
                //     return Ok((*id, Bytes::from(extended_bytes)));
                // }
            }
        }
        task::yield_now().await;
    }
    // Err(ConnError::LocalStreamClosed)
}

async fn race_tasks(
    session_key: SessionKey,
    socket: UdpSocket,
    send_recv_pairs: Vec<(
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
    sub_sender: Sender<Subscription>,
    shared_sender: Sender<(
        String,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    extend_receiver: Receiver<(
        String,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
) {
    let mut senders: HashMap<u8, (Sender<Message>, Sender<CastMessage>)> = HashMap::new();
    let mut receivers: HashMap<u8, Receiver<WrappedMessage>> = HashMap::new();
    let min_tokens_threshold: u64 = 128;
    let mut available_tokens: u64 = min_tokens_threshold;
    // println!("racing: {:?}", send_recv_pairs);
    for (i, (sender, c_sender, receiver)) in send_recv_pairs.into_iter().enumerate() {
        senders.insert(i as u8, (sender, c_sender));
        receivers.insert(i as u8, receiver);
    }
    let mut buf = [0u8; 1100];
    let mut buf2 = [0u8; 1100];
    // if let Some((sender, mut receiver)) = send_recv_pairs.pop() {
    loop {
        // print!("l");
        if let Ok((swarm_name, snd, cast_snd, recv)) = extend_receiver.try_recv() {
            println!("Extend req: {}", swarm_name);
            //TODO: extend senders and receivers, force send message
            // informing remote about new swarm neighbor
            for i in 0u8..64 {
                if let std::collections::hash_map::Entry::Vacant(e) = senders.entry(i) {
                    // !senders.contains_key(&i) {
                    // senders.insert(i, (snd, cast_snd));
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
        if let Ok(token_msg) = token_reciever.try_recv() {
            match token_msg {
                Token::Provision(count) => {
                    let _ = token_sender.send(Token::Unused(available_tokens));
                    // println!("{} unused, more tokens: {}", available_tokens, count);
                    available_tokens = count;
                }
                other => {
                    println!("Unexpected Token message: {:?}", other);
                }
            }
        }
        if available_tokens < min_tokens_threshold {
            println!("Requesting more tokens");
            let _ = token_sender.send(Token::Request(2 * min_tokens_threshold));
        }
        let t1 = read_bytes_from_socket(&socket, &mut buf2).fuse();
        // TODO: serv pairs of sender-receiver
        let t2 = read_bytes_from_local_stream(&mut receivers).fuse();

        pin_mut!(t1, t2);

        let (from_socket, result) = select! {
            result1 = t1 =>  (true, result1),
            result2 = t2 => (false, result2),
        };
        if let Err(_err) = result {
            println!("SRVC Error received: {:?}", _err);
            // TODO: should end serving this socket
            for (sender, _c_snd) in senders.values() {
                let _send_result = sender.send(Message::bye());
            }
            break;
            // } else {
        }
        if from_socket {
            // println!("From soket");
            // if let Ok((id, bytes)) = result {
            let read_result = socket.recv(&mut buf).await;
            if let Ok(count) = read_result {
                // println!("Read {} bytes", count);
                // if bytes.len() <= count {
                // println!("Count {}", count);
                if count > 0 {
                    let decr_res = session_key.decrypt(&buf[..count]);
                    // buf = BytesMut::zeroed(1100);
                    if let Ok(deciph) = decr_res {
                        // println!("Decrypted: {:?}", deciph);
                        // let deciph = Bytes::from(deciph);
                        // let mut byte_iterator = deciph.into_iter();
                        // let dgram_header = byte_iterator.next().unwrap();
                        let dgram_header = deciph.first().unwrap();
                        let swarm_id = dgram_header & 0b00111111;
                        if let Some((sender, cast_sender)) = senders.get(&swarm_id) {
                            if dgram_header & 0b11000000 == 0 {
                                // TODO: regular message
                                let deciphered = &deciph[1..];
                                // println!("received a regular message: {:?}", deciphered);
                                if let Ok(message) = bytes_to_message(deciphered) {
                                    // println!("decode OK: {:?}", message);
                                    let _send_result = sender.send(message);
                                    if _send_result.is_err() {
                                        // TODO: if failed maybe we are no longer interested?
                                        // Maybe send back a bye if possible?
                                        // Remove given Sender/Receiver pair
                                        println!("send result2: {:?}", _send_result);
                                    }
                                } else {
                                    println!("Failed to decode incoming stream");
                                }
                            } else if dgram_header & 0b11000000 == 64 {
                                let c_id = deciph[1];
                                match c_id {
                                    255 => {
                                        // TODO N_Req
                                        let n_req = bytes_to_neighbor_request(&deciph[2..]);
                                        let message = CastMessage::new_request(n_req);
                                        let _send_result = cast_sender.send(message);
                                        if _send_result.is_err() {
                                            println!("Unable to pass NeighborRequest to gnome");
                                        }
                                    }
                                    254 => {
                                        // TODO N_Resp
                                        let n_resp = bytes_to_neighbor_response(&deciph[2..]);
                                        let message = CastMessage::new_response(n_resp);
                                        let _send_result = cast_sender.send(message);
                                        if _send_result.is_err() {
                                            println!("Unable to pass NeighborResponseto gnome");
                                        }
                                    }
                                    _ => {
                                        if let Ok(message) = bytes_to_cast_message(&deciph) {
                                            // println!("decode OK: {:?}", message);
                                            let _send_result = cast_sender.send(message);
                                            if _send_result.is_err() {
                                                // TODO: if failed maybe we are no longer interested?
                                                // Maybe send back a bye if possible?
                                                // Remove given Sender/Receiver pair
                                                println!("send result2: {:?}", _send_result);
                                            }
                                        }
                                    }
                                }
                                // } else {
                                //     println!("Failed to decode incoming stream");
                            } else if let Ok(message) = bytes_to_cast_message(&deciph) {
                                // println!("decode OK: {:?}", message);
                                let _send_result = cast_sender.send(message);
                                if _send_result.is_err() {
                                    // TODO: if failed maybe we are no longer interested?
                                    // Maybe send back a bye if possible?
                                    // Remove given Sender/Receiver pair
                                    println!("send result2: {:?}", _send_result);
                                }
                                // }
                            } else {
                                println!("Unable to decode message");
                            }
                        } else {
                            println!("Got a message on new channel...");
                            // TODO: maybe we should instantiate a new
                            // Neighbor for a new swarm?
                            // For this we need Sender<Subscription> ?
                            let neighbor_request = bytes_to_neighbor_request(&deciph[2..]);
                            // println!("NR: {:?}", neighbor_request);
                            if let NeighborRequest::CreateNeighbor(remote_gnome_id, swarm_name) =
                                neighbor_request
                            {
                                println!("Create neighbor");
                                let (s1, r1) = channel();
                                let (s2, r2) = channel();
                                let (s3, r3) = channel();
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
                                let _ = sub_sender
                                    .send(Subscription::IncludeNeighbor(swarm_name, neighbor));
                            }
                        }
                    } else {
                        println!("Failed to decipher incoming stream {}", count);
                    }
                }
            } else {
                println!("SRCV Unable to recv supposedly ready data");
            }
        } else {
            let bytes = result.unwrap();
            if bytes.is_empty() {
                continue;
            }
            let ciphered = session_key.encrypt(&bytes);
            let len = 43 + ciphered.len() as u64;
            if len <= available_tokens {
                let _send_result = socket.send(&ciphered).await;
                // available_tokens -= len;
                available_tokens = if len > available_tokens {
                    0
                } else {
                    available_tokens - len
                };
            } else {
                println!("Waiting for tokens...");
                let _ = token_sender.send(Token::Request(2 * len));
                let res = token_reciever.recv();
                match res {
                    Ok(Token::Provision(amount)) => {
                        available_tokens += amount;
                        let _send_result = socket.send(&ciphered).await;
                        available_tokens = if len > available_tokens {
                            0
                        } else {
                            available_tokens - len
                        };
                    }
                    Ok(other) => println!("Received unexpected Token: {:?}", other),
                    Err(e) => {
                        panic!("Error while waiting for Tokens: {:?}", e);
                    }
                }
            }
        }
    }
    // }
    // }
}

pub async fn serve_socket(
    session_key: SessionKey,
    socket: UdpSocket,
    send_recv_pairs: Vec<(
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
    sender: Sender<Subscription>,
    shared_sender: Sender<(
        String,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
    extend_receiver: Receiver<(
        String,
        Sender<Message>,
        Sender<CastMessage>,
        Receiver<WrappedMessage>,
    )>,
) {
    println!("SRVC Racing tasks");
    race_tasks(
        session_key,
        socket,
        send_recv_pairs,
        token_sender,
        token_reciever,
        sender,
        shared_sender,
        extend_receiver,
    )
    .await;
    println!("SRVC Racing tasks over");
}
