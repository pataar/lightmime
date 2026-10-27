//! Hue Entertainment streaming: HueStream v2 messages over DTLS 1.2 with a
//! pre-shared key, sent to UDP port 2100 on the bridge.
//!
//! This is blocking code; run it on a dedicated thread.

use std::io::{self, Read, Write};
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use openssl::ssl::{HandshakeError, SslContext, SslMethod, SslOptions, SslStream, SslVerifyMode};

const PORT: u16 = 2100;
const HANDSHAKE_ATTEMPTS: u32 = 3;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// Conservative MTU so a full 20-channel message always fits one datagram.
const MTU: u32 = 1200;

/// Colour for one channel, 16 bits per component.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChannelColor {
    pub channel_id: u8,
    pub rgb: [u16; 3],
}

/// UDP socket where every `write` is one datagram and every `read` one
/// received datagram, which is what DTLS needs underneath it.
#[derive(Debug)]
struct Datagram(UdpSocket);

impl Read for Datagram {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.recv(buf)
    }
}

impl Write for Datagram {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.send(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct EntertainmentStream {
    tls: SslStream<Datagram>,
    config_id: [u8; 36],
    sequence: u8,
    buffer: Vec<u8>,
}

impl EntertainmentStream {
    /// Open the DTLS session. The entertainment area must already be started
    /// through the REST API ([`super::Bridge::set_streaming`]).
    pub fn connect(
        host: &str,
        psk_identity: &str,
        client_key: &str,
        config_id: &str,
    ) -> Result<Self> {
        let config_id: [u8; 36] = config_id
            .as_bytes()
            .try_into()
            .map_err(|_| anyhow!("entertainment area id must be a 36-character UUID"))?;
        let psk = decode_hex(client_key).context("client key must be hex")?;

        let mut last_error = anyhow!("no handshake attempted");
        for attempt in 1..=HANDSHAKE_ATTEMPTS {
            match handshake(host, psk_identity, &psk) {
                Ok(tls) => {
                    return Ok(Self {
                        tls,
                        config_id,
                        sequence: 0,
                        buffer: Vec::with_capacity(16 + 36 + 7 * 20),
                    });
                }
                Err(e) => {
                    tracing::warn!("DTLS handshake attempt {attempt} failed: {e:#}");
                    last_error = e;
                }
            }
        }
        Err(last_error.context("DTLS handshake with the bridge failed"))
    }

    /// Send one frame. Channels missing from `colors` keep their last colour.
    pub fn send(&mut self, colors: &[ChannelColor]) -> Result<()> {
        if colors.len() > 20 {
            bail!("an entertainment area has at most 20 channels");
        }
        self.buffer.clear();
        self.buffer.extend_from_slice(b"HueStream");
        self.buffer.extend_from_slice(&[
            0x02,
            0x00, // protocol version 2.0
            self.sequence,
            0x00,
            0x00, // reserved
            0x00, // colour space: RGB
            0x00, // reserved
        ]);
        self.buffer.extend_from_slice(&self.config_id);
        for color in colors {
            self.buffer.push(color.channel_id);
            for component in color.rgb {
                self.buffer.extend_from_slice(&component.to_be_bytes());
            }
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.tls
            .ssl_write(&self.buffer)
            .map_err(|e| anyhow!("sending to the bridge: {e}"))?;
        Ok(())
    }

    /// Close the DTLS session politely; errors are irrelevant at this point.
    pub fn close(mut self) {
        let _ = self.tls.shutdown();
    }
}

fn handshake(host: &str, identity: &str, psk: &[u8]) -> Result<SslStream<Datagram>> {
    let addr = (host, PORT)
        .to_socket_addrs()
        .with_context(|| format!("resolving {host}"))?
        .next()
        .ok_or_else(|| anyhow!("{host} did not resolve"))?;
    let socket = UdpSocket::bind(if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })?;
    socket.connect(addr)?;
    socket.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;

    let mut ctx = SslContext::builder(SslMethod::dtls_client())?;
    ctx.set_min_proto_version(Some(openssl::ssl::SslVersion::DTLS1_2))?;
    ctx.set_cipher_list("PSK-AES128-GCM-SHA256")?;
    ctx.set_verify(SslVerifyMode::NONE);
    ctx.set_options(SslOptions::NO_QUERY_MTU);
    let identity = identity.as_bytes().to_vec();
    let psk = psk.to_vec();
    ctx.set_psk_client_callback(move |_ssl, _hint, identity_out, psk_out| {
        // The identity is a NUL-terminated C string.
        if identity.len() + 1 > identity_out.len() || psk.len() > psk_out.len() {
            return Ok(0);
        }
        identity_out[..identity.len()].copy_from_slice(&identity);
        identity_out[identity.len()] = 0;
        psk_out[..psk.len()].copy_from_slice(&psk);
        Ok(psk.len())
    });
    let ctx = ctx.build();

    let mut ssl = openssl::ssl::Ssl::new(&ctx)?;
    ssl.set_mtu(MTU)?;
    let tls = match ssl.connect(Datagram(socket)) {
        Ok(tls) => tls,
        Err(HandshakeError::WouldBlock(_)) => {
            bail!("bridge did not answer within {HANDSHAKE_TIMEOUT:?}")
        }
        Err(HandshakeError::Failure(mid)) => bail!("{}", mid.error()),
        Err(HandshakeError::SetupFailure(e)) => return Err(e.into()),
    };
    // Streaming never reads, so the timeout only mattered for the handshake.
    Ok(tls)
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("odd number of hex digits");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_hex() {
        assert_eq!(decode_hex("00ff10").unwrap(), vec![0x00, 0xff, 0x10]);
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }
}
