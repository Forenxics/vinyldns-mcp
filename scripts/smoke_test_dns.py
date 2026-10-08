#!/usr/bin/env python3
"""Live smoke test for the DNS cross-check tools against the VinylDNS quickstart.

Creates a record through the MCP server, checks it is in sync with the
quickstart's BIND server, then (with --drift-container) changes the record
directly in BIND with nsupdate behind VinylDNS's back and checks that both
check_record_set_dns and check_zone_dns report the drift. Cleans up after itself.

Usage: scripts/smoke_test_dns.py [--drift-container vinyldns-api-integration] [path/to/vinyldns-mcp]
Environment: SMOKE_DNS_SERVER (default 127.0.0.1:19001), plus those of smoke_test.py.
"""
import argparse
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import smoke_test as base  # noqa: E402

DNS_SERVER = os.environ.get("SMOKE_DNS_SERVER", "127.0.0.1:19001")


def nsupdate(container, commands):
    """Runs nsupdate inside the BIND container with the quickstart's update key,
    read from the container's own configuration (nothing is hard-coded here)."""
    conf = subprocess.run(["docker", "exec", container, "sh", "-c", "cat /etc/bind/*.conf*"],
                          capture_output=True, text=True, check=True).stdout
    m = re.search(r'key "vinyldns\." \{\s*algorithm ([\w-]+);\s*secret "([^"]+)";', conf)
    assert m, "could not find the vinyldns. TSIG key in the container's BIND config"
    script = "server 127.0.0.1 19001\nzone ok.\n" + "\n".join(commands) + "\nsend\n"
    subprocess.run(["docker", "exec", "-i", container, "nsupdate", "-y", f"{m[1]}:vinyldns.:{m[2]}"],
                   input=script, text=True, check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--drift-container")
    ap.add_argument("binary", nargs="?")
    args = ap.parse_args()
    if args.binary:
        base.BINARY = args.binary

    zone_id = base.ensure_zone()
    mcp = base.Mcp()
    try:
        name = f"dnscheck-{int(time.time())}"
        fqdn = f"{name}.ok."
        plan = mcp.tool("plan_create_record_set", {"zone_id": zone_id, "name": name, "record_type": "A",
                                                   "ttl": 300, "records": [{"address": "192.0.2.60"}]})
        rs_id = base.wait_complete(mcp, zone_id, mcp.tool("confirm_change", {"token": plan["token"]}))

        out = mcp.tool("check_record_set_dns", {"zone_id": zone_id, "record_set_id": rs_id,
                                                "nameservers": [DNS_SERVER]})
        print(f"      {fqdn}: {out['status']} (authoritative={out['nameservers'][0].get('authoritative')})")
        assert out["status"] == "in_sync", out

        zone = mcp.tool("check_zone_dns", {"zone_id": zone_id, "nameservers": [DNS_SERVER]})
        print(f"      zone ok.: checked {zone['checked']}, counts {zone['counts']}")

        if args.drift_container:
            nsupdate(args.drift_container, [f"update delete {fqdn} A", f"update add {fqdn} 120 A 192.0.2.61"])
            out = mcp.tool("check_record_set_dns", {"zone_id": zone_id, "record_set_id": rs_id,
                                                    "nameservers": [DNS_SERVER]})
            ns = out["nameservers"][0]
            print(f"      after drift: {out['status']}, only_in_vinyldns={ns['only_in_vinyldns']}, "
                  f"only_in_dns={ns['only_in_dns']}, ttl={ns['ttl']}")
            assert out["status"] == "mismatch", out
            assert ns["only_in_vinyldns"] == ["192.0.2.60"] and ns["only_in_dns"] == ["192.0.2.61"], out

            zone = mcp.tool("check_zone_dns", {"zone_id": zone_id, "nameservers": [DNS_SERVER],
                                               "name_filter": name})
            print(f"      zone check (filtered): counts {zone['counts']}")
            assert zone["counts"].get("mismatch") == 1, zone

            # Put DNS back so the cleanup delete below matches.
            nsupdate(args.drift_container, [f"update delete {fqdn} A", f"update add {fqdn} 300 A 192.0.2.60"])

        plan = mcp.tool("plan_delete_record_set", {"zone_id": zone_id, "record_set_id": rs_id})
        base.wait_complete(mcp, zone_id, mcp.tool("confirm_change", {"token": plan["token"]}))
        out = mcp.tool("check_zone_dns", {"zone_id": zone_id, "nameservers": [DNS_SERVER], "name_filter": name})
        assert out["checked"] == 0, out
        print("DNS SMOKE TEST PASSED")
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
