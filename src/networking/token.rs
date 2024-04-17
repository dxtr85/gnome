use async_std::task;
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Debug)]
pub enum Token {
    Provision(u32),
    Unused(u32),
    Request(u32),
}

pub async fn token_dispenser(
    buffer_size_bytes: u32,
    bandwith_bytes_sec: u32,
    // sender: Sender<TokenMessage>,
    reciever: Receiver<(Sender<Token>, Receiver<Token>)>,
    band_reciever: Receiver<Sender<u32>>,
) {
    // Cover case when buffer size is less than bandwith
    let buffer_size_bytes = std::cmp::max(bandwith_bytes_sec, buffer_size_bytes);
    let mut available_buffer = buffer_size_bytes;
    let bytes_per_msec: u32 = bandwith_bytes_sec / 1000;
    let mut socket_pipes: VecDeque<(Sender<Token>, Receiver<Token>, u32, bool)> = VecDeque::new();
    let mut bandwith_notification_senders: VecDeque<Sender<u32>> = VecDeque::new();
    let dur = Duration::from_micros(1000);
    // let dur = Duration::from_millis(1000);
    let (timer_sender, timer_reciever) = channel();

    async fn timer(duration: Duration, sender: Sender<()>) {
        loop {
            task::sleep(duration).await;
            let res = sender.send(());
            if res.is_err() {
                break;
            }
        }
    }
    task::spawn(timer(dur, timer_sender));

    let mut sent_avail_bandwith: u32 = bandwith_bytes_sec;
    let token_size = 256;
    let min_token_size = 256;
    let min_overhead = 4;
    let max_overhead = 64;
    let mut overhead = 16;
    let mut used_bandwith_msec: u32 = 0;
    let mut used_bandwith: VecDeque<u32> = VecDeque::from(vec![0; 100]);
    let mut used_bandwith_ring: VecDeque<(bool, u32)> = VecDeque::from(vec![(false, 0); 9]);
    used_bandwith_ring.push_back((true, 0));
    let mut additional_request_received = false;

    loop {
        let mut send_tokens = false;

        while let Ok(sender) = band_reciever.try_recv() {
            let res = sender.send(sent_avail_bandwith);
            if res.is_ok() {
                bandwith_notification_senders.push_back(sender);
            }
        }
        if let Ok((s, r)) = reciever.try_recv() {
            // match tm {
            //     TokenMessage::Add((s, r)) => {
            socket_pipes.push_back((s, r, token_size, false));
            //     }
            //     TokenMessage::AvailBandwith(_) => {
            //         let _ = sender.send(TokenMessage::AvailBandwith(std::cmp::max(
            //             bandwith_bytes_sec - used_bandwith.iter().sum::<u32>(),
            //             0,
            //         )));
            //     }
            //     TokenMessage::SlowDown => {
            //         //TODO: better algo
            //         token_size = std::cmp::max(min_token_size, token_size >> 1);
            //     }
            // }
        }
        if let Ok(_) = timer_reciever.try_recv() {
            available_buffer = std::cmp::min(buffer_size_bytes, available_buffer + bytes_per_msec);
            // println!("AvaBuf: {} {}", available_buffer, bytes_per_msec);
            send_tokens = true;

            if let Some((push, value)) = used_bandwith_ring.pop_front() {
                used_bandwith_ring.push_back((push, used_bandwith_msec));
                used_bandwith_msec = 0;
                if push {
                    let sum = used_bandwith_ring
                        .iter()
                        .fold(value, |acc, (_i, val)| acc + val);
                    let _ = used_bandwith.pop_front();
                    used_bandwith.push_back(sum);
                    let used_bandwith_one_sec = used_bandwith.iter().sum::<u32>();
                    let new_avail_bandwith =
                        bandwith_bytes_sec.saturating_sub(used_bandwith_one_sec);
                    if sent_avail_bandwith.abs_diff(new_avail_bandwith) * 10 > sent_avail_bandwith {
                        let how_many = bandwith_notification_senders.len();
                        for _i in [0..how_many] {
                            if let Some(sender) = bandwith_notification_senders.pop_front() {
                                let res = sender.send(new_avail_bandwith);
                                if res.is_ok() {
                                    bandwith_notification_senders.push_back(sender);
                                }
                            };
                        }
                        sent_avail_bandwith = new_avail_bandwith;
                    }
                }
            }

            // TODO: some better algo, maybe individual overhead per socket?
            if additional_request_received {
                overhead = std::cmp::min(max_overhead, overhead + 32);
            } else {
                overhead = std::cmp::max(min_overhead, overhead - 1);
            }
            additional_request_received = false;
        }

        for _i in 0..socket_pipes.len() {
            if let Some((s, r, prev_size, additional_request)) = socket_pipes.pop_front() {
                let mut token_sent = false;
                let mut broken_pipe = false;
                let mut req_size = 0;
                let mut unused_size = 0;
                let mut new_size: u32 = prev_size;
                let mut new_add_req = additional_request;
                while let Ok(request) = r.try_recv() {
                    match request {
                        Token::Request(size) => {
                            req_size += size;
                            new_add_req = true;
                            additional_request_received = true;
                        }
                        Token::Unused(size) => unused_size += size,
                        _ => (),
                    }
                }
                if req_size > 0 && available_buffer >= req_size {
                    let res = s.send(Token::Provision(req_size));
                    if res.is_err() {
                        broken_pipe = true;
                    }
                    available_buffer -= req_size;
                    used_bandwith_msec += req_size;
                    new_size += req_size;
                    token_sent = true;
                    //                     } else {
                    //                         let res = s.send(Token::Provision(available_buffer));
                    //                         if res.is_err() {
                    //                             broken_pipe = true;
                    //                         }
                    //                         used_bandwith_msec += available_buffer;
                    //                         new_size += available_buffer;
                    //                         available_buffer = 0;
                    //                         token_sent = true;
                    // }
                }
                // println!("unused: {}, req: {}", unused_size, req_size);
                if unused_size > 0 && req_size == 0 {
                    // println!("unused>0");
                    available_buffer += unused_size;
                    //TODO make sure if it's needed:
                    used_bandwith_msec.checked_sub(unused_size).unwrap_or(0);
                    // print!(
                    //     "max(min:{}, prev:{} - unused:{} + overhead:{}) ",
                    //     min_token_size, new_size, unused_size, overhead
                    // );
                    new_size = std::cmp::max(
                        min_token_size,
                        new_size.checked_sub(unused_size).unwrap_or(0) + overhead,
                    );
                    // println!("computed: {}", new_size);
                }
                if send_tokens && !token_sent {
                    // println!("sending:{}", new_size);
                    let res = s.send(Token::Provision(new_size));
                    if res.is_err() {
                        broken_pipe = true;
                    }
                    available_buffer = if new_size > available_buffer {
                        0
                    } else {
                        available_buffer - new_size
                    };

                    used_bandwith_msec += new_size;
                }
                if !broken_pipe {
                    socket_pipes.push_back((s, r, new_size, new_add_req));
                }
            }
        }
    }
}
