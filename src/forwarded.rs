use std::net::IpAddr;

use http::uri::Scheme;

/// Envoy refuses to parse more x-forwarded-for entries than this when walking
/// it for trusted CIDRs, so we match it.
const MAX_XFF_ENTRIES: usize = 20;

/// A CIDR range of addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cidr {
    network: IpAddr,
    prefix_len: u8,
}

impl Cidr {
    /// Parses a CIDR range like `10.0.0.0/8`, or a single address like
    /// `127.0.0.1`. Bits beyond the prefix are ignored.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (address, prefix_len) = match value.split_once('/') {
            Some((address, prefix_len)) => (address, Some(prefix_len)),
            None => (value, None),
        };
        let network: IpAddr = address.parse().ok()?;
        let max_prefix_len = if network.is_ipv4() { 32 } else { 128 };
        let prefix_len = match prefix_len {
            Some(prefix_len) => prefix_len.parse().ok().filter(|p| *p <= max_prefix_len)?,
            None => max_prefix_len,
        };
        Some(Self {
            network,
            prefix_len,
        })
    }

    /// Whether the address is in this range. Like Envoy, an IPv4 address is
    /// never in an IPv6 range or vice versa.
    fn contains(&self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => {
                let (network, address) = (u32::from(network), u32::from(address));
                self.prefix_len == 0 || (network ^ address) >> (32 - self.prefix_len) == 0
            }
            (IpAddr::V6(network), IpAddr::V6(address)) => {
                let (network, address) = (u128::from(network), u128::from(address));
                self.prefix_len == 0 || (network ^ address) >> (128 - self.prefix_len) == 0
            }
            _ => false,
        }
    }
}

/// The reverse proxies trusted to set the x-forwarded-for and x-forwarded-proto
/// headers. The HTTP connection manager is configured to match so Envoy itself
/// resolves the client address of HTTP requests, but the scheme and WebSocket
/// connections, which do not go through the connection manager, are resolved
/// here.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TrustedProxies {
    /// Any peer is trusted, including a Unix socket peer.
    All,
    /// Peers with an address in any of the ranges are trusted. Empty trusts no
    /// peer.
    Cidrs(Box<[Cidr]>),
}

impl TrustedProxies {
    /// Whether the peer, which is None if it is not an IP address, is trusted.
    /// The peer is only resolved if needed.
    pub(crate) fn is_trusted(&self, peer: impl FnOnce() -> Option<IpAddr>) -> bool {
        match self {
            TrustedProxies::All => true,
            TrustedProxies::Cidrs(cidrs) if cidrs.is_empty() => false,
            TrustedProxies::Cidrs(cidrs) => {
                peer().is_some_and(|peer| cidrs.iter().any(|cidr| cidr.contains(peer)))
            }
        }
    }

    /// Resolves the client address from the x-forwarded-for header sent by a
    /// trusted peer, matching Envoy's resolution for HTTP requests. If all
    /// trusted, this is the last address. Otherwise, it is the last address not
    /// in a trusted range, or the first address if all are. None if the header
    /// cannot be parsed.
    pub(crate) fn client_from_forwarded_for(&self, forwarded_for: &str) -> Option<IpAddr> {
        match self {
            TrustedProxies::All => forwarded_for.rsplit(',').next()?.trim().parse().ok(),
            TrustedProxies::Cidrs(cidrs) => {
                let entries = forwarded_for.split(',');
                if entries.clone().count() > MAX_XFF_ENTRIES {
                    return None;
                }
                let mut client = None;
                for entry in entries.rev() {
                    let address: IpAddr = entry.trim().parse().ok()?;
                    client = Some(address);
                    if !cidrs.iter().any(|cidr| cidr.contains(address)) {
                        break;
                    }
                }
                client
            }
        }
    }
}

/// Parses the scheme of an x-forwarded-proto header, returning whether it is
/// secure. None if it is not a single HTTP or WebSocket scheme.
pub(crate) fn forwarded_proto_is_secure(value: &[u8]) -> Option<bool> {
    let value = value.trim_ascii();
    if value.eq_ignore_ascii_case(b"https") || value.eq_ignore_ascii_case(b"wss") {
        Some(true)
    } else if value.eq_ignore_ascii_case(b"http") || value.eq_ignore_ascii_case(b"ws") {
        Some(false)
    } else {
        None
    }
}

pub(crate) fn http_scheme(secure: bool) -> Scheme {
    if secure { Scheme::HTTPS } else { Scheme::HTTP }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cidrs(values: &[&str]) -> TrustedProxies {
        TrustedProxies::Cidrs(values.iter().map(|v| Cidr::parse(v).unwrap()).collect())
    }

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn cidr_parse() {
        assert_eq!(
            Cidr::parse("10.1.2.3/8"),
            Some(Cidr {
                network: ip("10.1.2.3"),
                prefix_len: 8
            })
        );
        assert_eq!(
            Cidr::parse("::1"),
            Some(Cidr {
                network: ip("::1"),
                prefix_len: 128
            })
        );
        assert_eq!(Cidr::parse("10.0.0.0/33"), None);
        assert_eq!(Cidr::parse("::/129"), None);
        assert_eq!(Cidr::parse("10.0.0.0/"), None);
        assert_eq!(Cidr::parse("localhost"), None);
        assert_eq!(Cidr::parse("*"), None);
    }

    #[test]
    fn cidr_contains() {
        let range = Cidr::parse("10.1.0.0/16").unwrap();
        assert!(range.contains(ip("10.1.255.3")));
        assert!(!range.contains(ip("10.2.0.1")));
        assert!(!range.contains(ip("::ffff:10.1.0.1")));

        let single = Cidr::parse("127.0.0.1").unwrap();
        assert!(single.contains(ip("127.0.0.1")));
        assert!(!single.contains(ip("127.0.0.2")));

        let v6 = Cidr::parse("fd00::/8").unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
        assert!(!v6.contains(ip("10.0.0.1")));

        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains(ip("1.2.3.4")));
        assert!(Cidr::parse("::/0").unwrap().contains(ip("::1")));
    }

    #[test]
    fn is_trusted() {
        let trusted = cidrs(&["10.0.0.0/8"]);
        assert!(trusted.is_trusted(|| Some(ip("10.0.0.1"))));
        assert!(!trusted.is_trusted(|| Some(ip("11.0.0.1"))));
        assert!(!trusted.is_trusted(|| None));

        assert!(TrustedProxies::All.is_trusted(|| None));
        assert!(!cidrs(&[]).is_trusted(|| unreachable!()));
    }

    #[test]
    fn client_from_forwarded_for_all() {
        let all = TrustedProxies::All;
        assert_eq!(
            all.client_from_forwarded_for("1.1.1.1, 2.2.2.2"),
            Some(ip("2.2.2.2"))
        );
        assert_eq!(all.client_from_forwarded_for("::1"), Some(ip("::1")));
        assert_eq!(all.client_from_forwarded_for("1.1.1.1, bad"), None);
        assert_eq!(all.client_from_forwarded_for(""), None);
    }

    #[test]
    fn client_from_forwarded_for_cidrs() {
        let trusted = cidrs(&["10.0.0.0/8"]);
        assert_eq!(
            trusted.client_from_forwarded_for("1.1.1.1, 2.2.2.2, 10.0.0.2, 10.0.0.1"),
            Some(ip("2.2.2.2"))
        );
        assert_eq!(
            trusted.client_from_forwarded_for("10.0.0.3,10.0.0.2"),
            Some(ip("10.0.0.3"))
        );
        assert_eq!(trusted.client_from_forwarded_for("bad, 10.0.0.1"), None);
        assert_eq!(trusted.client_from_forwarded_for("1.1.1.1:80"), None);

        let max_entries = vec!["1.1.1.1"; MAX_XFF_ENTRIES].join(",");
        assert_eq!(
            trusted.client_from_forwarded_for(&max_entries),
            Some(ip("1.1.1.1"))
        );
        let too_many_entries = vec!["1.1.1.1"; MAX_XFF_ENTRIES + 1].join(",");
        assert_eq!(trusted.client_from_forwarded_for(&too_many_entries), None);
    }

    #[test]
    fn forwarded_proto() {
        assert_eq!(forwarded_proto_is_secure(b"https"), Some(true));
        assert_eq!(forwarded_proto_is_secure(b" HTTPS "), Some(true));
        assert_eq!(forwarded_proto_is_secure(b"wss"), Some(true));
        assert_eq!(forwarded_proto_is_secure(b"http"), Some(false));
        assert_eq!(forwarded_proto_is_secure(b"ws"), Some(false));
        assert_eq!(forwarded_proto_is_secure(b"https, http"), None);
        assert_eq!(forwarded_proto_is_secure(b"ftp"), None);
        assert_eq!(forwarded_proto_is_secure(b""), None);
    }
}
