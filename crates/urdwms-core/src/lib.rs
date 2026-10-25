//! Urd-WMS rules that do not depend on storage or screens.
//!
//! Everything here is pure or talks only to a camera / recognition service, so
//! it is tested on its own and the desktop app is a thin layer over it.

pub mod camera;
pub mod matching;
pub mod normalize;
pub mod recognition;
pub mod units;

/// HTTP client for cameras and the recognition service.
///
/// Certificates are checked against the operating system's store, not a
/// bundled list: on a factory PC an antivirus or proxy that inspects HTTPS
/// re-signs traffic with its own root, which Windows trusts and a bundled
/// list does not — the failure is an instant "invalid certificate" that looks
/// like a bad API key.
pub fn http_agent(timeout: std::time::Duration) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .tls_config(tls)
        .build()
        .into()
}
