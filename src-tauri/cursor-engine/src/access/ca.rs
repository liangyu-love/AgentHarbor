//! The local certificate authority Cursor trusts so the front can read its
//! traffic to Cursor's servers. The authority is limited by a name constraint
//! to `cursor.sh`, `cursor.com` and their subdomains: even if its key leaked, it could not
//! vouch for any other site.
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, SanType, PKCS_ECDSA_P256_SHA256,
};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig,
};
use time::{Duration, OffsetDateTime};

use crate::{Error, Result};

const CERT_FILE: &str = "cursor-ca.crt";
const KEY_FILE: &str = "cursor-ca.key";
pub const COMMON_NAME: &str = "AgentHarbor Cursor Local CA";
const CURSOR_DOMAINS: [&str; 2] = ["cursor.sh", "cursor.com"];

pub struct Authority {
    issuer: Issuer<'static, KeyPair>,
    der: CertificateDer<'static>,
    pem: String,
    cert_path: PathBuf,
    configs: Mutex<HashMap<String, Arc<ServerConfig>>>,
}

impl Authority {
    /// Whether an authority has been made in `dir`.
    pub fn exists(dir: &Path) -> bool {
        dir.join(CERT_FILE).is_file() && dir.join(KEY_FILE).is_file()
    }

    /// The authority kept in `dir`, created there when missing or unusable.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join(CERT_FILE);
        let key_path = dir.join(KEY_FILE);
        if let (Ok(pem), Ok(key)) = (std::fs::read_to_string(&cert_path), std::fs::read_to_string(&key_path)) {
            match Self::parse(&pem, &key, cert_path.clone()) {
                Ok(authority) if !authority.expires_soon() && authority.covers_cursor_hosts() => return Ok(authority),
                Ok(_) => log::info!("Cursor CA needs renewal or new domain constraints; creating a new one"),
                Err(error) => log::warn!("Cursor CA is unusable ({error}); creating a new one"),
            }
        }
        std::fs::create_dir_all(dir)?;
        let (pem, key) = generate(&CURSOR_DOMAINS)?;
        write_private(&key_path, key.as_bytes())?;
        std::fs::write(&cert_path, pem.as_bytes())?;
        Self::parse(&pem, &key, cert_path)
    }

    fn parse(pem: &str, key: &str, cert_path: PathBuf) -> Result<Self> {
        let key = KeyPair::from_pem(key).map_err(|error| Error::Config(format!("Cursor CA key: {error}")))?;
        let der = CertificateDer::from_pem_slice(pem.as_bytes())
            .map_err(|error| Error::Config(format!("Cursor CA certificate: {error}")))?;
        let issuer = Issuer::from_ca_cert_der(&der, key)
            .map_err(|error| Error::Config(format!("Cursor CA certificate: {error}")))?;
        Ok(Self { issuer, der, pem: pem.to_owned(), cert_path, configs: Mutex::new(HashMap::new()) })
    }

    fn expires_soon(&self) -> bool {
        let Ok((_, parsed)) = x509_parser::parse_x509_certificate(&self.der) else { return true };
        parsed.validity().not_after.timestamp() - OffsetDateTime::now_utc().unix_timestamp() < 30 * 86_400
    }

    fn covers_cursor_hosts(&self) -> bool {
        let Ok((_, parsed)) = x509_parser::parse_x509_certificate(&self.der) else { return false };
        let Ok(Some(constraints)) = parsed.name_constraints() else { return false };
        let Some(permitted) = &constraints.value.permitted_subtrees else { return false };
        permitted.len() == CURSOR_DOMAINS.len() && CURSOR_DOMAINS.iter().all(|domain| {
            permitted.iter().any(|tree| matches!(tree.base, x509_parser::extensions::GeneralName::DNSName(name) if name == *domain))
        })
    }

    pub fn certificate_der(&self) -> &[u8] {
        &self.der
    }

    pub fn certificate_pem(&self) -> &str {
        &self.pem
    }

    pub fn certificate_path(&self) -> &Path {
        &self.cert_path
    }

    /// A TLS configuration presenting a certificate for `host`, made once per host.
    pub(crate) fn server_config(&self, host: &str) -> Result<Arc<ServerConfig>> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if !crate::api::cursor::proxy::is_cursor_host(&host) {
            return Err(Error::Config("certificate host must be a Cursor domain".into()));
        }
        if let Some(config) = self.configs.lock().get(&host) {
            return Ok(config.clone());
        }
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(certificate_error)?;
        let mut params = CertificateParams::new(vec![host.clone()]).map_err(certificate_error)?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host.as_str());
        params.distinguished_name = name;
        params.subject_alt_names = vec![SanType::DnsName(host.clone().try_into().map_err(certificate_error)?)];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let now = OffsetDateTime::now_utc();
        params.not_before = now - Duration::days(1);
        params.not_after = now + Duration::days(90);
        let leaf = params.signed_by(&key, &self.issuer).map_err(certificate_error)?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|error| Error::Config(format!("TLS setup: {error}")))?
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )
            .map_err(|error| Error::Config(format!("TLS setup: {error}")))?;
        // Cursor is told to stay on HTTP/1.1; offering only that keeps it there.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        self.configs.lock().insert(host, config.clone());
        Ok(config)
    }
}

fn certificate_error(error: rcgen::Error) -> Error {
    Error::Config(format!("certificate: {error}"))
}

/// A new authority: its certificate and private key, both PEM.
fn generate(domains: &[&str]) -> Result<(String, String)> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(certificate_error)?;
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(certificate_error)?;
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, COMMON_NAME);
    name.push(DnType::OrganizationName, "AgentHarbor");
    params.distinguished_name = name;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    params.name_constraints = Some(NameConstraints {
        permitted_subtrees: domains.iter().map(|domain| GeneralSubtree::DnsName((*domain).into())).collect(),
        // A DNS constraint says nothing about IP addresses; rule them all out.
        excluded_subtrees: vec![
            GeneralSubtree::IpAddress(CidrSubnet::V4(Ipv4Addr::UNSPECIFIED.octets(), [0; 4])),
            GeneralSubtree::IpAddress(CidrSubnet::V6(Ipv6Addr::UNSPECIFIED.octets(), [0; 16])),
        ],
    });
    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(3650);
    let certificate = params.self_signed(&key).map_err(certificate_error)?;
    Ok((certificate.pem(), key.serialize_pem()))
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// Whether this machine trusts a certificate as a root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustState {
    Trusted,
    Untrusted,
    /// The platform's store cannot be read or written from here.
    Manual,
}

#[cfg(windows)]
pub use windows::{distrust, trust, trust_state};

#[cfg(not(windows))]
pub fn trust_state(_der: &[u8]) -> TrustState {
    TrustState::Manual
}

#[cfg(not(windows))]
pub fn trust(_der: &[u8]) -> Result<()> {
    Err(Error::Config("install the certificate with the command shown".into()))
}

#[cfg(not(windows))]
pub fn distrust(_der: &[u8]) -> Result<()> {
    Err(Error::Config("remove the certificate with the system's certificate tool".into()))
}

/// The command that trusts the certificate where the app cannot do it itself.
pub fn trust_command(path: &Path) -> Option<String> {
    let path = path.display();
    if cfg!(target_os = "macos") {
        Some(format!("security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db '{path}'"))
    } else if cfg!(target_os = "linux") {
        Some(format!(
            "sudo cp '{path}' /usr/local/share/ca-certificates/agentharbor-cursor.crt && sudo update-ca-certificates"
        ))
    } else {
        None
    }
}

#[cfg(windows)]
mod windows {
    //! The current user's Root store: adding or removing a root makes Windows
    //! ask the user, and needs no administrator rights.
    use std::{ffi::c_void, io, ptr};

    use windows_sys::Win32::Security::Cryptography::{
        CertAddEncodedCertificateToStore, CertCloseStore, CertCreateCertificateContext,
        CertDeleteCertificateFromStore, CertFindCertificateInStore, CertFreeCertificateContext,
        CertOpenStore, CERT_FIND_EXISTING, CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W,
        CERT_SYSTEM_STORE_CURRENT_USER, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };

    use super::TrustState;
    use crate::{Error, Result};

    const ROOT: [u16; 5] = [b'R' as u16, b'O' as u16, b'O' as u16, b'T' as u16, 0];
    const ENCODING: u32 = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;

    struct Store(*mut c_void);

    impl Store {
        fn open() -> Result<Self> {
            let store = unsafe { CertOpenStore(CERT_STORE_PROV_SYSTEM_W, 0, 0, CERT_SYSTEM_STORE_CURRENT_USER, ROOT.as_ptr().cast()) };
            if store.is_null() {
                return Err(Error::Config(format!("open the Root certificate store: {}", io::Error::last_os_error())));
            }
            Ok(Self(store))
        }

        /// The store's copy of the certificate `der`, if it holds one.
        fn find(&self, der: &[u8]) -> Option<*const windows_sys::Win32::Security::Cryptography::CERT_CONTEXT> {
            let probe = unsafe { CertCreateCertificateContext(ENCODING, der.as_ptr(), der.len() as u32) };
            if probe.is_null() {
                return None;
            }
            let found = unsafe { CertFindCertificateInStore(self.0, ENCODING, 0, CERT_FIND_EXISTING, probe.cast(), ptr::null()) };
            unsafe { CertFreeCertificateContext(probe) };
            (!found.is_null()).then_some(found)
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            unsafe { CertCloseStore(self.0, 0) };
        }
    }

    pub fn trust_state(der: &[u8]) -> TrustState {
        let Ok(store) = Store::open() else { return TrustState::Manual };
        match store.find(der) {
            Some(found) => {
                unsafe { CertFreeCertificateContext(found) };
                TrustState::Trusted
            }
            None => TrustState::Untrusted,
        }
    }

    /// Adds the certificate as a root; Windows shows its own confirmation.
    pub fn trust(der: &[u8]) -> Result<()> {
        let store = Store::open()?;
        let added = unsafe {
            CertAddEncodedCertificateToStore(store.0, ENCODING, der.as_ptr(), der.len() as u32, CERT_STORE_ADD_REPLACE_EXISTING, ptr::null_mut())
        };
        if added == 0 {
            return Err(Error::Config(format!("the certificate was not installed: {}", io::Error::last_os_error())));
        }
        Ok(())
    }

    /// Removes the certificate from the roots; Windows shows its own confirmation.
    pub fn distrust(der: &[u8]) -> Result<()> {
        let store = Store::open()?;
        let Some(found) = store.find(der) else { return Ok(()) };
        // Deleting frees the context it was given.
        if unsafe { CertDeleteCertificateFromStore(found) } == 0 {
            return Err(Error::Config(format!("the certificate was not removed: {}", io::Error::last_os_error())));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authority_is_kept_and_limited_to_cursor_hosts() {
        let directory = tempfile::tempdir().unwrap();
        let first = Authority::load_or_create(directory.path()).unwrap();
        let again = Authority::load_or_create(directory.path()).unwrap();
        assert_eq!(first.certificate_der(), again.certificate_der());

        let (_, parsed) = x509_parser::parse_x509_certificate(first.certificate_der()).unwrap();
        assert!(parsed.is_ca());
        let constraints = parsed.name_constraints().unwrap().unwrap().value;
        let permitted = constraints.permitted_subtrees.as_ref().unwrap();
        assert_eq!(permitted.len(), 2);
        assert!(matches!(permitted[0].base, x509_parser::extensions::GeneralName::DNSName("cursor.sh")));
        assert!(matches!(permitted[1].base, x509_parser::extensions::GeneralName::DNSName("cursor.com")));
        assert_eq!(constraints.excluded_subtrees.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn the_old_cursor_sh_only_authority_is_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let (pem, key) = generate(&["cursor.sh"]).unwrap();
        std::fs::write(directory.path().join(CERT_FILE), &pem).unwrap();
        std::fs::write(directory.path().join(KEY_FILE), &key).unwrap();
        let upgraded = Authority::load_or_create(directory.path()).unwrap();
        assert_ne!(upgraded.certificate_pem(), pem);
        assert!(upgraded.covers_cursor_hosts());
        assert!(upgraded.server_config("cursor.com").is_ok());
        assert!(upgraded.server_config("cursor.com.evil.test").is_err());
    }

    #[test]
    fn each_host_gets_one_tls_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let authority = Authority::load_or_create(directory.path()).unwrap();
        let first = authority.server_config("api2.cursor.sh").unwrap();
        let again = authority.server_config("API2.cursor.sh.").unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(first.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }
}
