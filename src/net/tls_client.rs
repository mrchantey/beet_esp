//! Pinned-certificate TLS 1.3 client for the Wi-Fi transports (`secure`).
//!
//! [`connect`] wraps an `embedded_io_async` transport (the embassy `TcpSocket`)
//! in an [`embedded_tls`] session whose trust model is *pinning*: the peer must
//! present byte-for-byte the beet dev certificate embedded at build time
//! (`build.rs::embed_dev_cert`), and its `CertificateVerify` signature is
//! checked against that certificate's P-256 key, proving the peer holds the
//! private key. No chains, no CA set, no wall clock — strictly the one pinned
//! cert. Regenerating the dev cert (eg after a LAN IP change) re-pins on the
//! next firmware build via the build script's `rerun-if-changed`.

use beet::prelude::*;
use embedded_tls::Aes128GcmSha256;
use embedded_tls::CertificateEntryRef;
use embedded_tls::CertificateRef;
use embedded_tls::CertificateVerifyRef;
use embedded_tls::CryptoProvider;
use embedded_tls::CryptoRngCore;
use embedded_tls::SignatureScheme;
use embedded_tls::Sha256;
use embedded_tls::TlsConfig;
use embedded_tls::TlsConnection;
use embedded_tls::TlsContext;
use embedded_tls::TlsError;
use embedded_tls::TlsVerifier;
use sha2::Digest as _;

/// The pinned dev certificate (DER), embedded by `build.rs`.
static PINNED_CERT_DER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/dev_cert.der"));
/// Its uncompressed P-256 public key (SEC1), pre-extracted by `build.rs`.
static PINNED_CERT_PUBKEY: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/dev_cert_pubkey.sec1"));

/// TLS record read buffer size: must fit the largest encrypted record a peer
/// may send (embedded-tls documents 16640 as the safe value).
const READ_BUF_LEN: usize = 16640;
/// Record write buffer size: outbound records (handshake, ws frames, http
/// requests) are small, and embedded-tls splits larger writes across records.
const WRITE_BUF_LEN: usize = 4096;

/// A freshly allocated TLS record read buffer for [`connect`].
pub(crate) fn read_buf() -> Vec<u8> { alloc::vec![0; READ_BUF_LEN] }
/// A freshly allocated TLS record write buffer for [`connect`].
pub(crate) fn write_buf() -> Vec<u8> { alloc::vec![0; WRITE_BUF_LEN] }

/// Open a TLS 1.3 session over `transport` with the pinned-cert trust model.
/// The caller owns the record buffers ([`read_buf`]/[`write_buf`]); they must
/// outlive the returned session.
pub(crate) async fn connect<'buf, S>(
    transport: S,
    read_buf: &'buf mut [u8],
    write_buf: &'buf mut [u8],
) -> Result<TlsConnection<'buf, S, Aes128GcmSha256>>
where
    S: embedded_io_async::Read + embedded_io_async::Write + 'buf,
{
    let mut tls = TlsConnection::new(transport, read_buf, write_buf);
    // No server name: the agent is dialled by LAN IP and trust is the pinned
    // cert itself, so SNI/hostname checks add nothing.
    let config = TlsConfig::new();
    let mut provider = PinnedProvider::default();
    tls.open(TlsContext::new(&config, &mut provider))
        .await
        .map_err(|err| bevyhow!("TLS handshake failed: {:?}", err))?;
    Ok(tls)
}

/// The pinned-trust crypto provider: the hardware RNG plus [`PinnedVerifier`].
#[derive(Default)]
struct PinnedProvider {
    rng: EspRng,
    verifier: PinnedVerifier,
}

impl CryptoProvider for PinnedProvider {
    type CipherSuite = Aes128GcmSha256;
    type Signature = p256::ecdsa::DerSignature;

    fn rng(&mut self) -> impl CryptoRngCore { &mut self.rng }

    fn verifier(
        &mut self,
    ) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, TlsError> {
        Ok(&mut self.verifier)
    }
}

/// The esp-hal hardware RNG as a rand_core 0.6 `CryptoRngCore`.
///
/// `esp_hal::rng::Rng` reads the chip's hardware RNG; with the radio running
/// (always here: `secure` implies `wifi`) it is a true RNG per the ESP32-S3
/// reference manual, justifying the `CryptoRng` marker.
#[derive(Default, Clone, Copy)]
struct EspRng;

impl rand_core::RngCore for EspRng {
    fn next_u32(&mut self) -> u32 { esp_hal::rng::Rng::new().random() }

    fn next_u64(&mut self) -> u64 {
        let rng = esp_hal::rng::Rng::new();
        ((rng.random() as u64) << 32) | rng.random() as u64
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let rng = esp_hal::rng::Rng::new();
        for chunk in dest.chunks_mut(4) {
            let word = rng.random().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }

    fn try_fill_bytes(
        &mut self,
        dest: &mut [u8],
    ) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
impl rand_core::CryptoRng for EspRng {}

/// Trusts exactly the pinned certificate: the presented leaf must match
/// byte-for-byte, and the `CertificateVerify` signature must verify against its
/// P-256 key over the handshake transcript (RFC 8446 §4.4.3) — without that
/// second check any peer could replay the public cert and MITM the session.
#[derive(Default)]
struct PinnedVerifier {
    /// The transcript hash at the Certificate message, consumed by
    /// [`TlsVerifier::verify_signature`].
    transcript: Option<Sha256>,
}

impl TlsVerifier<Aes128GcmSha256> for PinnedVerifier {
    fn set_hostname_verification(
        &mut self,
        _hostname: &str,
    ) -> Result<(), TlsError> {
        // trust is the exact pinned cert; hostnames add nothing
        Ok(())
    }

    fn verify_certificate(
        &mut self,
        transcript: &Sha256,
        cert: CertificateRef,
    ) -> Result<(), TlsError> {
        let Some(CertificateEntryRef::X509(leaf)) = cert.entries.first() else {
            return Err(TlsError::InvalidCertificate);
        };
        if *leaf != PINNED_CERT_DER {
            error!(
                "peer certificate does not match the pinned beet dev cert \
                 (regenerated since this firmware was built? rebuild to re-pin)"
            );
            return Err(TlsError::InvalidCertificate);
        }
        self.transcript = Some(transcript.clone());
        Ok(())
    }

    fn verify_signature(
        &mut self,
        verify: CertificateVerifyRef,
    ) -> Result<(), TlsError> {
        use p256::ecdsa::signature::Verifier as _;
        let transcript =
            self.transcript.take().ok_or(TlsError::InvalidCertificate)?;
        // the dev cert is ECDSA P-256, so that is the only scheme pinned
        if verify.signature_scheme != SignatureScheme::EcdsaSecp256r1Sha256 {
            return Err(TlsError::InvalidSignatureScheme);
        }
        // RFC 8446 §4.4.3: 64 spaces, the context string, a NUL, then the
        // transcript hash up to the Certificate message.
        let mut message = Vec::with_capacity(130);
        message.resize(64, 0x20);
        message.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
        message.extend_from_slice(&transcript.finalize());
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(PINNED_CERT_PUBKEY)
            .map_err(|_| TlsError::DecodeError)?;
        let signature = p256::ecdsa::Signature::from_der(verify.signature)
            .map_err(|_| TlsError::InvalidSignature)?;
        key.verify(&message, &signature)
            .map_err(|_| TlsError::InvalidSignature)
    }
}
