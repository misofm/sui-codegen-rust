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
QUERY = """query($a: SuiAddress!) {
  checkpoint { sequenceNumber }
  package(address: $a) { address version digest packageBcs linkage { originalId upgradedId version } }
}"""

def gql(network, variables):
    req = urllib.request.Request(
        ENDPOINTS[network],
        data=json.dumps({"query": QUERY, "variables": variables}).encode(),
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
        (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"{network}: {len(manifest)} packages")

if __name__ == "__main__":
    main()
