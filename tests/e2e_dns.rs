//! End-to-end tests for the DNS cross-check tools: the MCP client calls the
//! server, which reads VinylDNS (wiremock) and queries a fake authoritative
//! DNS server running inside the test (UDP and TCP).

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use common::*;
use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, CNAME, TXT};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use vinyldns_mcp::config::ConfirmationMode;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

type Zone = HashMap<(String, RecordType), Vec<Record>>;

/// A tiny authoritative server. Names in `truncate` get an empty, truncated
/// UDP answer, so the client must retry over TCP.
struct FakeDns {
    addr: SocketAddr,
}

fn answer(zone: &Zone, truncate: &[String], query: &[u8], udp: bool) -> Vec<u8> {
    let req = Message::from_vec(query).unwrap();
    let q = req.queries[0].clone();
    let mut resp = Message::new(req.metadata.id, MessageType::Response, OpCode::Query);
    resp.metadata.authoritative = true;
    resp.add_query(q.clone());
    let qname = q.name().to_lowercase().to_ascii();
    if udp && truncate.contains(&qname) {
        resp.metadata.truncation = true;
        return resp.to_vec().unwrap();
    }
    let known_name = zone.keys().any(|(n, _)| *n == qname);
    match zone.get(&(qname, q.query_type())) {
        Some(records) => {
            resp.add_answers(records.clone());
        }
        None if !known_name => resp.metadata.response_code = ResponseCode::NXDomain,
        None => {}
    }
    resp.to_vec().unwrap()
}

impl FakeDns {
    async fn start(records: Vec<Record>, truncate: Vec<&str>) -> Self {
        let mut zone: Zone = HashMap::new();
        for r in records {
            zone.entry((r.name.to_lowercase().to_ascii(), r.record_type()))
                .or_default()
                .push(r);
        }
        let zone = Arc::new(zone);
        let truncate: Arc<Vec<String>> = Arc::new(truncate.into_iter().map(String::from).collect());

        // UDP and TCP on the same port.
        let (udp, tcp) = loop {
            let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            if let Ok(tcp) = TcpListener::bind(udp.local_addr().unwrap()).await {
                break (udp, tcp);
            }
        };
        let addr = udp.local_addr().unwrap();

        let (z, t) = (zone.clone(), truncate.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            while let Ok((n, peer)) = udp.recv_from(&mut buf).await {
                let out = answer(&z, &t, &buf[..n], true);
                let _ = udp.send_to(&out, peer).await;
            }
        });
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = tcp.accept().await {
                let (z, t) = (zone.clone(), truncate.clone());
                tokio::spawn(async move {
                    let len = stream.read_u16().await.unwrap();
                    let mut buf = vec![0u8; usize::from(len)];
                    stream.read_exact(&mut buf).await.unwrap();
                    let out = answer(&z, &t, &buf, false);
                    stream.write_u16(out.len() as u16).await.unwrap();
                    stream.write_all(&out).await.unwrap();
                });
            }
        });
        Self { addr }
    }
}

fn a(name: &str, ttl: u32, ip: &str) -> Record {
    Record::from_rdata(Name::from_ascii(name).unwrap(), ttl, RData::A(A(ip.parse().unwrap())))
}

fn rs(id: &str, name: &str, rtype: &str, ttl: u32, records: Value) -> Value {
    json!({ "id": id, "zoneId": "z1", "name": name, "type": rtype, "ttl": ttl, "records": records, "status": "Active" })
}

async fn mock_vinyldns(api: &MockServer, record_sets: Vec<Value>) {
    Mock::given(method("GET"))
        .and(path("/zones/z1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "zone": { "id": "z1", "name": "example.com." } })),
        )
        .mount(api)
        .await;
    for r in &record_sets {
        Mock::given(method("GET"))
            .and(path(format!("/zones/z1/recordsets/{}", r["id"].as_str().unwrap())))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "recordSet": r })))
            .mount(api)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/zones/z1/recordsets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "recordSets": record_sets, "maxItems": 100 })))
        .mount(api)
        .await;
}

async fn client(api: &MockServer) -> rmcp::service::RunningService<rmcp::RoleClient, TestClient> {
    connect(
        config(api, false, ConfirmationMode::Auto),
        TestClient { elicit_answer: None },
    )
    .await
}

#[tokio::test]
async fn record_set_in_sync() {
    let dns = FakeDns::start(vec![a("www.example.com.", 300, "10.0.0.5")], vec![]).await;
    let api = MockServer::start().await;
    mock_vinyldns(
        &api,
        vec![rs("r1", "www", "A", 300, json!([{ "address": "10.0.0.5" }]))],
    )
    .await;
    let client = client(&api).await;

    let out = json_of(
        &call(
            &client,
            "check_record_set_dns",
            json!({ "zone_id": "z1", "record_set_id": "r1", "nameservers": [dns.addr.to_string()] }),
        )
        .await,
    );
    assert_eq!(out["status"], "in_sync", "{out:#}");
    assert_eq!(out["name"], "www.example.com.");
    assert_eq!(out["nameserver_source"], "parameter");
    assert_eq!(out["nameservers"][0]["authoritative"], true);
}

#[tokio::test]
async fn zone_check_reports_each_kind_of_drift() {
    let dns = FakeDns::start(
        vec![
            a("ok.example.com.", 300, "10.0.0.1"),
            a("drift.example.com.", 300, "10.0.0.99"),
            a("ttl.example.com.", 60, "10.0.0.3"),
            Record::from_rdata(
                Name::from_ascii("alias.example.com.").unwrap(),
                300,
                RData::CNAME(CNAME(Name::from_ascii("ok.example.com.").unwrap())),
            ),
            // Long TXT split into several strings, served only over TCP.
            Record::from_rdata(
                Name::from_ascii("big.example.com.").unwrap(),
                300,
                RData::TXT(TXT::new(vec!["a".repeat(255), "tail".into()])),
            ),
        ],
        vec!["big.example.com."],
    )
    .await;
    let api = MockServer::start().await;
    mock_vinyldns(
        &api,
        vec![
            rs("r1", "ok", "A", 300, json!([{ "address": "10.0.0.1" }])),
            rs("r2", "drift", "A", 300, json!([{ "address": "10.0.0.2" }])),
            rs("r3", "ttl", "A", 300, json!([{ "address": "10.0.0.3" }])),
            rs("r4", "gone", "A", 300, json!([{ "address": "10.0.0.4" }])),
            rs("r5", "alias", "CNAME", 300, json!([{ "cname": "OK.example.com" }])),
            rs(
                "r6",
                "big",
                "TXT",
                300,
                json!([{ "text": format!("{}tail", "a".repeat(255)) }]),
            ),
            rs(
                "r7",
                "example.com.",
                "SOA",
                300,
                json!([{ "mname": "ns1.example.com." }]),
            ),
        ],
    )
    .await;
    let client = client(&api).await;

    let out = json_of(
        &call(
            &client,
            "check_zone_dns",
            json!({ "zone_id": "z1", "nameservers": [dns.addr.to_string()], "include_in_sync": true }),
        )
        .await,
    );
    assert_eq!(out["checked"], 7);
    assert_eq!(
        out["counts"],
        json!({ "in_sync": 3, "mismatch": 1, "ttl_mismatch": 1, "missing_in_dns": 1, "skipped": 1 }),
        "{out:#}"
    );
    assert_eq!(
        out["in_sync"],
        json!(["A ok.example.com.", "CNAME alias.example.com.", "TXT big.example.com."])
    );
    let problem = |name: &str| {
        out["problems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("no problem entry for {name}"))
            .clone()
    };
    let drift = problem("drift.example.com.");
    assert_eq!(drift["nameservers"][0]["only_in_vinyldns"], json!(["10.0.0.2"]));
    assert_eq!(drift["nameservers"][0]["only_in_dns"], json!(["10.0.0.99"]));
    assert_eq!(problem("ttl.example.com.")["nameservers"][0]["ttl"], 60);
    assert_eq!(
        problem("gone.example.com.")["nameservers"][0]["rcode"],
        "Non-Existent Domain"
    );
}

#[tokio::test]
async fn unreachable_nameserver_is_an_error_not_a_hang() {
    // Nothing listens here, so the query times out (the test config uses 2s).
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    let api = MockServer::start().await;
    mock_vinyldns(
        &api,
        vec![rs("r1", "www", "A", 300, json!([{ "address": "10.0.0.5" }]))],
    )
    .await;
    let client = client(&api).await;

    let out = json_of(
        &call(
            &client,
            "check_record_set_dns",
            json!({ "zone_id": "z1", "record_set_id": "r1", "nameservers": [addr.to_string()] }),
        )
        .await,
    );
    assert_eq!(out["status"], "error");
    assert!(
        out["nameservers"][0]["error"].as_str().unwrap().contains("timed out"),
        "{out:#}"
    );
}
