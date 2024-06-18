use crate::crypto::SessionKey;
use crate::data_conversion::bytes_to_message;
use crate::data_conversion::message_to_bytes;
use crate::networking::token::Token;
use async_std::net::UdpSocket;
use async_std::task;
// TODO: get rid of bytes crate
use bytes::{BufMut, Bytes, BytesMut};
use core::panic;
use futures::{
    future::FutureExt, // for `.fuse()`
    pin_mut,
    select,
};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use swarm_consensus::Message;

async fn read_bytes_from_socket(socket: &UdpSocket) -> Result<(u8, Bytes), String> {
    // println!("read_bytes_from_socket");
    // TODO: increase size of buffer everywhere
    let mut buf = [0u8; 1100];
    let recv_result = socket.peek(&mut buf[..]).await;
    if let Ok(count) = recv_result {
        // println!("<<<<< {:?}", String::from_utf8_lossy(&buf[..count]));
        // When first byte starts with 1_111, next byte identifies swarm
        // if buf[0] & 0b1_111_0000 == 240 {
        //     Ok((buf[1], Bytes::from(Vec::from(&buf[2..count - 2]))))
        // } else {
        Ok((0, Bytes::from(Vec::from(&buf[..count]))))
        // }
    } else {
        println!("SKTd{:?}", recv_result);
        Err("Disconnected".to_string())
    }
}

async fn read_bytes_from_local_stream(
    receivers: &mut HashMap<u8, Receiver<Message>>,
) -> Result<(u8, Bytes), String> {
    // println!("read_bytes_from_local_stream");
    loop {
        for (id, receiver) in receivers.iter_mut() {
            let next_option = receiver.try_recv();
            if let Ok(message) = next_option {
                let bytes = message_to_bytes(message);
                // println!(">>>>> {:?}", bytes);

                if *id == 0 {
                    return Ok((*id, bytes));
                } else {
                    let mut extended_bytes = BytesMut::with_capacity(bytes.len() + 2);
                    extended_bytes.put_u8(240);
                    extended_bytes.put_u8(*id);
                    extended_bytes.put(bytes);
                    return Ok((*id, Bytes::from(extended_bytes)));
                }
            }
        }
        task::yield_now().await;
    }
    // Err(ConnError::LocalStreamClosed)
}

async fn race_tasks(
    session_key: SessionKey,
    socket: UdpSocket,
    send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
) {
    let mut senders: HashMap<u8, Sender<Message>> = HashMap::new();
    let mut receivers: HashMap<u8, Receiver<Message>> = HashMap::new();
    let min_tokens_threshold: u64 = 128;
    let mut available_tokens: u64 = min_tokens_threshold;
    // println!("racing: {:?}", send_recv_pairs);
    for (i, (sender, receiver)) in send_recv_pairs.into_iter().enumerate() {
        senders.insert(i as u8, sender);
        receivers.insert(i as u8, receiver);
    }
    let mut buf = [0u8; 1100];
    // if let Some((sender, mut receiver)) = send_recv_pairs.pop() {
    loop {
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
        let t1 = read_bytes_from_socket(&socket).fuse();
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
            for sender in senders.values() {
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
                        let deciph = Bytes::from(deciph);
                        let mut byte_iterator = deciph.into_iter();
                        let first_byte = byte_iterator.next().unwrap();
                        let (id, deciphered) = if first_byte & 0b1_111_0000 == 240 {
                            (byte_iterator.next().unwrap(), byte_iterator.collect())
                        } else {
                            let mut second = vec![first_byte];
                            for bte in byte_iterator {
                                second.push(bte);
                            }
                            (0, second.into_iter().collect())
                        };
                        // let deciphered = byte_iterator.collect();
                        // println!("Decrypted msg: {:?}", deciphered);
                        if let Ok(message) = bytes_to_message(&deciphered) {
                            // println!("decode OK: {:?}", message);
                            if let Some(sender) = senders.get(&id) {
                                // if message.is_bye() {
                                //     println!("Sending: {:?}", message);
                                // }
                                let _send_result = sender.send(message);
                                if _send_result.is_err() {
                                    println!("send result2: {:?}", _send_result);
                                }
                            } else {
                                println!("Did not find sender for {}", id);
                            }
                        } else {
                            println!("Failed to decode incoming stream");
                        }
                    } else {
                        println!("Failed to decipher incoming stream {}", count);
                    }
                }
                // } else {
                //     println!("SRCV Peeked != recv");
                // }
            } else {
                println!("SRCV Unable to recv supposedly ready data");
            }
        } else {
            let (_id, bytes) = result.unwrap();
            let ciphered = Bytes::from(session_key.encrypt(&bytes));
            let len = 42 + ciphered.len() as u64;
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
    send_recv_pairs: Vec<(Sender<Message>, Receiver<Message>)>,
    token_sender: Sender<Token>,
    token_reciever: Receiver<Token>,
) {
    println!("SRVC Racing tasks");
    race_tasks(
        session_key,
        socket,
        send_recv_pairs,
        token_sender,
        token_reciever,
    )
    .await;
    println!("SRVC Racing tasks over");
}
