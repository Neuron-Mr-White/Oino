"""Merge Oino usage into MemPalace metadata without changing documents or sources.

Input is a JSON argument. This runs inside the existing MemPalace uv environment,
using its backend adapter and writer lease, never a second database implementation.
The retained sidecar is a retry journal when the palace is offline or locked.
"""
import json
import sys


def merge_usage(old, incoming):
    result = dict(old)
    result["last_used_at"] = max(old.get("last_used_at", 0), incoming.get("lastUsed", 0))
    result["use_count"] = max(old.get("use_count", 0), incoming.get("useCount", 0))
    for source, target in (("sessions", "usage_sessions"), ("sources", "usage_sources"), ("mergedFrom", "usage_merged_from")):
        previous = json.loads(old.get(target, "[]"))
        result[target] = json.dumps(list(dict.fromkeys(previous + incoming.get(source, []))))
    return result


def main(payload):
    from mempalace.config import MempalaceConfig
    from mempalace.palace import get_collection, mine_palace_lock

    cfg = MempalaceConfig(palace_path=payload.get("palace"))
    restored = {}
    with mine_palace_lock(cfg.palace_path):
        col = get_collection(cfg.palace_path, collection_name=cfg.collection_name, create=False)
        if col is None:
            raise RuntimeError("memory palace is unavailable")
        for key, incoming in payload["memories"].items():
            project, memory_id = key.split("/", 1)
            # A source can have several physical chunks. Merge existing metadata
            # on each chunk rather than replacing its source/embedding fields.
            paths = [f"{payload['root']}/{project}/{kind}/{memory_id}.md" for kind in ("preference", "decision", "pattern", "summary")]
            rows = col.get(where={"source_file": {"$in": paths}}, include=["metadatas"])
            merged = []
            for metadata in rows.get("metadatas", []):
                merged.append(merge_usage(metadata or {}, incoming))
            if not merged:
                continue
            col.update(ids=rows["ids"], metadatas=merged)
            usage = dict(incoming)
            usage["lastUsed"] = max(m["last_used_at"] for m in merged)
            usage["useCount"] = max(m["use_count"] for m in merged)
            for source, target in (("sessions", "usage_sessions"), ("sources", "usage_sources"), ("mergedFrom", "usage_merged_from")):
                usage[source] = list(dict.fromkeys(x for m in merged for x in json.loads(m[target])))
            restored[key] = usage
    print(json.dumps(restored))


if __name__ == "__main__":
    main(json.loads(sys.argv[1]))
