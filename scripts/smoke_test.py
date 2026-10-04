#!/usr/bin/env python3
"""Live smoke test: drives the vinyldns-mcp binary over stdio against a real VinylDNS API.

Intended for the VinylDNS quickstart (`quickstart/quickstart-vinyldns.sh --api`), which
loads test users and a bind9 server with an `ok.` zone. It:

  1. creates a group and connects the `ok.` zone directly through the API (setup only;
     zone management is not exposed through MCP),
  2. starts the MCP server with writes enabled and runs read tools,
  3. creates, updates and deletes a record set through plan_* + confirm_change,
  4. submits and inspects a batch change,
  5. checks that every applied record change completes.

Usage:
  scripts/smoke_test.py [path/to/vinyldns-mcp]

Environment (defaults suit the quickstart):
  VINYLDNS_API_URL=http://localhost:9000  VINYLDNS_ACCESS_KEY=okAccessKey  VINYLDNS_SECRET_KEY=okSecretKey
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

sys.path.insert(0, os.path.dirname(__file__))
from vinyldns_sig_reference import signature  # noqa: E402

API = os.environ.get("VINYLDNS_API_URL", "http://localhost:9000")
AK = os.environ.get("VINYLDNS_ACCESS_KEY", "okAccessKey")
SK = os.environ.get("VINYLDNS_SECRET_KEY", "okSecretKey")
BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/release/vinyldns-mcp"
ZONE = "ok."


def api(method, path, body=None):
    """Signed direct API call (used only for test setup)."""
    url = API.rstrip("/") + path
    data = json.dumps(body, separators=(",", ":")) if body is not None else ""
    amz = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    sig = signature(method, url, data, amz, SK)
    auth = (f"AWS4-HMAC-SHA256 Credential={AK}/{amz[:8]}/us-east-1/VinylDNS/aws4_request, "
            f"SignedHeaders=host;x-amz-date, Signature={sig}")
    req = urllib.request.Request(url, data=data.encode() if body is not None else None, method=method,
                                 headers={"Authorization": auth, "X-Amz-Date": amz,
                                          "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            text = r.read().decode()
            return r.status, json.loads(text) if text.strip() else None
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def ensure_zone():
    status, body = api("GET", f"/zones/name/{ZONE}")
    if status == 200:
        return body["zone"]["id"]
    status, group = api("POST", "/groups", {
        "name": f"mcp-smoke-{int(time.time())}", "email": "test@test.com",
        "members": [{"id": "ok"}], "admins": [{"id": "ok"}]})
    assert status == 200, (status, group)
    status, change = api("POST", "/zones", {"name": ZONE, "email": "test@test.com", "adminGroupId": group["id"]})
    assert status == 202, (status, change)
    for _ in range(60):
        status, body = api("GET", f"/zones/name/{ZONE}")
        if status == 200 and body["zone"]["status"] == "Active":
            return body["zone"]["id"]
        time.sleep(1)
    raise SystemExit("zone did not become active")


class Mcp:
    def __init__(self):
        env = dict(os.environ, VINYLDNS_API_URL=API, VINYLDNS_ACCESS_KEY=AK, VINYLDNS_SECRET_KEY=SK,
                   VINYLDNS_MCP_ENABLE_WRITES="true", VINYLDNS_MCP_CONFIRMATION="auto",
                   VINYLDNS_MCP_LOG="warn")
        self.p = subprocess.Popen([BINARY], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True)
        self.next_id = 0
        init = self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                           "clientInfo": {"name": "smoke", "version": "0"}})
        print(f"server: {init['serverInfo']['name']} {init['serverInfo']['version']}")
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, msg):
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        while True:
            msg = json.loads(self.p.stdout.readline())
            if msg.get("id") == self.next_id:
                if "error" in msg:
                    raise RuntimeError(msg["error"])
                return msg["result"]

    def tool(self, name, args=None, expect_error=False):
        result = self.request("tools/call", {"name": name, "arguments": args or {}})
        text = result["content"][0]["text"]
        if bool(result.get("isError")) != expect_error:
            raise AssertionError(f"{name}: unexpected isError={result.get('isError')}: {text}")
        print(f"  ok  {name}")
        return text if expect_error else json.loads(text)

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


def wait_complete(mcp, zone_id, applied):
    change = applied["result"]
    for _ in range(30):
        args = {"zone_id": zone_id, "record_set_id": change["recordSet"]["id"], "change_id": change["id"]}
        result = mcp.request("tools/call", {"name": "get_record_set_change", "arguments": args})
        if result.get("isError"):  # 404 while the change is still queued
            time.sleep(1)
            continue
        c = json.loads(result["content"][0]["text"])
        if c["status"] in ("Complete", "Failed"):
            assert c["status"] == "Complete", c
            print(f"  ok  {change['changeType']} {change['recordSet']['name']} -> Complete")
            return change["recordSet"]["id"]
        time.sleep(1)
    raise AssertionError("record change did not complete")


def main():
    zone_id = ensure_zone()
    mcp = Mcp()
    try:
        tools = [t["name"] for t in mcp.request("tools/list", {})["tools"]]
        print(f"{len(tools)} tools: {', '.join(tools)}")

        conn = mcp.tool("check_connection")
        assert conn["credentials_valid"], conn
        assert any(z["name"] == ZONE for z in mcp.tool("list_zones")["zones"])
        assert mcp.tool("get_zone", {"zone_name": ZONE})["zone"]["id"] == zone_id
        mcp.tool("get_zone", {"zone_name": "no-such-zone."}, expect_error=True)
        mcp.tool("list_groups")
        mcp.tool("get_user", {"user": "ok"})

        name = f"mcp-smoke-{int(time.time())}"
        plan = mcp.tool("plan_create_record_set", {"zone_id": zone_id, "name": name, "record_type": "A",
                                                   "ttl": 300, "records": [{"address": "192.0.2.10"}]})
        rs_id = wait_complete(mcp, zone_id, mcp.tool("confirm_change", {"token": plan["token"]}))
        mcp.tool("confirm_change", {"token": plan["token"]}, expect_error=True)  # single use

        plan = mcp.tool("plan_update_record_set", {"zone_id": zone_id, "record_set_id": rs_id, "ttl": 600,
                                                   "records": [{"address": "192.0.2.11"}]})
        assert plan["preview"]["before"]["ttl"] == 300 and plan["preview"]["after"]["ttl"] == 600
        wait_complete(mcp, zone_id, mcp.tool("confirm_change", {"token": plan["token"]}))
        rs = mcp.tool("get_record_set", {"zone_id": zone_id, "record_set_id": rs_id})["recordSet"]
        assert rs["ttl"] == 600 and rs["records"] == [{"address": "192.0.2.11"}], rs

        plan = mcp.tool("plan_delete_record_set", {"zone_id": zone_id, "record_set_id": rs_id})
        wait_complete(mcp, zone_id, mcp.tool("confirm_change", {"token": plan["token"]}))

        plan = mcp.tool("plan_batch_change", {"comments": "mcp smoke test", "changes": [
            {"change_type": "Add", "input_name": f"{name}-batch.{ZONE}", "record_type": "A", "ttl": 300,
             "record": {"address": "192.0.2.20"}}]})
        batch = mcp.tool("confirm_change", {"token": plan["token"]})["result"]
        for _ in range(30):
            b = mcp.tool("get_batch_change", {"id": batch["id"]})
            if b["status"] not in ("PendingProcessing",):
                break
            time.sleep(1)
        assert b["status"] == "Complete", b
        mcp.tool("list_batch_changes")
        mcp.tool("list_record_set_changes", {"zone_id": zone_id, "max_items": 5})

        plan = mcp.tool("plan_delete_record_set", {"zone_id": zone_id, "record_set_id": b["changes"][0]["recordSetId"]})
        mcp.tool("discard_pending_change", {"token": plan["token"]})
        assert mcp.tool("list_pending_changes")["pending"] == []
        print("SMOKE TEST PASSED")
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
