#!/usr/bin/env python3
"""Fetches the package corpus used by oracle and golden tests into fixtures/packages/.

For each network, starts from the roots in fixtures/corpus.json, follows every package's
linkage table (so all dependencies at their linked versions are included), and stores the raw
`MovePackage` BCS (GraphQL `packageBcs`) plus a manifest. Re-running is idempotent; the manifest
records the checkpoint each package was read at.
"""
import base64, json, pathlib, sys, urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
ENDPOINTS = {
    "mainnet": "https://graphql.mainnet.sui.io/graphql",
    "testnet": "https://graphql.testnet.sui.io/graphql",
}
SYSTEM_OBJECTS = ["0x5", "0x6", "0x8", "0xa", "0xc", "0xd", "0x403", "0xacc"]
CHAIN_QUERY = """query($keys: [ObjectKey!]!) {
  chainIdentifier
  checkpoint { sequenceNumber }
  multiGetObjects(keys: $keys) { address owner { __typename ... on Shared { initialSharedVersion } } }
}"""
QUERY = """query($a: SuiAddress!) {
  checkpoint { sequenceNumber }
  package(address: $a) { address version digest packageBcs linkage { originalId upgradedId version } }
}"""

def gql(network, variables, query=None):
    req = urllib.request.Request(
        ENDPOINTS[network],
        data=json.dumps({"query": query or QUERY, "variables": variables}).encode(),
        headers={"content-type": "application/json", "user-agent": "movegen-fixtures"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        body = json.load(r)
    if body.get("errors"):
        raise RuntimeError(body["errors"])
    return body["data"]

def main():
    corpus = json.loads((ROOT / "fixtures/corpus.json").read_text())
    for network, roots in corpus.items():
        out = ROOT / "fixtures/packages" / network
        out.mkdir(parents=True, exist_ok=True)
        manifest, queue, seen = [], list(roots.items()), set()
        while queue:
            name, address = queue.pop(0)
            if address in seen:
                continue
            seen.add(address)
            data = gql(network, {"a": address})
            pkg = data["package"]
            if pkg is None:
                print(f"{network}: {name} {address} not found", file=sys.stderr)
                continue
            (out / f"{pkg['address']}.bcs").write_bytes(base64.b64decode(pkg["packageBcs"]))
            manifest.append({
                "name": name, "requested": address, "storage_id": pkg["address"],
                "version": pkg["version"], "digest": pkg["digest"],
                "checkpoint": data["checkpoint"]["sequenceNumber"],
            })
            for link in pkg["linkage"] or []:
                queue.append((f"dep:{link['originalId'][:10]}", link["upgradedId"]))
        manifest.sort(key=lambda m: (m["name"], m["storage_id"]))
        chain = gql(network, {"keys": [{"address": a} for a in SYSTEM_OBJECTS]}, CHAIN_QUERY)
        (out / "chain.json").write_text(json.dumps({
            "chain_identifier": chain["chainIdentifier"],
            "checkpoint": chain["checkpoint"]["sequenceNumber"],
            "system_objects": {
                o["address"]: o["owner"]["initialSharedVersion"]
                for o in chain["multiGetObjects"] if o and o["owner"]["__typename"] == "Shared"
            },
        }, indent=2) + "\n")
        (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"{network}: {len(manifest)} packages")

if __name__ == "__main__":
    main()
