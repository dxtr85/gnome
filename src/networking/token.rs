use async_std::task::{self, yield_now};
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Debug)]
pub enum Token {
    Provision(u64),
    Unused(u64),
    Request(u64),
}

pub async fn token_dispenser(
    max_buffer_size_bytes: u64,
    bandwith_bytes_sec: u64,
    reciever: Receiver<(Sender<Token>, Receiver<Token>)>,
    // when we join a new swarm we receive a sender to inform that gnome about avail
    // bandwith
    band_reciever: Receiver<Sender<u64>>,
) {
    println!("Starting token dispenser service");
    let mut tokens_available_from_buffer = max_buffer_size_bytes;
    let mut tokens_available_from_time_pass = bandwith_bytes_sec;

    let bytes_per_msec: u64 = bandwith_bytes_sec >> 10;
    // this is to provision tokens on a per socket basis
    let mut token_eaters: Vec<TokenEater> = Vec::with_capacity(16);
    // this is to notify gnomes
    let mut bandwith_notification_senders: VecDeque<Sender<u64>> = VecDeque::new();
    let dur = Duration::from_millis(1);
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

    let sent_avail_bandwith: u64 = bandwith_bytes_sec;
    // TODO: we need to think about how to calculate this
    let token_size = bandwith_bytes_sec >> 6;

    yield_now().await;
    // New concept:
    // We have two token sources:
    // 1 - time every 1ms bandwith_bytes_sec>>10 is added to bucket
    // 2 - network card buffer remains at fixed size,
    //         unless we take from it, then it is being
    //         replenished at the same rate as 1st source
    // (only if 2nd source is full we increase the first one)
    // Add tokens to source 1 if 2 is full, otherwise only add to 2.
    // Try_recv requests from sockets and respond to them
    // Send out tokens to every socket every 128ms
    // Collect how many tokens were unused per socket
    //
    // We have to assume that network comms are jerky, we might only send
    // packets every 16 seconds or so.
    // For this we need to track token usage per socket on wider time-frame.
    // 64 seconds is the minimum.
    // We collect 8 Unused messages in 1 second per socket.
    // So we need a table of [u64;64] per socket.
    // If a socket didn't respond, we assume no tokens were consumed
    // in order not to starve our service due to unresponsive socket.
    // At the beginning we split tokens equally among all sockets.
    // Only when we see a noticable decrease in Unused tokens table we need to act.
    // First approach would be to take from every other socket and give it
    // to that heavily used one.
    // If other socket is also below safe threshold we leave it be.
    // We need a way to dynamically define safety thresholds.
    // If there is only one socket the situation is completely different to that
    // where we have 100 or more sockets to feed.
    // And in those 100 sockets there can be only a few that are being heavily used
    // the rest might be only using minimal amount of tokens.
    // Maybe we need to assign some categories to sockets depending on their usage
    // Maybe turtle, rabbit and cheetah?
    // First we assign all sockets a rabbit category. Then as we see their usage
    // we may change that.
    // These categories differ by their safety thresholds. Turtle can go down
    // a long way and raise no alarm, whereas cheetah will ring a bell in no time.
    // But if a turtle sends requests for more tokens that means we went too low.
    // Then we change category to rabbit to increase those thresholds.
    // If a cheetah is increasing it's unused tokens count, we change it to rabbit.

    let mut iteration: u8 = 0;
    loop {
        // print!("L");
        let send_tokens = if iteration >= 128 {
            iteration = 0;
            true
        } else {
            false
        };

        // We receive a tick every ms
        if timer_reciever.try_recv().is_ok() {
            iteration += 1;

            // We listen for new gnomes
            while let Ok(sender) = band_reciever.try_recv() {
                // Here we send to every gnome how many bps we have unused
                let res = sender.send(sent_avail_bandwith);
                if res.is_ok() {
                    bandwith_notification_senders.push_back(sender);
                }
            }

            // If we receive a new channel pair, we send some tokens immediately
            if let Ok((s, r)) = reciever.try_recv() {
                let mut broken_pipe = false;
                let new_size = token_size;
                let res = s.send(Token::Provision(new_size));
                if res.is_err() {
                    broken_pipe = true;
                }
                if !broken_pipe {
                    token_eaters.push(TokenEater::new(s, r));
                }
            }

            // update how many tokens we have to spend
            if tokens_available_from_buffer < max_buffer_size_bytes {
                tokens_available_from_buffer += bytes_per_msec;
                if tokens_available_from_buffer > max_buffer_size_bytes {
                    let tokens_left = tokens_available_from_buffer - max_buffer_size_bytes;
                    tokens_available_from_buffer = max_buffer_size_bytes;
                    tokens_available_from_buffer += tokens_left;
                }
            } else {
                tokens_available_from_time_pass += bytes_per_msec;
            }
        }

        if send_tokens {
            // By sending 0 we request a gnome to select any Neighbor
            // it has and send WrappedMessage::NoOp
            // to trigger token collection
            if let Some(sender) = bandwith_notification_senders.pop_front() {
                let _ = sender.send(0);
                // TODO: here we should send sum of unused tokens from last second
                let res = sender.send(tokens_available_from_time_pass << 3);
                if res.is_ok() {
                    bandwith_notification_senders.push_back(sender);
                }
            };

            let eaters_count: u64 = token_eaters.len() as u64;
            let tokens_per_eater = if eaters_count > 0 {
                tokens_available_from_time_pass / eaters_count
            } else {
                tokens_available_from_time_pass
            };
            let mut to_removal = vec![];
            for (i, token_eater) in token_eaters.iter_mut().enumerate() {
                token_eater.shift_history();
                let res = token_eater.send.send(Token::Provision(tokens_per_eater));
                if res.is_err() {
                    to_removal.push(i);
                } else {
                    tokens_available_from_time_pass -= tokens_per_eater;
                }
            }
            while let Some(index) = to_removal.pop() {
                token_eaters.swap_remove(index);
            }
        } else {
            // TODO: extend the logic to store unused tokens history per socket
            for token_eater in token_eaters.iter_mut() {
                while let Ok(request) = token_eater.recv.try_recv() {
                    match request {
                        Token::Request(req_size) => {
                            if tokens_available_from_time_pass >= req_size {
                                tokens_available_from_time_pass -= req_size;
                                let _ = token_eater.send.send(Token::Provision(req_size));
                            } else {
                                let mut missing_tokens = req_size - tokens_available_from_time_pass;
                                tokens_available_from_time_pass = 0;
                                if tokens_available_from_buffer >= missing_tokens {
                                    tokens_available_from_buffer -= missing_tokens;
                                    missing_tokens = 0;
                                    let _ = token_eater
                                        .send
                                        .send(Token::Provision(req_size - missing_tokens));
                                    // } else {
                                    //     missing_tokens -= tokens_available_from_buffer;
                                    //     tokens_available_from_buffer = 0;
                                    //     // We do not send tokens in this case
                                }
                            }
                        }
                        Token::Unused(size) => {
                            token_eater.add_to_pending(size);
                        }
                        _ => (),
                    }
                }
            }
        }
        yield_now().await;
    }
}
enum Category {
    Turtle,
    Rabbit,
    Cheetah,
}

struct TokenEater {
    cat: Category,
    send: Sender<Token>,
    recv: Receiver<Token>,
    history: [u64; 64],
    iter: u8,
}

impl TokenEater {
    pub fn new(send: Sender<Token>, recv: Receiver<Token>) -> Self {
        TokenEater {
            cat: Category::Rabbit,
            send,
            recv,
            history: [0; 64],
            iter: 0,
        }
    }
    pub fn add_to_pending(&mut self, value: u64) {
        self.history[0] += value;
    }
    pub fn shift_history(&mut self) {
        self.iter += 1;
        if self.iter >= 8 {
            self.iter = 0;
            let mut prev = 0;
            for i in 0..64 {
                std::mem::swap(&mut self.history[i], &mut prev);
            }
        }
    }
}
