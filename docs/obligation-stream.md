# Reading a Chunked Obligation Stream

`lean-typing-proof` writes a proof obligation's certificate as one JSON object per
line. On the largest declaration in the corpus that stream is 124 GB, so the
emitter can roll it into parts under `LTP_OBLIG_CHUNK=<bytes>` instead of writing
one file no reader can open. This note is for consumers: what the files are, how
to get one record, and what changes when a run was stopped instead of finished.

The measurements behind the design are in
[lean4mm0.md](lean4mm0.md) §13.32; the emitter's own report of a stopped run is
§13.34.

## The files

With `LTP_OBLIG_CHUNK=<bytes>` unset, there is exactly one file — the historical
artifact, unchanged:

```
<stem>_obligations.jsonl
```

With it set, the same bytes are cut into parts, and two small files describe them:

```
<stem>_obligations.part-000.jsonl
<stem>_obligations.part-001.jsonl
...
<stem>_obligations.manifest.json
<stem>_obligations.index.jsonl
```

`<stem>` is the emitter's: `<outdir>/<declaration>_typing`, e.g.
`out/foo_typing`. The concatenation of the parts, in part order, is **byte for
byte** the single file — the parts are not a new encoding.

The line format is the same in every case: one object, sorted keys, with `b`
(binders), `id` (the obligation's axiom name), `kind`, `lhs`, `r`, `rhs`, `x` —
unless the stream was recorded shared (`LTP_OBLIG_SHARE=1`), which drops `lhs` and
`rhs` and numbers the terms instead: see "The shared form" below.

## Which files a consumer needs

| what you want | read |
| --- | --- |
| the whole stream, in order | the parts, in part order |
| one obligation by id | the index, then `seek` |
| how big the stream is, how many records | the manifest |
| whether a record spans parts | the manifest's `fragmented`, or the index's `segs` |

## The index

One line per obligation, in stream order:

```json
{"id": "ax_subst_0", "part": 0, "off": 0, "len": 523}
{"id": "ax_subst_1", "part": 0, "off": 523, "len": 9633, "segs": [[0, 523, 477], [1, 0, 1000], [2, 0, 1000]]}
```

* `part`, `off`, `len` locate the record's **first** segment and its total length.
  A record that fits in one part has no `segs` key and is read with a single
  `seek`+`read`.
* `segs` lists `[part, offset, length]` for each piece, in order, when a part
  boundary fell inside the record. Concatenate them to get the record.
* The index is line buffered, so a run that was killed or stopped still has an
  entry for every certificate it finished — and no entry for the one it was
  writing.

```python
import json

def record(parts, index_line):
    """The bytes of one certificate, or None if the index line is empty."""
    rec = json.loads(index_line)
    segs = rec.get("segs") or [[rec["part"], rec["off"], rec["len"]]]
    out = bytearray()
    for part, off, length in segs:
        with open(parts[part], "rb") as fh:
            fh.seek(off)
            out += fh.read(length)
    return bytes(out)
```

Reading the whole stream in order is a plain concatenation:

```bash
cat out/foo_typing_obligations.part-*.jsonl > obligations.jsonl
```

## The manifest

```json
{
 "format": "lean-typing-proof/obligations",
 "records": 24041,
 "bytes": 199715045782,
 "part_bytes_limit": 2147483648,
 "fragmented": true,
 "parts": [
  {"path": "foo_typing_obligations.part-000.jsonl", "bytes": 2147614720,
   "records": 3, "first_id": "ax_subst_0", "last_id": "ax_subst_4"}
 ]
}
```

* `records` counts certificates, `bytes` counts stream bytes. Both are the totals
  for the whole stream, so they can be checked against the parts.
* `parts[i].records` counts the certificates that **end** in that part, so a
  certificate spanning two parts is counted once, in the part it ends in.
* `parts[i].first_id` of a part that only holds the tail of a certificate is the
  id of the record it helped hold.
* `fragmented` says whether any record spans parts. If it is `false`, every part
  is independently valid JSONL.

## Bounds

A part closes when its size first crosses `part_bytes_limit`, so it can exceed
that limit by at most one write piece — at most 64 KiB, whatever the limit. A
limit of 2 GiB therefore gives parts of 2 GiB to 2 GiB + 64 KiB, even on a
declaration whose individual certificates are tens of gigabytes.

`part_bytes_limit` in a **rebuilt** manifest is the largest part the tool found,
not the limit the emitter was asked for: a killed run never wrote that down.

## Stopped and killed runs

A run stopped by `LTP_STOP_RULES` / `LTP_STOP_AT` (§13.34) closes its streams, so
it has the manifest, the index, and `<stem>_stopped.json` with the state it
reached. It has **no** `.mm0`/`.mm1`: the theory needs a proof of the goal and
the walk did not reach it.

A run killed (by `timeout`, or a signal) has neither the manifest nor the state
file, and its last part ends wherever the kill landed. Rebuild the manifest with:

```bash
scripts/oblig-manifest out/foo_typing
```

That reads the index when it exists — records, per-part ids, `fragmented` — and
otherwise scans the parts, refusing when a part ends mid-record and the record
boundaries are therefore unrecoverable. When a record's bytes are in doubt, trust
the index: it never names a record whose bytes are not all on disk.

## Limits worth knowing

* A stopped or killed run's stream is a **prefix** of the finished one, byte for
  byte. It is a valid stream, not a truncated record: everything indexed is whole.
* The axiom spool (`<stem>_axioms.tmp`) is the other large file and has no index;
  it is concatenated into `.mm0`/`.mm1` only by a run that finishes.

## The shared form (`LTP_OBLIG_SHARE=1`)

The default line embeds each term twice: once as a tree (`b`, `r`, `x`) and once as
the MM0 text of the statement (`lhs`, `rhs`). A shared line drops the texts and
numbers the trees, so a subterm reached by two paths is stored once:

```json
{"id": "ax_subst_7", "kind": "substitution",
 "n": [["bvar", 0], ["const", "Foo.bar"], ["app", 0, 1], ...],
 "b": 2, "x": 1, "r": 5}
```

* `n` is the node table, in **post-order**: every entry refers only to smaller
  indices. Tags: `bvar`, `const`, `sort`, `nat`, `str`, `app`, `pi`, `lam`,
  `tsubst`, `proj`, `let`. A `let` stands for the application it means, exactly as
  the expanded writer puts it.
* `b`, `x`, `r` are indices into it.
* **There is no `lhs` and no `rhs`.** They are not information the record lacks:
  `lhs` is `(tsubst b x)` and `rhs` is node `r`. On FockPair the two texts were
  96.5 % of the stream (199.7 MB + 84.4 MB of 294.4 MB), so dropping them takes a
  294 MB stream to 10 MB, and the ns row's 90.7 GB to about 3.2 GB.

Rendering those texts back needs one thing the table cannot carry: how the encoder
*spelled* names, since a constant becomes `(tentity t_sym7)` by first appearance in
encoding order. The emitter therefore writes a small sidecar beside the stream:

```json
{"index_style": "succ", "levels": {"0": "t_lev0"}, "symbols": {"Foo.bar": "t_sym7"}}
```

`scripts/oblig-expand` reads the sidecar automatically (same stem as the stream),
and expands a shared stream back to the expanded line format — a check that the
shared form loses nothing:

```bash
scripts/oblig-expand out/foo_typing_obligations.jsonl > expanded.jsonl
cmp expanded.jsonl out-unshared/foo_typing_obligations.jsonl
```

One `*_naming.json` describes the whole run, so a consumer that only wants `b`, `x`
and `r` never needs it at all.
