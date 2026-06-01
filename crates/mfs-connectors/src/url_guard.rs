use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[derive(Debug)]
pub enum UrlGuardError {
    InvalidUrl(String),
    PrivateTarget { url: String, ip: String },
    DnsResolutionFailed { url: String, reason: String },
    WhitelistOnly { url: String },
}

impl std::fmt::Display for UrlGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "Invalid URL: {}", url),
            Self::PrivateTarget { url, ip } => {
                write!(f, "SSRF risk: {} resolves to non-public IP {}", url, ip)
            }
            Self::DnsResolutionFailed { url, reason } => {
                write!(f, "DNS resolution failed for {}: {}", url, reason)
            }
            Self::WhitelistOnly { url } => {
                write!(f, "URL {} not in allowed domains whitelist", url)
            }
        }
    }
}

impl std::error::Error for UrlGuardError {}

/// Validate that a URL target does not resolve to a private, link-local, or otherwise
/// non-public IP address.
///
/// This prevents Server-Side Request Forgery (SSRF) attacks where a malicious user could
/// trick the server into accessing internal resources (e.g., AWS metadata at 169.254.169.254,
/// or internal services at 10.0.0.1).
///
/// Loopback addresses (127.0.0.1, ::1) are intentionally NOT blocked because MemFuse is a
/// local-first engine — localhost URLs are legitimate targets for local file servers and
/// test fixtures.
///
/// When `allowed_domains` is non-empty, URLs whose host matches a whitelisted domain
/// bypass IP validation entirely (trusted external sources).
pub fn validate_url_target(url_str: &str, allowed_domains: &[&str]) -> Result<(), UrlGuardError> {
    let parsed =
        reqwest::Url::parse(url_str).map_err(|e| UrlGuardError::InvalidUrl(e.to_string()))?;

    let host = parsed.host_str().unwrap_or("");
    let scheme = parsed.scheme();

    // Only validate http/https URLs
    if scheme != "http" && scheme != "https" {
        return Ok(());
    }

    // Check whitelist first
    for domain in allowed_domains {
        if host == *domain || host.ends_with(&format!(".{}", domain)) {
            return Ok(());
        }
    }

    // If whitelist is exclusive (non-empty), reject non-whitelisted URLs
    if !allowed_domains.is_empty() {
        return Err(UrlGuardError::WhitelistOnly {
            url: url_str.to_owned(),
        });
    }

    // Resolve DNS and check IP
    let ips = dns_resolve(host)?;
    for ip in ips {
        if is_blocked_for_ssrf(&ip) {
            return Err(UrlGuardError::PrivateTarget {
                url: url_str.to_owned(),
                ip: ip.to_string(),
            });
        }
    }

    Ok(())
}

fn dns_resolve(host: &str) -> Result<Vec<IpAddr>, UrlGuardError> {
    // Try parsing as IP first (fast path, no DNS query needed)
    if let Ok(ip) = IpAddr::from_str(host) {
        return Ok(vec![ip]);
    }

    // DNS resolution using std::net::ToSocketAddrs (blocking but works in both sync/async)
    use std::net::ToSocketAddrs;
    let addr_str = format!("{}:80", host);
    let addrs: Vec<std::net::SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e: std::io::Error| UrlGuardError::DnsResolutionFailed {
            url: host.to_owned(),
            reason: e.to_string(),
        })?
        .collect();

    Ok(addrs
        .into_iter()
        .map(|a: std::net::SocketAddr| a.ip())
        .collect())
}

/// Check if an IP address should be blocked for SSRF protection.
///
/// Loopback (127.0.0.1, ::1) is NOT blocked (MemFuse is local-first).
///
/// Blocked ranges:
/// - IPv4: private (10/8, 172.16/12, 192.168/16), link-local (169.254/16),
///   carrier-grade NAT (100.64/10), unspecified (0/8), multicast (224/4),
///   reserved/broadcast (240/4 including 255.255.255.255)
/// - IPv6: unique-local (fc00::/7), link-local (fe80::/10), unspecified (::),
///   multicast (ff00::/8), IPv4-mapped (::ffff:0:0/96)
fn is_blocked_for_ssrf(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_ipv4_blocked(v4),
        IpAddr::V6(v6) => is_ipv6_blocked(v6),
    }
}

fn is_ipv4_blocked(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();

    // 0.0.0.0/8 — unspecified / current network
    if octets[0] == 0 {
        return true;
    }
    // 10.0.0.0/8 — private (class A)
    if octets[0] == 10 {
        return true;
    }
    // 100.64.0.0/10 — carrier-grade NAT / shared address space
    if octets[0] == 100 && (octets[1] & 0xc0) == 0x40 {
        return true;
    }
    // 127.0.0.0/8 — loopback (INTENTIONALLY NOT BLOCKED for local-first MemFuse)
    // 169.254.0.0/16 — link-local / APIPA (cloud metadata endpoints)
    if octets[0] == 169 && octets[1] == 254 {
        return true;
    }
    // 172.16.0.0/12 — private (172.16.x through 172.31.x)
    if octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31 {
        return true;
    }
    // 192.0.0.0/24 — IETF protocol assignments (low risk, but non-public)
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return true;
    }
    // 192.0.2.0/24 — TEST-NET-1 (documentation)
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return true;
    }
    // 192.168.0.0/16 — private (class C)
    if octets[0] == 192 && octets[1] == 168 {
        return true;
    }
    // 198.18.0.0/15 — benchmarking
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return true;
    }
    // 224.0.0.0/4 — multicast
    if octets[0] >= 224 && octets[0] <= 239 {
        return true;
    }
    // 240.0.0.0/4 — reserved for future use (includes 255.255.255.255 broadcast)
    if octets[0] >= 240 {
        return true;
    }

    false
}

fn is_ipv6_blocked(ip: &Ipv6Addr) -> bool {
    // :: (unspecified) — equivalent to 0.0.0.0
    if ip.is_unspecified() {
        return true;
    }

    let segments = ip.segments();

    // IPv4-mapped IPv6: ::ffff:0:0/96
    // This is a critical bypass vector — ::ffff:10.0.0.1 maps to 10.0.0.1
    // Format: first 80 bits are zero, next 16 bits are 0xffff, last 32 bits are IPv4
    if segments[0..5].iter().all(|s| *s == 0) && segments[5] == 0xffff {
        let v4 = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            (segments[6] & 0xff) as u8,
            (segments[7] >> 8) as u8,
            (segments[7] & 0xff) as u8,
        );
        return is_ipv4_blocked(&v4);
    }

    // fc00::/7 — unique local (equivalent to IPv4 private)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }

    // fe80::/10 — link-local (equivalent to IPv4 169.254.x.x)
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }

    // ff00::/8 — multicast
    if (segments[0] & 0xff00) == 0xff00 {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_ipv4_is_allowed_for_local_first_engine() {
        assert!(validate_url_target("http://127.0.0.1:6379/", &[]).is_ok());
    }

    #[test]
    fn loopback_ipv6_is_allowed_for_local_first_engine() {
        assert!(validate_url_target("http://[::1]:6379/", &[]).is_ok());
    }

    #[test]
    fn private_ip_is_rejected() {
        let err = validate_url_target("http://10.0.0.1/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn aws_metadata_is_rejected() {
        let err = validate_url_target("http://169.254.169.254/latest/meta-data/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn carrier_grade_nat_is_rejected() {
        let err = validate_url_target("http://100.64.0.1/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn zero_ip_is_rejected() {
        let err = validate_url_target("http://0.0.0.0/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv4_mapped_private_ip_is_rejected() {
        // ::ffff:10.0.0.1 maps to 10.0.0.1 — critical SSRF bypass vector
        let err = validate_url_target("http://[::ffff:10.0.0.1]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv4_mapped_aws_metadata_is_rejected() {
        // ::ffff:169.254.169.254 maps to AWS metadata endpoint
        let err = validate_url_target("http://[::ffff:169.254.169.254]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv6_unique_local_is_rejected() {
        let err = validate_url_target("http://[fc00::1]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv6_link_local_is_rejected() {
        let err = validate_url_target("http://[fe80::1]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv6_unspecified_is_rejected() {
        let err = validate_url_target("http://[::]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv6_multicast_is_rejected() {
        let err = validate_url_target("http://[ff00::1]/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv4_multicast_is_rejected() {
        let err = validate_url_target("http://224.0.0.1/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn ipv4_broadcast_is_rejected() {
        let err = validate_url_target("http://255.255.255.255/", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::PrivateTarget { .. }));
    }

    #[test]
    fn whitelisted_domain_bypasses_check() {
        assert!(validate_url_target("http://github.com/repo", &["github.com"]).is_ok());
    }

    #[test]
    fn non_whitelisted_domain_is_rejected_when_whitelist_exists() {
        let err = validate_url_target("http://example.com/", &["github.com"]).unwrap_err();
        assert!(matches!(err, UrlGuardError::WhitelistOnly { .. }));
    }

    #[test]
    fn invalid_url_is_rejected() {
        let err = validate_url_target("not-a-url", &[]).unwrap_err();
        assert!(matches!(err, UrlGuardError::InvalidUrl { .. }));
    }

    #[test]
    fn non_http_scheme_is_allowed() {
        assert!(validate_url_target("ftp://files.example.com/", &[]).is_ok());
    }
}
