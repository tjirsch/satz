#!/usr/bin/env python3
"""Drive `satz lsp` over stdio the way an editor does, and check its answers.

    lsp_client.py <satz binary> <estate .satz> <schema-less? no: run from the estate's config dir>

Speaks just enough of the Language Server Protocol: initialize, didOpen, a
completion inside a resource, a hover on an attribute, a definition on a `use`
path, a formatting request, didSave, shutdown. Every answer is asserted; the
process exit code is the verdict. Used by scripts/smoke.sh.
"""
import json
import os
import subprocess
import sys
import threading
import queue

binary, estate = sys.argv[1], os.path.abspath(sys.argv[2])
uri = "file://" + estate
text = open(estate, encoding="utf-8").read()

proc = subprocess.Popen([binary, "lsp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
inbox: "queue.Queue[dict]" = queue.Queue()


def reader():
    out = proc.stdout
    while True:
        headers = {}
        while True:
            line = out.readline()
            if not line:
                return
            line = line.decode().strip()
            if not line:
                break
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
        body = out.read(int(headers["content-length"]))
        inbox.put(json.loads(body))


threading.Thread(target=reader, daemon=True).start()
seq = 0


def send(method, params=None, request=True):
    global seq
    msg = {"jsonrpc": "2.0", "method": method, "params": params or {}}
    if request:
        seq += 1
        msg["id"] = seq
    data = json.dumps(msg).encode()
    proc.stdin.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    proc.stdin.flush()
    return seq if request else None


def wait_for(pred, what, timeout=60):
    import time
    end = time.time() + timeout
    while time.time() < end:
        try:
            msg = inbox.get(timeout=1)
        except queue.Empty:
            continue
        if pred(msg):
            return msg
    sys.exit(f"lsp: timed out waiting for {what}")


def response(id_):
    msg = wait_for(lambda m: m.get("id") == id_, f"response {id_}")
    if "error" in msg:
        sys.exit(f"lsp: request {id_} failed: {msg['error']}")
    return msg.get("result")


def line_of(needle):
    for n, l in enumerate(text.split("\n")):
        if needle in l:
            return n, l
    sys.exit(f"lsp: the estate has no line containing {needle!r}")


# initialize
r = response(send("initialize", {"processId": None, "rootUri": None, "capabilities": {}}))
caps = r["capabilities"]
for c in ("completionProvider", "hoverProvider", "definitionProvider", "documentFormattingProvider"):
    assert c in caps, f"initialize: no {c} in {sorted(caps)}"
assert r["serverInfo"]["name"] == "satz"
send("initialized", request=False)

# didOpen: no ERROR on a clean estate (parse and pipeline); what the compile warns
# about — the showcase's action, its trusted passthrough — arrives as warnings and notes
send("textDocument/didOpen", {"textDocument": {"uri": uri, "languageId": "satz", "version": 1, "text": text}}, request=False)
d = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri, "diagnostics")
errors = [x for x in d["params"]["diagnostics"] if x["severity"] == 1]
assert errors == [], f"a clean estate has errors: {errors}"
assert any("action" in x["message"] and x["severity"] == 2 for x in d["params"]["diagnostics"]), d["params"]["diagnostics"]
assert any("passthrough" in x["message"] and x["severity"] == 3 for x in d["params"]["diagnostics"]), d["params"]["diagnostics"]

# completion inside the audit bucket: the bucket's attributes come first
n, l = line_of("uniform_bucket_level_access = true")
r = response(send("textDocument/completion", {"textDocument": {"uri": uri}, "position": {"line": n, "character": len(l) - len(l.lstrip())}}))
items = r if isinstance(r, list) else r["items"]
labels = [i["label"] for i in items]
for want in ("name", "location", "lifecycle_rule", "use"):
    assert want in labels, f"completion in a bucket lacks {want}: {labels[:20]}"
first = [i["label"] for i in sorted(items, key=lambda i: i.get("sortText", i["label"]))][:40]
assert "location" in first, f"attributes are not offered first: {first}"

# hover on the attribute names its type
col = l.index("uniform_bucket_level_access") + 3
r = response(send("textDocument/hover", {"textDocument": {"uri": uri}, "position": {"line": n, "character": col}}))
assert r and "uniform_bucket_level_access" in r["contents"]["value"] and "bool" in r["contents"]["value"], f"hover: {r}"

# definition on a `use` path resolves to the pack file
n, l = line_of('use "')
col = l.index('"') + 2
r = response(send("textDocument/definition", {"textDocument": {"uri": uri}, "position": {"line": n, "character": col}}))
assert r and r["uri"].startswith("file://") and r["uri"].endswith(".satz"), f"definition: {r}"

# formatting a formatted file is no edit
r = response(send("textDocument/formatting", {"textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": True}}))
assert r == [], f"a formatted estate got edits: {r}"

# a parse error on change is a diagnostic at its line, cleared again on the fix
broken = text.replace("\nestate showcase\n", "\nestate \"showcase\n", 1)
assert broken != text, "the showcase header line moved; adjust the client"
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": broken}]}, request=False)
d = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri and m["params"]["diagnostics"], "an error diagnostic")
diag = d["params"]["diagnostics"][0]
assert diag["source"] == "satz" and diag["severity"] == 1, diag
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 3}, "contentChanges": [{"text": text}]}, request=False)
d = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri and not m["params"]["diagnostics"], "the diagnostic to clear")

# a pipeline error on save: an unknown resource type, named at its line
bad = text.replace("google_folder {", "google_folderx {", 1)
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 4}, "contentChanges": [{"text": bad}]}, request=False)
send("textDocument/didSave", {"textDocument": {"uri": uri}}, request=False)
d = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri and any(x["severity"] == 1 for x in m["params"]["diagnostics"]), "a pipeline diagnostic")
diag = next(x for x in d["params"]["diagnostics"] if x["severity"] == 1)
n, _ = line_of("google_folder {")
assert diag["range"]["start"]["line"] == n, f"pipeline error not at the type's line: {diag}"

# an emitter-stage refusal on save: the audit bucket without its location is a
# WARNING at the bucket's line naming the attribute; restored and saved, it clears
n_bucket, _ = line_of("audit_logs {")
without = text.replace('            location                    = "EU"\n', "", 1)
assert without != text, "the showcase bucket's location line moved; adjust the client"
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 5}, "contentChanges": [{"text": without}]}, request=False)
send("textDocument/didSave", {"textDocument": {"uri": uri}}, request=False)
d = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri and any("location" in x["message"] for x in m["params"]["diagnostics"]), "the missing-attribute warning")
diag = next(x for x in d["params"]["diagnostics"] if "location" in x["message"])
assert diag["severity"] == 2 and diag["range"]["start"]["line"] == n_bucket, diag
send("textDocument/didChange", {"textDocument": {"uri": uri, "version": 6}, "contentChanges": [{"text": text}]}, request=False)
send("textDocument/didSave", {"textDocument": {"uri": uri}}, request=False)
wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics" and m["params"]["uri"] == uri and not any("location" in x["message"] for x in m["params"]["diagnostics"]), "the warning to clear")

response(send("shutdown"))
send("exit", request=False)
proc.wait(timeout=10)
print("lsp: OK — initialize, diagnostics (parse, pipeline, emitter), completion, hover, definition, formatting")
