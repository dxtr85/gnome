use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::networking::{Nat, NetworkSettings, PortAllocationRule, Transport as GTransport};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    Udp,
    Tcp,
}
impl Transport {
    pub fn is_udp(&self) -> bool {
        matches!(self, Self::Udp)
    }
    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransportState {
    Unknown,
    NotAvailable,
    Filtered,
    Working,
}
impl TransportState {
    pub fn unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}
pub struct TransportStatus {
    state: TransportState,
    nat: Nat,
    port_rule: PortAllocationRule,
    last_port_number: u16,
    min_port_number: u16,
    max_port_number: u16,
    minimum_increase_between_ports: u16,
}
impl TransportStatus {
    pub fn new(port: u16) -> Self {
        TransportStatus {
            state: TransportState::Unknown,
            nat: Nat::Unknown,
            port_rule: PortAllocationRule::Random,
            last_port_number: port,
            min_port_number: 0,
            max_port_number: 0,
            minimum_increase_between_ports: 0,
        }
    }
    pub fn update(&mut self, nat: Nat, port: u16, (rule, _step): (PortAllocationRule, i8)) {
        self.state = TransportState::Working;
        self.nat.update(nat);
        self.port_rule.update(rule);
        if self.nat.no_nat() {
            // Do not update ports if no nat
            return;
        }
        let port_diff = if port > self.last_port_number {
            port - self.last_port_number
        } else {
            self.last_port_number - port
        };
        if port_diff < self.minimum_increase_between_ports {
            self.minimum_increase_between_ports = port_diff;
        }
        if port > self.max_port_number {
            self.max_port_number = port;
        }
        if port < self.min_port_number {
            self.min_port_number = port;
        }
        self.last_port_number = port;
        //todo
        // self.ip6.udp.state = TransportState::Working;
        // match nat {
        //     Nat::None | Nat::FullCone => {
        //         self.udpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
        //     }
        //     other => {
        //         self.udpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
        //     }
        // }
        //
        //
        //
        // //             if self.tcpv6.0 {
        //         match nat {
        //             Nat::None | Nat::FullCone => {
        //                 self.tcpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
        //             }
        //             other => self.tcpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule),
        //         }
        //     } else {
        //         if self.udp.0 {
        //             match nat {
        //                 Nat::None | Nat::FullCone => {
        //                     self.udp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
        //                 }
        //                 other => {
        //                     self.udp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
        //                 }
        //             }
        //         }
        //         if self.tcp.0 {
        //             match nat {
        //                 Nat::None | Nat::FullCone => {
        //                     self.tcp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
        //                 }
        //                 other => {
        //                     self.tcp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
        //                 }
        //             }
        //         }
        //     }
    }
    pub fn get_settings(&self, pub_ip: IpAddr, transport: GTransport) -> NetworkSettings {
        //TODO: port should depend on Nat type
        let pub_port = match self.nat {
            Nat::None | Nat::FullCone | Nat::SymmetricWithPortControl => {
                //todo: derive port form GnomeId
                // 0
                self.last_port_number
            }
            _other => self.last_port_number + self.minimum_increase_between_ports,
        };
        NetworkSettings {
            pub_ip,
            pub_port,
            nat_type: self.nat,
            port_allocation: (self.port_rule, self.minimum_increase_between_ports as i8),
            transport,
        }
    }
}

pub struct IPStatus {
    pub_address: IpAddr,
    udp: TransportStatus,
    tcp: TransportStatus,
}
impl IPStatus {
    pub fn new_ip4(port: u16) -> Self {
        IPStatus {
            pub_address: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            udp: TransportStatus::new(port),
            tcp: TransportStatus::new(port),
        }
    }
    pub fn new_ip6(port: u16) -> Self {
        IPStatus {
            pub_address: IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
            udp: TransportStatus::new(port),
            tcp: TransportStatus::new(port),
        }
    }
    pub fn is_ip_known(&self) -> bool {
        match self.pub_address {
            IpAddr::V4(v4) => {
                for oct in v4.octets() {
                    if oct > 0 {
                        return true;
                    }
                }
                false
            }
            IpAddr::V6(v6) => {
                for oct in v6.octets() {
                    if oct > 0 {
                        return true;
                    }
                }
                false
            }
        }
    }
    pub fn update(
        &mut self,
        ip: IpAddr,
        port: u16,
        nat: Nat,
        (rule, step): (PortAllocationRule, i8),
        transport: Transport,
    ) -> Option<Vec<(IpAddr, u16, Nat, (PortAllocationRule, i8))>> {
        let other_zeroes = {
            match ip {
                IpAddr::V4(ip) => {
                    ip.octets()
                        .into_iter()
                        .rev()
                        .skip(1)
                        .filter(|o| *o == 0)
                        .count()
                        == 3
                }
                IpAddr::V6(ip) => {
                    ip.octets()
                        .into_iter()
                        .rev()
                        .skip(1)
                        .filter(|o| *o == 0)
                        .count()
                        == 15
                }
            }
        };
        // let oct = match ip {
        //     IpAddr::V4(ip) => *ip.octets().last().take().unwrap(),
        //     IpAddr::V6(ip) => *ip.octets().last().take().unwrap(),
        // };
        //TODO: port numbers from 2 to 5 also have meaning
        // 2 - no data received - remote host offline
        // 3 - decode failure - something went wrong, but communication
        //     is possible
        // 4 - missing data - also something went wrong, but communication
        //     is possible
        // 5 - no data on dedicated socket - remote has symmetric NAT
        //
        // Only when STUN timeout we should assume UDP is blocked
        // and only use TCP
        if port < 6 {
            if other_zeroes {
                match transport {
                    Transport::Udp => {
                        if port == 0 {
                            self.udp.state = TransportState::NotAvailable;
                        } else if port == 1 {
                            // self.udp.state = TransportState::Working;
                        } else if port == 2 {
                            //TODO: maybe only STUN gets filtered?
                            self.udp.state = TransportState::Filtered;
                        } else {
                            eprintln!("Unexpected port value: {}", port);
                        }
                    }
                    Transport::Tcp => {
                        if port == 0 {
                            self.tcp.state = TransportState::NotAvailable;
                        } else if port == 1 {
                            // self.tcp.state = TransportState::Working;
                        } else if port == 2 {
                            //TODO: maybe only STUN gets filtered?
                            self.tcp.state = TransportState::Filtered;
                        } else {
                            eprintln!("Unexpected port value: {}", port);
                        }
                    }
                }
            } else {
                // Here we received info from a regular host, not STUN
                // but, something went wrong
                //
                // self.pub_address = ip;
                // self.ip6.=ip;
                // TODO: update port attrs
                match port {
                    2 => {
                        //TODO: filtered or host offline
                        eprintln!("Host: {} offline, or ISP filtered", ip);
                    }
                    3 => {
                        //TODO: decode
                        eprintln!("Host: {} sent corrupted data", ip);
                    }
                    4 => {
                        //TODO: missing
                        eprintln!("Host: {} failed to send required data", ip);
                    }
                    5 => {
                        //TODO: dedicated
                        eprintln!("Host: {} behind Symmetric NAT", ip);
                    }
                    other => {
                        eprintln!("Unexpected port: {} (from: {})", other, ip);
                    }
                }
                if self.udp.state.unknown() && transport.is_udp() {
                    self.udp.state = TransportState::Working;
                }
                if self.tcp.state.unknown() && transport.is_tcp() {
                    self.tcp.state = TransportState::Working;
                }
            }
        } else {
            // Here we received a proper port number,
            // so we should update ip & port attributes
            if self.udp.state.unknown() && transport.is_udp() {
                self.udp.state = TransportState::Working;
            }
            if self.tcp.state.unknown() && transport.is_tcp() {
                self.tcp.state = TransportState::Working;
            }
            //
            // But we receive this info from various sources,
            // (both internally & externally)
            // so we need to process it with caution
            //
            // For now any implementation will do
            self.pub_address = ip;
            match transport {
                Transport::Udp => {
                    self.udp.update(nat, port, (rule, step));
                    if nat.no_nat() {
                        self.tcp.update(nat, port, (rule, step));
                    }
                }
                Transport::Tcp => {
                    self.tcp.update(nat, port, (rule, step));
                    if nat.no_nat() {
                        self.udp.update(nat, port, (rule, step));
                    }
                }
            }
        }
        //TODO: add logic to check if all 4 conn_channels sent any response
        // if so, we check if there are any public ip assigned
        // if so, we send this info to our swarm
        // This might result in Manifest changing multiple times,
        // but for now it should be fine
        //
        //
        //
        // let mut comm_channels = vec![];
        // // Only when we hear back from all possible networking channel services
        // if self.udp.0 && self.tcp.0 && self.udpv6.0 && self.tcpv6.0 {
        //     if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udp.1 {
        //         comm_channels.push((ip, port, nat, port_rule));
        //     }
        //     if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udpv6.1 {
        //         comm_channels.push((ip, port, nat, port_rule));
        //     }
        // }
        // Some(comm_channels)

        None
    }
    pub fn get_settings(&self, is_ipv4: bool) -> Vec<NetworkSettings> {
        let mut res = vec![];
        if self.udp.state == TransportState::Working {
            let transport = if is_ipv4 {
                GTransport::UDPoverIP4
            } else {
                GTransport::UDPoverIP6
            };
            res.push(self.udp.get_settings(self.pub_address.clone(), transport));
        }
        if self.tcp.state == TransportState::Working {
            let transport = if is_ipv4 {
                GTransport::TCPoverIP4
            } else {
                GTransport::TCPoverIP6
            };
            res.push(self.tcp.get_settings(self.pub_address.clone(), transport));
        }
        eprintln!("ReturningMyNS: {:?}", res);
        res
    }
}

pub struct NetworkSummary {
    ip4: IPStatus,
    ip6: IPStatus,
}
impl NetworkSummary {
    pub fn empty(port: u16) -> Self {
        NetworkSummary {
            ip4: IPStatus::new_ip4(port),
            ip6: IPStatus::new_ip6(port + 1),
        }
    }
    pub fn get_network_settings(&self) -> Vec<NetworkSettings> {
        let mut bytes = Vec::with_capacity(4);
        // for ns in self.ip4.get_settings(true) {
        if self.ip4.is_ip_known() {
            bytes.append(&mut self.ip4.get_settings(true));
        }
        // }
        // for ns in self.ip6.get_settings(false) {
        if self.ip6.is_ip_known() {
            bytes.append(&mut self.ip6.get_settings(false));
        }
        // sett.extend(self.ip6.get_settings(false));
        bytes
    }
    pub fn get_network_settings_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(128);
        for ns in self.ip4.get_settings(true) {
            bytes.append(&mut ns.bytes());
        }
        for ns in self.ip6.get_settings(false) {
            bytes.append(&mut ns.bytes());
        }
        // sett.extend(self.ip6.get_settings(false));
        bytes
    }

    pub fn process(
        &mut self,
        ip: IpAddr,
        port: u16,
        nat: Nat,
        (rule, step): (PortAllocationRule, i8),
        transport: Transport,
    ) -> Option<Vec<(IpAddr, u16, Nat, (PortAllocationRule, i8))>> {
        eprintln!(
            "Processing PubAddr {}:{}({:?}, {:?})",
            ip, port, transport, nat
        );
        //TODO: return Some if our address has changed in a way
        // that it would not be possible to connect to us
        // None
        //TODO: extend this logic:
        // STUN messages can be filtered out
        // entire transport protocol can be blocked by ISP
        // - we need to store this information.
        // In case we can not discover port allocation rule via STUN,
        // we can try using public ip & port data we receive frow
        // every neighbor we attach to.
        //
        // We should also specify which transport protocol should
        // be used, in order to prevent pointless punching.
        //
        //
        //
        //
        //
        //
        // below is code copied from ChannelAvailablilty

        // eprintln!("get public comm channels: {}:{} {:?}", ip, port, nat);
        //TODO: we need to hold a structure for all 4 possible IP/Transport
        // combinations.
        // First we should receive info whether or not given pair is supported
        // by this host
        // Later, for those supported we are expected to receive our public IP&Port
        // Once we have all those public facing data we can send it to a swarm
        // that we are founder of in order for it to update Manifest
        // We currently only use STUN via UDP to query about public IP, so in some
        // cases this might fail to provide us with required data.
        //
        // if port == 0 then if ip = 0.0.0.2 then network failed to bind udp socket
        // if port == 1 then if ip = 0.0.0.2 then network bind udp socket is ok
        // if port == 2 then if ip = 0.0.0.2 then response was probably blocked
        // if port == 0 then if ip = ::2 then network failed to bind udpv6 socket
        // if port == 1 then if ip = ::2 then network bind udpv6 socket is ok
        // if port == 0 then if ip = 0.0.0.3 then network failed to bind tcp socket
        // if port == 1 then if ip = 0.0.0.3 then network bind tcp socket is ok
        // if port == 0 then if ip = ::3 then network failed to bind tcpv6 socket
        // if port == 1 then if ip = ::3 then network bind tcpv6 socket is ok
        // if port == 2 then if ip = ::3 then response was probably blocked
        // other port value means that we have a public ip and port
        // we assign those values to both udp & tcp (or udpv6 & tcpv6),
        // since over tcp we do not currently have any pub ip discovery logic
        // match port {
        //     0 => {
        //
        // not expecting to receive a legitlmate port less than 1024
        // but anyway…
        if ip.is_ipv4() {
            self.ip4.update(ip, port, nat, (rule, step), transport)
        } else {
            self.ip6.update(ip, port, nat, (rule, step), transport)
        }
    }
}

// enum ChannelAvailability {
//     Unknown,
//     Supported,
//     Available(IpAddr, u16, Nat, (PortAllocationRule, i8)),
//     NotAvailable,
// }

// bool in each transport says if gnome's networking service has tried to bind it
// pub struct CommunicationChannels {
//     udp: (bool, ChannelAvailability),
//     tcp: (bool, ChannelAvailability),
//     udpv6: (bool, ChannelAvailability),
//     tcpv6: (bool, ChannelAvailability),
// }
// impl CommunicationChannels {
//     fn get_public_comm_channels(
//         &mut self,
//         ip: IpAddr,
//         port: u16,
//         nat: Nat,
//         port_rule: (PortAllocationRule, i8),
//     ) -> Vec<(IpAddr, u16, Nat, (PortAllocationRule, i8))> {
//         // eprintln!("get public comm channels: {}:{} {:?}", ip, port, nat);
//         //TODO: we need to hold a structure for all 4 possible IP/Transport
//         // combinations.
//         // First we should receive info whether or not given pair is supported
//         // by this host
//         // Later, for those supported we are expected to receive our public IP&Port
//         // Once we have all those public facing data we can send it to a swarm
//         // that we are founder of in order for it to update Manifest
//         // We currently only use STUN via UDP to query about public IP, so in some
//         // cases this might fail to provide us with required data.
//         //
//         // if port == 0 then if ip = 0.0.0.2 then network failed to bind udp socket
//         // if port == 1 then if ip = 0.0.0.2 then network bind udp socket is ok
//         // if port == 2 then if ip = 0.0.0.2 then response was probably blocked
//         // if port == 0 then if ip = ::2 then network failed to bind udpv6 socket
//         // if port == 1 then if ip = ::2 then network bind udpv6 socket is ok
//         // if port == 0 then if ip = 0.0.0.3 then network failed to bind tcp socket
//         // if port == 1 then if ip = 0.0.0.3 then network bind tcp socket is ok
//         // if port == 0 then if ip = ::3 then network failed to bind tcpv6 socket
//         // if port == 1 then if ip = ::3 then network bind tcpv6 socket is ok
//         // if port == 2 then if ip = ::3 then response was probably blocked
//         // other port value means that we have a public ip and port
//         // we assign those values to both udp & tcp (or udpv6 & tcpv6),
//         // since over tcp we do not currently have any pub ip discovery logic
//         // match port {
//         //     0 => {
//         let (ipv6, oct) = match ip {
//             IpAddr::V4(ip) => (false, *ip.octets().last().take().unwrap()),
//             IpAddr::V6(ip) => (true, *ip.octets().last().take().unwrap()),
//         };
//         if port < 2 {
//             match oct {
//                 2 => {
//                     if port == 0 {
//                         if ipv6 {
//                             //TODO: udpv6 disabled
//                             self.udpv6 = (true, ChannelAvailability::NotAvailable);
//                         } else {
//                             //TODO: udp disabled
//                             self.udp = (true, ChannelAvailability::NotAvailable);
//                         }
//                     } else if port == 1 {
//                         if ipv6 {
//                             //TODO: udpv6 enabled
//                             self.udpv6 = (true, ChannelAvailability::Supported);
//                         } else {
//                             //TODO: udp enabled
//                             self.udp = (true, ChannelAvailability::Supported);
//                         }
//                     } else {
//                         eprintln!("Unexpected port value: {}", port);
//                     }
//                 }
//                 3 => {
//                     if port == 0 {
//                         if ipv6 {
//                             //TODO: tcpv6 disabled
//                             self.tcpv6 = (true, ChannelAvailability::NotAvailable);
//                         } else {
//                             //TODO: tcp disabled
//                             self.tcp = (true, ChannelAvailability::NotAvailable);
//                         }
//                     } else if port == 1 {
//                         if ipv6 {
//                             //TODO: tcpv6 enabled
//                             self.tcpv6 = (true, ChannelAvailability::Supported);
//                         } else {
//                             //TODO: tcp enabled
//                             self.tcp = (true, ChannelAvailability::Supported);
//                         }
//                     } else {
//                         eprintln!("Unexpected port value: {}", port);
//                     }
//                 }
//                 other => {
//                     eprintln!("Unexpected last ip octet value: {}", other);
//                 }
//             }
//         } else if ipv6 {
//             if self.udpv6.0 {
//                 match nat {
//                     Nat::None | Nat::FullCone => {
//                         self.udpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
//                     }
//                     other => self.udpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule),
//                 }
//             }
//             if self.tcpv6.0 {
//                 match nat {
//                     Nat::None | Nat::FullCone => {
//                         self.tcpv6.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
//                     }
//                     other => self.tcpv6.1 = ChannelAvailability::Available(ip, 0, other, port_rule),
//                 }
//             } else {
//                 if self.udp.0 {
//                     match nat {
//                         Nat::None | Nat::FullCone => {
//                             self.udp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
//                         }
//                         other => {
//                             self.udp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
//                         }
//                     }
//                 }
//                 if self.tcp.0 {
//                     match nat {
//                         Nat::None | Nat::FullCone => {
//                             self.tcp.1 = ChannelAvailability::Available(ip, port, nat, port_rule)
//                         }
//                         other => {
//                             self.tcp.1 = ChannelAvailability::Available(ip, 0, other, port_rule)
//                         }
//                     }
//                 }
//             }
//         }
//         //TODO: add logic to check if all 4 conn_channels sent any response
//         // if so, we check if there are any public ip assigned
//         // if so, we send this info to our swarm
//         // This might result in Manifest changing multiple times,
//         // but for now it should be fine
//         let mut comm_channels = vec![];
//         // Only when we hear back from all possible networking channel services
//         if self.udp.0 && self.tcp.0 && self.udpv6.0 && self.tcpv6.0 {
//             if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udp.1 {
//                 comm_channels.push((ip, port, nat, port_rule));
//             }
//             if let ChannelAvailability::Available(ip, port, nat, port_rule) = self.udpv6.1 {
//                 comm_channels.push((ip, port, nat, port_rule));
//             }
//         }
//         comm_channels
//     }
// }
