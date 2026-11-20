//! TLS support for the native protocol listener.
//!
//! Certificate and private-key files are read as PEM. Callers that enable TLS
//! load an acceptor with [`load_acceptor`] and pass it to
//! [`crate::Server::run_tls_listener`].

use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;
use std::sync::Arc;

use tokio_rustls::TlsAcceptor;

pub use rustls;
pub use tokio_rustls;

/// Load a Rustls server configuration from PEM certificate and key files.
///
/// # Errors
///
/// Returns an I/O error when the files cannot be read or parsed, or when the
/// certificate and key do not form a valid server identity.
pub fn load_server_config(
    certificate_path: &Path,
    private_key_path: &Path,
) -> io::Result<Arc<rustls::ServerConfig>> {
    let certificates = load_certificates(certificate_path)?;
    let private_key = load_private_key(private_key_path)?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, private_key)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    config.alpn_protocols = vec![b"storkv".to_vec()];
    Ok(Arc::new(config))
}

/// Load a token-safe TLS acceptor for the native protocol listener.
///
/// # Errors
///
/// Returns an I/O error when the certificate or key cannot be loaded.
pub fn load_acceptor(certificate_path: &Path, private_key_path: &Path) -> io::Result<TlsAcceptor> {
    Ok(TlsAcceptor::from(load_server_config(
        certificate_path,
        private_key_path,
    )?))
}

/// Read every PEM certificate from a file.
///
/// # Errors
///
/// Returns an I/O error when the file cannot be read or contains no
/// certificate.
pub fn load_certificates(
    certificate_path: &Path,
) -> io::Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let mut reader = BufReader::new(File::open(certificate_path)?);
    let certificates = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid certificate PEM: {error}"),
            )
        })?;
    if certificates.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no certificate found in PEM file",
        ));
    }
    Ok(certificates)
}

/// Read one PEM private key from a file.
///
/// # Errors
///
/// Returns an I/O error when the file cannot be read or contains no key.
pub fn load_private_key(
    private_key_path: &Path,
) -> io::Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(File::open(private_key_path)?);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid private key PEM: {error}"),
            )
        })?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "no private key found in PEM file",
            )
        })
}
