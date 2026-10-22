#!/usr/bin/env python3
"""AgentHarbor web search: an MCP server (stdio) with one tool, web_search.

Deployed by AgentHarbor beside config.json, which names the backend and holds
its key (readable by its owner only):
  deepseek - DeepSeek's search API, returning ranked web pages;
  mimo     - MiMo's own search on its Chat API, returning a sourced answer.
Standard library only, so any Python 3.8+ runs it.
"""
import json
import os
import sys
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
SERVER = {"name": "agentharbor-web-search", "version": "1"}
TOOL = {
    "name": "web_search",
    "description": (
        "Search the web for current information: news, releases, documentation, prices, "
        "anything after your training data. Returns sources with links; cite them."
    ),
    "inputSchema": {
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "What to search for, as a search engine query."},
            "count": {"type": "integer", "minimum": 1, "maximum": 20, "description": "How many results (default 8)."},
        },
        "required": ["query"],
    },
}
TIMEOUT = 60
SUMMARY_CHARS = 700


def load_config():
    with open(os.path.join(HERE, "config.json"), encoding="utf-8") as handle:
        return json.load(handle)


def post(url, key, body):
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        headers={"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        raise RuntimeError("search service answered HTTP %d" % error.code) from None
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise RuntimeError("search service unreachable: %s" % getattr(error, "reason", error)) from None


def clip(text, limit):
    text = " ".join((text or "").split())
    return text if len(text) <= limit else text[: limit - 1] + "…"


def deepseek(config, query, count):
    # DeepSeek refuses queries longer than 128 characters.
    reply = post(config["endpoint"], config["key"], {"query": query[:128], "count": count})
    lines = []
    for index, result in enumerate(reply.get("results") or [], 1):
        title = result.get("title") or result.get("site_name") or result.get("url", "")
        lines.append("%d. %s" % (index, title))
        lines.append("   %s" % result.get("url", ""))
        if result.get("date"):
            lines.append("   %s" % result["date"][:10])
        summary = clip(result.get("snippet") or result.get("summary"), SUMMARY_CHARS)
        if summary:
            lines.append("   " + summary)
    return "\n".join(lines) or "No results."


def mimo(config, query, count):
    reply = post(
        config["endpoint"],
        config["key"],
        {
            "model": config["model"],
            "messages": [{"role": "user", "content": "Search the web and answer with sources: " + query}],
            "tools": [{"type": "web_search", "force_search": True, "max_keyword": 3}],
            "max_tokens": 2048,
        },
    )
    message = ((reply.get("choices") or [{}])[0]).get("message") or {}
    lines = [(message.get("content") or "").strip(), "", "Sources:"]
    seen = set()
    for annotation in message.get("annotations") or []:
        url = annotation.get("url")
        if annotation.get("type") != "url_citation" or not url or url in seen:
            continue
        seen.add(url)
        date = (annotation.get("publish_time") or "")[:10]
        lines.append("- %s — %s%s" % (annotation.get("title") or url, url, " (%s)" % date if date else ""))
        if len(seen) >= count:
            break
    return "\n".join(lines).strip()


def search(arguments):
    query = str(arguments.get("query") or "").strip()
    if not query:
        raise RuntimeError("query is empty")
    count = arguments.get("count") or 8
    count = max(1, min(20, int(count))) if isinstance(count, (int, float)) else 8
    config = load_config()
    backend = {"deepseek": deepseek, "mimo": mimo}.get(config.get("backend"))
    if backend is None:
        raise RuntimeError("unknown backend in config.json")
    return backend(config, query, count)


def answer(message):
    method = message.get("method")
    params = message.get("params") or {}
    if method == "initialize":
        return {
            "protocolVersion": params.get("protocolVersion") or "2025-06-18",
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": SERVER,
        }
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": [TOOL]}
    if method == "tools/call":
        if params.get("name") != TOOL["name"]:
            raise LookupError("unknown tool")
        try:
            text, failed = search(params.get("arguments") or {}), False
        except Exception as error:  # reported to the model, never raised to the client
            text, failed = "web_search failed: %s" % error, True
        return {"content": [{"type": "text", "text": text}], "isError": failed}
    raise LookupError("unknown method")


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if "id" not in message:
            continue  # notifications need no reply
        try:
            reply = {"jsonrpc": "2.0", "id": message["id"], "result": answer(message)}
        except LookupError as error:
            reply = {"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": str(error)}}
        sys.stdout.write(json.dumps(reply, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
