//! Read-only tools that compare VinylDNS's records with live DNS.

use std::net::SocketAddr;

use futures_util::{StreamExt, stream};
use rmcp::{
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{ToolResult, VinylDnsServer, api_error, fqdn, ok_json, tool_error};
use crate::client::segment;
use crate::dns::{self, SyncStatus};

/// Default and maximum number of record sets `check_zone_dns` inspects.
const DEFAULT_ZONE_LIMIT: usize = 200;
const MAX_ZONE_LIMIT: usize = 1000;
/// Record sets checked in parallel.
const CONCURRENCY: usize = 8;
/// Full per-record-set detail is included for at most this many problems.
const MAX_DETAILED_PROBLEMS: usize = 50;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckRecordSetDnsParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Record set ID (UUID).
    pub record_set_id: String,
    /// Nameservers to query (`ip`, `ip:port` or `host[:port]`). Default: the server's configured
    /// list, otherwise the zone's authoritative nameservers from its NS records.
    #[serde(default)]
    pub nameservers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckZoneDnsParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Nameservers to query; see `check_record_set_dns`.
    #[serde(default)]
    pub nameservers: Option<Vec<String>>,
    /// Only record sets whose name matches this filter (supports `*`).
    #[serde(default)]
    pub name_filter: Option<String>,
    /// Only these record types, comma-separated, e.g. `A,CNAME`.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Maximum record sets to check (default 200, max 1000).
    #[serde(default)]
    pub max_record_sets: Option<usize>,
    /// Also list the names of record sets that are in sync (default false: counts only).
    #[serde(default)]
    pub include_in_sync: Option<bool>,
}

/// Overall severity across nameservers: the worst result wins.
fn severity(s: SyncStatus) -> u8 {
    match s {
        SyncStatus::InSync => 0,
        SyncStatus::Error => 1,
        SyncStatus::TtlMismatch => 2,
        SyncStatus::MissingInDns => 3,
        SyncStatus::Mismatch => 4,
    }
}

fn status_name(s: SyncStatus) -> &'static str {
    match s {
        SyncStatus::InSync => "in_sync",
        SyncStatus::TtlMismatch => "ttl_mismatch",
        SyncStatus::Mismatch => "mismatch",
        SyncStatus::MissingInDns => "missing_in_dns",
        SyncStatus::Error => "error",
    }
}

impl VinylDnsServer {
    /// Picks the nameservers to query: explicit parameter, then configuration,
    /// then discovery from the zone's NS records.
    async fn nameservers_for(
        &self,
        zone_name: &str,
        requested: Option<Vec<String>>,
    ) -> Result<(Vec<(String, SocketAddr)>, &'static str), String> {
        let (specs, source) = match requested.filter(|v| !v.is_empty()) {
            Some(v) => (v, "parameter"),
            None if !self.dns_nameservers.is_empty() => (self.dns_nameservers.clone(), "configuration"),
            None => return dns::discover_nameservers(zone_name).await.map(|ns| (ns, "NS records")),
        };
        let mut out = Vec::new();
        for spec in specs {
            for addr in dns::resolve_nameserver(&spec).await? {
                out.push((spec.clone(), addr));
            }
        }
        Ok((out, source))
    }

    /// Compares one record set (as returned by VinylDNS) with each nameserver.
    async fn check_one(&self, zone_name: &str, rs: &Value, servers: &[(String, SocketAddr)]) -> Value {
        let name = rs.get("name").and_then(Value::as_str).unwrap_or_default();
        let rtype_name = rs.get("type").and_then(Value::as_str).unwrap_or_default();
        let fqdn = fqdn(name, zone_name);
        let base = json!({ "name": fqdn, "type": rtype_name, "record_set_id": rs.get("id") });

        let skip = |reason: &str| {
            let mut v = base.clone();
            v["status"] = json!("skipped");
            v["reason"] = json!(reason);
            v
        };
        if rtype_name.eq_ignore_ascii_case("SOA") {
            return skip("SOA serial numbers change on every update, so SOA is not compared");
        }
        let Some(rtype) = dns::record_type(rtype_name) else {
            return skip("record type not supported by the DNS check");
        };
        let records: Vec<Map<String, Value>> = rs
            .get("records")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| r.as_object().cloned())
            .collect();
        let mut expected: Vec<String> = records
            .iter()
            .filter_map(|r| dns::canonical_vinyldns(rtype_name, r))
            .collect();
        expected.sort();
        expected.dedup();
        let expected_ttl = rs
            .get("ttl")
            .and_then(Value::as_u64)
            .and_then(|t| u32::try_from(t).ok());

        let mut worst = SyncStatus::InSync;
        let mut answers: Vec<Vec<String>> = Vec::new();
        let mut notes = Vec::new();
        let mut results = Vec::with_capacity(servers.len());
        for (label, addr) in servers {
            let entry = match dns::query(*addr, &fqdn, rtype, self.dns_timeout).await {
                Ok(answer) => {
                    let cmp = dns::compare(&expected, expected_ttl, &answer);
                    if !answer.authoritative {
                        notes.push(format!(
                            "{label} answered without the authoritative flag (a cache, a forwarder, or a network that intercepts DNS traffic?); its TTLs are not compared and its data may be stale"
                        ));
                    }
                    answers.push(answer.records.clone());
                    if severity(cmp.status) > severity(worst) {
                        worst = cmp.status;
                    }
                    json!({
                        "nameserver": label,
                        "address": addr.to_string(),
                        "status": status_name(cmp.status),
                        "rcode": answer.rcode,
                        "authoritative": answer.authoritative,
                        "ttl": answer.ttl,
                        "records": answer.records,
                        "only_in_vinyldns": cmp.only_in_vinyldns,
                        "only_in_dns": cmp.only_in_dns,
                        "via_tcp": answer.via_tcp,
                    })
                }
                Err(error) => {
                    if severity(SyncStatus::Error) > severity(worst) {
                        worst = SyncStatus::Error;
                    }
                    json!({ "nameserver": label, "address": addr.to_string(), "status": "error", "error": error })
                }
            };
            results.push(entry);
        }
        answers.dedup();
        if answers.len() > 1 {
            notes.push("Nameservers disagree with each other (a secondary may be lagging behind the primary).".into());
        }
        let rs_status = rs.get("status").and_then(Value::as_str).unwrap_or("Active");
        if rs_status != "Active" && worst != SyncStatus::InSync {
            notes.push(format!(
                "The record set is '{rs_status}' in VinylDNS: a change is in progress, so differences may be temporary."
            ));
        }
        if expected.len() < records.len() {
            notes.push("Some VinylDNS record data could not be interpreted and was left out of the comparison.".into());
        }
        notes.dedup();

        let mut v = base;
        v["status"] = json!(status_name(worst));
        v["vinyldns"] = json!({ "ttl": expected_ttl, "records": expected, "status": rs_status });
        v["nameservers"] = json!(results);
        if !notes.is_empty() {
            v["notes"] = json!(notes);
        }
        v
    }

    async fn zone_name(&self, zone_id: &str) -> Result<String, crate::client::ApiError> {
        let v = self.client.get(&format!("zones/{}", segment(zone_id)), &[]).await?;
        Ok(v.pointer("/zone/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }
}

#[tool_router(router = dns_router, vis = "pub(super)")]
impl VinylDnsServer {
    #[tool(
        description = "Check whether one record set in VinylDNS matches what DNS actually serves. Queries the zone's \
                       authoritative nameservers directly (no caches) and reports, per nameserver, whether the records \
                       and TTL match: in_sync, ttl_mismatch, mismatch, missing_in_dns or error. Read-only.",
        annotations(
            title = "Check record set against DNS",
            read_only_hint = true,
            open_world_hint = true
        )
    )]
    async fn check_record_set_dns(&self, Parameters(p): Parameters<CheckRecordSetDnsParams>) -> ToolResult {
        let zone_name = api!(self.zone_name(&p.zone_id).await);
        let rs = api!(
            self.client
                .get(
                    &format!("zones/{}/recordsets/{}", segment(&p.zone_id), segment(&p.record_set_id)),
                    &[],
                )
                .await
        );
        let rs = rs.get("recordSet").cloned().unwrap_or(rs);
        let (servers, source) = match self.nameservers_for(&zone_name, p.nameservers).await {
            Ok(s) => s,
            Err(e) => return tool_error(format!("{e}. Pass `nameservers` explicitly (e.g. [\"10.0.0.53\"]).")),
        };
        let mut result = self.check_one(&zone_name, &rs, &servers).await;
        result["nameserver_source"] = json!(source);
        ok_json(result)
    }

    #[tool(
        description = "Check a whole zone (or a filtered part of it) against live DNS: every record set in VinylDNS is \
                       looked up on the zone's authoritative nameservers. Returns counts per status and details of every \
                       record set that is not in sync. Records that exist only in DNS (not in VinylDNS) are not detected; \
                       use plan_sync_zone for that. Read-only.",
        annotations(title = "Check zone against DNS", read_only_hint = true, open_world_hint = true)
    )]
    async fn check_zone_dns(&self, Parameters(p): Parameters<CheckZoneDnsParams>) -> ToolResult {
        let limit = p.max_record_sets.unwrap_or(DEFAULT_ZONE_LIMIT).clamp(1, MAX_ZONE_LIMIT);
        let zone_name = api!(self.zone_name(&p.zone_id).await);
        let (servers, source) = match self.nameservers_for(&zone_name, p.nameservers).await {
            Ok(s) => s,
            Err(e) => return tool_error(format!("{e}. Pass `nameservers` explicitly (e.g. [\"10.0.0.53\"]).")),
        };

        // Collect record sets page by page.
        let path = format!("zones/{}/recordsets", segment(&p.zone_id));
        let mut record_sets: Vec<Value> = Vec::new();
        let mut start_from: Option<String> = None;
        let mut truncated = false;
        loop {
            let page = api!(
                self.client
                    .get(
                        &path,
                        &[
                            ("recordNameFilter", p.name_filter.clone()),
                            ("recordTypeFilter", p.type_filter.clone()),
                            ("startFrom", start_from.clone()),
                            ("maxItems", Some("100".into())),
                        ],
                    )
                    .await
            );
            record_sets.extend(
                page.get("recordSets")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            start_from = page.get("nextId").and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            });
            if record_sets.len() >= limit {
                truncated = start_from.is_some() || record_sets.len() > limit;
                record_sets.truncate(limit);
                break;
            }
            if start_from.is_none() {
                break;
            }
        }

        // Build the futures first: a closure inside the stream would hit
        // rustc's higher-ranked lifetime limitation for `Send` futures.
        let futures: Vec<_> = record_sets
            .iter()
            .map(|rs| self.check_one(&zone_name, rs, &servers))
            .collect();
        let checks: Vec<Value> = stream::iter(futures).buffer_unordered(CONCURRENCY).collect().await;

        let mut counts: Map<String, Value> = Map::new();
        let mut problems = Vec::new();
        let mut in_sync = Vec::new();
        for c in checks {
            let status = c["status"].as_str().unwrap_or("error").to_string();
            let n = counts.get(&status).and_then(Value::as_u64).unwrap_or(0) + 1;
            counts.insert(status.clone(), json!(n));
            match status.as_str() {
                "in_sync" => in_sync.push(format!(
                    "{} {}",
                    c["type"].as_str().unwrap_or("?"),
                    c["name"].as_str().unwrap_or("?")
                )),
                "skipped" => {}
                _ => problems.push(c),
            }
        }
        problems.sort_by_key(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["type"].as_str().unwrap_or("").to_string(),
            )
        });
        in_sync.sort();
        let more_problems: Vec<String> = problems
            .iter()
            .skip(MAX_DETAILED_PROBLEMS)
            .map(|c| {
                format!(
                    "{} {} ({})",
                    c["type"].as_str().unwrap_or("?"),
                    c["name"].as_str().unwrap_or("?"),
                    c["status"].as_str().unwrap_or("?")
                )
            })
            .collect();
        problems.truncate(MAX_DETAILED_PROBLEMS);

        let mut out = json!({
            "zone": zone_name,
            "nameservers": servers.iter().map(|(l, a)| format!("{l} ({a})")).collect::<Vec<_>>(),
            "nameserver_source": source,
            "checked": record_sets.len(),
            "truncated": truncated,
            "counts": counts,
            "problems": problems,
        });
        if !more_problems.is_empty() {
            out["more_problems"] = json!(more_problems);
        }
        if p.include_in_sync.unwrap_or(false) {
            out["in_sync"] = json!(in_sync);
        }
        if truncated {
            out["note"] = json!(format!(
                "Only the first {limit} record sets were checked; narrow with name_filter/type_filter or raise max_record_sets (max {MAX_ZONE_LIMIT})."
            ));
        }
        ok_json(out)
    }
}
