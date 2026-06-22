//! DMX-over-IP packet encoders (sACN E1.31 + Art-Net) and the UDP sink.
//!
//! Both protocols are plain UDP: a [`DmxSink`] owns a socket and a destination,
//! and [`DmxSink::send`] encodes one 512-slot universe frame and transmits it.
//! The encoders are free functions so they can be golden-tested without any
//! network (see the unit tests).

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Number of DMX slots in one universe.
pub const DMX_UNIVERSE_SIZE: usize = 512;

/// Standard UDP ports.
const SACN_PORT: u16 = 5568;
const ARTNET_PORT: u16 = 6454;

/// sACN total packet size: 38 (root) + 77 (framing) + 523 (DMP) = 638 bytes.
const SACN_PACKET_LEN: usize = 638;
/// Art-Net ArtDMX size: 18-byte header + 512 data bytes.
const ARTNET_PACKET_LEN: usize = 18 + DMX_UNIVERSE_SIZE;

/// Which DMX-over-IP protocol a universe is sent with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputProtocol {
    /// ANSI E1.31 (sACN) — multicast by default, priority-aware.
    Sacn,
    /// Art-Net — the older, broadcast/unicast standard.
    ArtNet,
}

/// One workspace-level universe output: where universe `universe` is sent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseOutput {
    /// Logical universe number (referenced by patched fixtures).
    pub universe: u16,
    /// Transport protocol.
    pub protocol: OutputProtocol,
    /// Explicit destination IP. For sACN, `None` selects the multicast group for
    /// the universe; Art-Net requires an explicit unicast/broadcast destination.
    pub destination: Option<IpAddr>,
    /// When false, the universe is not transmitted.
    pub enabled: bool,
}

/// The sACN multicast group address for a universe: `239.255.{hi}.{lo}`.
pub fn sacn_multicast_ip(universe: u16) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(239, 255, (universe >> 8) as u8, (universe & 0xff) as u8))
}

// ---------------------------------------------------------------------------
// Encoders
// ---------------------------------------------------------------------------

/// Encode one sACN (E1.31) DMX data packet.
///
/// `cid` is the 16-byte sender component identifier (stable per process);
/// `source_name` is shown by receivers; `priority` is 0–200 (100 = default).
pub fn encode_sacn(
    cid: &[u8; 16],
    source_name: &str,
    universe: u16,
    priority: u8,
    sequence: u8,
    dmx: &[u8; DMX_UNIVERSE_SIZE],
) -> Vec<u8> {
    let mut p = vec![0u8; SACN_PACKET_LEN];

    // ── Root layer ──────────────────────────────────────────────────────────
    p[0..2].copy_from_slice(&0x0010u16.to_be_bytes()); // preamble size
    // [2..4] post-amble size = 0
    // ACN packet identifier "ASC-E1.17\0\0\0"
    p[4..16].copy_from_slice(&[
        0x41, 0x53, 0x43, 0x2d, 0x45, 0x31, 0x2e, 0x31, 0x37, 0x00, 0x00, 0x00,
    ]);
    p[16..18].copy_from_slice(&pdu_flags_len(SACN_PACKET_LEN - 16)); // root PDU length
    p[18..22].copy_from_slice(&0x0000_0004u32.to_be_bytes()); // VECTOR_ROOT_E131_DATA
    p[22..38].copy_from_slice(cid);

    // ── Framing layer ───────────────────────────────────────────────────────
    p[38..40].copy_from_slice(&pdu_flags_len(SACN_PACKET_LEN - 38));
    p[40..44].copy_from_slice(&0x0000_0002u32.to_be_bytes()); // VECTOR_E131_DATA_PACKET
    let name = source_name.as_bytes();
    let n = name.len().min(63);
    p[44..44 + n].copy_from_slice(&name[..n]); // 64-byte field, null-padded
    p[108] = priority;
    // [109..111] synchronization address = 0
    p[111] = sequence;
    // [112] options = 0
    p[113..115].copy_from_slice(&universe.to_be_bytes());

    // ── DMP layer ───────────────────────────────────────────────────────────
    p[115..117].copy_from_slice(&pdu_flags_len(SACN_PACKET_LEN - 115));
    p[117] = 0x02; // VECTOR_DMP_SET_PROPERTY
    p[118] = 0xa1; // address type & data type
    p[119..121].copy_from_slice(&0x0000u16.to_be_bytes()); // first property address
    p[121..123].copy_from_slice(&0x0001u16.to_be_bytes()); // address increment
    p[123..125].copy_from_slice(&0x0201u16.to_be_bytes()); // property value count = 513
    p[125] = 0x00; // DMX start code
    p[126..638].copy_from_slice(dmx);

    p
}

/// E1.31 flags-and-length field: top nibble `0x7`, low 12 bits = `len`.
fn pdu_flags_len(len: usize) -> [u8; 2] {
    ((0x7000u16) | (len as u16 & 0x0fff)).to_be_bytes()
}

/// Encode one Art-Net `ArtDMX` packet.
pub fn encode_artnet(sequence: u8, universe: u16, dmx: &[u8; DMX_UNIVERSE_SIZE]) -> Vec<u8> {
    let mut p = vec![0u8; ARTNET_PACKET_LEN];
    p[0..8].copy_from_slice(b"Art-Net\0");
    p[8] = 0x00; // OpCode low byte
    p[9] = 0x50; // OpCode high byte → 0x5000 (OpOutput / ArtDMX)
    p[10] = 0x00; // protocol version high
    p[11] = 0x0e; // protocol version low (14)
    p[12] = sequence;
    p[13] = 0x00; // physical
    p[14] = (universe & 0xff) as u8; // SubUni (low 8 bits of port-address)
    p[15] = ((universe >> 8) & 0x7f) as u8; // Net (high 7 bits)
    p[16..18].copy_from_slice(&(DMX_UNIVERSE_SIZE as u16).to_be_bytes()); // length (big-endian)
    p[18..18 + DMX_UNIVERSE_SIZE].copy_from_slice(dmx);
    p
}

// ---------------------------------------------------------------------------
// Runtime UDP sink
// ---------------------------------------------------------------------------

/// A bound UDP socket targeting one universe's destination.
pub struct DmxSink {
    socket: UdpSocket,
    addr: SocketAddr,
    protocol: OutputProtocol,
    universe: u16,
    cid: [u8; 16],
    source_name: String,
}

impl DmxSink {
    /// Bind a socket and resolve the destination for `output`.
    pub fn new(output: &UniverseOutput, cid: [u8; 16], source_name: String) -> Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .map_err(|e| anyhow!("DMX socket bind failed: {e}"))?;

        let ip = match output.destination {
            Some(ip) => ip,
            None => match output.protocol {
                OutputProtocol::Sacn => sacn_multicast_ip(output.universe),
                OutputProtocol::ArtNet => {
                    return Err(anyhow!("Art-Net output requires an explicit destination IP"));
                }
            },
        };
        let port = match output.protocol {
            OutputProtocol::Sacn => SACN_PORT,
            OutputProtocol::ArtNet => ARTNET_PORT,
        };

        Ok(Self {
            socket,
            addr: SocketAddr::new(ip, port),
            protocol: output.protocol,
            universe: output.universe,
            cid,
            source_name,
        })
    }

    /// Encode and transmit one universe frame.
    pub fn send(&self, sequence: u8, dmx: &[u8; DMX_UNIVERSE_SIZE]) -> Result<()> {
        let packet = match self.protocol {
            OutputProtocol::Sacn => {
                encode_sacn(&self.cid, &self.source_name, self.universe, 100, sequence, dmx)
            }
            OutputProtocol::ArtNet => encode_artnet(sequence, self.universe, dmx),
        };
        self.socket
            .send_to(&packet, self.addr)
            .map_err(|e| anyhow!("DMX send failed: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sacn_packet_layout() {
        let cid = [0xAB; 16];
        let mut dmx = [0u8; DMX_UNIVERSE_SIZE];
        dmx[0] = 255;
        dmx[511] = 7;
        let p = encode_sacn(&cid, "WinCue", 1, 100, 42, &dmx);

        assert_eq!(p.len(), 638);
        assert_eq!(&p[0..2], &[0x00, 0x10]); // preamble size
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0"); // ACN identifier
        assert_eq!(&p[16..18], &[0x72, 0x6e]); // root flags+length (0x7000|622)
        assert_eq!(&p[18..22], &[0, 0, 0, 4]); // root vector
        assert_eq!(&p[22..38], &cid); // CID
        assert_eq!(&p[38..40], &[0x72, 0x58]); // framing flags+length (0x7000|600)
        assert_eq!(&p[40..44], &[0, 0, 0, 2]); // framing vector
        assert_eq!(&p[44..50], b"WinCue"); // source name
        assert_eq!(p[108], 100); // priority
        assert_eq!(p[111], 42); // sequence
        assert_eq!(&p[113..115], &[0, 1]); // universe
        assert_eq!(&p[115..117], &[0x72, 0x0b]); // DMP flags+length (0x7000|523)
        assert_eq!(p[117], 0x02); // DMP vector
        assert_eq!(p[118], 0xa1); // address & data type
        assert_eq!(&p[123..125], &[0x02, 0x01]); // property value count = 513
        assert_eq!(p[125], 0x00); // DMX start code
        assert_eq!(p[126], 255); // first slot
        assert_eq!(p[637], 7); // last slot
    }

    #[test]
    fn artnet_packet_layout() {
        let mut dmx = [0u8; DMX_UNIVERSE_SIZE];
        dmx[0] = 128;
        let p = encode_artnet(7, 1, &dmx);

        assert_eq!(p.len(), 530);
        assert_eq!(&p[0..8], b"Art-Net\0");
        assert_eq!(&p[8..10], &[0x00, 0x50]); // OpCode 0x5000
        assert_eq!(&p[10..12], &[0x00, 0x0e]); // protocol version 14
        assert_eq!(p[12], 7); // sequence
        assert_eq!(p[14], 1); // SubUni
        assert_eq!(p[15], 0); // Net
        assert_eq!(&p[16..18], &[0x02, 0x00]); // length = 512
        assert_eq!(p[18], 128); // first slot
    }

    #[test]
    fn artnet_universe_split() {
        let dmx = [0u8; DMX_UNIVERSE_SIZE];
        let p = encode_artnet(0, 0x0123, &dmx);
        assert_eq!(p[14], 0x23); // SubUni = low byte
        assert_eq!(p[15], 0x01); // Net = high byte
    }

    #[test]
    fn sacn_multicast_group() {
        assert_eq!(sacn_multicast_ip(1), IpAddr::V4(Ipv4Addr::new(239, 255, 0, 1)));
        assert_eq!(sacn_multicast_ip(0x0102), IpAddr::V4(Ipv4Addr::new(239, 255, 1, 2)));
    }
}
