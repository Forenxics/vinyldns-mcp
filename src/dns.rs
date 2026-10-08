//! Direct DNS queries used to cross-check VinylDNS against live DNS.
//!
//! Queries go straight to a zone's authoritative nameservers (non-recursive,
//! no cache), so the answer is what the DNS server holds right now. Records
//! from both sides are reduced to one canonical text form per type, and then
//! compared as sets.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use hickory_proto::op::{Edns, Message, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use hickory_resolver::Resolver;
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

/// SPF has no dedicated variant in hickory; it is RR type 99.
const SPF_CODE: u16 = 99;
/// Upper bound on nameservers discovered from NS records.
pub const MAX_DISCOVERED_NAMESERVERS: usize = 4;

/// Parses a VinylDNS record type name into a DNS record type.
pub fn record_type(name: &str) -> Option<RecordType> {
    let rt = match name.to_ascii_uppercase().as_str() {
        "A" => RecordType::A,
        "AAAA" => RecordType::AAAA,
        "CNAME" => RecordType::CNAME,
        "DS" => RecordType::DS,
        "MX" => RecordType::MX,
        "NAPTR" => RecordType::NAPTR,
        "NS" => RecordType::NS,
        "PTR" => RecordType::PTR,
        "SOA" => RecordType::SOA,
        "SPF" => RecordType::Unknown(SPF_CODE),
        "SRV" => RecordType::SRV,
        "SSHFP" => RecordType::SSHFP,
        "TXT" => RecordType::TXT,
        _ => return None,
    };
    Some(rt)
}

/// Lower-cased absolute domain name with exactly one trailing dot.
pub fn canonical_name(name: &str) -> String {
    format!("{}.", name.trim().trim_end_matches('.').to_ascii_lowercase())
}

fn canonical_ip(s: &str) -> String {
    s.trim()
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| s.trim().to_string())
}

/// Canonical text for one VinylDNS record data object, or `None` for types
/// that are not compared (SOA, whose serial always moves) or malformed data.
pub fn canonical_vinyldns(record_type: &str, data: &Map<String, Value>) -> Option<String> {
    let s = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_string);
    let n = |k: &str| data.get(k).and_then(Value::as_u64);
    Some(match record_type.to_ascii_uppercase().as_str() {
        "A" | "AAAA" => canonical_ip(&s("address")?),
        "CNAME" => canonical_name(&s("cname")?),
        "PTR" => canonical_name(&s("ptrdname")?),
        "NS" => canonical_name(&s("nsdname")?),
        "TXT" | "SPF" => s("text")?,
        "MX" => format!("{} {}", n("preference")?, canonical_name(&s("exchange")?)),
        "SRV" => format!(
            "{} {} {} {}",
            n("priority")?,
            n("weight")?,
            n("port")?,
            canonical_name(&s("target")?)
        ),
        "NAPTR" => format!(
            "{} {} \"{}\" \"{}\" \"{}\" {}",
            n("order")?,
            n("preference")?,
            s("flags")?.to_ascii_lowercase(),
            s("service")?,
            s("regexp")?,
            canonical_name(&s("replacement")?)
        ),
        "SSHFP" => format!(
            "{} {} {}",
            n("algorithm")?,
            n("type")?,
            s("fingerprint")?.to_ascii_lowercase()
        ),
        "DS" => format!(
            "{} {} {} {}",
            n("keytag")?,
            n("algorithm")?,
            n("digesttype")?,
            s("digest")?.to_ascii_lowercase()
        ),
        _ => return None,
    })
}

/// Splits DNS <character-string>s (length-prefixed) and joins them.
fn character_strings(bytes: &[u8]) -> String {
    let mut out = Vec::new();
    let mut rest = bytes;
    while let Some((&len, tail)) = rest.split_first() {
        let len = usize::from(len).min(tail.len());
        out.extend_from_slice(&tail[..len]);
        rest = &tail[len..];
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Canonical text for one DNS answer record, matching [`canonical_vinyldns`].
pub fn canonical_rdata(data: &RData) -> Option<String> {
    Some(match data {
        RData::A(a) => a.0.to_string(),
        RData::AAAA(a) => a.0.to_string(),
        RData::CNAME(n) => canonical_name(&n.0.to_ascii()),
        RData::NS(n) => canonical_name(&n.0.to_ascii()),
        RData::PTR(n) => canonical_name(&n.0.to_ascii()),
        RData::TXT(t) => t
            .txt_data
            .iter()
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect::<String>(),
        RData::MX(mx) => format!("{} {}", mx.preference, canonical_name(&mx.exchange.to_ascii())),
        RData::SRV(srv) => format!(
            "{} {} {} {}",
            srv.priority,
            srv.weight,
            srv.port,
            canonical_name(&srv.target.to_ascii())
        ),
        RData::NAPTR(n) => format!(
            "{} {} \"{}\" \"{}\" \"{}\" {}",
            n.order,
            n.preference,
            String::from_utf8_lossy(&n.flags).to_ascii_lowercase(),
            String::from_utf8_lossy(&n.services),
            String::from_utf8_lossy(&n.regexp),
            canonical_name(&n.replacement.to_ascii())
        ),
        RData::SSHFP(s) => format!(
            "{} {} {}",
            u8::from(s.algorithm),
            u8::from(s.fingerprint_type),
            hex::encode(&s.fingerprint)
        ),
        RData::Unknown { code, rdata } => match u16::from(*code) {
            SPF_CODE => character_strings(&rdata.anything),
            // DS without hickory's DNSSEC feature: keytag(2) algorithm(1) digest type(1) digest.
            43 if rdata.anything.len() > 4 => {
                let b = &rdata.anything;
                format!(
                    "{} {} {} {}",
                    u16::from_be_bytes([b[0], b[1]]),
                    b[2],
                    b[3],
                    hex::encode(&b[4..])
                )
            }
            _ => return None,
        },
        _ => return None,
    })
}

/// What one nameserver answered for one name and type.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DnsAnswer {
    pub rcode: String,
    /// The AA bit: whether the server answered as an authority for the zone.
    pub authoritative: bool,
    /// Smallest TTL among the matching records.
    pub ttl: Option<u32>,
    /// Canonical record texts, sorted.
    pub records: Vec<String>,
    /// The UDP answer was truncated and the query was repeated over TCP.
    pub via_tcp: bool,
}

/// Sends one non-recursive query to `server`, falling back to TCP when the
/// UDP answer is truncated.
pub async fn query(server: SocketAddr, name: &str, rtype: RecordType, timeout: Duration) -> Result<DnsAnswer, String> {
    let qname = Name::from_ascii(canonical_name(name)).map_err(|e| format!("invalid name '{name}': {e}"))?;
    let mut msg = Message::query();
    msg.metadata.recursion_desired = false;
    msg.add_query(Query::query(qname.clone(), rtype));
    let mut edns = Edns::new();
    edns.set_max_payload(1232);
    msg.set_edns(edns);
    let id = msg.metadata.id;
    let wire = msg.to_vec().map_err(|e| format!("could not encode query: {e}"))?;

    let mut response = tokio::time::timeout(timeout, query_udp(server, &wire, id))
        .await
        .map_err(|_| format!("timed out after {}s (UDP)", timeout.as_secs_f32()))??;
    let mut via_tcp = false;
    if response.metadata.truncation {
        response = tokio::time::timeout(timeout, query_tcp(server, &wire, id))
            .await
            .map_err(|_| format!("timed out after {}s (TCP)", timeout.as_secs_f32()))??;
        via_tcp = true;
    }

    let rcode = response.metadata.response_code;
    if !matches!(rcode, ResponseCode::NoError | ResponseCode::NXDomain) {
        return Err(format!("server answered {rcode}"));
    }
    let wanted = qname.to_lowercase();
    let mut ttl: Option<u32> = None;
    let mut records: Vec<String> = response
        .answers
        .iter()
        .filter(|r| r.record_type() == rtype && r.name.to_lowercase() == wanted)
        .filter_map(|r| {
            ttl = Some(ttl.map_or(r.ttl, |t| t.min(r.ttl)));
            canonical_rdata(&r.data)
        })
        .collect();
    records.sort();
    records.dedup();
    Ok(DnsAnswer {
        rcode: rcode.to_string(),
        authoritative: response.metadata.authoritative,
        ttl,
        records,
        via_tcp,
    })
}

async fn query_udp(server: SocketAddr, wire: &[u8], id: u16) -> Result<Message, String> {
    let bind: SocketAddr = if server.is_ipv4() {
        "0.0.0.0:0".parse().expect("valid literal")
    } else {
        "[::]:0".parse().expect("valid literal")
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| format!("UDP bind failed: {e}"))?;
    socket
        .connect(server)
        .await
        .map_err(|e| format!("UDP connect failed: {e}"))?;
    socket.send(wire).await.map_err(|e| format!("UDP send failed: {e}"))?;
    let mut buf = vec![0u8; 65535];
    loop {
        let n = socket
            .recv(&mut buf)
            .await
            .map_err(|e| format!("UDP receive failed: {e}"))?;
        // Ignore stray datagrams that are not the answer to this query.
        if let Ok(msg) = Message::from_vec(&buf[..n])
            && msg.metadata.id == id
        {
            return Ok(msg);
        }
    }
}

async fn query_tcp(server: SocketAddr, wire: &[u8], id: u16) -> Result<Message, String> {
    let mut stream = TcpStream::connect(server)
        .await
        .map_err(|e| format!("TCP connect failed: {e}"))?;
    let len = u16::try_from(wire.len()).map_err(|_| "query too large".to_string())?;
    let mut framed = Vec::with_capacity(wire.len() + 2);
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(wire);
    stream
        .write_all(&framed)
        .await
        .map_err(|e| format!("TCP send failed: {e}"))?;
    let resp_len = stream
        .read_u16()
        .await
        .map_err(|e| format!("TCP receive failed: {e}"))?;
    let mut buf = vec![0u8; usize::from(resp_len)];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("TCP receive failed: {e}"))?;
    let msg = Message::from_vec(&buf).map_err(|e| format!("could not decode answer: {e}"))?;
    if msg.metadata.id != id {
        return Err("TCP answer had a different query ID".into());
    }
    Ok(msg)
}

/// Parses `ip`, `ip:port`, `[v6]:port`, `host` or `host:port` into socket
/// addresses (port 53 by default); host names go through the system resolver.
pub async fn resolve_nameserver(spec: &str) -> Result<Vec<SocketAddr>, String> {
    let spec = spec.trim();
    if let Ok(addr) = spec.parse::<SocketAddr>() {
        return Ok(vec![addr]);
    }
    if let Ok(ip) = spec.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, 53)]);
    }
    let (host, port) = match spec.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse::<u16>().map_err(|_| format!("invalid port in '{spec}'"))?),
        _ => (spec, 53),
    };
    let resolver = system_resolver()?;
    let ips = resolver
        .lookup_ip(host)
        .await
        .map_err(|e| format!("could not resolve nameserver '{host}': {e}"))?;
    preferred_ip(ips.iter())
        .map(|ip| vec![SocketAddr::new(ip, port)])
        .ok_or_else(|| format!("nameserver '{host}' has no addresses"))
}

/// Picks one address per nameserver, preferring IPv4: IPv4 works almost
/// everywhere, while many hosts have no IPv6 route.
fn preferred_ip(ips: impl Iterator<Item = IpAddr>) -> Option<IpAddr> {
    let ips: Vec<IpAddr> = ips.collect();
    ips.iter().find(|ip| ip.is_ipv4()).or_else(|| ips.first()).copied()
}

/// Finds a zone's authoritative nameservers through its NS records, using
/// the system resolver. Returns `(ns host name, address)` pairs.
pub async fn discover_nameservers(zone: &str) -> Result<Vec<(String, SocketAddr)>, String> {
    let resolver = system_resolver()?;
    let ns = resolver
        .lookup(canonical_name(zone), RecordType::NS)
        .await
        .map_err(|e| format!("NS lookup for {zone} failed: {e}"))?;
    let mut hosts: Vec<String> = ns
        .answers()
        .iter()
        .filter_map(|r| match &r.data {
            RData::NS(n) => Some(canonical_name(&n.0.to_ascii())),
            _ => None,
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    let mut out = Vec::new();
    for host in hosts.into_iter().take(MAX_DISCOVERED_NAMESERVERS) {
        match resolver.lookup_ip(host.as_str()).await {
            Ok(ips) => {
                if let Some(ip) = preferred_ip(ips.iter()) {
                    out.push((host, SocketAddr::new(ip, 53)));
                }
            }
            Err(e) => tracing::warn!(%host, error = %e, "could not resolve nameserver"),
        }
    }
    if out.is_empty() {
        return Err(format!("no reachable nameservers found for {zone}"));
    }
    Ok(out)
}

fn system_resolver() -> Result<Resolver<hickory_resolver::net::runtime::TokioRuntimeProvider>, String> {
    Resolver::builder_tokio()
        .and_then(|b| b.build())
        .map_err(|e| format!("could not load the system DNS configuration: {e}"))
}

/// Result of comparing VinylDNS's view of a record set with one nameserver.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    InSync,
    TtlMismatch,
    Mismatch,
    MissingInDns,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Comparison {
    pub status: SyncStatus,
    pub only_in_vinyldns: Vec<String>,
    pub only_in_dns: Vec<String>,
}

/// Compares expected records/TTL with a DNS answer. TTLs are only compared
/// for authoritative answers, since caches count TTLs down.
pub fn compare(expected: &[String], expected_ttl: Option<u32>, answer: &DnsAnswer) -> Comparison {
    let only_in_vinyldns: Vec<String> = expected
        .iter()
        .filter(|r| !answer.records.contains(r))
        .cloned()
        .collect();
    let only_in_dns: Vec<String> = answer
        .records
        .iter()
        .filter(|r| !expected.contains(r))
        .cloned()
        .collect();
    let status = if answer.records.is_empty() && !expected.is_empty() {
        SyncStatus::MissingInDns
    } else if !only_in_vinyldns.is_empty() || !only_in_dns.is_empty() {
        SyncStatus::Mismatch
    } else if answer.authoritative && expected_ttl.is_some() && answer.ttl.is_some() && expected_ttl != answer.ttl {
        SyncStatus::TtlMismatch
    } else {
        SyncStatus::InSync
    };
    Comparison {
        status,
        only_in_vinyldns,
        only_in_dns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::{A, CNAME, MX, SRV, TXT};
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn both_sides_canonicalize_identically() {
        let name = |s: &str| Name::from_ascii(s).unwrap();
        let cases: Vec<(&str, Value, RData)> = vec![
            (
                "A",
                json!({"address": "10.0.0.5"}),
                RData::A(A("10.0.0.5".parse().unwrap())),
            ),
            (
                "CNAME",
                json!({"cname": "Target.Example.com"}),
                RData::CNAME(CNAME(name("target.example.com."))),
            ),
            (
                "MX",
                json!({"preference": 10, "exchange": "mx.example.com."}),
                RData::MX(MX::new(10, name("MX.example.com."))),
            ),
            (
                "SRV",
                json!({"priority": 1, "weight": 2, "port": 443, "target": "svc.example.com."}),
                RData::SRV(SRV::new(1, 2, 443, name("svc.example.com."))),
            ),
            (
                "TXT",
                json!({"text": "v=spf1 -all"}),
                RData::TXT(TXT::new(vec!["v=spf1 ".into(), "-all".into()])),
            ),
        ];
        for (t, vinyl, rdata) in cases {
            assert_eq!(
                canonical_vinyldns(t, &obj(vinyl)),
                canonical_rdata(&rdata),
                "type {t} canonicalizes differently"
            );
        }
    }

    #[test]
    fn ipv6_is_canonicalized() {
        assert_eq!(
            canonical_vinyldns("AAAA", &obj(json!({"address": "2001:DB8:0:0::1"}))).unwrap(),
            "2001:db8::1"
        );
    }

    #[test]
    fn soa_and_unknown_types_are_not_compared() {
        assert_eq!(canonical_vinyldns("SOA", &obj(json!({"mname": "a."}))), None);
        assert_eq!(record_type("BOGUS"), None);
        assert_eq!(record_type("spf"), Some(RecordType::Unknown(SPF_CODE)));
    }

    #[test]
    fn ipv4_is_preferred() {
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        let v4: IpAddr = "192.0.2.1".parse().unwrap();
        assert_eq!(preferred_ip([v6, v4].into_iter()), Some(v4));
        assert_eq!(preferred_ip([v6].into_iter()), Some(v6));
        assert_eq!(preferred_ip(std::iter::empty()), None);
    }

    #[test]
    fn character_strings_are_joined() {
        assert_eq!(character_strings(b"\x03abc\x02de"), "abcde");
        assert_eq!(character_strings(b"\x05ab"), "ab", "truncated input must not panic");
    }

    fn answer(records: &[&str], ttl: u32, authoritative: bool) -> DnsAnswer {
        DnsAnswer {
            rcode: "No Error".into(),
            authoritative,
            ttl: (!records.is_empty()).then_some(ttl),
            records: records.iter().map(|s| s.to_string()).collect(),
            via_tcp: false,
        }
    }

    #[test]
    fn comparison_statuses() {
        let expected = vec!["10.0.0.1".to_string()];
        assert_eq!(
            compare(&expected, Some(300), &answer(&["10.0.0.1"], 300, true)).status,
            SyncStatus::InSync
        );
        assert_eq!(
            compare(&expected, Some(300), &answer(&["10.0.0.1"], 600, true)).status,
            SyncStatus::TtlMismatch
        );
        // Caches count TTLs down, so non-authoritative TTLs are not compared.
        assert_eq!(
            compare(&expected, Some(300), &answer(&["10.0.0.1"], 17, false)).status,
            SyncStatus::InSync
        );
        assert_eq!(
            compare(&expected, Some(300), &answer(&[], 0, true)).status,
            SyncStatus::MissingInDns
        );
        let c = compare(&expected, Some(300), &answer(&["10.0.0.2"], 300, true));
        assert_eq!(c.status, SyncStatus::Mismatch);
        assert_eq!(c.only_in_vinyldns, vec!["10.0.0.1"]);
        assert_eq!(c.only_in_dns, vec!["10.0.0.2"]);
    }

    /// Needs Internet DNS access; run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore = "requires network access to public DNS"]
    async fn discovers_and_queries_public_nameservers() {
        let servers = discover_nameservers("example.com").await.unwrap();
        assert!(!servers.is_empty());
        for (host, addr) in servers {
            let answer = query(addr, "example.com.", RecordType::NS, Duration::from_secs(5))
                .await
                .unwrap();
            assert!(!answer.records.is_empty(), "{host} ({addr}) returned no NS records");
            if !answer.authoritative {
                // Some networks intercept all outbound DNS and answer from a resolver.
                eprintln!("warning: {host} ({addr}) answered without AA; outbound DNS may be intercepted");
            }
        }
    }

    #[tokio::test]
    async fn nameserver_specs_parse() {
        assert_eq!(
            resolve_nameserver("127.0.0.1:5353").await.unwrap(),
            vec!["127.0.0.1:5353".parse::<SocketAddr>().unwrap()]
        );
        assert_eq!(
            resolve_nameserver("10.0.0.53").await.unwrap(),
            vec!["10.0.0.53:53".parse::<SocketAddr>().unwrap()]
        );
        assert_eq!(
            resolve_nameserver("[2001:db8::1]:5300").await.unwrap(),
            vec!["[2001:db8::1]:5300".parse::<SocketAddr>().unwrap()]
        );
        assert!(resolve_nameserver("host:notaport").await.is_err());
    }
}
