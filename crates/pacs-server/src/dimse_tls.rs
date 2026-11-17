//! TLS for DIMSE, from the settings: what the listener accepts callers with, and what
//! pacs-rs calls remote AEs with.
//!
//! The rules are those of the BCP 195 RFC 8996, 9325 TLS Secure Transport Connection
//! Profile (PS3.15 B.12): TLS 1.2 and 1.3 and nothing older, 1.3 preferred, no cipher suite
//! without encryption or integrity (rustls has no other kind), strict TLS on the port
//! rather than an upgrade from plain, and mutual authentication supported and, unless
//! turned off, required.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use pacs_dimse::ClientTls;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use rustls::{ClientConfig, RootCertStore, ServerConfig};

use crate::{ClientAuth, Settings};

/// The TLS of the DIMSE side.
pub(crate) struct DimseTls {
    /// For the listener; none when pacs-rs has no certificate, and the port is plain.
    pub listener: Option<Arc<ServerConfig>>,
    /// For the remote AEs reached over TLS; none when there is none.
    pub caller: Option<ClientTls>,
}

/// The certificates of a PEM file, which must hold one at least; `setting` names the file
/// in an error.
pub(crate) fn certificates(
    file: &Path,
    setting: &str,
) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let certificates = CertificateDer::pem_file_iter(file)
        .and_then(|found| found.collect::<Result<Vec<_>, _>>())
        .with_context(|| format!("reading {setting} {}", file.display()))?;
    if certificates.is_empty() {
        bail!("{setting} {} holds no certificate", file.display());
    }
    Ok(certificates)
}

/// The certificate authorities of a PEM file.
pub(crate) fn authorities(file: &Path, setting: &str) -> anyhow::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    for certificate in certificates(file, setting)? {
        roots
            .add(certificate)
            .with_context(|| format!("an authority of {setting} {}", file.display()))?;
    }
    Ok(roots)
}

/// A certificate with its chain, and its private key, from two PEM files.
pub(crate) fn identity(
    certificate: &Path,
    key: &Path,
    settings: (&str, &str),
) -> anyhow::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let chain = certificates(certificate, settings.0)?;
    let key = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("reading {} {}", settings.1, key.display()))?;
    Ok((chain, key))
}

/// The TLS the settings ask for. `calls_over_tls`: a remote AE is configured with `/tls`.
/// What is asked without what it needs is an error, so that a server never starts with
/// less protection than its settings say.
pub(crate) fn open(settings: &Settings, calls_over_tls: bool) -> anyhow::Result<DimseTls> {
    let own = match (&settings.dimse_tls_cert_file, &settings.dimse_tls_key_file) {
        (Some(certificate), Some(key)) => Some(identity(
            certificate,
            key,
            ("PACS_DIMSE_TLS_CERT_FILE", "PACS_DIMSE_TLS_KEY_FILE"),
        )?),
        (None, None) => None,
        _ => bail!("PACS_DIMSE_TLS_CERT_FILE and PACS_DIMSE_TLS_KEY_FILE go together"),
    };
    let asks_callers = own.is_some() && settings.dimse_tls_client_auth == ClientAuth::Required;
    let roots = match &settings.dimse_tls_ca_file {
        Some(file) => Some(authorities(file, "PACS_DIMSE_TLS_CA_FILE")?),
        None => None,
    };
    if roots.is_none() && asks_callers {
        bail!(
            "DIMSE over TLS asks its callers for a certificate: name the authorities that sign \
             them in PACS_DIMSE_TLS_CA_FILE, or set PACS_DIMSE_TLS_CLIENT_AUTH=off"
        );
    }
    if roots.is_none() && calls_over_tls {
        bail!(
            "a remote AE of PACS_DIMSE_REMOTES is reached over TLS: name the authorities that \
             sign its certificate in PACS_DIMSE_TLS_CA_FILE"
        );
    }
    if roots.is_some() && !asks_callers && !calls_over_tls {
        bail!(
            "PACS_DIMSE_TLS_CA_FILE is set and nothing reads it: there is no certificate for \
             the listener to ask callers by, and no remote AE with /tls"
        );
    }
    // named, not the process default: the build holds two cryptography providers
    let provider = Arc::new(rustls::crypto::ring::default_provider());

    let listener = match &own {
        Some((chain, key)) => {
            let builder = ServerConfig::builder_with_provider(provider.clone())
                .with_safe_default_protocol_versions()
                .context("the TLS versions of the DIMSE listener")?;
            let builder = match (&roots, asks_callers) {
                (Some(roots), true) => {
                    let verifier = WebPkiClientVerifier::builder_with_provider(
                        Arc::new(roots.clone()),
                        provider.clone(),
                    )
                    .build()
                    .context("the authorities of PACS_DIMSE_TLS_CA_FILE")?;
                    builder.with_client_cert_verifier(verifier)
                }
                _ => builder.with_no_client_auth(),
            };
            let config = builder
                .with_single_cert(chain.clone(), key.clone_key())
                .context(
                    "PACS_DIMSE_TLS_CERT_FILE and PACS_DIMSE_TLS_KEY_FILE do not go together",
                )?;
            Some(Arc::new(config))
        }
        None => None,
    };

    let caller = match (roots, calls_over_tls) {
        (Some(roots), true) => {
            let builder = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .context("the TLS versions of DIMSE calls")?
                .with_root_certificates(roots);
            let config = match own {
                // shown to a remote AE that asks who is calling
                Some((chain, key)) => builder.with_client_auth_cert(chain, key).context(
                    "PACS_DIMSE_TLS_CERT_FILE and PACS_DIMSE_TLS_KEY_FILE do not go together",
                )?,
                None => builder.with_no_client_auth(),
            };
            Some(ClientTls(Arc::new(config)))
        }
        _ => None,
    };
    Ok(DimseTls { listener, caller })
}
